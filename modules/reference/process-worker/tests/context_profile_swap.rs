//! A config-only context swap through the real workflow/context/search workers.
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use proteus_contracts::{
    contracts::{ApprovalRequest, ApprovalResponse, ApprovalTransport},
    model_standard::ContentPart,
};
use proteus_core::core::{
    AgentRuntime, AppConfig, JournalEntry, ModuleCatalog, SessionStore, TurnSettlementStatus,
    WorkflowReplayOptions, replay_workflow,
};
use serde_json::json;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

#[tokio::test]
async fn context_search_profile_prefetches_code_and_preserves_workflow_replay() {
    let workspace = tempfile::tempdir().unwrap();
    std::fs::write(
        workspace.path().join("AGENTS.md"),
        "Keep public names stable.\n",
    )
    .unwrap();
    std::fs::write(
        workspace.path().join("names.py"),
        "def normalize_name(value): return value.strip()\n",
    )
    .unwrap();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    for (profile, prefetch) in [("codex-chatgpt", false), ("context-search-chatgpt", true)] {
        let store_root = tempfile::tempdir().unwrap();
        let config_path = store_root.path().join("config.json");
        let mut config =
            AppConfig::load(Some(&root.join(format!("configs/{profile}.config.toml"))))
                .await
                .unwrap();
        for component in config.components.values_mut() {
            let mut value = serde_json::to_value(&*component).unwrap();
            value["command"] = json!(env!("CARGO_BIN_EXE_proteus-reference-worker"));
            *component = serde_json::from_value(value).unwrap();
        }
        // Preserve the provider capabilities; only HTTP inference is stubbed.
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let model = config
            .module_config
            .get_mut("model")
            .unwrap()
            .get_mut("openai_codex")
            .unwrap();
        model["implementation"] = json!("openai");
        model.as_object_mut().unwrap().remove("auth_file");
        model["api_key"] = json!("fixture-key");
        model["base_url"] = json!(format!("http://{}", listener.local_addr().unwrap()));
        // Ignore the owner's proxy environment for this loopback fixture.
        let component = config.components.get_mut("reference-model").unwrap();
        let mut value = serde_json::to_value(&*component).unwrap();
        value["env_allowlist"] = json!([]);
        *component = serde_json::from_value(value).unwrap();
        let server = tokio::spawn(respond_once(listener));
        let runtime = AgentRuntime::builder(config.clone(), workspace.path().to_path_buf())
            .with_approval(Arc::new(ClientApproval))
            .with_config_path(Some(&config_path))
            .with_module_catalog(ModuleCatalog::from_config(&config).unwrap())
            .build_async()
            .await
            .unwrap();
        tokio::time::timeout(
            Duration::from_secs(30),
            runtime.run("Проверь normalize_name.".to_owned()),
        )
        .await
        .unwrap()
        .unwrap();
        tokio::time::timeout(Duration::from_secs(5), server)
            .await
            .unwrap()
            .unwrap();
        let session_dir: PathBuf = runtime.session_dir().unwrap().into();
        drop(runtime);
        let projection = SessionStore::open(session_dir.clone())
            .unwrap()
            .load_projection()
            .unwrap();
        let request = projection
            .records
            .iter()
            .find_map(|record| match &record.entry {
                JournalEntry::ModelRequestRecorded(model) => Some(&model.request),
                _ => None,
            })
            .expect("canonical model request");
        let chunks = request
            .messages
            .iter()
            .flat_map(|message| &message.parts)
            .filter_map(|part| match &part.payload {
                ContentPart::Context { chunk } => Some(chunk),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(
            chunks
                .iter()
                .any(|chunk| chunk.content.contains("Keep public names stable."))
        );
        assert_eq!(
            chunks
                .iter()
                .any(|chunk| chunk.content.contains("def normalize_name(value)")),
            prefetch,
            "{profile}"
        );
        assert!(projection.unsettled_turns.is_empty());

        // Prove replay uses the recorded context, even after the source changes.
        std::fs::write(
            workspace.path().join("names.py"),
            "def normalize_name(value): return value.lower()\n",
        )
        .unwrap();
        let replay = replay_workflow(
            &session_dir,
            &config,
            &ModuleCatalog::from_config(&config).unwrap(),
            WorkflowReplayOptions::default(),
        )
        .await
        .unwrap();
        assert!(replay.comparison.matched, "{:?}", replay.comparison.issues);
        assert_eq!(replay.recorded.status, TurnSettlementStatus::Success);
        assert_eq!(replay.replay.status, TurnSettlementStatus::Success);
        assert!(replay.source_journal_unchanged);
    }
}

// Match the app-server's interactive approval surface. Headless mode hides
// Ask tools, so it is a different setup for tool visibility and replay.
struct ClientApproval;

#[async_trait::async_trait]
impl ApprovalTransport for ClientApproval {
    fn can_request_approval(&self) -> bool {
        true
    }

    async fn request_approval(&self, _: ApprovalRequest) -> anyhow::Result<ApprovalResponse> {
        anyhow::bail!("context fixture must not execute tools")
    }
}

async fn respond_once(listener: TcpListener) {
    let (mut socket, _) = listener.accept().await.unwrap();
    let mut bytes = Vec::new();
    let (header_end, length) = loop {
        let mut buffer = [0; 4096];
        let n = socket.read(&mut buffer).await.unwrap();
        assert!(n > 0);
        bytes.extend_from_slice(&buffer[..n]);
        if let Some(offset) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            let headers = std::str::from_utf8(&bytes[..offset]).unwrap();
            let length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().unwrap())
                })
                .unwrap();
            break (offset + 4, length);
        }
    };
    while bytes.len() < header_end + length {
        let mut buffer = [0; 4096];
        let n = socket.read(&mut buffer).await.unwrap();
        assert!(n > 0);
        bytes.extend_from_slice(&buffer[..n]);
    }
    let response = json!({"id":"fixture", "output":[{"id":"answer", "type":"message", "role":"assistant", "phase":"final_answer", "content":[{"type":"output_text", "text":"Проверено."}]}], "usage":{"input_tokens":10, "output_tokens":2, "total_tokens":12}});
    let body = format!(
        "event: response.completed\ndata: {}\n\n",
        json!({"response": response})
    );
    socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
}
