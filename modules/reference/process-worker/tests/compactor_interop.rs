//! Swappable compactors obey canonical scopes and typed report fields,
//! independently of descriptive message names and module-specific diagnostics.
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use async_trait::async_trait;
use proteus_contracts::{
    contracts::{CompactionHost, CompactionInput},
    domain::{AgentTask, ContextChunk, ContextRenderMode, HistoryCompactionReport, ModelRef},
    model_standard::{
        CanonicalMessage, CanonicalModelRequest, CanonicalModelResponse, CanonicalPart,
        ContentPart, FinishReason, MessageRole, PartProvenance, PartScope,
    },
};
use proteus_core::core::{AppConfig, RuntimeRegistry};
use proteus_module_protocol::{
    ProcessComponentBinding, ProcessExportBinding, current_process_contract_authority,
    v3::{ComponentBroker, ComponentBrokerOptions, InvocationTerminal},
};
use proteus_process_host::ProcessSpec;
use serde_json::{Value, json};

fn strategy(module_id: &str) -> Value {
    if module_id == "codex" {
        json!({"trigger_tokens": 100})
    } else {
        json!({"trigger_messages": 3, "retain_user_turns": 1})
    }
}

struct SummaryHost;

#[async_trait]
impl CompactionHost for SummaryHost {
    async fn complete_model(
        &self,
        _request: CanonicalModelRequest,
    ) -> anyhow::Result<CanonicalModelResponse> {
        Ok(CanonicalModelResponse::new(
            CanonicalMessage::text(MessageRole::Assistant, "Continue the current task."),
            vec![],
            FinishReason::Stop,
        ))
    }
}

fn python_worker() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../examples/modules/compactor-process/compact.py")
}

fn config(module_id: &str) -> AppConfig {
    let command = if module_id == "codex" {
        json!({"command": env!("CARGO_BIN_EXE_proteus-reference-worker")})
    } else {
        json!({"command": "python3", "args": [python_worker()],
            "env": {"PYTHONDONTWRITEBYTECODE": "1"}})
    };
    let mut component = command;
    component["exports"] = json!({"compactor": {module_id: {"timeout_ms": 5000}}});
    let mut config = AppConfig::default();
    config
        .module_config
        .entry("model".into())
        .or_default()
        .insert("fake".into(), json!({"implementation": "fake"}));
    config.components.insert(
        "model".into(),
        serde_json::from_value(json!({
            "command": env!("CARGO_BIN_EXE_proteus-reference-worker"),
            "exports": {"model": {"fake": {}}}
        }))
        .unwrap(),
    );
    config.modules.compactor = Some(module_id.to_owned());
    config
        .module_config
        .entry("compactor".to_owned())
        .or_default()
        .insert(module_id.to_owned(), strategy(module_id));
    config.components.insert(
        "swappable-compactor".to_owned(),
        serde_json::from_value(component).unwrap(),
    );
    config
}

fn history() -> Vec<CanonicalMessage> {
    vec![
        // Mixed payloads: retention comes from scope, not a context-pack marker;
        // both compactors must also preserve the explicit model render mode.
        CanonicalMessage::from_parts(
            MessageRole::User,
            vec![
                CanonicalPart::new(
                    PartProvenance::ContextBuilder,
                    PartScope::Request,
                    ContentPart::Text {
                        text: "Fresh workspace instructions.".to_owned(),
                    },
                ),
                CanonicalPart::new(
                    PartProvenance::ContextBuilder,
                    PartScope::Request,
                    ContentPart::Context {
                        chunk: ContextChunk::new("external", "Exact instructions.\n")
                            .with_render_mode(ContextRenderMode::Verbatim),
                    },
                ),
            ],
        )
        .with_name("context"),
        CanonicalMessage::text(MessageRole::User, "Old task."),
        CanonicalMessage::text(MessageRole::Assistant, "Old answer."),
        CanonicalMessage::text(MessageRole::User, "Current task."),
    ]
}

async fn check_names(module_id: &str) {
    let workspace = tempfile::tempdir().unwrap();
    let cwd = workspace.path().to_path_buf();
    let registry = tokio::task::spawn_blocking({
        let cwd = cwd.clone();
        let config = config(module_id);
        move || RuntimeRegistry::from_config(&config, cwd)
    })
    .await
    .unwrap()
    .unwrap();
    let strategy = strategy(module_id);
    let original = history();
    let mut baseline = None;
    for (case, rename) in [
        ("baseline", None),
        (
            "renamed request context",
            Some((0, Some("another-workflow-context"))),
        ),
        ("unnamed request context", Some((0, None))),
        ("current user named context", Some((3, Some("context")))),
        ("old user named context", Some((1, Some("context")))),
    ] {
        let mut messages = original.clone();
        if let Some((index, name)) = rename {
            messages[index].name = name.map(str::to_owned);
        }
        let input = CompactionInput::new(
            AgentTask::new("Current task.", cwd.clone()),
            proteus_contracts::model_standard::CanonicalModelRequest::new(
                ModelRef::new("fake", "fixture"),
                messages.clone(),
            ),
        )
        .with_token_estimate(Some(100_000))
        .with_config(strategy.clone());
        let output = registry
            .compactor
            .compact(input.clone(), Arc::new(SummaryHost))
            .await
            .unwrap();
        assert!(output.changed, "{module_id}: {case} must still compact");
        let report = HistoryCompactionReport::from_compaction_output(&input, &output);
        assert_eq!(report.input_messages, messages.len());
        assert_eq!(report.output_messages, output.messages.len());
        assert_eq!(report.original_token_estimate, Some(100_000));
        assert_eq!(report.output_token_estimate, output.token_estimate);
        assert_eq!(report.skipped_reason, None);
        if module_id == "codex" {
            assert_eq!(report.trigger_tokens, Some(100));
            assert_eq!(report.summary_source.as_deref(), Some("model"));
            assert!(report.output_token_estimate.is_some());
        } else {
            // A message-count strategy has no token trigger to invent.
            assert_eq!(report.trigger_tokens, None);
            assert_eq!(
                report.summary_source.as_deref(),
                Some("deterministic_suffix")
            );
        }
        for key in [
            "input_messages",
            "output_messages",
            "original_token_estimate",
            "output_token_estimate",
            "trigger_tokens",
            "summary_source",
            "skipped_reason",
        ] {
            assert!(
                output.metadata.get(key).is_none(),
                "{module_id}: duplicated {key}"
            );
        }
        for index in [0, 3] {
            assert!(
                output.messages.contains(&messages[index]),
                "{module_id}: {case} lost or changed message {index}"
            );
        }
        // Strategies may retain different history and create their own summary.
        // Only the same strategy under label-only changes must be equivalent.
        let retained: Vec<_> = output
            .messages
            .iter()
            .filter(|message| original.iter().any(|input| input.id == message.id))
            .map(|message| {
                let mut canonical = message.clone();
                canonical.name = None;
                canonical
            })
            .collect();
        if let Some(expected) = &baseline {
            assert_eq!(&retained, expected, "{module_id}: {case} changed retention");
        } else {
            baseline = Some(retained);
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rust_compactor_uses_scope_not_workflow_message_names() {
    check_names("codex").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn python_compactor_uses_scope_not_workflow_message_names() {
    check_names("python_suffix").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn python_compactor_rejects_missing_or_invalid_scope_without_name_fallback() {
    let workspace = tempfile::tempdir().unwrap();
    let version = current_process_contract_authority("compactor")
        .unwrap()
        .contract_version;
    let export =
        ProcessExportBinding::new("compactor", "python_suffix", version, json!({})).unwrap();
    let target = export.export_ref();
    let binding = ProcessComponentBinding::new("python-compactor", [export]).unwrap();
    let spec = ProcessSpec::new("python3")
        .arg(python_worker().display().to_string())
        .cwd(workspace.path())
        .env("PYTHONDONTWRITEBYTECODE", "1");
    let broker = tokio::task::spawn_blocking(move || {
        ComponentBroker::connect(spec, binding, ComponentBrokerOptions::default())
    })
    .await
    .unwrap()
    .unwrap();
    let input = CompactionInput::new(
        AgentTask::new("Current task.", workspace.path().to_path_buf()),
        proteus_contracts::model_standard::CanonicalModelRequest::new(
            ModelRef::new("fake", "fixture"),
            history(),
        ),
    );
    let valid = serde_json::to_value(input).unwrap();
    for scope in [
        None,
        Some(json!("unknown")),
        Some(Value::Null),
        Some(json!([])),
    ] {
        let mut invalid = valid.clone();
        let part = invalid["request"]["messages"][0]["parts"][0]
            .as_object_mut()
            .unwrap();
        if let Some(scope) = scope {
            part.insert("scope".to_owned(), scope);
        } else {
            part.remove("scope");
        }
        let terminal = broker
            .invoke(&target, "compact", invalid, Duration::from_secs(5))
            .await
            .unwrap();
        assert!(
            matches!(terminal, InvocationTerminal::ModuleError(ref error)
            if error.message.contains("canonical part scope")),
            "{terminal:?}"
        );
    }
    // Malformed input must neither crash the component nor cause a permissive
    // fallback. The same worker still accepts a correctly marked request.
    let terminal = broker
        .invoke(&target, "compact", valid, Duration::from_secs(5))
        .await
        .unwrap();
    assert!(
        matches!(terminal, InvocationTerminal::Success(_)),
        "{terminal:?}"
    );
}
