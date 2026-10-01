//! Cancellation after durable partial progress stops the workflow retry backoff.
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use proteus_contracts::{contracts::CancellationToken, model_standard::ModelFailureKind};
use proteus_core::core::{
    AgentRuntime, JournalEntry, ModelResponseOutcome, SessionStore, TurnSettlementStatus,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::Mutex,
};

use super::{
    ApprovingTransport, CALL_ID, EFFECT, EFFECT_FILE, PROMPT, UNFINISHED, configure,
    partial_sse_body, read_json_request, write_sse,
};

const CANCELED_PROGRESS: &str = "Этот завершённый фрагмент сохранён перед отменой.";

async fn serve(
    listener: TcpListener,
    request_count: Arc<AtomicUsize>,
    requests: Arc<Mutex<Vec<serde_json::Value>>>,
    keep_open: bool,
) {
    loop {
        let Ok((mut socket, _)) = listener.accept().await else {
            return;
        };
        let index = request_count.fetch_add(1, Ordering::SeqCst);
        requests
            .lock()
            .await
            .push(read_json_request(&mut socket).await);
        let body = format!(
            "{}{}",
            super::tool_progress::completed_tool_sse(),
            partial_sse_body(&format!("canceled_progress_{index}"), CANCELED_PROGRESS)
        );
        if keep_open {
            socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n").await.unwrap();
            socket.write_all(body.as_bytes()).await.unwrap();
            let mut byte = [0u8; 1];
            // The pump's cancellation must drop the provider connection.
            assert_eq!(socket.read(&mut byte).await.unwrap(), 0);
            return;
        }
        write_sse(&mut socket, &body).await;
    }
}

async fn check(keep_open: bool) {
    let root = tempfile::tempdir().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let config = configure(
        root.path(),
        &format!("http://{}", listener.local_addr().unwrap()),
        5,
    )
    .await;
    let config_path = root.path().join("config.json");
    std::fs::write(&config_path, serde_json::to_vec(&config).unwrap()).unwrap();
    let request_count = Arc::new(AtomicUsize::new(0));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let server = tokio::spawn(serve(
        listener,
        request_count.clone(),
        requests.clone(),
        keep_open,
    ));
    let runtime = Arc::new(
        AgentRuntime::builder(config.clone(), root.path().join("workspace"))
            .with_config_path(Some(&config_path))
            .with_approval(Arc::new(ApprovingTransport))
            .build_async()
            .await
            .unwrap(),
    );
    let session_dir = runtime.session_dir().unwrap().to_owned();
    let cancel = CancellationToken::new();
    let run = tokio::spawn({
        let runtime = runtime.clone();
        let cancel = cancel.clone();
        async move {
            runtime
                .run_with_cancellation(PROMPT.to_owned(), cancel)
                .await
        }
    });

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        // Directory creation precedes the atomic identity metadata write.
        let projection = if session_dir.join("session.json").is_file() {
            SessionStore::open(session_dir.clone())
                .unwrap()
                .load_projection()
                .unwrap()
        } else {
            assert!(
                Instant::now() < deadline,
                "session directory was not created"
            );
            tokio::time::sleep(Duration::from_millis(2)).await;
            continue;
        };
        let progress_is_durable = projection
            .history
            .iter()
            .any(|message| message.display_text() == CANCELED_PROGRESS);
        let stream_error_is_durable = projection.records.iter().any(|record| {
            matches!(
                &record.entry,
                JournalEntry::ModelResponseRecorded(response)
                    if matches!(response.outcome, ModelResponseOutcome::Error {
                        ref failure
                    } if failure.kind == ModelFailureKind::StreamDisconnected)
            )
        });
        let tool_result_is_durable = projection.records.iter().any(|record| {
            matches!(&record.entry, JournalEntry::ToolResultRecorded(tool) if tool.result.call_id == CALL_ID)
        });
        if progress_is_durable && (keep_open || stream_error_is_durable) && tool_result_is_durable {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "partial checkpoint was not recorded"
        );
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    if keep_open {
        let projection = SessionStore::open(session_dir.clone())
            .unwrap()
            .load_projection()
            .unwrap();
        assert!(
            !projection
                .records
                .iter()
                .any(|record| matches!(&record.entry, JournalEntry::ModelResponseRecorded(_)))
        );
    }
    cancel.cancel();
    let result = tokio::time::timeout(Duration::from_secs(5), run)
        .await
        .unwrap()
        .unwrap();
    assert!(result.is_err());
    tokio::time::sleep(Duration::from_millis(350)).await;
    assert_eq!(
        request_count.load(Ordering::SeqCst),
        1,
        "cancellation during retry backoff must prevent the next request"
    );
    if keep_open {
        tokio::time::timeout(Duration::from_secs(3), server)
            .await
            .unwrap()
            .unwrap();
    } else {
        server.abort();
    }
    assert_eq!(requests.lock().await.len(), 1);
    assert_eq!(
        std::fs::read_to_string(root.path().join("workspace").join(EFFECT_FILE)).unwrap(),
        EFFECT
    );

    let projection = SessionStore::open(session_dir.clone())
        .unwrap()
        .load_projection()
        .unwrap();
    assert_eq!(
        projection
            .records
            .iter()
            .filter_map(|record| match &record.entry {
                JournalEntry::TurnSettled(settled) => Some(settled.status),
                _ => None,
            })
            .collect::<Vec<_>>(),
        [TurnSettlementStatus::Canceled]
    );
    assert!(
        projection
            .history
            .iter()
            .any(|message| message.display_text() == CANCELED_PROGRESS)
    );
    assert!(
        projection
            .history
            .iter()
            .all(|message| !message.display_text().contains(UNFINISHED))
    );

    let cold = proteus_core::app_server::AgentAppServer::launch_resumed(
        config,
        root.path().join("workspace"),
        Some(&config_path),
        session_dir,
    )
    .await
    .unwrap();
    let transcript = cold.transcript().await.unwrap();
    assert_eq!(
        transcript
            .iter()
            .filter_map(|item| item.tool.as_ref())
            .filter(|tool| tool.call_id == CALL_ID && tool.status == "done")
            .count(),
        1
    );
    assert_eq!(
        transcript
            .iter()
            .filter(|item| item.text == CANCELED_PROGRESS)
            .count(),
        1
    );
    assert!(
        transcript
            .iter()
            .all(|item| !item.text.contains(UNFINISHED))
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancel_after_partial_checkpoint_stops_next_stream_attempt() {
    check(false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancel_while_stream_open_preserves_early_tool_and_closes_provider() {
    check(true).await;
}
