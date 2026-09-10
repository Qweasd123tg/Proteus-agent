use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

fn message(id: u64, text: &str) -> Message {
    Message {
        id,
        message_id: Some(format!("item-{id}")),
        version: 0,
        phase: None,
        text_offset: 0,
        role: MessageRole::Assistant,
        text: text.to_owned(),
        tool: None,
        subagent: None,
        streaming: false,
    }
}

#[tokio::test]
async fn streaming_updates_one_card_without_rebuilding_history_order() {
    _ = any_spawner::Executor::init_tokio();
    let owner = Owner::new();
    let count = Arc::new(AtomicUsize::new(0));
    let order_count = Arc::new(AtomicUsize::new(0));
    let (read, write) = owner.with(|| {
        let (read, write) = transcript((0..2000).map(|id| message(id, "history")).collect());
        for id in 0..2000 {
            let item = read.message(id);
            let count = count.clone();
            Effect::new_isomorphic(move |_| {
                item.track();
                let _ = item.get();
                count.fetch_add(1, Ordering::Relaxed);
            });
        }
        let order_count = order_count.clone();
        Effect::new_isomorphic(move |_| {
            let _ = read.ids();
            order_count.fetch_add(1, Ordering::Relaxed);
        });
        (read, write)
    });
    for _ in 0..100 {
        if count.load(Ordering::Relaxed) == 2000 {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert_eq!(count.swap(0, Ordering::Relaxed), 2000);
    assert_eq!(order_count.swap(0, Ordering::Relaxed), 1);
    assert_eq!(
        read.data
            .with_value(|data| data.message_reads.swap(0, Ordering::Relaxed)),
        2000
    );
    for _ in 0..20 {
        assert!(write.update_matching(
            |m| m.id == 1999,
            |m| {
                m.text.push('!');
                m.version += 1;
            }
        ));
        tokio::task::yield_now().await;
    }
    assert_eq!(
        count.load(Ordering::Relaxed),
        20,
        "only the changed card should run"
    );
    assert_eq!(
        order_count.load(Ordering::Relaxed),
        0,
        "streaming must not rebuild list keys"
    );
    assert_eq!(
        read.data
            .with_value(|data| data.message_reads.load(Ordering::Relaxed)),
        20,
        "unchanged message selectors must not run, even if their output would compare equal"
    );
    assert_eq!(read.len(), 2000);
    owner.cleanup();
}

#[test]
fn history_replacement_reorder_and_removal_keep_subscriptions_correct() {
    Owner::new().with(|| {
        let (read, write) = transcript(vec![message(1, "first"), message(2, "second")]);
        let first = read.message(1);
        let second = read.message(2);
        assert_eq!(first.get().unwrap().text, "first");
        assert_eq!(second.get().unwrap().text, "second");
        // Same id, version and text length, as happens after loading a different snapshot.
        write.set(vec![message(2, "second"), message(1, "other")]);
        assert_eq!(read.ids(), [2, 1]);
        assert_eq!(first.get().unwrap().text, "other");
        write.update(|items| items.retain(|item| item.id != 1));
        assert!(first.get().is_none());
        assert_eq!(second.get().unwrap().text, "second");
        write.set(Vec::new());
        assert!(second.get().is_none());
        assert_eq!(read.len(), 0);
    });
}
