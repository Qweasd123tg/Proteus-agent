//! Pending-approval control plane app-server-а.
//!
//! Каждый approval-запрос регистрируется в общей map + получает
//! watcher-таску, которая владеет responder-ом оркестратора. Watcher — это
//! единственное место, где запрос разрешается: и явный ответ клиента
//! (`resolve_pending_approval`), и timeout, и массовый deny при shutdown
//! проходят через resolve-канал записи. Если же сам запросивший умирает
//! (отмена turn-а, timeout субагента) — watcher видит `responder.closed()`,
//! убирает запись и сообщает клиентам `ApprovalResolved`. Благодаря этому
//! отмена одного turn-а не деняет чужие pending approvals.

use std::{collections::HashMap, sync::Arc, time::Duration};

use anyhow::{Result, anyhow};
use tokio::sync::{Mutex, broadcast, mpsc, oneshot};
use uuid::Uuid;

use crate::{contracts::ApprovalResponse, core::approval::PendingApproval};

use super::approval_preview::approval_preview_for;
use proteus_contracts::app_protocol::{AppApprovalId, AppApprovalRequest, AppServerEvent};

pub(super) struct PendingApprovalEntry {
    pub(super) request: AppApprovalRequest,
    /// Канал к watcher-таске; отправка ответа сюда разрешает запрос.
    resolve: oneshot::Sender<ApprovalResponse>,
}

pub(super) type PendingApprovalResponders =
    Arc<Mutex<HashMap<AppApprovalId, PendingApprovalEntry>>>;

/// Принимает approvals из `ChannelApprovalTransport`, присваивает id и
/// монотонный `seq`, прокидывает attribution (`origin`) из contract-запроса
/// и регистрирует запись с watcher-таской.
pub(super) fn spawn_approval_forwarder(
    mut approval_rx: mpsc::Receiver<PendingApproval>,
    events: broadcast::Sender<AppServerEvent>,
    pending_approvals: PendingApprovalResponders,
    approval_timeout: Duration,
) {
    tokio::spawn(async move {
        let mut next_seq: u64 = 0;
        while let Some(PendingApproval { request, responder }) = approval_rx.recv().await {
            next_seq += 1;
            let approval_id = Uuid::new_v4().to_string();
            let preview = approval_preview_for(&request.call, &request.cwd);
            let app_request = AppApprovalRequest::new(
                approval_id.clone(),
                request.call,
                request.cwd,
                request.reason,
                request.tool_spec,
            )
            .with_preview(preview)
            .with_origin(request.origin)
            .with_seq(next_seq);

            register_pending_approval(&pending_approvals, &events, app_request, responder).await;

            if !approval_timeout.is_zero() {
                spawn_approval_timeout(approval_id, pending_approvals.clone(), approval_timeout);
            }
        }
    });
}

/// Кладёт запись в map, спавнит watcher и анонсирует запрос клиентам.
/// Используется forwarder-ом и тестами.
pub(super) async fn register_pending_approval(
    pending_approvals: &PendingApprovalResponders,
    events: &broadcast::Sender<AppServerEvent>,
    app_request: AppApprovalRequest,
    responder: oneshot::Sender<ApprovalResponse>,
) {
    let approval_id = app_request.approval_id.clone();
    let (resolve_tx, resolve_rx) = oneshot::channel();
    pending_approvals.lock().await.insert(
        approval_id.clone(),
        PendingApprovalEntry {
            request: app_request.clone(),
            resolve: resolve_tx,
        },
    );
    tokio::spawn(watch_pending_approval(
        approval_id,
        responder,
        resolve_rx,
        pending_approvals.clone(),
        events.clone(),
    ));
    let _ = events.send(AppServerEvent::ApprovalRequested {
        request: Box::new(app_request),
    });
}

/// Разрешает pending approval ответом клиента. Возвращает ошибку для
/// неизвестного id и для гонки «запросивший умер во время ответа».
pub(super) async fn resolve_pending_approval(
    pending_approvals: &PendingApprovalResponders,
    approval_id: &str,
    response: ApprovalResponse,
) -> Result<()> {
    let entry = pending_approvals
        .lock()
        .await
        .remove(approval_id)
        .ok_or_else(|| anyhow!("unknown approval id: {approval_id}"))?;
    entry
        .resolve
        .send(response)
        .map_err(|_| anyhow!("approval response channel dropped"))?;
    Ok(())
}

/// Массовый deny всех pending approvals (shutdown app-server-а).
pub(super) async fn deny_pending_approvals(
    pending_approvals: PendingApprovalResponders,
    note: String,
) {
    let pending = std::mem::take(&mut *pending_approvals.lock().await);
    for (_, entry) in pending {
        let _ = entry.resolve.send(ApprovalResponse::deny(note.clone()));
    }
}

fn spawn_approval_timeout(
    approval_id: AppApprovalId,
    pending_approvals: PendingApprovalResponders,
    approval_timeout: Duration,
) {
    tokio::spawn(async move {
        tokio::time::sleep(approval_timeout).await;
        let timeout_ms = approval_timeout.as_millis() as u64;
        let _ = resolve_pending_approval(
            &pending_approvals,
            &approval_id,
            ApprovalResponse::deny(format!("approval request timed out after {timeout_ms}ms")),
        )
        .await;
    });
}

/// Владеет responder-ом оркестратора. Либо форвардит ответ из resolve-канала
/// и эмитит `ApprovalResolved`, либо — если запросивший дропнул receiver —
/// убирает осиротевшую запись из map и тоже эмитит `ApprovalResolved`.
async fn watch_pending_approval(
    approval_id: AppApprovalId,
    mut responder: oneshot::Sender<ApprovalResponse>,
    resolve_rx: oneshot::Receiver<ApprovalResponse>,
    pending_approvals: PendingApprovalResponders,
    events: broadcast::Sender<AppServerEvent>,
) {
    tokio::select! {
        biased;
        resolved = resolve_rx => {
            // Err означает, что resolve_tx дропнули без ответа; записи в map
            // уже нет, событие эмитить не о чем.
            if let Ok(response) = resolved {
                let approved = response.approved;
                let _ = responder.send(response);
                let _ = events.send(AppServerEvent::ApprovalResolved {
                    approval_id,
                    approved,
                });
            }
        }
        _ = responder.closed() => {
            let removed = pending_approvals.lock().await.remove(&approval_id).is_some();
            if removed {
                let _ = events.send(AppServerEvent::ApprovalResolved {
                    approval_id,
                    approved: false,
                });
            }
        }
    }
}
