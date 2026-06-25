use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PermissionMode {
    Plan,
    Normal,
    Auto,
}

impl PermissionMode {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Plan => "plan",
            Self::Normal => "normal",
            Self::Auto => "auto",
        }
    }

    pub(crate) fn description(self) -> &'static str {
        match self {
            Self::Plan => "только чтение",
            Self::Normal => "спрашивать перед записью",
            Self::Auto => "писать без запросов",
        }
    }

    pub(crate) fn from_value(value: &str) -> Self {
        match value.to_ascii_lowercase().as_str() {
            "plan" => Self::Plan,
            "auto" => Self::Auto,
            _ => Self::Normal,
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) enum ReasoningEffort {
    #[default]
    Config,
    Custom(String),
}

impl ReasoningEffort {
    pub(crate) fn label(&self) -> String {
        match self {
            Self::Config => "auto".to_owned(),
            Self::Custom(value) => value.clone(),
        }
    }

    pub(crate) fn value(&self) -> String {
        match self {
            Self::Config => "auto".to_owned(),
            Self::Custom(value) => value.clone(),
        }
    }

    pub(crate) fn effort(&self) -> Option<String> {
        match self {
            Self::Config => None,
            Self::Custom(value) => Some(value.clone()),
        }
    }

    pub(crate) fn from_value(value: &str) -> Self {
        let value = value.trim();
        if value.is_empty()
            || value.eq_ignore_ascii_case("auto")
            || value.eq_ignore_ascii_case("config")
        {
            Self::Config
        } else {
            Self::Custom(value.to_owned())
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ApprovalCacheScope {
    #[default]
    None,
    ExactCall,
    ExactCommand,
    ToolInCwd,
    WorkspaceWrite,
}

impl ApprovalCacheScope {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::None => "Один раз",
            Self::ExactCall => "Точно",
            Self::ExactCommand => "Команда",
            Self::ToolInCwd => "Tool/CWD",
            Self::WorkspaceWrite => "Workspace",
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct SessionToken(Option<String>);

impl SessionToken {
    pub(crate) fn new(value: impl Into<String>) -> Self {
        let value = value.into();
        let value = value.trim();
        if value.is_empty() {
            Self(None)
        } else {
            Self(Some(value.to_owned()))
        }
    }

    pub(crate) fn missing() -> Self {
        Self(None)
    }

    pub(crate) fn as_deref(&self) -> Option<&str> {
        self.0.as_deref()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MessageRole {
    User,
    Assistant,
    System,
    /// Поток reasoning-summary модели (OpenAI o-series). Рендерится
    /// отдельным сворачиваемым блоком, не как обычное сообщение.
    Reasoning,
}

impl MessageRole {
    pub(crate) fn label(&self) -> &'static str {
        match self {
            Self::User => "Вы",
            Self::Assistant => "Proteus",
            Self::System => "Система",
            Self::Reasoning => "Размышления",
        }
    }

    pub(crate) fn message_class(&self) -> &'static str {
        match self {
            Self::User => "message user-message",
            Self::Assistant => "message assistant-message",
            Self::System | Self::Reasoning => "message system-message",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Message {
    pub(crate) id: u64,
    pub(crate) version: u64,
    pub(crate) role: MessageRole,
    pub(crate) text: String,
    pub(crate) tool: Option<ToolActivity>,
    pub(crate) streaming: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ToastMessage {
    pub(crate) id: u64,
    pub(crate) text: String,
}

/// Заполнение контекстного окна по данным события `TokenUsageUpdated`.
/// Последний валидный снимок сохраняется клиентом, чтобы бублик сразу
/// восстанавливался при возврате в чат.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct ContextUsage {
    pub(crate) used_tokens: u32,
    pub(crate) max_tokens: u32,
    /// Порог токенов, на котором сервер запускает автокомпакт. `None`, если
    /// автокомпакт не настроен — тогда метка на бублике не рисуется.
    pub(crate) compaction_trigger_tokens: Option<u32>,
}

impl ContextUsage {
    pub(crate) fn percent(&self) -> u8 {
        Self::ratio_percent(self.used_tokens, self.max_tokens)
    }

    /// Позиция метки автокомпакта в процентах окна. `None`, если порога нет
    /// или он за пределами окна (рисовать метку негде).
    pub(crate) fn compaction_percent(&self) -> Option<u8> {
        let trigger = self.compaction_trigger_tokens?;
        if trigger == 0 || trigger >= self.max_tokens {
            return None;
        }
        Some(Self::ratio_percent(trigger, self.max_tokens))
    }

    fn ratio_percent(value: u32, total: u32) -> u8 {
        if total == 0 {
            return 0;
        }
        ((f64::from(value) / f64::from(total)) * 100.0)
            .round()
            .clamp(0.0, 100.0) as u8
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ToolActivity {
    pub(crate) call_id: String,
    pub(crate) name: String,
    pub(crate) args_preview: String,
    pub(crate) started_at_ms: u64,
    pub(crate) status: ToolActivityStatus,
    pub(crate) result_preview: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ToolActivityStatus {
    Running,
    WaitingApproval,
    Approved,
    Denied,
    Done,
    Failed,
}

impl ToolActivityStatus {
    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::WaitingApproval => "waiting_approval",
            Self::Approved => "approved",
            Self::Denied => "denied",
            Self::Done => "done",
            Self::Failed => "failed",
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Running => "выполняется",
            Self::WaitingApproval => "ждёт доступ",
            Self::Approved => "разрешено",
            Self::Denied => "отклонено",
            Self::Done => "готово",
            Self::Failed => "ошибка",
        }
    }

    pub(crate) fn badge_class(self) -> &'static str {
        match self {
            Self::Running | Self::WaitingApproval | Self::Approved => "status-badge running",
            Self::Done => "status-badge completed",
            Self::Denied | Self::Failed => "status-badge failed",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub(crate) struct ToolCallInfo {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) args: Value,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub(crate) struct ApprovalRequestInfo {
    pub(crate) approval_id: String,
    pub(crate) call: ToolCallInfo,
    pub(crate) cwd: String,
    pub(crate) reason: String,
    pub(crate) tool_spec: Option<Value>,
    #[serde(default)]
    pub(crate) preview: Option<ApprovalPreviewInfo>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub(crate) struct ApprovalPreviewInfo {
    pub(crate) kind: String,
    pub(crate) title: String,
    pub(crate) summary: String,
    #[serde(default)]
    pub(crate) affected_files: Vec<String>,
    #[serde(default)]
    pub(crate) body: Option<String>,
    #[serde(default)]
    pub(crate) language: Option<String>,
    #[serde(default)]
    pub(crate) metadata: Value,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
pub(crate) struct UserInputOption {
    pub(crate) label: String,
    pub(crate) description: String,
    pub(crate) preview: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
pub(crate) struct UserInputQuestion {
    pub(crate) id: String,
    pub(crate) header: String,
    pub(crate) question: String,
    #[serde(default)]
    pub(crate) is_other: bool,
    #[serde(default)]
    pub(crate) is_secret: bool,
    #[serde(default, alias = "multiSelect")]
    pub(crate) multi_select: bool,
    #[serde(default)]
    pub(crate) options: Vec<UserInputOption>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
pub(crate) struct UserInputRequestInfo {
    pub(crate) request_id: String,
    pub(crate) cwd: String,
    pub(crate) title: Option<String>,
    pub(crate) questions: Vec<UserInputQuestion>,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub(crate) struct PendingControlPlaneInfo {
    #[serde(default)]
    pub(crate) approvals: Vec<ApprovalRequestInfo>,
    #[serde(default)]
    pub(crate) user_inputs: Vec<UserInputRequestInfo>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
pub(crate) struct SessionSummary {
    pub(crate) session_dir: String,
    pub(crate) session_id: Option<String>,
    pub(crate) workspace_path: Option<String>,
    pub(crate) message_count: usize,
    pub(crate) updated_at_ms: Option<u64>,
    pub(crate) preview: Option<String>,
    pub(crate) resumable: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
pub(crate) struct TranscriptMessage {
    pub(crate) role: String,
    pub(crate) text: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum TransportStatus {
    Connecting,
    Connected,
    Error(String),
    Shutdown,
}

impl TransportStatus {
    pub(crate) fn label(&self) -> String {
        match self {
            Self::Connecting => "подключение".to_owned(),
            Self::Connected => "подключено".to_owned(),
            Self::Error(message) => format!("ошибка: {message}"),
            Self::Shutdown => "остановлено".to_owned(),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum StdioOutput {
    Event {
        event: Box<AppServerEvent>,
    },
    Response {
        id: Option<String>,
        ok: bool,
        output: Option<Value>,
        error: Option<String>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum AppServerEvent {
    Runtime {
        envelope: Value,
    },
    UserMessageSubmitted {
        text: String,
    },
    TurnOutput {
        output: Value,
    },
    ApprovalRequested {
        request: Box<ApprovalRequestInfo>,
    },
    ApprovalResolved {
        approval_id: String,
        approved: bool,
    },
    UserInputRequested {
        request: Box<UserInputRequestInfo>,
    },
    UserInputResolved {
        request_id: String,
    },
    Error {
        message: String,
    },
    Shutdown,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Serialize)]
pub(crate) struct SendRequest {
    pub(crate) id: Option<String>,
    pub(crate) text: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct SetPermissionModeRequest {
    pub(crate) id: Option<String>,
    pub(crate) mode: PermissionMode,
}

#[derive(Debug, Serialize)]
pub(crate) struct SetModelRequest {
    pub(crate) id: Option<String>,
    pub(crate) model: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct SetReasoningEffortRequest {
    pub(crate) id: Option<String>,
    pub(crate) effort: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct SetReasoningEnabledRequest {
    pub(crate) id: Option<String>,
    pub(crate) enabled: bool,
}

#[derive(Debug, Serialize)]
pub(crate) struct ResolveApprovalRequest {
    pub(crate) id: Option<String>,
    pub(crate) approval_id: String,
    pub(crate) approved: bool,
    pub(crate) note: Option<String>,
    #[serde(default)]
    pub(crate) cache: ApprovalCacheScope,
}

#[derive(Debug, Serialize)]
pub(crate) struct UserInputSubmitRequest {
    pub(crate) id: Option<String>,
    pub(crate) request_id: String,
    pub(crate) response: UserInputResponseBody,
}

#[derive(Debug, Serialize)]
pub(crate) struct UserInputResponseBody {
    pub(crate) answers: HashMap<String, UserInputAnswerBody>,
}

#[derive(Debug, Serialize)]
pub(crate) struct UserInputAnswerBody {
    pub(crate) answers: Vec<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct CancelRequest {
    pub(crate) id: Option<String>,
    pub(crate) target_id: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct ResumeSessionRequest {
    pub(crate) id: Option<String>,
    pub(crate) session_dir: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct DeleteSessionRequest {
    pub(crate) id: Option<String>,
    pub(crate) session_dir: String,
}

#[cfg(test)]
mod protocol_contract_tests {
    use std::{collections::HashMap, path::PathBuf};

    use proteus_contracts::{
        app_protocol as contract_protocol, contracts as contract_contracts,
        domain as contract_domain,
    };
    use serde::Serialize;
    use serde_json::{Value, json};

    use super::*;

    fn decode_web_output(output: contract_protocol::StdioOutput) -> StdioOutput {
        let value = serde_json::to_value(output).expect("contract output JSON");
        serde_json::from_value(value).expect("web output decodes")
    }

    fn endpoint_body<T: Serialize>(request: T) -> Value {
        let mut value = serde_json::to_value(request).expect("request JSON");
        let Value::Object(fields) = &mut value else {
            panic!("contract request must serialize to an object");
        };
        fields.remove("type");
        value
    }

    fn assert_endpoint_body_matches_contract<T: Serialize, U: Serialize>(
        web_request: T,
        contract_request: U,
    ) {
        assert_eq!(
            serde_json::to_value(web_request).expect("web request JSON"),
            endpoint_body(contract_request)
        );
    }

    fn contract_approval_request() -> contract_protocol::AppApprovalRequest {
        contract_protocol::AppApprovalRequest::new(
            "approval-1".to_owned(),
            contract_domain::ToolCall::new(
                "call-1",
                "write_file",
                json!({ "path": "README.md", "content": "hello" }),
            ),
            PathBuf::from("/workspace"),
            "Need write access".to_owned(),
            Some(
                contract_domain::ToolSpec::new(
                    "write_file",
                    "Writes a file",
                    json!({
                        "type": "object",
                        "properties": {
                            "path": { "type": "string" },
                            "content": { "type": "string" }
                        }
                    }),
                    contract_domain::ToolSafety::WritesFiles,
                )
                .with_metadata(json!({ "approval_cache_scope": "workspace_write" })),
            ),
        )
        .with_preview(Some(
            contract_protocol::AppApprovalPreview::new(
                "write_file",
                "Write README.md",
                "Updates README.md",
            )
            .with_affected_files(vec!["README.md".to_owned()])
            .with_body("hello", "text")
            .with_metadata(json!({ "operation": "update" })),
        ))
    }

    fn contract_user_input_request() -> contract_contracts::UserInputRequest {
        contract_contracts::UserInputRequest::new(
            "input-1",
            PathBuf::from("/workspace"),
            vec![
                contract_contracts::UserInputQuestion::new(
                    "scope",
                    "Scope",
                    "What should Proteus change?",
                    vec![
                        contract_contracts::UserInputQuestionOption::new(
                            "Minimal",
                            "Only the direct fix",
                        )
                        .with_preview("recommended"),
                        contract_contracts::UserInputQuestionOption::new(
                            "Broad",
                            "Include nearby cleanup",
                        ),
                    ],
                )
                .with_other(false)
                .with_secret(false)
                .with_multi_select(true),
            ],
        )
        .with_title("Choose implementation scope")
    }

    fn contract_response() -> contract_contracts::UserInputResponse {
        contract_contracts::UserInputResponse::new(HashMap::from([(
            "scope".to_owned(),
            contract_contracts::UserInputAnswer::new(vec!["Minimal".to_owned()]),
        )]))
    }

    #[test]
    fn web_decodes_contract_stdio_output_events() {
        let session_id = contract_domain::new_session_id();
        let thread_id = contract_domain::new_thread_id();
        let events = vec![
            contract_protocol::AppServerEvent::Runtime {
                envelope: Box::new(contract_domain::EventEnvelope::new(
                    contract_domain::EventContext::new(session_id, thread_id, None),
                    7,
                    contract_domain::Event::SessionStarted {
                        session_id,
                        cwd: PathBuf::from("/workspace"),
                        model: None,
                        session_dir: None,
                    },
                )),
            },
            contract_protocol::AppServerEvent::UserMessageSubmitted {
                text: "hello".to_owned(),
            },
            contract_protocol::AppServerEvent::TurnOutput {
                output: Box::new(contract_domain::AgentOutput::new(
                    "done",
                    json!({ "ok": true }),
                )),
            },
            contract_protocol::AppServerEvent::ApprovalRequested {
                request: Box::new(contract_approval_request()),
            },
            contract_protocol::AppServerEvent::ApprovalResolved {
                approval_id: "approval-1".to_owned(),
                approved: true,
            },
            contract_protocol::AppServerEvent::UserInputRequested {
                request: Box::new(contract_user_input_request()),
            },
            contract_protocol::AppServerEvent::UserInputResolved {
                request_id: "input-1".to_owned(),
            },
            contract_protocol::AppServerEvent::ModulesReloaded {
                old_epoch: 1,
                new_epoch: 2,
                tool_names: vec!["read_file".to_owned()],
            },
            contract_protocol::AppServerEvent::Error {
                message: "boom".to_owned(),
            },
            contract_protocol::AppServerEvent::Shutdown,
        ];

        for event in events {
            let output = decode_web_output(contract_protocol::StdioOutput::Event {
                event: Box::new(event),
            });

            let StdioOutput::Event { event } = output else {
                panic!("unexpected output: {output:?}");
            };

            match *event {
                AppServerEvent::Runtime { envelope } => assert_eq!(envelope["seq"], 7),
                AppServerEvent::UserMessageSubmitted { text } => assert_eq!(text, "hello"),
                AppServerEvent::TurnOutput { output } => {
                    assert_eq!(output["text"], "done");
                    assert_eq!(output["metadata"]["ok"], true);
                }
                AppServerEvent::ApprovalRequested { request } => {
                    assert_eq!(request.approval_id, "approval-1");
                    assert_eq!(request.call.name, "write_file");
                    assert_eq!(request.cwd, "/workspace");
                    let preview = request.preview.expect("approval preview");
                    assert_eq!(preview.kind, "write_file");
                    assert_eq!(preview.affected_files, vec!["README.md"]);
                    assert_eq!(preview.metadata["operation"], "update");
                }
                AppServerEvent::ApprovalResolved {
                    approval_id,
                    approved,
                } => {
                    assert_eq!(approval_id, "approval-1");
                    assert!(approved);
                }
                AppServerEvent::UserInputRequested { request } => {
                    assert_eq!(request.request_id, "input-1");
                    assert_eq!(
                        request.title.as_deref(),
                        Some("Choose implementation scope")
                    );
                    let question = request.questions.first().expect("question");
                    assert!(question.multi_select);
                    assert_eq!(
                        question
                            .options
                            .first()
                            .and_then(|option| option.preview.as_deref()),
                        Some("recommended")
                    );
                }
                AppServerEvent::UserInputResolved { request_id } => {
                    assert_eq!(request_id, "input-1")
                }
                AppServerEvent::Error { message } => assert_eq!(message, "boom"),
                AppServerEvent::Shutdown => {}
                AppServerEvent::Unknown => {}
            }
        }
    }

    #[test]
    fn web_decodes_contract_pending_requests() {
        let pending = contract_protocol::AppPendingRequests::new(
            vec![contract_approval_request()],
            vec![contract_user_input_request()],
        );

        let value = serde_json::to_value(pending).expect("pending JSON");
        let decoded: PendingControlPlaneInfo =
            serde_json::from_value(value).expect("web pending requests");

        assert_eq!(decoded.approvals.len(), 1);
        assert_eq!(decoded.approvals[0].approval_id, "approval-1");
        assert_eq!(decoded.approvals[0].call.args["path"], "README.md");
        assert_eq!(decoded.user_inputs.len(), 1);
        assert_eq!(decoded.user_inputs[0].request_id, "input-1");
        assert_eq!(
            decoded.user_inputs[0].questions[0].options[0].label,
            "Minimal"
        );
    }

    #[test]
    fn web_endpoint_request_bodies_match_contract_stdio_requests_without_transport_tag() {
        assert_endpoint_body_matches_contract(
            SendRequest {
                id: Some("send-1".to_owned()),
                text: "hello".to_owned(),
            },
            contract_protocol::StdioRequest::Send {
                id: Some("send-1".to_owned()),
                text: "hello".to_owned(),
            },
        );
        assert_endpoint_body_matches_contract(
            SetPermissionModeRequest {
                id: Some("mode-1".to_owned()),
                mode: PermissionMode::Auto,
            },
            contract_protocol::StdioRequest::SetPermissionMode {
                id: Some("mode-1".to_owned()),
                mode: contract_domain::PermissionMode::Auto,
            },
        );
        assert_endpoint_body_matches_contract(
            SetModelRequest {
                id: Some("model-1".to_owned()),
                model: "gpt-5".to_owned(),
            },
            contract_protocol::StdioRequest::SetModel {
                id: Some("model-1".to_owned()),
                model: "gpt-5".to_owned(),
            },
        );
        assert_endpoint_body_matches_contract(
            SetReasoningEffortRequest {
                id: Some("effort-1".to_owned()),
                effort: Some("high".to_owned()),
            },
            contract_protocol::StdioRequest::SetReasoningEffort {
                id: Some("effort-1".to_owned()),
                effort: Some("high".to_owned()),
            },
        );
        assert_endpoint_body_matches_contract(
            SetReasoningEnabledRequest {
                id: Some("reasoning-1".to_owned()),
                enabled: true,
            },
            contract_protocol::StdioRequest::SetReasoningEnabled {
                id: Some("reasoning-1".to_owned()),
                enabled: true,
            },
        );
        assert_endpoint_body_matches_contract(
            ResolveApprovalRequest {
                id: Some("approval-1".to_owned()),
                approval_id: "approval-1".to_owned(),
                approved: true,
                note: Some("ok".to_owned()),
                cache: ApprovalCacheScope::WorkspaceWrite,
            },
            contract_protocol::StdioRequest::Approval {
                id: Some("approval-1".to_owned()),
                approval_id: "approval-1".to_owned(),
                approved: true,
                note: Some("ok".to_owned()),
                cache: contract_contracts::ApprovalCacheScope::WorkspaceWrite,
            },
        );
        assert_endpoint_body_matches_contract(
            UserInputSubmitRequest {
                id: Some("input-1".to_owned()),
                request_id: "input-1".to_owned(),
                response: UserInputResponseBody {
                    answers: HashMap::from([(
                        "scope".to_owned(),
                        UserInputAnswerBody {
                            answers: vec!["Minimal".to_owned()],
                        },
                    )]),
                },
            },
            contract_protocol::StdioRequest::UserInput {
                id: Some("input-1".to_owned()),
                request_id: "input-1".to_owned(),
                response: contract_response(),
            },
        );
        assert_endpoint_body_matches_contract(
            CancelRequest {
                id: Some("cancel-1".to_owned()),
                target_id: "send-1".to_owned(),
            },
            contract_protocol::StdioRequest::Cancel {
                id: Some("cancel-1".to_owned()),
                target_id: "send-1".to_owned(),
            },
        );
    }

    #[test]
    fn web_http_only_session_request_shapes_stay_stable() {
        assert_eq!(
            serde_json::to_value(ResumeSessionRequest {
                id: Some("resume-1".to_owned()),
                session_dir: "/tmp/proteus-session".to_owned(),
            })
            .expect("resume request JSON"),
            json!({
                "id": "resume-1",
                "session_dir": "/tmp/proteus-session"
            })
        );
        assert_eq!(
            serde_json::to_value(DeleteSessionRequest {
                id: Some("delete-1".to_owned()),
                session_dir: "/tmp/proteus-session".to_owned(),
            })
            .expect("delete request JSON"),
            json!({
                "id": "delete-1",
                "session_dir": "/tmp/proteus-session"
            })
        );
    }
}
