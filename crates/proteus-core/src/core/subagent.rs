//! Builtin slot `subagent`: последовательный дочерний агентский цикл.
//!
//! `SequentialSubagentRunner` владеет циклом ребёнка целиком
//! (модель → tools → модель), не вызывая slot `workflow`. Ребёнок изолирован:
//! свой `ThreadId`, своя история (только `role.prompt` + `request.prompt`),
//! свой отбор tools по фазе роли. Tool calls ребёнка идут через тот же
//! `ToolOrchestrator` (policy/approval-контур), что и родительские.

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
    time::Duration,
};

use anyhow::{Context, Result, anyhow, bail};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::time::timeout;

use crate::{
    contracts::{
        RuntimeContext, SubagentLimits, SubagentRequest, SubagentResult, SubagentRoleSpec,
        SubagentRunner, SubagentStatus, ToolExposureInput, ToolExposureRequest,
    },
    core::ToolOrchestrator,
    domain::{Event, SessionId, ThreadId, ToolSpec, new_thread_id},
    model_standard::{
        CanonicalMessage, CanonicalModelRequest, CanonicalModelResponse, ContentPart, MessageRole,
        TokenUsage,
    },
};

/// Имя tool'а делегирования, который workflow генерирует из ролей.
/// Убирается из тулсета ребёнка, чтобы запретить рекурсию на уровне тулсета.
const TASK_TOOL_NAME: &str = "task";

/// Формат `module_config.subagent.sequential`.
#[derive(Debug, Clone, Deserialize)]
struct SequentialSubagentConfig {
    #[serde(default)]
    roles: Vec<SequentialRoleConfig>,
    #[serde(default)]
    roles_dir: Option<PathBuf>,
    #[serde(default = "default_max_depth")]
    max_depth: u64,
    #[serde(default = "default_max_resumable")]
    max_resumable: usize,
}

impl Default for SequentialSubagentConfig {
    fn default() -> Self {
        Self {
            roles: Vec::new(),
            roles_dir: None,
            max_depth: default_max_depth(),
            max_resumable: default_max_resumable(),
        }
    }
}

/// Роль в конфиге: лимиты заданы плоскими полями рядом с prompt.
#[derive(Debug, Clone, Deserialize)]
struct SequentialRoleConfig {
    name: String,
    description: String,
    prompt: String,
    #[serde(default)]
    exposure_phase: Option<String>,
    #[serde(default)]
    tools: Option<Vec<String>>,
    #[serde(default)]
    max_iterations: Option<u32>,
    #[serde(default)]
    timeout_ms: Option<u64>,
    #[serde(default)]
    max_summary_bytes: Option<usize>,
}

fn default_max_depth() -> u64 {
    1
}

fn default_max_resumable() -> usize {
    8
}

#[derive(Debug, Clone, Deserialize)]
struct MarkdownRoleFrontmatter {
    description: String,
    #[serde(default)]
    exposure_phase: Option<String>,
    #[serde(default)]
    tools: Option<Vec<String>>,
    #[serde(default)]
    max_iterations: Option<u32>,
    #[serde(default)]
    timeout_ms: Option<u64>,
    #[serde(default)]
    max_summary_bytes: Option<usize>,
}

#[derive(Debug, Default)]
struct ResumableStore {
    snapshots: HashMap<String, ResumableSnapshot>,
    clock: u64,
}

#[derive(Debug, Clone)]
struct ResumableSnapshot {
    session_id: SessionId,
    role_name: String,
    history: Vec<CanonicalMessage>,
    last_used: u64,
}

impl ResumableStore {
    fn get(&mut self, task_id: &str) -> Option<ResumableSnapshot> {
        self.clock = self.clock.saturating_add(1);
        let snapshot = self.snapshots.get_mut(task_id)?;
        snapshot.last_used = self.clock;
        Some(snapshot.clone())
    }

    fn save(
        &mut self,
        key: String,
        session_id: SessionId,
        role_name: String,
        history: Vec<CanonicalMessage>,
        max_resumable: usize,
    ) -> bool {
        if max_resumable == 0 {
            self.snapshots.remove(&key);
            return false;
        }

        self.clock = self.clock.saturating_add(1);
        self.snapshots.insert(
            key,
            ResumableSnapshot {
                session_id,
                role_name,
                history,
                last_used: self.clock,
            },
        );
        while self.snapshots.len() > max_resumable {
            let Some(evicted_key) = self
                .snapshots
                .iter()
                .min_by_key(|(_, snapshot)| snapshot.last_used)
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            self.snapshots.remove(&evicted_key);
        }
        true
    }
}

#[derive(Debug)]
pub struct SequentialSubagentRunner {
    roles: Vec<SubagentRoleSpec>,
    max_depth: u64,
    max_resumable: usize,
    resumable: Mutex<ResumableStore>,
}

impl SequentialSubagentRunner {
    /// Строит runner из значения `module_config.subagent.sequential`.
    /// `Null` (конфига нет) — валидно: ролей нет, делегирование выключено.
    pub fn from_config(config: Value) -> Result<Self> {
        let cwd = std::env::current_dir().context("failed to resolve process cwd")?;
        Self::from_config_with_cwd(config, &cwd)
    }

    pub fn from_config_with_cwd(config: Value, cwd: &Path) -> Result<Self> {
        let parsed: SequentialSubagentConfig = if config.is_null() {
            SequentialSubagentConfig::default()
        } else {
            serde_json::from_value(config)
                .context("failed to parse module_config.subagent.sequential")?
        };

        let mut role_configs = parsed.roles;
        if let Some(roles_dir) = parsed.roles_dir.as_ref() {
            role_configs.extend(load_markdown_roles(roles_dir, cwd)?);
        }

        let mut roles: Vec<SubagentRoleSpec> = Vec::with_capacity(role_configs.len());
        for role in role_configs {
            if role.name.trim().is_empty() {
                bail!("subagent role name must not be empty");
            }
            if roles.iter().any(|existing| existing.name == role.name) {
                bail!("duplicate subagent role: {}", role.name);
            }
            let mut limits = SubagentLimits::default();
            if let Some(max_iterations) = role.max_iterations {
                limits.max_iterations = max_iterations;
            }
            limits.timeout_ms = role.timeout_ms;
            limits.max_summary_bytes = role.max_summary_bytes;
            let mut spec =
                SubagentRoleSpec::new(role.name, role.description, role.prompt).with_limits(limits);
            if let Some(phase) = role.exposure_phase {
                spec = spec.with_exposure_phase(phase);
            }
            if let Some(tools) = role.tools {
                spec = spec.with_config(json!({ "tools": tools }));
            }
            roles.push(spec);
        }

        Ok(Self {
            roles,
            max_depth: parsed.max_depth,
            max_resumable: parsed.max_resumable,
            resumable: Mutex::new(ResumableStore::default()),
        })
    }

    fn resumable_snapshot(&self, task_id: &str) -> Result<Option<ResumableSnapshot>> {
        Ok(self
            .resumable
            .lock()
            .map_err(|_| anyhow!("subagent resumable store lock poisoned"))?
            .get(task_id))
    }

    fn save_resumable_snapshot(
        &self,
        child_thread_id: ThreadId,
        session_id: SessionId,
        role_name: String,
        history: Vec<CanonicalMessage>,
    ) -> Result<bool> {
        Ok(self
            .resumable
            .lock()
            .map_err(|_| anyhow!("subagent resumable store lock poisoned"))?
            .save(
                child_thread_id.to_string(),
                session_id,
                role_name,
                history,
                self.max_resumable,
            ))
    }
}

fn load_markdown_roles(roles_dir: &Path, cwd: &Path) -> Result<Vec<SequentialRoleConfig>> {
    let dir = if roles_dir.is_absolute() {
        roles_dir.to_path_buf()
    } else {
        cwd.join(roles_dir)
    };
    let mut markdown_files = Vec::new();
    for entry in fs::read_dir(&dir)
        .with_context(|| format!("failed to read subagent roles_dir {}", dir.display()))?
    {
        let entry = entry.with_context(|| format!("failed to read entry in {}", dir.display()))?;
        let path = entry.path();
        if path.is_file() && path.extension().is_some_and(|extension| extension == "md") {
            markdown_files.push(path);
        }
    }
    markdown_files.sort();

    markdown_files
        .into_iter()
        .map(|path| {
            parse_markdown_role(&path)
                .with_context(|| format!("failed to parse subagent role file {}", path.display()))
        })
        .collect()
}

fn parse_markdown_role(path: &Path) -> Result<SequentialRoleConfig> {
    let name = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|name| !name.trim().is_empty())
        .ok_or_else(|| anyhow!("markdown role file name must be valid UTF-8 with non-empty stem"))?
        .to_owned();
    let content = fs::read_to_string(path)
        .with_context(|| format!("failed to read markdown role file {}", path.display()))?;
    let mut lines = content.lines();
    let Some(first) = lines.next() else {
        bail!("missing YAML frontmatter");
    };
    if first.trim() != "---" {
        bail!("missing opening YAML frontmatter marker");
    }

    let mut yaml = String::new();
    let mut body = Vec::new();
    let mut closed = false;
    for line in lines.by_ref() {
        if line.trim() == "---" {
            closed = true;
            break;
        }
        yaml.push_str(line);
        yaml.push('\n');
    }
    if !closed {
        bail!("missing closing YAML frontmatter marker");
    }
    body.extend(lines);

    let frontmatter: MarkdownRoleFrontmatter =
        serde_yaml::from_str(&yaml).context("invalid YAML frontmatter")?;
    if frontmatter.description.trim().is_empty() {
        bail!("markdown role description must not be empty");
    }

    Ok(SequentialRoleConfig {
        name,
        description: frontmatter.description,
        prompt: body.join("\n").trim().to_owned(),
        exposure_phase: frontmatter.exposure_phase,
        tools: frontmatter.tools,
        max_iterations: frontmatter.max_iterations,
        timeout_ms: frontmatter.timeout_ms,
        max_summary_bytes: frontmatter.max_summary_bytes,
    })
}

#[async_trait]
impl SubagentRunner for SequentialSubagentRunner {
    fn roles(&self) -> Vec<SubagentRoleSpec> {
        self.roles.clone()
    }

    async fn run(&self, request: SubagentRequest, ctx: RuntimeContext) -> Result<SubagentResult> {
        let role = self
            .roles
            .iter()
            .find(|role| role.name == request.role)
            .cloned()
            .ok_or_else(|| anyhow!("unknown subagent role: {}", request.role))?;

        let depth = request
            .metadata
            .get("subagent_depth")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        if depth >= self.max_depth {
            bail!(
                "subagent depth limit reached (depth {depth}, max_depth {})",
                self.max_depth
            );
        }

        let resume_task_id = request.metadata.get("task_id").and_then(Value::as_str);
        let (child_thread_id, history) = if let Some(task_id) = resume_task_id {
            let snapshot = self
                .resumable_snapshot(task_id)?
                .ok_or_else(|| anyhow!("unknown task_id (expired or from another session)"))?;
            if snapshot.session_id != ctx.session_id {
                bail!("unknown task_id (expired or from another session)");
            }
            if snapshot.role_name != request.role {
                bail!(
                    "task_id belongs to subagent role {}, but request role is {}",
                    snapshot.role_name,
                    request.role
                );
            }
            let child_thread_id = task_id
                .parse::<ThreadId>()
                .with_context(|| format!("invalid task_id for resumable subagent: {task_id}"))?;
            let mut history = snapshot.history;
            history.push(CanonicalMessage::text(
                MessageRole::User,
                request.prompt.clone(),
            ));
            (child_thread_id, history)
        } else {
            (
                new_thread_id(),
                vec![
                    CanonicalMessage::text(MessageRole::System, role.prompt.clone()),
                    CanonicalMessage::text(MessageRole::User, request.prompt.clone()),
                ],
            )
        };
        let mut child_ctx = ctx.clone();
        child_ctx.thread_id = child_thread_id;

        // Started/Finished эмитятся под родительским thread_id; события
        // самого цикла (tool calls) — под child_thread_id через child_ctx.
        ctx.emit(Event::SubagentStarted {
            role: role.name.clone(),
            description: request.description.clone(),
            child_thread_id,
        })
        .await?;

        // Ребёнок не наследует родительские ctx.instructions: system-слой —
        // только prompt роли.
        let mut state = ChildLoopState {
            history,
            iterations: 0,
            last_text: None,
            usage: None,
        };

        let body_result: Result<(SubagentResult, String)> = async {
            let orchestrator = ToolOrchestrator::default();
            let tools = select_child_tools(&child_ctx, &orchestrator, &request, &role).await?;

            let status = match role.limits.timeout_ms {
                Some(timeout_ms) => {
                    match timeout(
                        Duration::from_millis(timeout_ms),
                        run_child_loop(
                            &role,
                            &request,
                            &child_ctx,
                            &orchestrator,
                            &tools,
                            &mut state,
                        ),
                    )
                    .await
                    {
                        Ok(status) => status?,
                        Err(_elapsed) => SubagentStatus::TimedOut,
                    }
                }
                None => {
                    run_child_loop(
                        &role,
                        &request,
                        &child_ctx,
                        &orchestrator,
                        &tools,
                        &mut state,
                    )
                    .await?
                }
            };

            let status_label = subagent_status_label(status);
            let mut summary = state.last_text.clone().unwrap_or_default();
            if let Some(max_bytes) = role.limits.max_summary_bytes {
                summary = truncate_at_char_boundary(summary, max_bytes);
            }

            let mut result = SubagentResult::new(summary, status, state.iterations)
                .with_child_thread_id(child_thread_id)
                .with_metadata(json!({ "resumable": false }));
            if let Some(usage) = state.usage.clone() {
                result = result.with_usage(usage);
            }
            Ok((result, status_label))
        }
        .await;

        match body_result {
            Ok((mut result, status)) => {
                let resumable = matches!(
                    result.status,
                    SubagentStatus::Completed | SubagentStatus::MaxIterationsReached
                ) && self.save_resumable_snapshot(
                    child_thread_id,
                    ctx.session_id,
                    role.name.clone(),
                    state.history.clone(),
                )?;
                result.metadata = json!({ "resumable": resumable });
                ctx.emit(Event::SubagentFinished {
                    role: role.name.clone(),
                    status,
                    iterations: state.iterations,
                    child_thread_id,
                })
                .await?;
                Ok(result)
            }
            Err(error) => {
                let _ = ctx
                    .emit(Event::SubagentFinished {
                        role: role.name.clone(),
                        status: "errored".into(),
                        iterations: state.iterations,
                        child_thread_id,
                    })
                    .await;
                Err(error)
            }
        }
    }
}

struct ChildLoopState {
    history: Vec<CanonicalMessage>,
    iterations: u32,
    last_text: Option<String>,
    usage: Option<TokenUsage>,
}

/// Отбор tools ребёнка: сперва policy-видимость (тот же гейт, что
/// `visible_tool_specs` у workflow host), затем `ToolExposure::select` с
/// фазой роли. Tool `task` выкидывается из итогового списка.
async fn select_child_tools(
    ctx: &RuntimeContext,
    orchestrator: &ToolOrchestrator,
    request: &SubagentRequest,
    role: &SubagentRoleSpec,
) -> Result<Vec<ToolSpec>> {
    let candidates = orchestrator.visible_tool_specs(ctx, &request.task.cwd);
    let exposure_request =
        ToolExposureRequest::new(request.task.clone()).with_phase(role.effective_exposure_phase());
    let output = ctx
        .tool_exposure
        .select(ToolExposureInput::new(exposure_request, candidates))
        .await?;
    Ok(apply_child_tool_filters(output.tools, role))
}

fn apply_child_tool_filters(tools: Vec<ToolSpec>, role: &SubagentRoleSpec) -> Vec<ToolSpec> {
    let allowlist = role
        .config
        .get("tools")
        .and_then(Value::as_array)
        .map(|tools| tools.iter().filter_map(Value::as_str).collect::<Vec<_>>());

    tools
        .into_iter()
        .filter(|spec| spec.name != TASK_TOOL_NAME)
        .filter(|spec| match &allowlist {
            Some(allowed) => allowed.iter().any(|name| *name == spec.name),
            None => true,
        })
        .collect()
}

async fn run_child_loop(
    role: &SubagentRoleSpec,
    request: &SubagentRequest,
    ctx: &RuntimeContext,
    orchestrator: &ToolOrchestrator,
    tools: &[ToolSpec],
    state: &mut ChildLoopState,
) -> Result<SubagentStatus> {
    for _ in 0..role.limits.max_iterations {
        if ctx.is_cancelled() {
            return Ok(SubagentStatus::Cancelled);
        }

        // Delta-события ModelService эмитятся с контекстом родительского
        // хода (set_event_context ставит runtime до запуска workflow), то
        // есть стрим ребёнка утёк бы в родительский транскрипт как обычный
        // AssistantTextDelta и «переписался» финальным текстом родителя в
        // конце хода. Пока у карточки субагента нет собственного stream-slot,
        // дельты ребёнка глушим — итог приходит через SubagentResult.
        let model_request =
            CanonicalModelRequest::new(ctx.model_ref.clone(), state.history.clone())
                .with_tools(tools.to_vec())
                .with_reasoning(ctx.reasoning.clone())
                .with_metadata(json!({ "suppress_stream_deltas": true }));
        let response = match complete_model(ctx, model_request).await {
            Ok(response) => response,
            Err(_) if ctx.is_cancelled() => return Ok(SubagentStatus::Cancelled),
            Err(error) => return Err(error),
        };
        state.iterations += 1;
        accumulate_usage(&mut state.usage, response.usage.as_ref());
        if let Some(text) = message_text(&response.message) {
            state.last_text = Some(text);
        }
        state.history.push(response.message.clone());

        if response.tool_calls.is_empty() {
            return Ok(SubagentStatus::Completed);
        }

        for call in response.tool_calls {
            if ctx.is_cancelled() {
                return Ok(SubagentStatus::Cancelled);
            }
            let result = match orchestrator.execute(ctx, &request.task, call).await {
                Ok(result) => result,
                Err(_) if ctx.is_cancelled() => return Ok(SubagentStatus::Cancelled),
                Err(error) => return Err(error),
            };
            let call_id = result.call_id.clone();
            state.history.push(
                CanonicalMessage::new(MessageRole::Tool, vec![ContentPart::ToolResult { result }])
                    .with_tool_call_id(call_id),
            );
        }
    }
    Ok(SubagentStatus::MaxIterationsReached)
}

/// Model call с таймаутом родительского runtime и отменой через
/// `ctx.cancellation` — тот же контур, что у workflow plugin host.
async fn complete_model(
    ctx: &RuntimeContext,
    request: CanonicalModelRequest,
) -> Result<CanonicalModelResponse> {
    let completion = async {
        if ctx.model_timeout_ms == 0 {
            ctx.model.complete(request).await
        } else {
            timeout(
                Duration::from_millis(ctx.model_timeout_ms),
                ctx.model.complete(request),
            )
            .await
            .map_err(|_| anyhow!("model request timed out after {}ms", ctx.model_timeout_ms))?
        }
    };
    tokio::select! {
        result = completion => result,
        _ = ctx.cancellation.cancelled() => Err(anyhow!("turn canceled by client")),
    }
}

fn message_text(message: &CanonicalMessage) -> Option<String> {
    let text = message
        .parts
        .iter()
        .filter_map(|part| match part {
            ContentPart::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    (!text.trim().is_empty()).then_some(text)
}

fn accumulate_usage(total: &mut Option<TokenUsage>, usage: Option<&TokenUsage>) {
    let Some(usage) = usage else {
        return;
    };
    match total {
        None => *total = Some(usage.clone()),
        Some(total) => {
            total.input_tokens = total.input_tokens.saturating_add(usage.input_tokens);
            total.output_tokens = total.output_tokens.saturating_add(usage.output_tokens);
            total.cached_input_tokens =
                sum_optional_tokens(total.cached_input_tokens, usage.cached_input_tokens);
            total.cache_creation_input_tokens = sum_optional_tokens(
                total.cache_creation_input_tokens,
                usage.cache_creation_input_tokens,
            );
            total.reasoning_output_tokens =
                sum_optional_tokens(total.reasoning_output_tokens, usage.reasoning_output_tokens);
        }
    }
}

fn sum_optional_tokens(left: Option<u32>, right: Option<u32>) -> Option<u32> {
    match (left, right) {
        (None, None) => None,
        (left, right) => Some(left.unwrap_or(0).saturating_add(right.unwrap_or(0))),
    }
}

/// Обрезает строку по границе char так, чтобы результат был <= `max_bytes`.
fn truncate_at_char_boundary(mut text: String, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text;
    }
    let mut end = max_bytes;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
    text
}

/// snake_case строка статуса для `Event::SubagentFinished` — через serde,
/// чтобы не дублировать rename_all руками.
fn subagent_status_label(status: SubagentStatus) -> String {
    serde_json::to_value(status)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| format!("{status:?}"))
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex as StdMutex};

    use serde_json::json;

    use super::*;
    use crate::{
        contracts::{
            EventEmitter, ModelClient, PolicyContext, PolicyVisibilityContext, ToolRegistry,
        },
        core::{HeadlessApprovalTransport, HeadlessUserInputTransport, InMemoryEventStore},
        domain::{
            AgentTask, ModelRef, PolicyDecision, ReasoningConfig, ToolCall, ToolSafety,
            new_session_id, new_thread_id, new_turn_id,
        },
        stubs::{
            AllVisibleToolExposure, EmptyContextBuilder, FakeModelClient, NoCompactor, NoMemory,
            NoSubagent, NullPatchApplier, NullSearch,
        },
        tools::RememberFactTool,
    };

    struct AllowAllPolicy;

    impl crate::contracts::ApprovalPolicy for AllowAllPolicy {
        fn evaluate(&self, _call: &ToolCall, _ctx: &PolicyContext) -> PolicyDecision {
            PolicyDecision::Allow
        }

        fn evaluate_visibility(&self, _ctx: &PolicyVisibilityContext) -> PolicyDecision {
            PolicyDecision::Allow
        }
    }

    struct FailingModelClient;

    #[async_trait]
    impl ModelClient for FailingModelClient {
        fn id(&self) -> std::borrow::Cow<'static, str> {
            std::borrow::Cow::Borrowed("failing")
        }

        fn capabilities(&self, model: &ModelRef) -> crate::model_standard::ModelCapabilities {
            FakeModelClient::default().capabilities(model)
        }

        async fn complete(
            &self,
            _request: CanonicalModelRequest,
        ) -> Result<CanonicalModelResponse> {
            Err(anyhow!("model boom"))
        }

        async fn stream(
            &self,
            _request: CanonicalModelRequest,
        ) -> Result<
            std::pin::Pin<
                Box<
                    dyn futures_util::Stream<Item = Result<crate::model_standard::ModelStreamEvent>>
                        + Send,
                >,
            >,
        > {
            Err(anyhow!("model stream boom"))
        }
    }

    #[derive(Default)]
    struct RecordingFakeModelClient {
        inner: FakeModelClient,
        histories: StdMutex<Vec<Vec<CanonicalMessage>>>,
        metadatas: StdMutex<Vec<Value>>,
    }

    impl RecordingFakeModelClient {
        fn histories(&self) -> Vec<Vec<CanonicalMessage>> {
            self.histories.lock().expect("histories lock").clone()
        }

        fn metadatas(&self) -> Vec<Value> {
            self.metadatas.lock().expect("metadatas lock").clone()
        }
    }

    #[async_trait]
    impl ModelClient for RecordingFakeModelClient {
        fn id(&self) -> std::borrow::Cow<'static, str> {
            self.inner.id()
        }

        fn capabilities(&self, model: &ModelRef) -> crate::model_standard::ModelCapabilities {
            self.inner.capabilities(model)
        }

        async fn complete(&self, request: CanonicalModelRequest) -> Result<CanonicalModelResponse> {
            self.histories
                .lock()
                .expect("histories lock")
                .push(request.messages.clone());
            self.metadatas
                .lock()
                .expect("metadatas lock")
                .push(request.metadata.clone());
            self.inner.complete(request).await
        }

        async fn stream(
            &self,
            request: CanonicalModelRequest,
        ) -> Result<
            std::pin::Pin<
                Box<
                    dyn futures_util::Stream<Item = Result<crate::model_standard::ModelStreamEvent>>
                        + Send,
                >,
            >,
        > {
            self.inner.stream(request).await
        }
    }

    fn test_runtime_context(events: Arc<InMemoryEventStore>) -> RuntimeContext {
        test_runtime_context_with_model(events, Arc::new(FakeModelClient::default()))
    }

    fn test_runtime_context_with_model<M>(
        events: Arc<InMemoryEventStore>,
        model: Arc<M>,
    ) -> RuntimeContext
    where
        M: ModelClient + 'static,
    {
        let mut tools = ToolRegistry::new();
        tools
            .register(RememberFactTool::new(Arc::new(NoMemory)))
            .expect("register remember_fact");
        RuntimeContext::new(
            new_session_id(),
            new_thread_id(),
            new_turn_id(),
            ModelRef::new("fake", "fake-tool-model"),
            ReasoningConfig::default(),
            120_000,
            30_000,
            Arc::new(EventEmitter::new(events)),
            model,
            Arc::new(NullSearch),
            Arc::new(NoMemory),
            Arc::new(EmptyContextBuilder),
            tools,
            Arc::new(AllowAllPolicy),
            Arc::new(HeadlessApprovalTransport),
            Arc::new(HeadlessUserInputTransport),
            Arc::new(NullPatchApplier),
            Arc::new(NoCompactor),
            Arc::new(AllVisibleToolExposure),
            Arc::new(NoSubagent),
        )
    }

    fn explorer_config() -> Value {
        json!({
            "roles": [
                {
                    "name": "explore",
                    "description": "Read-only exploration",
                    "prompt": "You are a read-only explorer.",
                    "max_iterations": 15,
                    "timeout_ms": 60000,
                    "max_summary_bytes": 2048,
                    "exposure_phase": "explore_phase",
                    "tools": ["remember_fact"]
                },
                {
                    "name": "reviewer",
                    "description": "Review changes",
                    "prompt": "You review diffs."
                }
            ],
            "max_depth": 2
        })
    }

    #[test]
    fn parses_roles_and_limits_from_config() {
        let runner = SequentialSubagentRunner::from_config(explorer_config()).unwrap();
        let roles = runner.roles();

        assert_eq!(roles.len(), 2);
        let explore = &roles[0];
        assert_eq!(explore.name, "explore");
        assert_eq!(explore.description, "Read-only exploration");
        assert_eq!(explore.prompt, "You are a read-only explorer.");
        assert_eq!(explore.limits.max_iterations, 15);
        assert_eq!(explore.limits.timeout_ms, Some(60000));
        assert_eq!(explore.limits.max_summary_bytes, Some(2048));
        assert_eq!(explore.effective_exposure_phase(), "explore_phase");
        assert_eq!(
            explore
                .config
                .get("tools")
                .and_then(Value::as_array)
                .unwrap(),
            &vec![json!("remember_fact")]
        );

        let reviewer = &roles[1];
        assert_eq!(
            reviewer.limits.max_iterations,
            SubagentLimits::default().max_iterations
        );
        assert_eq!(reviewer.limits.timeout_ms, None);
        assert_eq!(reviewer.effective_exposure_phase(), "subagent:reviewer");
        assert_eq!(runner.max_depth, 2);
    }

    #[test]
    fn missing_config_means_no_roles() {
        let runner = SequentialSubagentRunner::from_config(Value::Null).unwrap();
        assert!(runner.roles().is_empty());
        assert_eq!(runner.max_depth, 1);
    }

    #[test]
    fn duplicate_role_names_are_rejected() {
        let error = SequentialSubagentRunner::from_config(json!({
            "roles": [
                { "name": "explore", "description": "a", "prompt": "p" },
                { "name": "explore", "description": "b", "prompt": "p" }
            ]
        }))
        .unwrap_err();
        assert!(error.to_string().contains("duplicate subagent role"));
    }

    #[test]
    fn markdown_role_is_loaded_from_roles_dir() {
        let workspace = tempfile::tempdir().expect("workspace");
        let roles_dir = workspace.path().join("roles");
        std::fs::create_dir(&roles_dir).expect("roles dir");
        std::fs::write(
            roles_dir.join("markdown.md"),
            "---\n\
description: Markdown role\n\
exposure_phase: md_phase\n\
tools:\n\
  - remember_fact\n\
max_iterations: 3\n\
timeout_ms: 42\n\
max_summary_bytes: 7\n\
---\n\
\n\
Markdown prompt.\n",
        )
        .expect("role file");

        let runner = SequentialSubagentRunner::from_config_with_cwd(
            json!({ "roles_dir": "roles" }),
            workspace.path(),
        )
        .unwrap();
        let roles = runner.roles();

        assert_eq!(roles.len(), 1);
        let role = &roles[0];
        assert_eq!(role.name, "markdown");
        assert_eq!(role.description, "Markdown role");
        assert_eq!(role.prompt, "Markdown prompt.");
        assert_eq!(role.limits.max_iterations, 3);
        assert_eq!(role.limits.timeout_ms, Some(42));
        assert_eq!(role.limits.max_summary_bytes, Some(7));
        assert_eq!(role.effective_exposure_phase(), "md_phase");
        assert_eq!(
            role.config.get("tools").and_then(Value::as_array).unwrap(),
            &vec![json!("remember_fact")]
        );
    }

    #[test]
    fn invalid_markdown_frontmatter_is_rejected_with_file_name() {
        let workspace = tempfile::tempdir().expect("workspace");
        let roles_dir = workspace.path().join("roles");
        std::fs::create_dir(&roles_dir).expect("roles dir");
        std::fs::write(
            roles_dir.join("bad.md"),
            "---\ndescription: [\n---\nPrompt\n",
        )
        .expect("role file");

        let error = SequentialSubagentRunner::from_config_with_cwd(
            json!({ "roles_dir": "roles" }),
            workspace.path(),
        )
        .unwrap_err();

        assert!(error.to_string().contains("bad.md"), "{error:#}");
    }

    #[test]
    fn markdown_role_duplicate_with_inline_role_is_rejected() {
        let workspace = tempfile::tempdir().expect("workspace");
        let roles_dir = workspace.path().join("roles");
        std::fs::create_dir(&roles_dir).expect("roles dir");
        std::fs::write(
            roles_dir.join("explore.md"),
            "---\ndescription: Markdown role\n---\nMarkdown prompt\n",
        )
        .expect("role file");

        let error = SequentialSubagentRunner::from_config_with_cwd(
            json!({
                "roles": [
                    { "name": "explore", "description": "Inline", "prompt": "prompt" }
                ],
                "roles_dir": "roles"
            }),
            workspace.path(),
        )
        .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("duplicate subagent role: explore"),
            "{error:#}"
        );
    }

    #[tokio::test]
    async fn depth_limit_is_enforced() {
        let runner = SequentialSubagentRunner::from_config(explorer_config()).unwrap();
        let events = Arc::new(InMemoryEventStore::new());
        let ctx = test_runtime_context(events);
        let cwd = tempfile::tempdir().expect("workspace");
        let task = AgentTask::new("explore", cwd.path().to_path_buf());

        let request = SubagentRequest::new("explore", "look around", task)
            .with_metadata(json!({ "subagent_depth": 2 }));
        let error = runner.run(request, ctx).await.unwrap_err();

        assert!(
            error.to_string().contains("subagent depth limit reached"),
            "{error:#}"
        );
    }

    #[tokio::test]
    async fn unknown_role_is_rejected() {
        let runner = SequentialSubagentRunner::from_config(explorer_config()).unwrap();
        let events = Arc::new(InMemoryEventStore::new());
        let ctx = test_runtime_context(events);
        let cwd = tempfile::tempdir().expect("workspace");
        let task = AgentTask::new("explore", cwd.path().to_path_buf());

        let error = runner
            .run(SubagentRequest::new("mystery", "look", task), ctx)
            .await
            .unwrap_err();

        assert!(
            error.to_string().contains("unknown subagent role: mystery"),
            "{error:#}"
        );
    }

    #[test]
    fn summary_truncation_respects_char_boundaries() {
        // "й" — 2 байта; лимит 5 байт должен резать по границе char (4 байта).
        let text = "й".repeat(4);
        let truncated = truncate_at_char_boundary(text, 5);
        assert_eq!(truncated, "йй");
        assert!(truncated.len() <= 5);
        assert!(truncated.is_char_boundary(truncated.len()));

        // Строка короче лимита не меняется.
        assert_eq!(truncate_at_char_boundary("abc".to_owned(), 5), "abc");
    }

    #[test]
    fn usage_accumulation_sums_option_fields() {
        let mut total = None;
        accumulate_usage(&mut total, Some(&TokenUsage::new(10, 2)));
        accumulate_usage(
            &mut total,
            Some(
                &TokenUsage::new(5, 3)
                    .with_cached_input_tokens(Some(4))
                    .with_reasoning_output_tokens(Some(7)),
            ),
        );
        accumulate_usage(&mut total, None);

        let total = total.expect("usage accumulated");
        assert_eq!(total.input_tokens, 15);
        assert_eq!(total.output_tokens, 5);
        assert_eq!(total.cached_input_tokens, Some(4));
        assert_eq!(total.cache_creation_input_tokens, None);
        assert_eq!(total.reasoning_output_tokens, Some(7));
    }

    #[test]
    fn status_label_is_snake_case() {
        assert_eq!(
            subagent_status_label(SubagentStatus::Completed),
            "completed"
        );
        assert_eq!(
            subagent_status_label(SubagentStatus::MaxIterationsReached),
            "max_iterations_reached"
        );
        assert_eq!(subagent_status_label(SubagentStatus::TimedOut), "timed_out");
        assert_eq!(
            subagent_status_label(SubagentStatus::Cancelled),
            "cancelled"
        );
    }

    #[test]
    fn role_tools_allowlist_filters_exposed_tools() {
        let role = SubagentRoleSpec::new("explore", "Explore", "prompt")
            .with_config(json!({ "tools": ["remember_fact"] }));
        let tools = apply_child_tool_filters(
            vec![
                ToolSpec::new("remember_fact", "Remember", json!({}), ToolSafety::ReadOnly),
                ToolSpec::new("search", "Search", json!({}), ToolSafety::ReadOnly),
                ToolSpec::new(TASK_TOOL_NAME, "Delegate", json!({}), ToolSafety::ReadOnly),
            ],
            &role,
        );

        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "remember_fact");
    }

    #[tokio::test]
    async fn run_emits_errored_finished_when_model_errors() {
        let runner = SequentialSubagentRunner::from_config(json!({
            "roles": [
                {
                    "name": "explore",
                    "description": "Explore",
                    "prompt": "prompt",
                    "max_iterations": 3
                }
            ]
        }))
        .unwrap();
        let events = Arc::new(InMemoryEventStore::new());
        let ctx = test_runtime_context_with_model(events.clone(), Arc::new(FailingModelClient));
        let parent_thread_id = ctx.thread_id;
        let cwd = tempfile::tempdir().expect("workspace");
        let task = AgentTask::new("explore", cwd.path().to_path_buf());

        let error = runner
            .run(SubagentRequest::new("explore", "look", task), ctx)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("model boom"), "{error:#}");

        let envelopes = events.envelopes().await;
        let started_child_thread_id = envelopes
            .iter()
            .find_map(|envelope| match &envelope.event {
                Event::SubagentStarted {
                    child_thread_id, ..
                } => {
                    assert_eq!(envelope.thread_id, parent_thread_id);
                    Some(*child_thread_id)
                }
                _ => None,
            })
            .expect("SubagentStarted");

        let finished = envelopes
            .iter()
            .find(|envelope| matches!(envelope.event, Event::SubagentFinished { .. }))
            .expect("SubagentFinished");
        assert_eq!(finished.thread_id, parent_thread_id);
        match &finished.event {
            Event::SubagentFinished {
                role,
                status,
                iterations,
                child_thread_id,
            } => {
                assert_eq!(role, "explore");
                assert_eq!(status, "errored");
                assert_eq!(*iterations, 0);
                assert_eq!(*child_thread_id, started_child_thread_id);
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[tokio::test]
    async fn child_loop_runs_tool_call_round_trip_with_fake_model() {
        let runner = SequentialSubagentRunner::from_config(explorer_config()).unwrap();
        let events = Arc::new(InMemoryEventStore::new());
        let ctx = test_runtime_context(events.clone());
        let parent_thread_id = ctx.thread_id;
        let cwd = tempfile::tempdir().expect("workspace");
        let task = AgentTask::new("remember", cwd.path().to_path_buf());

        // FakeModelClient: "remember_fact <text>" → tool call remember_fact,
        // после tool result — финальный текст без tool calls.
        let result = runner
            .run(
                SubagentRequest::new("explore", "remember_fact user prefers tabs", task)
                    .with_description("remember tabs"),
                ctx,
            )
            .await
            .unwrap();

        assert_eq!(result.status, SubagentStatus::Completed);
        assert_eq!(result.iterations, 2);
        assert!(
            result.summary.contains("Fake final answer"),
            "summary: {}",
            result.summary
        );
        let child_thread_id = result.child_thread_id.expect("child thread id");
        assert_ne!(child_thread_id, parent_thread_id);

        let envelopes = events.envelopes().await;
        let started = envelopes
            .iter()
            .find(|envelope| matches!(envelope.event, Event::SubagentStarted { .. }))
            .expect("SubagentStarted");
        assert_eq!(started.thread_id, parent_thread_id);
        match &started.event {
            Event::SubagentStarted {
                role,
                description,
                child_thread_id: event_child,
            } => {
                assert_eq!(role, "explore");
                assert_eq!(description.as_deref(), Some("remember tabs"));
                assert_eq!(*event_child, child_thread_id);
            }
            other => panic!("unexpected event: {other:?}"),
        }

        // Tool события ребёнка идут под child_thread_id.
        let tool_started = envelopes
            .iter()
            .find(|envelope| matches!(envelope.event, Event::ToolCallRequested { .. }))
            .expect("ToolCallRequested");
        assert_eq!(tool_started.thread_id, child_thread_id);

        let finished = envelopes
            .iter()
            .find(|envelope| matches!(envelope.event, Event::SubagentFinished { .. }))
            .expect("SubagentFinished");
        assert_eq!(finished.thread_id, parent_thread_id);
        match &finished.event {
            Event::SubagentFinished {
                role,
                status,
                iterations,
                child_thread_id: event_child,
            } => {
                assert_eq!(role, "explore");
                assert_eq!(status, "completed");
                assert_eq!(*iterations, 2);
                assert_eq!(*event_child, child_thread_id);
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[tokio::test]
    async fn child_loop_model_requests_suppress_stream_deltas() {
        let runner = SequentialSubagentRunner::from_config(explorer_config()).unwrap();
        let events = Arc::new(InMemoryEventStore::new());
        let model = Arc::new(RecordingFakeModelClient::default());
        let ctx = test_runtime_context_with_model(events, model.clone());
        let cwd = tempfile::tempdir().expect("workspace");
        let task = AgentTask::new("explore", cwd.path().to_path_buf());

        runner
            .run(SubagentRequest::new("explore", "look around", task), ctx)
            .await
            .unwrap();

        // Delta-контекст ModelService указывает на родительский ход: без
        // подавления стрим ребёнка утёк бы в родительский транскрипт.
        let metadatas = model.metadatas();
        assert!(!metadatas.is_empty());
        assert!(
            metadatas
                .iter()
                .all(|metadata| metadata["suppress_stream_deltas"] == json!(true))
        );
    }

    #[tokio::test]
    async fn resumable_task_id_round_trips_history_and_thread_id() {
        let runner = SequentialSubagentRunner::from_config(json!({
            "roles": [
                {
                    "name": "explore",
                    "description": "Explore",
                    "prompt": "prompt",
                    "max_iterations": 5,
                    "tools": ["remember_fact"]
                }
            ]
        }))
        .unwrap();
        let events = Arc::new(InMemoryEventStore::new());
        let model = Arc::new(RecordingFakeModelClient::default());
        let ctx = test_runtime_context_with_model(events.clone(), model.clone());
        let parent_thread_id = ctx.thread_id;
        let cwd = tempfile::tempdir().expect("workspace");
        let task = AgentTask::new("explore", cwd.path().to_path_buf());

        let first = runner
            .run(
                SubagentRequest::new("explore", "first prompt", task.clone()),
                ctx.clone(),
            )
            .await
            .unwrap();
        assert_eq!(first.status, SubagentStatus::Completed);
        assert_eq!(first.iterations, 1);
        assert_eq!(first.metadata["resumable"], json!(true));
        let child_thread_id = first.child_thread_id.expect("child thread id");

        let second = runner
            .run(
                SubagentRequest::new("explore", "remember_fact resumed fact", task)
                    .with_metadata(json!({ "task_id": child_thread_id.to_string() })),
                ctx,
            )
            .await
            .unwrap();

        assert_eq!(second.child_thread_id, Some(child_thread_id));
        assert_eq!(second.status, SubagentStatus::Completed);
        assert_eq!(second.iterations, 2);
        assert!(
            second
                .summary
                .contains("Fake final answer after tool result")
        );
        assert_eq!(second.metadata["resumable"], json!(true));

        let histories = model.histories();
        assert_eq!(histories.len(), 3);
        assert_eq!(histories[0].len(), 2);
        assert_eq!(histories[1].len(), 4);
        assert_eq!(histories[1][0].role, MessageRole::System);
        assert_eq!(histories[1][1].role, MessageRole::User);
        assert_eq!(histories[1][2].role, MessageRole::Assistant);
        assert_eq!(histories[1][3].role, MessageRole::User);

        let envelopes = events.envelopes().await;
        let started_with_child = envelopes
            .iter()
            .filter(|envelope| match &envelope.event {
                Event::SubagentStarted {
                    child_thread_id: event_child,
                    ..
                } => {
                    assert_eq!(envelope.thread_id, parent_thread_id);
                    *event_child == child_thread_id
                }
                _ => false,
            })
            .count();
        assert_eq!(started_with_child, 2);
        assert!(
            envelopes.iter().any(|envelope| {
                envelope.thread_id == child_thread_id
                    && matches!(envelope.event, Event::ToolCallRequested { .. })
            }),
            "expected resumed tool event under original child thread"
        );
    }

    #[tokio::test]
    async fn unknown_task_id_is_rejected() {
        let runner = SequentialSubagentRunner::from_config(explorer_config()).unwrap();
        let events = Arc::new(InMemoryEventStore::new());
        let ctx = test_runtime_context(events);
        let cwd = tempfile::tempdir().expect("workspace");
        let task = AgentTask::new("explore", cwd.path().to_path_buf());

        let error = runner
            .run(
                SubagentRequest::new("explore", "look", task)
                    .with_metadata(json!({ "task_id": new_thread_id().to_string() })),
                ctx,
            )
            .await
            .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("unknown task_id (expired or from another session)"),
            "{error:#}"
        );
    }

    #[tokio::test]
    async fn task_id_from_another_session_is_rejected() {
        let runner = SequentialSubagentRunner::from_config(explorer_config()).unwrap();
        let events = Arc::new(InMemoryEventStore::new());
        let ctx = test_runtime_context(events.clone());
        let cwd = tempfile::tempdir().expect("workspace");
        let task = AgentTask::new("explore", cwd.path().to_path_buf());

        let first = runner
            .run(SubagentRequest::new("explore", "first", task.clone()), ctx)
            .await
            .unwrap();
        let task_id = first.child_thread_id.expect("child thread id").to_string();

        let other_ctx = test_runtime_context(events);
        let error = runner
            .run(
                SubagentRequest::new("explore", "second", task)
                    .with_metadata(json!({ "task_id": task_id })),
                other_ctx,
            )
            .await
            .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("unknown task_id (expired or from another session)"),
            "{error:#}"
        );
    }

    #[tokio::test]
    async fn resumable_store_evicts_least_recently_used_snapshot() {
        let runner = SequentialSubagentRunner::from_config(json!({
            "roles": [
                { "name": "explore", "description": "Explore", "prompt": "prompt" }
            ],
            "max_resumable": 1
        }))
        .unwrap();
        let events = Arc::new(InMemoryEventStore::new());
        let ctx = test_runtime_context(events.clone());
        let cwd = tempfile::tempdir().expect("workspace");
        let task = AgentTask::new("explore", cwd.path().to_path_buf());

        let first = runner
            .run(SubagentRequest::new("explore", "first", task.clone()), ctx)
            .await
            .unwrap();
        let first_task_id = first.child_thread_id.expect("child thread id").to_string();

        let ctx = test_runtime_context(events.clone());
        runner
            .run(SubagentRequest::new("explore", "second", task.clone()), ctx)
            .await
            .unwrap();

        let ctx = test_runtime_context(events);
        let error = runner
            .run(
                SubagentRequest::new("explore", "resume first", task)
                    .with_metadata(json!({ "task_id": first_task_id })),
                ctx,
            )
            .await
            .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("unknown task_id (expired or from another session)"),
            "{error:#}"
        );
    }
}
