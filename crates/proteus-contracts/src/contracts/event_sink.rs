use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use tokio::sync::Mutex;

use crate::domain::{Event, EventContext, EventEnvelope};

#[async_trait]
pub trait EventSink: Send + Sync {
    async fn append(&self, envelope: EventEnvelope) -> Result<()>;
}

pub struct EventEmitter {
    sink: Arc<dyn EventSink>,
    seq: Mutex<u64>,
}

impl EventEmitter {
    pub fn new(sink: Arc<dyn EventSink>) -> Self {
        Self {
            sink,
            seq: Mutex::new(0),
        }
    }

    pub async fn emit(&self, context: EventContext, event: Event) -> Result<()> {
        let mut seq = self.seq.lock().await;
        *seq += 1;
        let envelope = EventEnvelope::new(context, *seq, event);
        self.sink.append(envelope).await
    }
}

/// Оборачивает `EventSink` и пропускает к нему только envelope'ы, для
/// которых predicate возвращает `true`. Используется, чтобы отбрасывать
/// delta-события на пути к durable `JsonlEventStore`, оставляя их
/// доступными для broadcast-подписчиков (UI).
pub struct FilteredEventSink {
    inner: Arc<dyn EventSink>,
    predicate: Box<dyn Fn(&Event) -> bool + Send + Sync>,
}

impl FilteredEventSink {
    pub fn new(
        inner: Arc<dyn EventSink>,
        predicate: impl Fn(&Event) -> bool + Send + Sync + 'static,
    ) -> Self {
        Self {
            inner,
            predicate: Box::new(predicate),
        }
    }
}

#[async_trait]
impl EventSink for FilteredEventSink {
    async fn append(&self, envelope: EventEnvelope) -> Result<()> {
        if !(self.predicate)(&envelope.event) {
            return Ok(());
        }
        self.inner.append(envelope).await
    }
}

/// Возвращает `true` если событие — частичный поток (delta), который
/// не следует persist'ить в durable log по умолчанию.
pub fn is_streaming_delta(event: &Event) -> bool {
    matches!(
        event,
        Event::AssistantTextDelta { .. }
            | Event::AssistantToolArgsDelta { .. }
            | Event::AssistantReasoningDelta { .. }
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{EventContext, new_session_id, new_thread_id, new_turn_id};
    use crate::model_standard::FinishReason;
    use tokio::sync::Mutex as AsyncMutex;

    #[derive(Default)]
    struct CollectingSink {
        events: AsyncMutex<Vec<EventEnvelope>>,
    }

    #[async_trait]
    impl EventSink for CollectingSink {
        async fn append(&self, envelope: EventEnvelope) -> Result<()> {
            self.events.lock().await.push(envelope);
            Ok(())
        }
    }

    fn ctx() -> EventContext {
        EventContext::new(new_session_id(), new_thread_id(), Some(new_turn_id()))
    }

    #[tokio::test]
    async fn filtered_sink_drops_deltas() {
        let inner = Arc::new(CollectingSink::default());
        let filtered = FilteredEventSink::new(inner.clone(), |e| !is_streaming_delta(e));

        filtered
            .append(EventEnvelope::new(
                ctx(),
                1,
                Event::AssistantTextDelta {
                    text: "hi".to_owned(),
                },
            ))
            .await
            .unwrap();
        filtered
            .append(EventEnvelope::new(
                ctx(),
                2,
                Event::ModelResponseReceived {
                    finish_reason: FinishReason::Stop,
                },
            ))
            .await
            .unwrap();

        let captured = inner.events.lock().await;
        assert_eq!(captured.len(), 1, "delta dropped, response kept");
        assert!(matches!(
            captured[0].event,
            Event::ModelResponseReceived { .. }
        ));
    }

    #[tokio::test]
    async fn filtered_sink_forwards_when_predicate_true() {
        let inner = Arc::new(CollectingSink::default());
        let filtered = FilteredEventSink::new(inner.clone(), |_| true);
        filtered
            .append(EventEnvelope::new(
                ctx(),
                1,
                Event::AssistantTextDelta {
                    text: "hi".to_owned(),
                },
            ))
            .await
            .unwrap();
        assert_eq!(inner.events.lock().await.len(), 1);
    }

    #[test]
    fn is_streaming_delta_covers_all_delta_variants() {
        assert!(is_streaming_delta(&Event::AssistantTextDelta {
            text: "x".to_owned()
        }));
        assert!(is_streaming_delta(&Event::AssistantToolArgsDelta {
            call_id: "call-1".to_owned(),
            args_delta: "{".to_owned()
        }));
        assert!(is_streaming_delta(&Event::AssistantReasoningDelta {
            text: "x".to_owned()
        }));
        assert!(!is_streaming_delta(&Event::ModelResponseReceived {
            finish_reason: FinishReason::Stop
        }));
    }
}
