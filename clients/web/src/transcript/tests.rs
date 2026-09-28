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
    let projection_count = Arc::new(AtomicUsize::new(0));
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
        let projection_count = projection_count.clone();
        Effect::new_isomorphic(move |_| {
            read.with_user_messages(|_| ());
            read.with_tool_messages(|_| ());
            projection_count.fetch_add(1, Ordering::Relaxed);
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
    assert_eq!(projection_count.swap(0, Ordering::Relaxed), 1);
    assert_eq!(
        read.data
            .with_value(|data| data.message_reads.swap(0, Ordering::Relaxed)),
        2000
    );
    let probes = std::cell::Cell::new(0);
    for _ in 0..20 {
        assert!(write.update_matching(
            |m| {
                probes.set(probes.get() + 1);
                m.id == 1999
            },
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
    assert_eq!(
        projection_count.load(Ordering::Relaxed),
        0,
        "streaming must not rescan navigation or plan projections"
    );
    assert_eq!(
        probes.get(),
        20,
        "tail streaming lookup must not scan old messages"
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

#[test]
fn user_projection_tracks_edits_replacement_and_role_changes() {
    Owner::new().with(|| {
        let mut user = message(1, "first");
        user.role = MessageRole::User;
        let (read, write) = transcript(vec![user]);
        let users = Memo::new(move |_| {
            read.with_user_messages(|items| {
                items
                    .iter()
                    .filter(|m| m.role == MessageRole::User)
                    .map(|m| m.text.clone())
                    .collect::<Vec<_>>()
            })
        });
        assert_eq!(users.get(), ["first"]);
        write.update_matching(
            |m| m.id == 1,
            |m| {
                m.text = "edited".into();
                m.version += 1;
            },
        );
        assert_eq!(users.get(), ["edited"]);
        let mut replacement = message(1, "other!");
        replacement.role = MessageRole::User;
        replacement.version = 1;
        write.set(vec![replacement]);
        assert_eq!(users.get(), ["other!"]);
        write.update_where(
            |_| true,
            |m| {
                m.role = MessageRole::Assistant;
                m.version += 1;
            },
        );
        assert!(users.get().is_empty());
        write.update(|items| {
            items[0].role = MessageRole::User;
            items[0].version += 1;
        });
        assert_eq!(users.get(), ["other!"]);
        write.set(vec![]);
        assert!(users.get().is_empty());
    });
}
