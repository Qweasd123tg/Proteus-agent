//! Финальное представление canonical [`AgentOutput`].
//!
//! Runtime trait не является внешней ABI-границей. Выбранная реализация
//! вызывается через process adapter, а клиенты могут строить собственные
//! projections из того же canonical output.

use anyhow::Result;

use crate::domain::AgentOutput;

pub trait Renderer: Send + Sync + 'static {
    fn render(&self, output: &AgentOutput) -> Result<String>;
}
