use std::path::PathBuf;

use anyhow::{Result, bail};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{
    contracts::workflow::RuntimeContext,
    domain::{AgentTask, SessionId, ThreadId},
    model_standard::TokenUsage,
};

/// Описание роли субагента.
///
/// Роль — декларативная единица делегирования: системный промпт, фаза
/// tool exposure и лимиты дочернего цикла. Workflow использует список ролей
/// для генерации спеки task-тула (описания ролей вклеиваются в параметр
/// `agent_type`), но сам дочерний цикл исполняет slot `subagent`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct SubagentRoleSpec {
    /// Идентификатор роли ("explore", "reviewer", ...). Модель передаёт его
    /// в task-тул как `agent_type`.
    pub name: String,
    /// Однострочное описание для модели: когда эту роль звать.
    pub description: String,
    /// Системный промпт дочернего цикла.
    pub prompt: String,
    /// Фаза для `ToolExposure::select` при отборе tools ребёнка.
    /// По умолчанию реализация использует `"subagent:<name>"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exposure_phase: Option<String>,
    /// Роль объявлена безопасной для конкурентного запуска рядом с другими
    /// субагентами (обычно строго read-only профиль). Флаг задаёт оператор
    /// в конфиге реализации; consumers (workflow) используют его, чтобы
    /// решить, можно ли исполнять несколько task-вызовов параллельно через
    /// `spawn`/`wait`.
    #[serde(default)]
    pub parallel_safe: bool,
    /// Изоляция рабочей копии для пишущих ролей. `Worktree` — каждый fresh
    /// запуск роли получает собственный git worktree (lifecycle оркестрирует
    /// родительский workflow, подменяя `task.cwd` перед spawn); такая роль
    /// пригодна для конкурентного батча наравне с `parallel_safe`.
    #[serde(default)]
    pub isolation: SubagentIsolation,
    /// Лимиты дочернего цикла.
    #[serde(default)]
    pub limits: SubagentLimits,
    /// Implementation-specific настройки роли (например, model override
    /// в будущих реализациях). Ядро содержимое не интерпретирует.
    #[serde(default)]
    pub config: serde_json::Value,
}

impl SubagentRoleSpec {
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        prompt: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            prompt: prompt.into(),
            exposure_phase: None,
            parallel_safe: false,
            isolation: SubagentIsolation::None,
            limits: SubagentLimits::default(),
            config: serde_json::Value::Null,
        }
    }

    pub fn with_exposure_phase(mut self, phase: impl Into<String>) -> Self {
        self.exposure_phase = Some(phase.into());
        self
    }

    pub fn with_parallel_safe(mut self, parallel_safe: bool) -> Self {
        self.parallel_safe = parallel_safe;
        self
    }

    pub fn with_isolation(mut self, isolation: SubagentIsolation) -> Self {
        self.isolation = isolation;
        self
    }

    pub fn with_limits(mut self, limits: SubagentLimits) -> Self {
        self.limits = limits;
        self
    }

    pub fn with_config(mut self, config: serde_json::Value) -> Self {
        self.config = config;
        self
    }

    /// Эффективная фаза exposure: явная или `"subagent:<name>"`.
    pub fn effective_exposure_phase(&self) -> String {
        self.exposure_phase
            .clone()
            .unwrap_or_else(|| format!("subagent:{}", self.name))
    }
}

/// Изоляция рабочей копии дочернего цикла.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum SubagentIsolation {
    /// Ребёнок работает в `task.cwd` родителя (по умолчанию).
    #[default]
    None,
    /// Каждый fresh запуск получает собственный git worktree.
    Worktree,
}

/// Лимиты дочернего цикла. Реализация обязана останавливать цикл при
/// достижении любого из них и возвращать соответствующий `SubagentStatus`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub struct SubagentLimits {
    /// Максимум итераций модель→tools дочернего цикла.
    pub max_iterations: u32,
    /// Общий таймаут дочернего цикла. None — ограничен только таймаутом
    /// родительского turn'а.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    /// Обрезка итогового summary. None — без обрезки.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_summary_bytes: Option<usize>,
    /// Token-бюджет цикла: потолок суммы `input + output` по всем
    /// model-запросам ребёнка (см. `BudgetTracker`). Реализация обязана
    /// останавливать цикл при превышении и возвращать
    /// `SubagentStatus::TokenBudgetExceeded`. None — безлимит.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_total_tokens: Option<u64>,
}

impl Default for SubagentLimits {
    fn default() -> Self {
        Self {
            max_iterations: 12,
            timeout_ms: None,
            max_summary_bytes: None,
            max_total_tokens: None,
        }
    }
}

/// Запрос на прогон субагента.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct SubagentRequest {
    /// Имя роли из `SubagentRunner::roles`.
    pub role: String,
    /// Задание ребёнку. Единственный контекст, который ребёнок получает
    /// от родителя: история родителя не передаётся.
    pub prompt: String,
    /// Task родителя — источник cwd и контекста для tool execution
    /// и `ToolExposureRequest`.
    pub task: AgentTask,
    /// Короткая метка задачи для событий/UI (3-5 слов).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Каркас для caller-specific данных (например, маркер глубины
    /// вложенности). Ядро содержимое не интерпретирует.
    #[serde(default)]
    pub metadata: serde_json::Value,
}

impl SubagentRequest {
    pub fn new(role: impl Into<String>, prompt: impl Into<String>, task: AgentTask) -> Self {
        Self {
            role: role.into(),
            prompt: prompt.into(),
            task,
            description: None,
            metadata: serde_json::Value::Null,
        }
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn with_metadata(mut self, metadata: serde_json::Value) -> Self {
        self.metadata = metadata;
        self
    }
}

/// Терминальный статус дочернего цикла.
///
/// Ошибки инфраструктуры (модель недоступна, роль не найдена) — через
/// `Err` из `SubagentRunner::run`; статус описывает штатные исходы.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum SubagentStatus {
    /// Ребёнок завершил задачу финальным текстовым ответом.
    Completed,
    /// Достигнут `max_iterations`; summary — последний доступный текст.
    MaxIterationsReached,
    /// Истёк `timeout_ms` роли.
    TimedOut,
    /// Отменён через cancellation родителя.
    Cancelled,
    /// Превышен `max_total_tokens` роли; summary — последний доступный
    /// текст, продолжение возможно через resume по `task_id`.
    TokenBudgetExceeded,
}

/// Результат прогона субагента — единственное, что попадает к родителю.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct SubagentResult {
    /// Финальный текст ребёнка (обрезанный по `max_summary_bytes`).
    pub summary: String,
    pub status: SubagentStatus,
    /// Сколько итераций модель→tools выполнено.
    pub iterations: u32,
    /// ThreadId, под которым эмитились события дочернего цикла.
    /// Клиенты используют его для группировки вложенной активности.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child_thread_id: Option<ThreadId>,
    /// Суммарный token usage дочернего цикла (все model-запросы ребёнка).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<TokenUsage>,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

impl SubagentResult {
    pub fn new(summary: impl Into<String>, status: SubagentStatus, iterations: u32) -> Self {
        Self {
            summary: summary.into(),
            status,
            iterations,
            child_thread_id: None,
            usage: None,
            metadata: serde_json::Value::Null,
        }
    }

    pub fn with_child_thread_id(mut self, thread_id: ThreadId) -> Self {
        self.child_thread_id = Some(thread_id);
        self
    }

    pub fn with_usage(mut self, usage: TokenUsage) -> Self {
        self.usage = Some(usage);
        self
    }

    pub fn with_metadata(mut self, metadata: serde_json::Value) -> Self {
        self.metadata = metadata;
        self
    }
}

/// Запрос workflow-хосту на создание изолированного git worktree для
/// пишущего ребёнка (`SubagentIsolation::Worktree`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct SubagentWorkspaceRequest {
    /// cwd родительской задачи — от него резолвится repo root.
    pub parent_cwd: PathBuf,
    /// Имя workspace (санитизированное): задаёт имя worktree и ветки.
    pub name: String,
}

impl SubagentWorkspaceRequest {
    pub fn new(parent_cwd: PathBuf, name: impl Into<String>) -> Self {
        Self {
            parent_cwd,
            name: name.into(),
        }
    }
}

/// Созданный worktree-workspace ребёнка. Родительский workflow подменяет
/// `task.cwd` на `path` перед spawn и передаёт весь DTO обратно хосту
/// для cleanup.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct WorkspaceInfo {
    /// Root основного checkout (для worktree- и branch-операций cleanup-а).
    pub repo_root: PathBuf,
    /// Путь worktree — новый cwd ребёнка.
    pub path: PathBuf,
    /// Ветка worktree (`proteus/<name>`), которую мержит родитель.
    pub branch: String,
    /// Коммит, от которого создан worktree: cleanup сравнивает с ним HEAD,
    /// чтобы понять «изменений нет».
    pub base_commit: String,
}

impl WorkspaceInfo {
    pub fn new(
        repo_root: PathBuf,
        path: PathBuf,
        branch: impl Into<String>,
        base_commit: impl Into<String>,
    ) -> Self {
        Self {
            repo_root,
            path,
            branch: branch.into(),
            base_commit: base_commit.into(),
        }
    }
}

/// Handle запущенного (`spawn`) дочернего цикла: ключ для `wait`/`cancel`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct SubagentHandle {
    /// Opaque id запуска, уникальный в пределах runner-а. В отличие от
    /// `child_thread_id`, не переиспользуется при resume той же задачи.
    pub spawn_id: String,
    /// Роль запущенного ребёнка.
    pub role: String,
    /// ThreadId, под которым эмитятся события ребёнка (известен сразу).
    pub child_thread_id: ThreadId,
}

impl SubagentHandle {
    pub fn new(
        spawn_id: impl Into<String>,
        role: impl Into<String>,
        child_thread_id: ThreadId,
    ) -> Self {
        Self {
            spawn_id: spawn_id.into(),
            role: role.into(),
            child_thread_id,
        }
    }
}

/// Slot `subagent`: исполнение дочерних агентских циклов с изолированным
/// контекстом.
///
/// Контракт владеет дочерним циклом целиком (модель → tools → модель), не
/// вызывая slot `workflow` — это разрывает цикл зависимостей между слотами.
/// Реализация обязана гонять tool calls ребёнка через тот же
/// policy/approval-контур, что и родительские (безопасность не ослабляется
/// делегированием), и уважать `ctx.cancellation`.
///
/// Исполнение — `run` (запустить и дождаться) либо `spawn`/`wait`/`cancel`
/// (фоновый запуск нескольких детей). `spawn`-путь опционален: реализации
/// без него не должны объявлять роли `parallel_safe`.
#[async_trait]
pub trait SubagentRunner: Send + Sync {
    /// Роли, доступные для делегирования. Пустой список = делегирование
    /// выключено (workflow не генерирует task-тул).
    fn roles(&self) -> Vec<SubagentRoleSpec>;

    /// Whether this implementation owns a working spawn/wait/cancel
    /// lifecycle. False is the safe default for legacy/plugin adapters whose
    /// ABI currently exposes only blocking `run`.
    fn supports_collaboration(&self) -> bool {
        false
    }

    /// Прогоняет дочерний цикл и возвращает результат. `ctx` — контекст
    /// родительского turn'а; реализация сама изолирует ребёнка (свой
    /// thread_id, своя история, свой отбор tools по фазе роли).
    async fn run(&self, request: SubagentRequest, ctx: RuntimeContext) -> Result<SubagentResult>;

    /// Запускает дочерний цикл в фоне и сразу возвращает handle.
    /// `Event::SubagentStarted` эмитится до возврата. Ошибки подготовки
    /// (unknown role, depth limit, невалидный task_id) — через `Err`
    /// отсюда; исход самого цикла забирается через `wait`. Запущенный
    /// ребёнок обязан жить на child-токене `ctx.cancellation`: cancel
    /// родителя каскадится вниз, cancel ребёнка родителя не трогает.
    ///
    /// Default — «не поддерживается»: реализации без фонового запуска
    /// переопределять не обязаны.
    async fn spawn(&self, request: SubagentRequest, ctx: RuntimeContext) -> Result<SubagentHandle> {
        let _ = (request, ctx);
        bail!("this subagent runner does not support spawn/wait/cancel");
    }

    /// Дожидается завершения запущенного ребёнка и отдаёт результат.
    /// Реализации с фоновым control plane должны кешировать terminal result:
    /// отмена future ожидания не отменяет ребёнка и не потребляет handle, так
    /// что `wait` можно повторить. Успешно полученный результат потребляет
    /// handle ровно один раз.
    async fn wait(&self, handle: &SubagentHandle) -> Result<SubagentResult> {
        let _ = handle;
        bail!("this subagent runner does not support spawn/wait/cancel");
    }

    /// Отменяет запущенного ребёнка, не трогая остальных детей и
    /// родительский turn. Результат (обычно `Cancelled` + resumable
    /// snapshot) забирается через `wait`.
    async fn cancel(&self, handle: &SubagentHandle) -> Result<()> {
        let _ = handle;
        bail!("this subagent runner does not support spawn/wait/cancel");
    }
}

/// Узкая capability, которую runtime выдаёт facade-tool `task` на время
/// обычного `Tool::invoke`. Tool не получает весь [`RuntimeContext`] и не
/// знает concrete runner: host сам связывает запрос с текущим thread/turn,
/// policy, cancellation и event emitter.
#[async_trait]
pub trait SubagentToolHost: Send + Sync {
    /// Session owner of model-facing facade calls. Collaboration handles are
    /// scoped to this id and cannot be addressed from another session.
    fn session_id(&self) -> Option<SessionId> {
        None
    }

    async fn run_subagent(&self, request: SubagentRequest) -> Result<SubagentResult>;

    async fn spawn_subagent(&self, request: SubagentRequest) -> Result<SubagentHandle> {
        let _ = request;
        bail!("subagent host does not support collaboration control");
    }

    async fn wait_subagent(&self, handle: &SubagentHandle) -> Result<SubagentResult> {
        let _ = handle;
        bail!("subagent host does not support collaboration control");
    }

    async fn cancel_subagent(&self, handle: &SubagentHandle) -> Result<()> {
        let _ = handle;
        bail!("subagent host does not support collaboration control");
    }
}
