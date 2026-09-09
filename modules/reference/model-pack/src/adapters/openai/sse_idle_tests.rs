use eventsource_stream::Eventsource;
use serde_json::json;
use tokio::time::{Instant, sleep};

use super::*;

#[test]
fn config_matches_codex_default_and_rejects_invalid_milliseconds() {
    for (config, expected) in [
        (json!({}), 300_000),
        (json!({"stream_idle_timeout_ms": 0}), 0),
        (json!({"stream_idle_timeout_ms": 12_345}), 12_345),
    ] {
        let client = super::super::OpenAiResponsesClient::from_provider_config(config).unwrap();
        assert_eq!(
            client.stream_idle_timeout.0,
            Duration::from_millis(expected)
        );
    }
    for value in [
        json!(-1),
        json!(1.5),
        json!("100"),
        json!(true),
        Value::Null,
    ] {
        assert!(
            super::super::OpenAiResponsesClient::from_provider_config(
                json!({"stream_idle_timeout_ms": value})
            )
            .unwrap_err()
            .to_string()
            .contains("stream_idle_timeout_ms must be a non-negative integer")
        );
    }
}

#[tokio::test(start_paused = true)]
async fn complete_events_reset_idle_even_when_the_adapter_ignores_them() {
    let bytes = async_stream::stream! {
        for frame in ["event: unknown\ndata: {}\n\n", "data: invalid json\n\n", "data:\n\n"] {
            sleep(Duration::from_millis(80)).await;
            yield Ok::<_, std::io::Error>(frame.as_bytes());
        }
        std::future::pending::<()>().await;
    };
    let mut sse = Box::pin(bytes.eventsource());
    let timeout = SseIdleTimeout(Duration::from_millis(100));
    let mut state = super::super::OpenAiStreamState::default();
    let start = Instant::now();
    for _ in 0..3 {
        let event = timeout.next(&mut sse).await.unwrap().unwrap().unwrap();
        assert!(state.translate(&event.event, &event.data).is_empty());
    }
    assert!(start.elapsed() >= Duration::from_millis(240));
    let failure = timeout.next(&mut sse).await.unwrap_err();
    assert_eq!(failure.kind, ModelFailureKind::StreamDisconnected);
    assert_eq!(failure.message, "idle timeout waiting for SSE");
}

#[tokio::test(start_paused = true)]
async fn comments_and_incomplete_byte_fragments_do_not_reset_idle() {
    let bytes = async_stream::stream! {
        for frame in [": keep-alive\n\n", "event: response.output_text.delta\n", "data: {\"delta\":\"partial"] {
            sleep(Duration::from_millis(30)).await;
            yield Ok::<_, std::io::Error>(frame.as_bytes());
        }
        std::future::pending::<()>().await;
    };
    let mut sse = Box::pin(bytes.eventsource());
    let start = Instant::now();
    let failure = SseIdleTimeout(Duration::from_millis(100))
        .next(&mut sse)
        .await
        .unwrap_err();
    assert_eq!(failure.kind, ModelFailureKind::StreamDisconnected);
    assert!(
        start.elapsed() < Duration::from_millis(130),
        "byte arrivals must not extend the poll"
    );
}

#[tokio::test(start_paused = true)]
async fn time_between_polls_is_not_provider_idle_and_zero_does_not_disable_timeout() {
    let bytes = async_stream::stream! {
        for _ in 0..2 {
            sleep(Duration::from_millis(80)).await;
            yield Ok::<_, std::io::Error>(b"data: {}\n\n");
        }
        std::future::pending::<()>().await;
    };
    let mut sse = Box::pin(bytes.eventsource());
    let timeout = SseIdleTimeout(Duration::from_millis(100));
    timeout.next(&mut sse).await.unwrap().unwrap().unwrap();
    sleep(Duration::from_secs(1)).await;
    timeout.next(&mut sse).await.unwrap().unwrap().unwrap();
    let failure = SseIdleTimeout(Duration::ZERO)
        .next(&mut sse)
        .await
        .unwrap_err();
    assert_eq!(failure.kind, ModelFailureKind::StreamDisconnected);
}
