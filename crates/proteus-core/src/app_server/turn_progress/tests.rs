use serde_json::json;

use super::*;
use crate::domain::{
    EventContext, ThreadId, ToolResult, new_session_id, new_thread_id, new_turn_id,
};

fn envelope(thread_id: ThreadId, event: Event) -> EventEnvelope {
    EventEnvelope::new(
        EventContext::new(new_session_id(), thread_id, Some(new_turn_id())),
        1,
        event,
    )
}

fn root_thread_id() -> ThreadId {
    ThreadId::parse_str("00000000-0000-0000-0000-000000000001").expect("root thread id")
}

fn child_thread_id() -> ThreadId {
    ThreadId::parse_str("00000000-0000-0000-0000-000000000002").expect("child thread id")
}

fn apply(progress: &mut TurnProgress, event: Event) {
    progress.apply(&envelope(root_thread_id(), event));
}

fn delta_for(message_id: u128, text: &str) -> Event {
    Event::AssistantTextDelta {
        offset: 0,
        message_id: uuid::Uuid::from_u128(message_id),
        phase: None,
        text: text.to_owned(),
    }
}

fn delta(text: &str) -> Event {
    delta_for(1, text)
}

#[test]
fn accumulates_text_segments_around_tool_calls() {
    let mut progress = TurnProgress::default();
    apply(&mut progress, delta("Сначала "));
    apply(&mut progress, delta("посмотрю файл."));
    apply(
        &mut progress,
        Event::ToolCallRequested {
            call: ToolCall::new("call-1", "read_file", json!({ "path": "src/lib.rs" })),
        },
    );
    apply(
        &mut progress,
        Event::ToolFinished {
            result: ToolResult::ok("call-1".to_owned(), "contents"),
        },
    );
    apply(&mut progress, delta_for(2, "Теперь answer."));

    let snapshot = progress.snapshot();
    assert_eq!(snapshot.len(), 3);
    assert_eq!(snapshot[0].text, "Сначала посмотрю файл.");
    assert!(!snapshot[0].streaming);
    let tool = snapshot[1].tool.as_ref().expect("tool entry");
    assert_eq!(tool.status, "done");
    assert_eq!(tool.result.as_deref(), Some("contents"));
    // Последний текстовый сегмент — живой, клиент достримит в него.
    assert_eq!(snapshot[2].text, "Теперь answer.");
    assert!(snapshot[2].streaming);
}

#[test]
fn trailing_tool_is_not_marked_streaming() {
    let mut progress = TurnProgress::default();
    apply(&mut progress, delta("Запускаю."));
    apply(
        &mut progress,
        Event::ToolCallRequested {
            call: ToolCall::new("call-1", "shell", json!({})),
        },
    );

    let snapshot = progress.snapshot();
    assert_eq!(snapshot.len(), 2);
    assert!(!snapshot[1].streaming);
    assert_eq!(
        snapshot[1].tool.as_ref().map(|tool| tool.status.as_str()),
        Some("running")
    );
}

#[test]
fn child_thread_text_deltas_do_not_pollute_parent_progress() {
    let mut progress = TurnProgress::default();
    apply(
        &mut progress,
        Event::TurnStarted {
            session_id: new_session_id(),
            thread_id: root_thread_id(),
            turn_id: new_turn_id(),
        },
    );
    apply(&mut progress, delta("родительский текст"));
    // Стрим дочернего цикла под child thread не должен доклеиваться к
    // родительскому сегменту (и не должен создавать свой).
    progress.apply(&envelope(
        child_thread_id(),
        Event::AssistantTextDelta {
            offset: 0,
            message_id: proteus_contracts::domain::new_message_id(),
            phase: None,
            text: "детский стрим".to_owned(),
        },
    ));

    let snapshot = progress.snapshot();
    assert_eq!(snapshot.len(), 1);
    assert_eq!(snapshot[0].text, "родительский текст");
}

#[test]
fn turn_boundaries_clear_progress() {
    let mut progress = TurnProgress::default();
    apply(&mut progress, delta("старый ход"));
    apply(
        &mut progress,
        Event::TurnStarted {
            session_id: new_session_id(),
            thread_id: new_thread_id(),
            turn_id: new_turn_id(),
        },
    );
    assert!(progress.snapshot().is_empty());

    apply(&mut progress, delta("новый ход"));
    apply(
        &mut progress,
        Event::Error {
            message: "boom".to_owned(),
        },
    );
    assert_eq!(
        progress.snapshot()[0].text,
        "новый ход",
        "runtime error alone is not durable settlement"
    );
    progress.finish_parent_turn();
    assert!(progress.snapshot().is_empty());
}

#[test]
fn snapshots_subagent_card_and_nested_child_tools() {
    let mut progress = TurnProgress::default();
    let child_thread_id = child_thread_id();

    apply(
        &mut progress,
        Event::SubagentStarted {
            role: "reviewer".to_owned(),
            description: Some("check patch".to_owned()),
            child_thread_id,
        },
    );
    progress.apply(&envelope(
        child_thread_id,
        Event::ToolCallRequested {
            call: ToolCall::new("call-child", "read_file", json!({ "path": "src/lib.rs" })),
        },
    ));
    progress.apply(&envelope(
        child_thread_id,
        Event::ToolFinished {
            result: ToolResult::ok("call-child".to_owned(), "contents"),
        },
    ));
    apply(
        &mut progress,
        Event::SubagentFinished {
            role: "reviewer".to_owned(),
            status: "completed".to_owned(),
            iterations: 2,
            child_thread_id,
        },
    );

    let snapshot = progress.snapshot();
    assert_eq!(snapshot.len(), 1);
    assert!(!snapshot[0].streaming);
    assert!(snapshot[0].tool.is_none());
    let subagent = snapshot[0].subagent.as_ref().expect("subagent");
    assert_eq!(subagent.role, "reviewer");
    assert_eq!(subagent.description.as_deref(), Some("check patch"));
    assert_eq!(subagent.child_thread_id, child_thread_id.to_string());
    assert_eq!(subagent.status, "completed");
    assert_eq!(subagent.iterations, Some(2));
    assert_eq!(subagent.tools.len(), 1);
    assert_eq!(subagent.tools[0].call_id, "call-child");
    assert_eq!(subagent.tools[0].status, "done");
    assert_eq!(subagent.tools[0].result.as_deref(), Some("contents"));
}

#[test]
fn collaboration_subagent_survives_parent_turn_and_keeps_late_tools_nested() {
    let mut progress = TurnProgress::default();
    let child_thread_id = child_thread_id();

    apply(
        &mut progress,
        Event::ToolCallRequested {
            call: ToolCall::new(
                "spawn-1",
                "spawn_agent",
                json!({
                    "task_name": "scan",
                    "message": "inspect",
                    "agent_type": "explore"
                }),
            ),
        },
    );
    apply(
        &mut progress,
        Event::SubagentStarted {
            role: "explore".to_owned(),
            description: Some("scan".to_owned()),
            child_thread_id,
        },
    );
    apply(
        &mut progress,
        Event::ToolFinished {
            result: ToolResult::ok("spawn-1".to_owned(), "started"),
        },
    );
    apply(
        &mut progress,
        Event::TurnFinished {
            output: crate::domain::AgentOutput::text("spawned"),
        },
    );
    progress.finish_parent_turn();

    progress.apply(&envelope(
        child_thread_id,
        Event::ToolCallRequested {
            call: ToolCall::new("child-1", "read_file", json!({ "path": "src/lib.rs" })),
        },
    ));
    progress.apply(&envelope(
        child_thread_id,
        Event::ToolFinished {
            result: ToolResult::ok("child-1".to_owned(), "contents"),
        },
    ));

    let snapshot = progress.snapshot();
    assert_eq!(
        snapshot.len(),
        1,
        "late child tool must not become flat progress"
    );
    let subagent = snapshot[0].subagent.as_ref().expect("background subagent");
    assert_eq!(subagent.status, "running");
    assert_eq!(subagent.tools.len(), 1);
    assert_eq!(subagent.tools[0].status, "done");

    apply(
        &mut progress,
        Event::TurnStarted {
            session_id: new_session_id(),
            thread_id: root_thread_id(),
            turn_id: new_turn_id(),
        },
    );
    apply(&mut progress, delta("new parent turn"));
    let snapshot = progress.snapshot();
    assert_eq!(snapshot.len(), 2, "next turn retains card");
    assert!(
        snapshot[0].streaming,
        "background card must not hide stream tail"
    );
    assert!(snapshot[1].subagent.is_some());

    apply(
        &mut progress,
        Event::SubagentFinished {
            role: "explore".to_owned(),
            status: "completed".to_owned(),
            iterations: 2,
            child_thread_id,
        },
    );
    let snapshot = progress.snapshot();
    assert_eq!(
        snapshot[1]
            .subagent
            .as_ref()
            .map(|subagent| subagent.status.as_str()),
        Some("completed")
    );
}

#[test]
fn collaboration_followup_owns_a_background_card() {
    let mut progress = TurnProgress::default();
    let child_thread_id = child_thread_id();

    apply(
        &mut progress,
        Event::ToolCallRequested {
            call: ToolCall::new(
                "followup-1",
                "followup_task",
                json!({ "target": "/root/scan", "message": "continue" }),
            ),
        },
    );
    apply(
        &mut progress,
        Event::SubagentStarted {
            role: "explore".to_owned(),
            description: Some("scan".to_owned()),
            child_thread_id,
        },
    );
    apply(
        &mut progress,
        Event::ToolFinished {
            result: ToolResult::ok("followup-1".to_owned(), "resumed"),
        },
    );
    apply(
        &mut progress,
        Event::TurnFinished {
            output: crate::domain::AgentOutput::text("follow-up started"),
        },
    );
    progress.finish_parent_turn();
    progress.apply(&envelope(
        child_thread_id,
        Event::ToolCallRequested {
            call: ToolCall::new("child-late", "grep", json!({ "pattern": "mailbox" })),
        },
    ));

    let snapshot = progress.snapshot();
    assert_eq!(snapshot.len(), 1);
    let subagent = snapshot[0].subagent.as_ref().expect("follow-up child");
    assert_eq!(subagent.status, "running");
    assert_eq!(subagent.tools.len(), 1);
    assert_eq!(subagent.tools[0].call_id, "child-late");
}

#[test]
fn steering_user_message_splits_assistant_stream_segments() {
    let mut progress = TurnProgress::default();
    apply(
        &mut progress,
        Event::TurnStarted {
            session_id: new_session_id(),
            thread_id: root_thread_id(),
            turn_id: new_turn_id(),
        },
    );
    apply(&mut progress, delta("before steering"));
    apply(
        &mut progress,
        Event::SteeringDelivered {
            message_id: crate::domain::new_message_id(),
            text: "change direction".to_owned(),
            kind: SteeringDeliveryKind::Steering,
            queued_count: 0,
        },
    );
    apply(&mut progress, delta_for(2, "after steering"));

    let snapshot = progress.snapshot();
    assert_eq!(snapshot.len(), 3);
    assert_eq!(snapshot[0].role, "assistant");
    assert_eq!(snapshot[0].text, "before steering");
    assert_eq!(snapshot[1].role, "user");
    assert_eq!(snapshot[1].text, "change direction");
    assert_eq!(snapshot[2].role, "assistant");
    assert_eq!(snapshot[2].text, "after steering");
    assert!(snapshot[2].streaming);
}
