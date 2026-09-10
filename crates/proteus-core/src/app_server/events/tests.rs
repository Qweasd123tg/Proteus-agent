use super::*;
use crate::{
    contracts::EventSink,
    domain::{Event, EventContext, EventEnvelope, new_message_id, new_thread_id, new_turn_id},
};

#[tokio::test]
async fn subscription_snapshot_covers_buffered_events_and_recovers_after_overflow() {
    let (events, _) = test_event_channel(2);
    let sink = RuntimeEventSink::default();
    assert!(sink.0.set(events.clone()).is_ok());
    let context = EventContext::new(
        events.pending_snapshot().session_id,
        new_thread_id(),
        Some(new_turn_id()),
    );
    let message_id = new_message_id();
    let mut subscription = events.subscribe_session();
    sink.append(EventEnvelope::new(
        context.clone(),
        1,
        Event::TurnStarted {
            session_id: context.session_id,
            thread_id: context.thread_id,
            turn_id: context.turn_id.unwrap(),
        },
    ))
    .await
    .unwrap();
    for seq in 2..12 {
        sink.append(EventEnvelope::new(
            context.clone(),
            seq,
            Event::AssistantTextDelta {
                message_id,
                phase: None,
                text: "x".into(),
                offset: (seq - 2) as usize,
            },
        ))
        .await
        .unwrap();
    }
    assert!(matches!(
        subscription.recv().await.unwrap(),
        AppServerEvent::PendingRequestsUpdated { .. }
    ));
    let AppServerEvent::SessionSnapshot { snapshot } = subscription.recv().await.unwrap() else {
        panic!("baseline")
    };
    assert_eq!(snapshot.transcript[0].text, "xxxxxxxxxx");
    assert_eq!(snapshot.seq, 11);
    assert_eq!(snapshot.root_thread_id, Some(context.thread_id));
    // The ring overflowed before sampling. Its retained deltas are already in
    // the baseline and must never be applied to its text a second time.
    let AppServerEvent::EventStreamLagged { .. } = subscription.recv().await.unwrap() else {
        panic!("lag")
    };
    assert!(matches!(
        subscription.recv().await.unwrap(),
        AppServerEvent::SessionSnapshot { .. }
    ));
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(20), subscription.recv())
            .await
            .is_err()
    );
    sink.append(EventEnvelope::new(
        context.clone(),
        12,
        Event::AssistantTextDelta {
            message_id,
            phase: None,
            text: "y".into(),
            offset: 10,
        },
    ))
    .await
    .unwrap();
    let AppServerEvent::Runtime { envelope } = subscription.recv().await.unwrap() else {
        panic!("new delta")
    };
    assert!(matches!(envelope.event, Event::AssistantTextDelta { ref text, .. } if text == "y"));
    // The authoritative view has no lossy forwarder ahead of it.
    assert_eq!(
        events.session_snapshot().unwrap().transcript[0].text,
        "xxxxxxxxxxy"
    );
}
