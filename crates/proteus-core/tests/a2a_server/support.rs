use std::{path::PathBuf, process::Stdio, time::Duration};

use a2a::*;
use a2a_client::{A2AClient, jsonrpc::JsonRpcTransport};
use proteus_core::core::{AppConfig, SessionStore, list_session_summaries};
use serde_json::json;
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::{Child, Command},
};

pub struct Peer {
    pub root: tempfile::TempDir,
    pub config: AppConfig,
    pub client: A2AClient<JsonRpcTransport>,
    pub url: String,
    child: Child,
}

impl Peer {
    pub async fn start() -> Self {
        let root = tempfile::tempdir().unwrap();
        let configs = root.path().join("configs");
        std::fs::create_dir(&configs).unwrap();
        let path = configs.join("peer.json");
        let mut config = super::test_model::config();
        config.modules.workflow = Some("coding.single_loop".into());
        config.modules.context = Some("simple".into());
        config.modules.policy = Some("ask_write".into());
        config.modules.patch = Some("direct".into());
        config.tools.enabled = vec!["request_user_input".into(), "apply_patch".into()];
        config.components.insert(
            "reference".into(),
            serde_json::from_value(json!({
                "command": super::test_model::worker(),
                "exports": {"workflow": {"coding.single_loop": {}}, "context": {"simple": {}},
                    "policy": {"ask_write": {}}, "patch": {"direct": {}}}
            }))
            .unwrap(),
        );
        config
            .module_config
            .entry("model".into())
            .or_default()
            .insert(
                "fake".into(),
                json!({"implementation": "fake", "stream_delay_ms": 25}),
            );
        config
            .module_config
            .entry("policy".into())
            .or_default()
            .insert(
                "ask_write".into(),
                json!({"ask_before": ["apply_patch"], "allow": ["request_user_input"]}),
            );
        std::fs::write(&path, serde_json::to_vec_pretty(&config).unwrap()).unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_proteus"))
            .arg("--config")
            .arg(&path)
            .arg("--cwd")
            .arg(root.path())
            .args(["server", "a2a", "--ready-stdout"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let mut stdout = BufReader::new(child.stdout.take().unwrap()).lines();
        let url = tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                let line = stdout
                    .next_line()
                    .await
                    .unwrap()
                    .expect("peer exited before readiness");
                if let Ok(value) = serde_json::from_str::<serde_json::Value>(&line)
                    && value["type"] == "a2a_ready"
                {
                    break value["url"].as_str().unwrap().to_owned();
                }
            }
        })
        .await
        .unwrap();
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("A2A-Version", "1.0".parse().unwrap());
        headers.insert(
            "A2A-Extensions",
            "urn:proteus:a2a:interaction:v1".parse().unwrap(),
        );
        let http = reqwest::Client::builder()
            .default_headers(headers)
            .no_proxy()
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap();
        Self {
            root,
            config,
            client: A2AClient::new(JsonRpcTransport::new(http, url.clone())),
            url,
            child,
        }
    }

    pub async fn send(&self, request: SendMessageRequest) -> Task {
        match self.client.send_message(&request).await.unwrap() {
            SendMessageResponse::Task(task) => task,
            other => panic!("expected task, got {other:?}"),
        }
    }

    pub async fn get(&self, id: &str) -> Task {
        self.client
            .get_task(&GetTaskRequest {
                id: id.into(),
                history_length: None,
                tenant: None,
            })
            .await
            .unwrap()
    }

    pub fn sessions(&self) -> Vec<SessionStore> {
        list_session_summaries(self.root.path())
            .unwrap()
            .into_iter()
            .map(|summary| SessionStore::open(summary.session_dir).unwrap())
            .collect()
    }

    pub async fn stop(&mut self) {
        self.child.kill().await.unwrap();
        self.child.wait().await.unwrap();
    }

    pub fn workspace_file(&self, file: &str) -> PathBuf {
        self.root.path().join(file)
    }
}

pub fn request(text: &str, task: Option<&str>, context: Option<&str>) -> SendMessageRequest {
    let mut message = Message::new(Role::User, vec![Part::text(text)]);
    message.task_id = task.map(str::to_owned);
    message.context_id = context.map(str::to_owned);
    SendMessageRequest {
        message,
        configuration: None,
        metadata: None,
        tenant: None,
    }
}

pub fn interaction(task: &Task, data: serde_json::Value) -> SendMessageRequest {
    let mut req = request("", Some(&task.id), Some(&task.context_id));
    req.message.parts = vec![Part::data(data)];
    req.message.extensions = Some(vec!["urn:proteus:a2a:interaction:v1".into()]);
    req
}

pub fn pending(task: &Task) -> &serde_json::Value {
    task.status
        .message
        .as_ref()
        .unwrap()
        .parts
        .iter()
        .find_map(|part| match &part.content {
            PartContent::Data(data) => Some(data),
            _ => None,
        })
        .expect("pending interaction data")
}

pub async fn wait_working(peer: &Peer, id: &str) {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let task = peer.get(id).await;
            if task.status.state == TaskState::Working {
                assert!(
                    task.status
                        .message
                        .as_ref()
                        .is_none_or(|message| !message.parts.is_empty())
                );
                break;
            }
            assert!(!task.status.state.is_terminal(), "{task:?}");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}
