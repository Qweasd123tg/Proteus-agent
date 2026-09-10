use super::*;

#[tokio::test]
async fn queue_edit_validates_size_without_changing_order_or_budget_on_failure() {
    let queue = SessionSteering::default();
    let snapshots = queue.subscribe_queue();
    assert_eq!(snapshots.borrow().revision, 0);
    queue.reserve("initial".into()).await.unwrap();
    let receipt = |reservation| match reservation {
        UserMessageReservation::Queued(receipt) => receipt,
        _ => panic!("expected queued"),
    };
    let first = receipt(queue.reserve("a".repeat(MAX_MESSAGE_BYTES)).await.unwrap());
    let second = receipt(
        queue
            .reserve("b".repeat(MAX_MESSAGE_BYTES - 1))
            .await
            .unwrap(),
    );
    let third = receipt(queue.reserve("c".into()).await.unwrap());
    assert!(
        queue
            .update_pending(third.message_id, Some("cc".into()))
            .is_err()
    );
    assert!(
        queue
            .update_pending(first.message_id, Some(" ".into()))
            .is_err()
    );
    assert_eq!(
        snapshots.borrow().revision,
        3,
        "failed edits do not publish changes"
    );
    queue
        .update_pending(first.message_id, Some("short".into()))
        .unwrap();
    queue
        .update_pending(third.message_id, Some("longer".into()))
        .unwrap();
    queue.update_pending(second.message_id, None).unwrap();
    assert_eq!(
        queue.queued_messages().await,
        vec![
            (first.message_id, "short".into()),
            (third.message_id, "longer".into())
        ]
    );
    assert_eq!(snapshots.borrow().revision, 6);
    assert_eq!(snapshots.borrow().messages, queue.queued_messages().await);
    assert_eq!(queue.state.lock().unwrap().queued_bytes, 11);
    assert_eq!(queue.queued_count.load(Ordering::Acquire), 2);
    let delivered = queue
        .take_for_steering(first.active_turn_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(delivered.message.id, first.message_id);
    assert_eq!(delivered.text, "short");
    assert_eq!(snapshots.borrow().revision, 7);
    assert_eq!(
        snapshots.borrow().messages,
        vec![(third.message_id, "longer".into())]
    );
    assert!(queue.update_pending(first.message_id, None).is_err());
    assert!(
        queue
            .update_pending(first.message_id, Some("late".into()))
            .is_err()
    );
}

#[tokio::test]
async fn queue_watch_tracks_followup_and_drop_cleanup_without_runtime_events() {
    let queue = Arc::new(SessionSteering::default());
    let initial = match queue.reserve("initial".into()).await.unwrap() {
        UserMessageReservation::Start(initial) => initial,
        _ => unreachable!(),
    };
    queue.reserve("follow-up".into()).await.unwrap();
    queue.reserve("canceled".into()).await.unwrap();
    // Watch retains the current value even when nobody was subscribed.
    let snapshots = queue.subscribe_queue();
    assert_eq!(snapshots.borrow().messages.len(), 2);
    let revision = snapshots.borrow().revision;
    let RootTurnSettlement::FollowUp(next) = queue
        .settle_and_take_followup(initial.turn_id)
        .await
        .unwrap()
    else {
        panic!("follow-up expected");
    };
    assert_eq!(next.text, "follow-up");
    assert_eq!(snapshots.borrow().messages.len(), 1);
    assert!(snapshots.borrow().revision > revision);
    let revision = snapshots.borrow().revision;
    drop(queue.run_guard());
    assert!(snapshots.borrow().messages.is_empty());
    assert!(snapshots.borrow().revision > revision);
}
