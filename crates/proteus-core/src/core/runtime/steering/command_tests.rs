use super::*;

#[tokio::test]
async fn queue_edit_validates_size_without_changing_order_or_budget_on_failure() {
    let queue = SessionSteering::default();
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
    assert_eq!(queue.state.lock().unwrap().queued_bytes, 11);
    assert_eq!(queue.queued_count.load(Ordering::Acquire), 2);
    let delivered = queue
        .take_for_steering(first.active_turn_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(delivered.message.id, first.message_id);
    assert_eq!(delivered.text, "short");
    assert!(queue.update_pending(first.message_id, None).is_err());
    assert!(
        queue
            .update_pending(first.message_id, Some("late".into()))
            .is_err()
    );
}
