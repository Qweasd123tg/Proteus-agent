//! Pending-approval control plane app-server-а.
//!
//! Map хранит только каналы ответа. Публичный pending snapshot принадлежит
//! общей projection в `events.rs`. Регистрация и удаление публикуются под
//! lock map до передачи управления watcher-у, поэтому Requested не может
//! прийти после Resolved. Watcher владеет runtime responder-ом и удаляет
//! запрос при закрытии requester-а. Закрытие UI не закрывает requester.

use std::{collections::HashMap, sync::Arc, time::Duration};

use super::events::AppEventPublisher;
use anyhow::{Result, anyhow};
use tokio::sync::{Mutex, mpsc, oneshot};
use uuid::Uuid;

use crate::{contracts::ApprovalResponse, core::PendingApproval};

use super::approval_preview::approval_preview_for;
use proteus_contracts::app_protocol::{AppApprovalId, AppApprovalRequest, AppServerEvent};

pub(super) struct PendingApprovalEntry {
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
    events: AppEventPublisher,
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
                spawn_approval_timeout(
                    approval_id,
                    pending_approvals.clone(),
                    events.clone(),
                    approval_timeout,
                );
            }
        }
    });
}

/// Регистрирует и публикует запрос, затем запускает watcher.
/// Используется forwarder-ом и тестами.
pub(super) async fn register_pending_approval(
    pending_approvals: &PendingApprovalResponders,
    events: &AppEventPublisher,
    app_request: AppApprovalRequest,
    responder: oneshot::Sender<ApprovalResponse>,
) {
    let approval_id = app_request.approval_id.clone();
    let (resolve_tx, resolve_rx) = oneshot::channel();
    let mut pending = pending_approvals.lock().await;
    pending.insert(
        approval_id.clone(),
        PendingApprovalEntry {
            resolve: resolve_tx,
        },
    );
    let _ = events.send(AppServerEvent::ApprovalRequested {
        request: Box::new(app_request),
    });
    drop(pending);
    tokio::spawn(watch_pending_approval(
        approval_id,
        responder,
        resolve_rx,
        pending_approvals.clone(),
        events.clone(),
    ));
}

/// Разрешает pending approval ответом клиента. Возвращает ошибку для
/// неизвестного id и для гонки «запросивший умер во время ответа».
pub(super) async fn resolve_pending_approval(
    pending_approvals: &PendingApprovalResponders,
    events: &AppEventPublisher,
    approval_id: &str,
    response: ApprovalResponse,
) -> Result<()> {
    let mut pending = pending_approvals.lock().await;
    let entry = pending
        .remove(approval_id)
        .ok_or_else(|| anyhow!("unknown approval id: {approval_id}"))?;
    let _ = events.send(AppServerEvent::ApprovalResolved {
        approval_id: approval_id.to_owned(),
        approved: response.approved,
    });
    drop(pending);
    entry
        .resolve
        .send(response)
        .map_err(|_| anyhow!("approval response channel dropped"))?;
    Ok(())
}

/// Массовый deny всех pending approvals (shutdown app-server-а).
pub(super) async fn deny_pending_approvals(
    pending_approvals: PendingApprovalResponders,
    events: &AppEventPublisher,
    note: String,
) {
    let mut guard = pending_approvals.lock().await;
    let pending = std::mem::take(&mut *guard);
    for (approval_id, entry) in pending {
        let _ = events.send(AppServerEvent::ApprovalResolved {
            approval_id,
            approved: false,
        });
        let _ = entry.resolve.send(ApprovalResponse::deny(note.clone()));
    }
}

fn spawn_approval_timeout(
    approval_id: AppApprovalId,
    pending_approvals: PendingApprovalResponders,
    events: AppEventPublisher,
    approval_timeout: Duration,
) {
    tokio::spawn(async move {
        tokio::time::sleep(approval_timeout).await;
        let timeout_ms = approval_timeout.as_millis() as u64;
        let _ = resolve_pending_approval(
            &pending_approvals,
            &events,
            &approval_id,
            ApprovalResponse::deny(format!("approval request timed out after {timeout_ms}ms")),
        )
        .await;
    });
}

/// Владеет responder-ом оркестратора. Форвардит уже опубликованное решение
/// либо удаляет осиротевший запрос и публикует отказ при закрытии requester-а.
async fn watch_pending_approval(
    approval_id: AppApprovalId,
    mut responder: oneshot::Sender<ApprovalResponse>,
    resolve_rx: oneshot::Receiver<ApprovalResponse>,
    pending_approvals: PendingApprovalResponders,
    events: AppEventPublisher,
) {
    tokio::select! {
        biased;
        resolved = resolve_rx => {
            // Err означает, что resolve_tx дропнули без ответа; записи в map
            // уже нет, событие эмитить не о чем.
            if let Ok(response) = resolved {
                let _ = responder.send(response);
            }
        }

        _ = responder.closed() => {
            let mut pending = pending_approvals.lock().await;
            let removed = pending.remove(&approval_id).is_some();
            if removed {
                let _ = events.send(AppServerEvent::ApprovalResolved {
                    approval_id,
                    approved: false,
                });
            }
        }
    }
}
