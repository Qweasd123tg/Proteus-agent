use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

fn message(id: u64, text: &str) -> Message {
    Message {
        images: Vec::new(),
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
        write.update(|items| {
            for m in items {
                m.role = MessageRole::Assistant;
                m.version += 1;
            }
        });
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

#[test]
fn reasoning_flush_visits_only_active_rows_and_survives_replacement() {
    Owner::new().with(|| {
        let (read, write) = transcript((0..3000).map(|id| message(id, "history")).collect());
        for _ in 0..20 {
            write.finish_streaming_reasoning();
        }
        assert_eq!(
            read.data
                .with_value(|data| data.reasoning_examined.load(Ordering::Relaxed)),
            0
        );

        write.update(|items| {
            for item in items
                .iter_mut()
                .filter(|item| item.id == 5 || item.id == 2900)
            {
                item.role = MessageRole::Reasoning;
                item.streaming = true;
                item.version += 1;
            }
        });
        assert_eq!(
            read.data.with_value(|data| data.streaming_reasoning.len()),
            2
        );
        write.finish_streaming_reasoning();
        assert_eq!(
            read.data
                .with_value(|data| data.reasoning_examined.load(Ordering::Relaxed)),
            2
        );
        assert!(!read.with_untracked(|items| items[5].streaming || items[2900].streaming));
        write.finish_streaming_reasoning();
        assert_eq!(
            read.data
                .with_value(|data| data.reasoning_examined.load(Ordering::Relaxed)),
            2
        );

        let mut replacement = message(5, "new");
        replacement.role = MessageRole::Reasoning;
        replacement.streaming = true;
        write.set(vec![replacement, message(2900, "same id, no reasoning")]);
        assert_eq!(
            read.data.with_value(|data| data.streaming_reasoning.len()),
            1
        );
        write.finish_streaming_reasoning();
        assert_eq!(
            read.data
                .with_value(|data| data.reasoning_examined.load(Ordering::Relaxed)),
            3
        );
        assert!(!read.with_untracked(|items| items[0].streaming));
        write.update_matching(
            |item| item.id == 5,
            |item| {
                item.streaming = true;
                item.version += 1;
            },
        );
        write.update(|items| {
            items.remove(0);
        });
        assert_eq!(
            read.data.with_value(|data| data.streaming_reasoning.len()),
            0
        );
        write.finish_streaming_reasoning();
        assert_eq!(
            read.data
                .with_value(|data| data.reasoning_examined.load(Ordering::Relaxed)),
            3
        );
    });
}

#[tokio::test]
async fn child_heavy_tool_events_do_not_clone_cards_or_rescan_chat_and_plan() {
    use crate::types::{
        SubagentActivity, SubagentActivityStatus, ToolActivity, ToolActivityStatus,
    };
    _ = any_spawner::Executor::init_tokio();
    let owner = Owner::new();
    let projection_count = Arc::new(AtomicUsize::new(0));
    let selected_count = Arc::new(AtomicUsize::new(0));
    let order_count = Arc::new(AtomicUsize::new(0));
    let (read, write, set_activities) = owner.with(|| {
        let tools = (0..64)
            .map(|id| ToolActivity {
                call_id: format!("child-call-{id}"),
                name: "shell".into(),
                args: serde_json::json!({}),
                args_preview: String::new(),
                started_at_ms: 1,
                finished_at_ms: None,
                status: ToolActivityStatus::Running,
                result_preview: Some("x".repeat(10_000)),
            })
            .collect::<Vec<_>>();
        let mut child = message(2000, "");
        // Facade tool attached to the child must not invalidate root-tool projections.
        child.tool = Some(ToolActivity {
            call_id: "spawn".into(),
            name: "spawn_agent".into(),
            args: serde_json::json!({}),
            args_preview: String::new(),
            started_at_ms: 0,
            finished_at_ms: None,
            status: ToolActivityStatus::Done,
            result_preview: Some("child created".into()),
        });
        child.subagent = Some(SubagentActivity {
            child_thread_id: "child-thread".into(),
            role: "reviewer".into(),
            description: None,
            status: SubagentActivityStatus::Running,
            iterations: None,
            started_at_ms: 1,
            finished_at_ms: None,
            tools: tools.clone(),
        });
        let mut history = (0..2000)
            .map(|id| message(id, "history"))
            .collect::<Vec<_>>();
        history.push(child);
        let (read, write) = transcript(history);
        let (_, set_activities) = signal(tools);
        let projection_count = projection_count.clone();
        Effect::new_isomorphic(move |_| {
            read.with_tool_messages(|items| {
                items
                    .iter()
                    .filter(|item| item.tool.is_some() && item.subagent.is_none())
                    .count()
            });
            projection_count.fetch_add(1, Ordering::Relaxed);
        });
        let order_count = order_count.clone();
        Effect::new_isomorphic(move |_| {
            read.ids();
            order_count.fetch_add(1, Ordering::Relaxed);
        });
        let selected = read.select(2000, |message| {
            message
                .and_then(|m| m.subagent.as_ref())
                .map(|child| child.tools[63].status)
        });
        let selected_count = selected_count.clone();
        Effect::new_isomorphic(move |_| {
            selected.get();
            selected_count.fetch_add(1, Ordering::Relaxed);
        });
        (read, write, set_activities)
    });
    for _ in 0..100 {
        if selected_count.load(Ordering::Relaxed) == 1 {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert_eq!(projection_count.swap(0, Ordering::Relaxed), 1);
    assert_eq!(order_count.swap(0, Ordering::Relaxed), 1);
    assert_eq!(selected_count.swap(0, Ordering::Relaxed), 1);
    for event in 0..20 {
        let status = if event % 2 == 0 {
            ToolActivityStatus::Done
        } else {
            ToolActivityStatus::Running
        };
        assert!(crate::messages::update_tool_status(
            set_activities,
            write,
            "child-call-63",
            status,
            Some("complete output".into()),
            42
        ));
        tokio::task::yield_now().await;
    }
    assert_eq!(
        selected_count.load(Ordering::Relaxed),
        20,
        "selected child tool stays live"
    );
    assert_eq!(
        projection_count.load(Ordering::Relaxed),
        0,
        "child events must not rebuild grouping or plan projections"
    );
    assert_eq!(
        order_count.load(Ordering::Relaxed),
        0,
        "point updates must not rebuild transcript membership"
    );
    assert_eq!(
        read.data
            .with_value(|data| data.message_reads.load(Ordering::Relaxed)),
        0,
        "compact child selectors must not clone its growing nested history"
    );
    read.with_untracked(|items| {
        let child = items.last().unwrap();
        assert_eq!(
            child.tool.as_ref().unwrap().result_preview.as_deref(),
            Some("child created")
        );
        assert_eq!(
            child.subagent.as_ref().unwrap().tools[63]
                .result_preview
                .as_deref(),
            Some("complete output")
        );
    });
    owner.cleanup();
}

#[tokio::test]
async fn borrowed_selector_tracks_replacement_removal_and_reinsertion_without_clones() {
    _ = any_spawner::Executor::init_tokio();
    let owner = Owner::new();
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let (read, write) = owner.with(|| {
        let (read, write) = transcript(vec![message(1, "first"), message(2, "other")]);
        let seen = seen.clone();
        Effect::new_isomorphic(move |_| {
            read.with_message(1, |message| {
                seen.lock()
                    .unwrap()
                    .push(message.map(|message| message.text.clone()))
            });
        });
        (read, write)
    });
    tokio::task::yield_now().await;
    write.update_matching(|message| message.id == 2, |message| message.text.push('!'));
    tokio::task::yield_now().await;
    // A new transcript row (tool call, next answer) must not re-render settled ones.
    write.update(|items| items.push(message(3, "appended")));
    tokio::task::yield_now().await;
    assert_eq!(seen.lock().unwrap().len(), 1);
    write.set(vec![message(1, "snapshot")]);
    tokio::task::yield_now().await;
    write.set(Vec::new());
    tokio::task::yield_now().await;
    write.set(vec![message(1, "reinserted")]);
    tokio::task::yield_now().await;
    write.update_matching(|message| message.id == 1, |message| message.text.push('!'));
    tokio::task::yield_now().await;
    assert_eq!(
        seen.lock().unwrap().last().unwrap().as_deref(),
        Some("reinserted!")
    );
    assert!(seen.lock().unwrap().contains(&None));
    assert_eq!(
        read.data
            .with_value(|data| data.message_reads.load(Ordering::Relaxed)),
        0
    );
}
