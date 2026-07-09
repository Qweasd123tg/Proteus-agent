//! Unit-тесты process-runner-а: фильтр форвардинга и трекер turn-а.
//! Round-trip с реальным дочерним процессом живёт в
//! `crates/proteus-core/tests/process_subagent.rs`.

use serde_json::json;

use super::*;
use crate::{
    domain::{ModelRef, TokenUsageSnapshot, ToolCall, ToolResult},
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

#[test]
fn turn_tracker_accumulates_actual_usage_only() {
    let mut tracker = TurnTracker::default();

    let mut snapshot =
        TokenUsageSnapshot::new(ModelRef::new("fake", "fake-model"), 100, Vec::new());
    tracker.observe(&Event::TokenUsageUpdated {
        usage: snapshot.clone(),
    });
    assert!(tracker.usage.is_none(), "estimate-only snapshot is ignored");

    snapshot.actual = Some(TokenUsage::new(10, 5));
    tracker.observe(&Event::TokenUsageUpdated {
        usage: snapshot.clone(),
    });
    tracker.observe(&Event::TokenUsageUpdated { usage: snapshot });

    let usage = tracker.usage.expect("accumulated usage");
    assert_eq!(usage.input_tokens, 20);
    assert_eq!(usage.output_tokens, 10);
}

#[test]
fn turn_tracker_budget_trips_on_accumulated_usage() {
    let mut tracker = TurnTracker::with_budget(Some(25));
    assert!(!tracker.budget.exceeded());

    let mut snapshot =
        TokenUsageSnapshot::new(ModelRef::new("fake", "fake-model"), 100, Vec::new());
    // Estimate-only снапшоты бюджет не двигают — считаем только actual.
    tracker.observe(&Event::TokenUsageUpdated {
        usage: snapshot.clone(),
    });
    assert!(!tracker.budget.exceeded());

    snapshot.actual = Some(TokenUsage::new(10, 5));
    tracker.observe(&Event::TokenUsageUpdated {
        usage: snapshot.clone(),
    });
    assert!(!tracker.budget.exceeded(), "15 <= 25");

    tracker.observe(&Event::TokenUsageUpdated { usage: snapshot });
    assert!(tracker.budget.exceeded(), "30 > 25");
}

#[test]
fn turn_tracker_without_budget_never_trips() {
    let mut tracker = TurnTracker::with_budget(None);
    let mut snapshot =
        TokenUsageSnapshot::new(ModelRef::new("fake", "fake-model"), 100, Vec::new());
    snapshot.actual = Some(TokenUsage::new(u32::MAX, u32::MAX));
    tracker.observe(&Event::TokenUsageUpdated { usage: snapshot });
    assert!(!tracker.budget.exceeded());
}
