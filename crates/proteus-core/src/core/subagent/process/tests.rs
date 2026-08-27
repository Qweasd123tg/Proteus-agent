//! Unit-тесты process-runner-а: фильтр форвардинга и трекер turn-а.
//! Round-trip с реальным дочерним процессом живёт в
//! `crates/proteus-core/tests/process_subagent.rs`.

use serde_json::json;

use super::child::ChildProcess;
use super::*;
use crate::{
    domain::{AgentOutput, ModelRef, ToolCall, ToolResult},
    model_standard::FinishReason,
};

#[test]
fn forward_filter_passes_tool_lifecycle_and_drops_child_telemetry() {
    assert!(should_forward_child_event(&Event::ToolCallRequested {
        call: ToolCall::new("call-1", "shell", json!({})),
    }));
    assert!(should_forward_child_event(&Event::ToolFinished {
        result: ToolResult::ok("call-1".to_owned(), "done"),
    }));
    assert!(should_forward_child_event(&Event::Error {
        message: "boom".to_owned(),
    }));

    assert!(!should_forward_child_event(&Event::ModelRequestPrepared {
        model: ModelRef::new("fake", "fake-model"),
    }));
    assert!(!should_forward_child_event(&Event::AssistantTextDelta {
        text: "chunk".to_owned(),
    }));
    assert!(!should_forward_child_event(&Event::TurnFinished {
        output: AgentOutput::text("final"),
    }));
}

#[test]
fn turn_tracker_counts_iterations_and_collects_partial_text() {
    let mut tracker = TurnTracker::default();

    tracker.observe(&Event::ModelRequestPrepared {
        model: ModelRef::new("fake", "fake-model"),
    });
    tracker.observe(&Event::AssistantTextDelta {
        text: "first ".to_owned(),
    });
    tracker.observe(&Event::AssistantTextDelta {
        text: "answer".to_owned(),
    });
    tracker.observe(&Event::ModelResponseReceived {
        finish_reason: FinishReason::Stop,
    });

    assert_eq!(tracker.iterations, 1);
    assert_eq!(tracker.partial_text(), "first answer");

    // Вторая итерация прервана посреди стрима: хвост стрима приоритетнее
    // последнего завершённого текста.
    tracker.observe(&Event::ModelRequestPrepared {
        model: ModelRef::new("fake", "fake-model"),
    });
    tracker.observe(&Event::AssistantTextDelta {
        text: "partial tail".to_owned(),
    });
    assert_eq!(tracker.iterations, 2);
    assert_eq!(tracker.partial_text(), "partial tail");
}

#[tokio::test]
async fn reserved_and_leased_children_are_never_idle_eviction_victims() {
    let workspace = tempfile::tempdir().expect("workspace");
    let cwd = std::fs::canonicalize(workspace.path()).expect("canonical cwd");
    let session_id = crate::domain::new_session_id();
    let mut pool = ProcessPool::new(1);
    let first_task_id = crate::domain::new_thread_id().to_string();

    let first = PooledChild {
        id: 1,
        child: ChildProcess::test_fixture(),
        cwd: cwd.clone(),
        role: "helper".to_owned(),
        used: true,
    };
    let first_release = pool.release(first, true, session_id, first_task_id.clone());
    assert!(first_release.retained);
    assert!(first_release.evicted.is_empty());

    let reservation = pool
        .reserve_resume(&first_task_id, session_id, "helper", &cwd)
        .expect("reserve first child");
    let second = PooledChild {
        id: 2,
        child: ChildProcess::test_fixture(),
        cwd: cwd.clone(),
        role: "helper".to_owned(),
        used: true,
    };
    let second_release = pool.release(
        second,
        true,
        session_id,
        crate::domain::new_thread_id().to_string(),
    );
    assert!(second_release.retained);
    assert!(
        second_release.evicted.is_empty(),
        "reserved first child is outside the idle cap"
    );

    let mut leased = pool
        .lease_reserved(&reservation)
        .expect("lease reserved first child");
    assert_eq!(leased.id, 1);
    let third = PooledChild {
        id: 3,
        child: ChildProcess::test_fixture(),
        cwd,
        role: "helper".to_owned(),
        used: true,
    };
    let mut third_release = pool.release(
        third,
        true,
        session_id,
        crate::domain::new_thread_id().to_string(),
    );
    assert_eq!(third_release.evicted.len(), 1);
    assert_eq!(third_release.evicted[0].id, 2);
    leased.child.kill().await;
    for child in &mut third_release.evicted {
        child.child.kill().await;
    }
}
