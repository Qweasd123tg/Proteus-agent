use std::{
    collections::HashMap,
    fmt,
    path::PathBuf,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, Ordering},
    },
};

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::Notify;

use crate::{
    contracts::{SubagentToolHost, UserInputTransport},
    domain::{AgentTask, SessionId, ThreadId, ToolCall, ToolResult, ToolSpec, TurnId},
};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(deny_unknown_fields)]
pub struct ToolInvocationOwner {
    pub session_id: SessionId,
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
}

impl ToolInvocationOwner {
    pub fn new(session_id: SessionId, thread_id: ThreadId, turn_id: TurnId) -> Self {
        Self {
            session_id,
            thread_id,
            turn_id,
        }
    }
}

#[derive(Clone)]
pub struct ToolContext {
    pub cwd: PathBuf,
    pub owner: ToolInvocationOwner,
    pub cancellation: CancellationToken,
    pub user_input: Option<Arc<dyn UserInputTransport>>,
    /// Текущий canonical task. Обычным tools достаточно `cwd`; facade-tools,
    /// создающие дочернюю работу, сохраняют исходный task как parent context.
    pub task: Option<AgentTask>,
    /// Runtime-bound capability для facade-tool `task`. Dylib tools её не
    /// получают через свой ABI и не могут вызывать subagent slot напрямую.
    pub subagent: Option<Arc<dyn SubagentToolHost>>,
}

impl ToolContext {
    pub fn new(cwd: PathBuf, owner: ToolInvocationOwner) -> Self {
        Self {
            cwd,
            owner,
            cancellation: CancellationToken::new(),
            user_input: None,
            task: None,
            subagent: None,
        }
    }
}

#[derive(Clone, Default)]
pub struct CancellationToken {
    inner: Arc<CancellationState>,
}

#[derive(Default)]
struct CancellationState {
    cancelled: AtomicBool,
    notify: Notify,
    /// Дочерние токены (`child_token`): cancel родителя каскадится вниз,
    /// cancel ребёнка родителя не трогает. Weak — жизнью ребёнка владеет
    /// его собственный держатель, родитель только доставляет cancel.
    children: Mutex<Vec<Weak<CancellationState>>>,
}

impl fmt::Debug for CancellationToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CancellationToken")
            .field("cancelled", &self.is_cancelled())
            .finish()
    }
}

impl CancellationToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        cancel_state(&self.inner);
    }

    pub fn is_cancelled(&self) -> bool {
        self.inner.cancelled.load(Ordering::SeqCst)
    }

    pub async fn cancelled(&self) {
        if self.is_cancelled() {
            return;
        }
        self.inner.notify.notified().await;
    }

    /// Дочерний токен: отменяется вместе с родителем, но его собственный
    /// `cancel()` не влияет на родителя. Основа per-child cancellation
    /// (например, отмена одного субагента без отмены родительского turn-а).
    pub fn child_token(&self) -> Self {
        let child = Arc::new(CancellationState::default());
        {
            let mut children = self
                .inner
                .children
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            children.retain(|entry| entry.strong_count() > 0);
            children.push(Arc::downgrade(&child));
        }
        // Закрывает гонку: cancel родителя мог пройти до регистрации ребёнка
        // и не увидеть его в списке.
        if self.is_cancelled() {
            cancel_state(&child);
        }
        Self { inner: child }
    }
}

fn cancel_state(state: &Arc<CancellationState>) {
    if state.cancelled.swap(true, Ordering::SeqCst) {
        return;
    }
    state.notify.notify_waiters();
    let children = {
        let mut children = state
            .children
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        std::mem::take(&mut *children)
    };
    for child in children {
        if let Some(child) = child.upgrade() {
            cancel_state(&child);
        }
    }
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn spec(&self) -> ToolSpec;
    async fn invoke(&self, call: &ToolCall, ctx: ToolContext) -> Result<ToolResult>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn cancellation_token_wakes_waiters() {
        let token = CancellationToken::new();
        let waiter = token.clone();
        let (tx, rx) = tokio::sync::oneshot::channel();

        tokio::spawn(async move {
            waiter.cancelled().await;
            tx.send(()).expect("send wake");
        });

        token.cancel();
        rx.await.expect("waiter should wake");
        assert!(token.is_cancelled());
    }

    #[tokio::test]
    async fn cancellation_token_returns_immediately_after_cancel() {
        let token = CancellationToken::new();
        token.cancel();

        token.cancelled().await;

        assert!(token.is_cancelled());
    }

    #[tokio::test]
    async fn parent_cancel_propagates_to_child_tokens() {
        let parent = CancellationToken::new();
        let child = parent.child_token();
        let grandchild = child.child_token();

        let waiter = grandchild.clone();
        let (tx, rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            waiter.cancelled().await;
            tx.send(()).expect("send wake");
        });

        parent.cancel();
        rx.await.expect("grandchild waiter should wake");
        assert!(child.is_cancelled());
        assert!(grandchild.is_cancelled());
    }

    #[test]
    fn child_cancel_does_not_affect_parent() {
        let parent = CancellationToken::new();
        let child = parent.child_token();
        let sibling = parent.child_token();

        child.cancel();

        assert!(child.is_cancelled());
        assert!(!parent.is_cancelled());
        assert!(!sibling.is_cancelled());
    }

    #[test]
    fn child_token_of_cancelled_parent_is_already_cancelled() {
        let parent = CancellationToken::new();
        parent.cancel();

        let child = parent.child_token();

        assert!(child.is_cancelled());
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ToolSource {
    Builtin { provider: String },
    ProviderHosted { provider: String },
    Config { origin: String },
    Mcp { server: String },
    Dynamic { origin: String },
}

impl ToolSource {
    pub fn builtin(provider: impl Into<String>) -> Self {
        Self::Builtin {
            provider: provider.into(),
        }
    }

    pub fn label(&self) -> String {
        match self {
            Self::Builtin { provider } => format!("builtin:{provider}"),
            Self::ProviderHosted { provider } => format!("provider_hosted:{provider}"),
            Self::Config { origin } => format!("config:{origin}"),
            Self::Mcp { server } => format!("mcp:{server}"),
            Self::Dynamic { origin } => format!("dynamic:{origin}"),
        }
    }
}

#[derive(Clone)]
pub struct ToolEntry {
    pub source: ToolSource,
    pub tool: Arc<dyn Tool>,
}

#[derive(Clone, Default)]
pub struct ToolRegistry {
    tools: HashMap<String, ToolEntry>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register<T>(&mut self, tool: T) -> Result<()>
    where
        T: Tool + 'static,
    {
        self.register_with_source(ToolSource::builtin("core"), tool)
    }

    pub fn register_with_source<T>(&mut self, source: ToolSource, tool: T) -> Result<()>
    where
        T: Tool + 'static,
    {
        self.register_arc(source, Arc::new(tool))
    }

    pub fn register_arc(&mut self, source: ToolSource, tool: Arc<dyn Tool>) -> Result<()> {
        let spec = tool.spec();
        if let Some(existing) = self.tools.get(&spec.name) {
            return Err(anyhow!(
                "duplicate tool registration: {} from {} conflicts with {}",
                spec.name,
                source.label(),
                existing.source.label()
            ));
        }
        self.tools.insert(spec.name, ToolEntry { source, tool });
        Ok(())
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.get(name).map(|entry| entry.tool.clone())
    }

    pub fn entry(&self, name: &str) -> Option<ToolEntry> {
        self.tools.get(name).cloned()
    }

    pub fn specs(&self) -> Vec<ToolSpec> {
        let mut entries = self
            .tools
            .values()
            .map(|entry| (entry.tool.spec(), entry.source.label()))
            .collect::<Vec<_>>();
        entries.sort_by(|(left, left_source), (right, right_source)| {
            left.name
                .cmp(&right.name)
                .then_with(|| left_source.cmp(right_source))
        });
        entries.into_iter().map(|(spec, _source)| spec).collect()
    }

    pub fn entries(&self) -> Vec<(ToolSource, ToolSpec)> {
        let mut entries = self
            .tools
            .values()
            .map(|entry| (entry.source.clone(), entry.tool.spec()))
            .collect::<Vec<_>>();
        entries.sort_by(|(left_source, left), (right_source, right)| {
            left.name
                .cmp(&right.name)
                .then_with(|| left_source.label().cmp(&right_source.label()))
        });
        entries
    }

    pub fn spec(&self, name: &str) -> Result<ToolSpec> {
        self.get(name)
            .map(|tool| tool.spec())
            .ok_or_else(|| anyhow!("unknown tool: {name}"))
    }
}
