use proteus_core::core::{AppConfig, SessionStore, list_workspace_session_summaries};
use serde_json::{Value, json};
use std::{path::PathBuf, process::Stdio, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines},
    process::{Child, ChildStdin, ChildStdout, Command},
};

pub struct Client {
    pub child: Child,
    pub stdin: Option<ChildStdin>,
    lines: Lines<BufReader<ChildStdout>>,
    pub dir: tempfile::TempDir,
    pub config: AppConfig,
    pub cwd: PathBuf,
}

impl Client {
    pub async fn launch(delay: u64) -> Self {
        Self::launch_with_timeout(delay, 300_000).await
    }

    pub async fn launch_with_timeout(delay: u64, timeout_ms: u64) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let cwd = dir.path().join("workspace");
        std::fs::create_dir(&cwd).unwrap();
        let mut config = super::test_model::config();
        config.components.insert(
            "runtime".into(),
            serde_json::from_value(json!({
                "command": super::test_model::worker(),
                "exports": {"workflow":{"coding.single_loop":{}},"context":{"simple":{}},
                    "policy":{"ask_write":{}},"patch":{"direct":{}}}
            }))
            .unwrap(),
        );
        config.modules.workflow = Some("coding.single_loop".into());
        config.modules.context = Some("simple".into());
        config.modules.policy = Some("ask_write".into());
        config.modules.patch = Some("direct".into());
        config.tools.enabled = vec!["apply_patch".into(), "request_user_input".into()];
        config
            .module_config
            .get_mut("model")
            .unwrap()
            .get_mut("fake")
            .unwrap()["stream_delay_ms"] = json!(delay);
        config.event_log.path = dir.path().join("events.jsonl");
        config.app_server.approval_timeout_ms = 0;
        config.runtime.workflow_timeout_ms = timeout_ms;
        let path = dir.path().join("config.json");
        std::fs::write(&path, serde_json::to_vec_pretty(&config).unwrap()).unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_proteus"))
            .arg("--config")
            .arg(path)
            .args(["server", "acp"])
            .current_dir(dir.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let stdin = child.stdin.take();
        let lines = BufReader::new(child.stdout.take().unwrap()).lines();
        Self {
            child,
            stdin,
            lines,
            dir,
            config,
            cwd,
        }
    }

    pub async fn write(&mut self, value: Value) {
        let mut bytes = serde_json::to_vec(&value).unwrap();
        bytes.push(b'\n');
        let stdin = self.stdin.as_mut().unwrap();
        stdin.write_all(&bytes).await.unwrap();
        stdin.flush().await.unwrap();
    }

    pub async fn read(&mut self) -> Value {
        let line = tokio::time::timeout(Duration::from_secs(30), self.lines.next_line())
            .await
            .expect("ACP output timed out")
            .unwrap()
            .expect("ACP closed stdout");
        let value: Value = serde_json::from_str(&line).expect("stdout must contain only JSON-RPC");
        assert_eq!(value["jsonrpc"], "2.0");
        value
    }

    pub async fn request(&mut self, id: u64, method: &str, params: Value) {
        self.write(json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))
            .await;
    }

    pub async fn response(&mut self, id: u64) -> (Value, Vec<Value>) {
        let mut updates = Vec::new();
        loop {
            let value = self.read().await;
            if value["id"] == id {
                return (value, updates);
            }
            assert!(
                value.get("id").is_none(),
                "unexpected request/response: {value}"
            );
            updates.push(value);
        }
    }

    pub async fn initialize(&mut self) {
        self.request(
            1,
            "initialize",
            json!({"protocolVersion":1,"clientCapabilities":{}}),
        )
        .await;
        let (response, _) = self.response(1).await;
        assert_eq!(response["result"]["protocolVersion"], 1, "{response}");
        assert_eq!(response["result"]["agentInfo"]["name"], "proteus");
        assert_eq!(
            response["result"]["agentCapabilities"]["loadSession"],
            false
        );
    }

    pub async fn new_session(&mut self, id: u64) -> String {
        self.request(id, "session/new", json!({"cwd":self.cwd,"mcpServers":[]}))
            .await;
        let (response, _) = self.response(id).await;
        response["result"]["sessionId"]
            .as_str()
            .expect(&response.to_string())
            .to_owned()
    }

    pub async fn prompt(&mut self, id: u64, session: &str, text: &str) {
        self.request(
            id,
            "session/prompt",
            json!({"sessionId":session,"prompt":[{"type":"text","text":text}]}),
        )
        .await;
    }

    pub async fn cancel(&mut self, session: &str) {
        self.write(
            json!({"jsonrpc":"2.0","method":"session/cancel","params":{"sessionId":session}}),
        )
        .await;
    }

    pub async fn close(&mut self) {
        self.stdin.take();
        let status = tokio::time::timeout(Duration::from_secs(10), self.child.wait())
            .await
            .expect("ACP must settle turns and exit on EOF")
            .unwrap();
        assert!(status.success(), "{status}");
    }

    pub fn store(&self, session: &str) -> SessionStore {
        let summary = list_workspace_session_summaries(self.dir.path(), &self.cwd)
            .unwrap()
            .into_iter()
            .find(|s| s.session_id.to_string() == session)
            .unwrap();
        SessionStore::open(summary.session_dir).unwrap()
    }
}

pub fn text(updates: &[Value]) -> String {
    updates
        .iter()
        .filter(|v| v["params"]["update"]["sessionUpdate"] == "agent_message_chunk")
        .filter_map(|v| v["params"]["update"]["content"]["text"].as_str())
        .collect()
}
