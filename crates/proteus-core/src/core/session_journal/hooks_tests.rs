use super::*;
use crate::{
    contracts::{ExecutionAttribution, HookInput, HookResponse, HookStep, HookStepOutcome},
    core::{JournalEntry, SessionStore},
    domain::{ToolCall, ToolSafety, ToolSpec, new_execution_id, new_session_id},
};
use serde_json::json;

fn trace(schema: serde_json::Value, args: serde_json::Value) -> HookTrace {
    let mut spec = ToolSpec::new("fixture", "fixture", schema, ToolSafety::ReadOnly);
    spec.metadata = json!({"password": "spec-secret"});
    let input = HookInput {
        conversation: None,
        event: HookEvent::BeforeTool {
            call: ToolCall::new("fixture-call", "fixture", args.clone()),
            spec: Some(spec),
            blocked: None,
        },
        attribution: ExecutionAttribution::detached(new_execution_id()),
        cwd: "/tmp".into(),
    };
    let response = HookResponse::ToolArguments { args };
    let output = apply_hook_response(&input.event, &response).expect("live rewrite accepted");
    HookTrace {
        input,
        steps: vec![HookStep {
            module_id: "rewrite".into(),
            outcome: HookStepOutcome::Accepted { response },
        }],
        output: Some(output),
    }
}

#[tokio::test]
async fn strict_secret_schema_survives_hook_journal_while_values_are_redacted() {
    for (property, secret) in [
        (json!({"type":"string"}), json!("string-secret")),
        (json!({"type":"number"}), json!(12345)),
        (json!({"const":"constant-secret"}), json!("constant-secret")),
        (
            json!({"type":"string", "pattern":"^pattern-secret$"}),
            json!("pattern-secret"),
        ),
    ] {
        let config = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let store = SessionStore::new(config.path(), workspace.path(), new_session_id()).unwrap();
        let schema = json!({"type":"object", "properties":{"password":property, "other":{"type":"integer"}}, "required":["password","other"], "additionalProperties":false});
        let trace = trace(schema.clone(), json!({"password":secret, "other":42}));
        let HookEvent::BeforeTool { call, .. } = &trace.input.event else {
            unreachable!()
        };
        store
            .append_execution_journal_entry(
                trace.input.attribution,
                JournalEntry::ToolCallRecorded(crate::core::ToolCallRecorded {
                    call: call.clone(),
                    phase: crate::core::ToolCallRecordPhase::Requested,
                }),
            )
            .await
            .expect("requested call");
        store
            .append_execution_journal_entry(
                trace.input.attribution,
                JournalEntry::HookInvoked(trace),
            )
            .await
            .expect("append accepted live trace");
        let cold = store.load_projection().expect("cold trace validates");
        let JournalEntry::HookInvoked(stored) = &cold.records[1].entry else {
            panic!("hook trace")
        };
        let HookEvent::BeforeTool {
            call,
            spec: Some(spec),
            ..
        } = &stored.input.event
        else {
            panic!("input")
        };
        assert_eq!(spec.input_schema, schema);
        assert_eq!(spec.metadata["password"], "[REDACTED]");
        assert_eq!(call.args["password"], "[REDACTED]");
        let HookEvent::BeforeTool {
            call,
            spec: Some(spec),
            ..
        } = stored.output.as_ref().unwrap()
        else {
            panic!("output")
        };
        assert_eq!(spec.input_schema, schema);
        assert_eq!(call.args["password"], "[REDACTED]");
        let HookStepOutcome::Accepted {
            response: HookResponse::ToolArguments { args },
        } = &stored.steps[0].outcome
        else {
            panic!("accepted")
        };
        assert_eq!(args["password"], "[REDACTED]");
    }
}

#[tokio::test]
async fn raw_trace_rejects_invalid_unrelated_field_even_with_secret_redaction_marker() {
    let config = tempfile::tempdir().unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let store = SessionStore::new(config.path(), workspace.path(), new_session_id()).unwrap();
    let schema = json!({"type":"object", "properties":{"password":{"type":"string"}, "other":{"type":"integer"}}, "required":["other"]});
    let mut trace = trace(schema, json!({"password":"[REDACTED]", "other":42}));
    let invalid = json!({"password":"[REDACTED]", "other":"invalid"});
    let HookStepOutcome::Accepted {
        response: HookResponse::ToolArguments { args },
    } = &mut trace.steps[0].outcome
    else {
        unreachable!()
    };
    *args = invalid.clone();
    let HookEvent::BeforeTool { call, .. } = trace.output.as_mut().unwrap() else {
        unreachable!()
    };
    call.args = invalid;
    let error = store
        .append_execution_journal_entry(trace.input.attribution, JournalEntry::HookInvoked(trace))
        .await
        .expect_err("raw validation must remain strict");
    assert!(
        format!("{error:#}").contains("invalid hook tool arguments"),
        "{error:#}"
    );
    assert!(store.load_records().unwrap().is_empty());
}

#[test]
fn spec_metadata_marker_does_not_disable_persisted_argument_validation() {
    let mut trace = trace(
        json!({"type":"object", "properties":{"other":{"type":"integer"}}}),
        json!({"other":42}),
    );
    let HookEvent::BeforeTool {
        spec: Some(spec), ..
    } = &mut trace.input.event
    else {
        unreachable!()
    };
    spec.metadata = json!({"password":"[REDACTED]"});
    let HookStepOutcome::Accepted {
        response: HookResponse::ToolArguments { args },
    } = &mut trace.steps[0].outcome
    else {
        unreachable!()
    };
    *args = json!({"other":"invalid"});
    let record = JournalRecord {
        schema_version: crate::core::JOURNAL_SCHEMA_VERSION,
        record_id: crate::domain::new_record_id(),
        session_seq: 1,
        timestamp_ms: 0,
        session_id: new_session_id(),
        execution_id: Some(trace.input.attribution.execution_id),
        thread_id: None,
        turn_id: None,
        entry: JournalEntry::HookInvoked(trace.clone()),
    };
    assert!(validate_trace(&record, &trace).is_err());
}
