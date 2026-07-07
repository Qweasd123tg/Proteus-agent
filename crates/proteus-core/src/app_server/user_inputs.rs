//! Pending-user-input control plane app-server-а.
//!
//! Зеркало `approvals.rs`: каждый user-input запрос регистрируется в общей
//! map + получает watcher-таску, владеющую responder-ом tool-а. Watcher —
//! единственное место разрешения запроса: явный ответ клиента
//! (`resolve_pending_user_input`), timeout и массовый resolve при shutdown
//! проходят через resolve-канал записи. Если сам запросивший умирает (отмена
//! turn-а, timeout субагента) — watcher видит `responder.closed()`, убирает
//! запись и сообщает клиентам `UserInputResolved`. Благодаря этому отмена
//! одного turn-а не резолвит чужие pending user inputs.

use std::{collections::HashMap, sync::Arc, time::Duration};

use anyhow::{Result, anyhow};
use tokio::sync::{Mutex, broadcast, mpsc, oneshot};

use crate::{
    contracts::{UserInputRequest, UserInputResponse},
    core::PendingUserInput,
};

use proteus_contracts::app_protocol::{AppServerEvent, AppUserInputRequestId};

pub(super) struct PendingUserInputEntry {
    pub(super) request: UserInputRequest,
    /// Канал к watcher-таске; отправка ответа сюда разрешает запрос.
    resolve: oneshot::Sender<UserInputResponse>,
}

pub(super) type PendingUserInputResponders =
    Arc<Mutex<HashMap<AppUserInputRequestId, PendingUserInputEntry>>>;

/// Принимает user inputs из `ChannelUserInputTransport`, присваивает
/// монотонный `seq` (attribution — `origin` — уже проставлен orchestrator-ом)
/// и регистрирует запись с watcher-таской.
pub(super) fn spawn_user_input_forwarder(
    mut user_input_rx: mpsc::Receiver<PendingUserInput>,
    events: broadcast::Sender<AppServerEvent>,
    pending_user_inputs: PendingUserInputResponders,
    timeout: Duration,
) {
    tokio::spawn(async move {
        let mut next_seq: u64 = 0;
        while let Some(PendingUserInput { request, responder }) = user_input_rx.recv().await {
            next_seq += 1;
            let request = request.with_seq(next_seq);
            let request_id = request.request_id.clone();

            register_pending_user_input(&pending_user_inputs, &events, request, responder).await;

            if !timeout.is_zero() {
                spawn_user_input_timeout(request_id, pending_user_inputs.clone(), timeout);
            }
        }
    });
}

/// Кладёт запись в map, спавнит watcher и анонсирует запрос клиентам.
/// Используется forwarder-ом и тестами.
pub(super) async fn register_pending_user_input(
    pending_user_inputs: &PendingUserInputResponders,
    events: &broadcast::Sender<AppServerEvent>,
    request: UserInputRequest,
    responder: oneshot::Sender<UserInputResponse>,
) {
    let request_id = request.request_id.clone();
    let (resolve_tx, resolve_rx) = oneshot::channel();
    pending_user_inputs.lock().await.insert(
        request_id.clone(),
        PendingUserInputEntry {
            request: request.clone(),
            resolve: resolve_tx,
        },
    );
    tokio::spawn(watch_pending_user_input(
        request_id,
        responder,
        resolve_rx,
        pending_user_inputs.clone(),
        events.clone(),
    ));
    let _ = events.send(AppServerEvent::UserInputRequested {
        request: Box::new(request),
    });
}

/// Разрешает pending user input ответом клиента. Возвращает ошибку для
/// неизвестного id и для гонки «запросивший умер во время ответа».
pub(super) async fn resolve_pending_user_input(
    pending_user_inputs: &PendingUserInputResponders,
    request_id: &str,
    response: UserInputResponse,
) -> Result<()> {
    let entry = pending_user_inputs
        .lock()
        .await
        .remove(request_id)
        .ok_or_else(|| anyhow!("unknown user input request id: {request_id}"))?;
    entry
        .resolve
        .send(response)
        .map_err(|_| anyhow!("user input response channel dropped"))?;
    Ok(())
}

/// Массовый resolve пустыми ответами (shutdown app-server-а).
pub(super) async fn resolve_pending_user_inputs_empty(
    pending_user_inputs: PendingUserInputResponders,
) {
    let pending = std::mem::take(&mut *pending_user_inputs.lock().await);
    for (_, entry) in pending {
        let _ = entry.resolve.send(UserInputResponse::empty());
    }
}

fn spawn_user_input_timeout(
    request_id: AppUserInputRequestId,
    pending_user_inputs: PendingUserInputResponders,
    timeout: Duration,
) {
    tokio::spawn(async move {
        tokio::time::sleep(timeout).await;
        let _ = resolve_pending_user_input(
            &pending_user_inputs,
            &request_id,
            UserInputResponse::empty(),
        )
        .await;
    });
}

/// Владеет responder-ом tool-а. Либо форвардит ответ из resolve-канала и
/// эмитит `UserInputResolved`, либо — если запросивший дропнул receiver —
/// убирает осиротевшую запись из map и тоже эмитит `UserInputResolved`.
async fn watch_pending_user_input(
    request_id: AppUserInputRequestId,
    mut responder: oneshot::Sender<UserInputResponse>,
    resolve_rx: oneshot::Receiver<UserInputResponse>,
    pending_user_inputs: PendingUserInputResponders,
    events: broadcast::Sender<AppServerEvent>,
) {
    tokio::select! {
        biased;
        resolved = resolve_rx => {
            // Err означает, что resolve_tx дропнули без ответа; записи в map
            // уже нет, событие эмитить не о чем.
            if let Ok(response) = resolved {
                let _ = responder.send(response);
                let _ = events.send(AppServerEvent::UserInputResolved { request_id });
            }
        }
        _ = responder.closed() => {
            let removed = pending_user_inputs.lock().await.remove(&request_id).is_some();
            if removed {
                let _ = events.send(AppServerEvent::UserInputResolved { request_id });
            }
        }
    }
}
