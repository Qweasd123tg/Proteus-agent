use std::{
    collections::HashMap,
    fmt,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use tokio::sync::Notify;

use crate::domain::{ToolCall, ToolResult, ToolSpec};

#[derive(Debug, Clone)]
pub struct ToolContext {
    pub cwd: PathBuf,
    pub cancellation: CancellationToken,
}

impl ToolContext {
    pub fn new(cwd: PathBuf) -> Self {
        Self {
            cwd,
            cancellation: CancellationToken::new(),
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
        if !self.inner.cancelled.swap(true, Ordering::SeqCst) {
            self.inner.notify.notify_waiters();
        }
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ToolSource {
    Builtin { provider: String },
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
