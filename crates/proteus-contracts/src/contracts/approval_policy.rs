use std::{
    collections::BTreeSet,
    path::PathBuf,
    sync::{Mutex, MutexGuard},
};

use crate::domain::{PolicyDecision, ToolCall, ToolSpec};

/// Turn-scoped permission grants.
///
/// Конвенция approval-gated grants: если tool-вызов прошёл через явный user
/// approval (`PolicyDecision::Ask` → approved) и его успешный результат
/// содержит `metadata.granted_permissions` (массив строк), ядро мержит эти
/// строки в гранты текущего хода. Policy видит их в
/// `PolicyContext::granted_permissions` и может пропускать последующие вызовы
/// без повторного Ask (например, `escalated_exec` для unsandboxed shell).
///
/// Гранты живут только до конца хода: `RuntimeContext` создаётся на каждый
/// ход заново, и вместе с ним обнуляются гранты. Ядро учитывает
/// `granted_permissions` только на approved-пути, поэтому tools, выдающие
/// гранты (`request_permissions`), должны стоять в `ask_before` конфигурации
/// policy — иначе grant не запишется.
#[derive(Debug, Default)]
pub struct TurnPermissionGrants {
    granted: Mutex<BTreeSet<String>>,
}

impl TurnPermissionGrants {
    pub fn grant(&self, permissions: impl IntoIterator<Item = String>) {
        self.lock().extend(permissions);
    }

    /// Отсортированный снимок для передачи в `PolicyContext`/DTO.
    pub fn snapshot(&self) -> Vec<String> {
        self.lock().iter().cloned().collect()
    }

    fn lock(&self) -> MutexGuard<'_, BTreeSet<String>> {
        self.granted
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct PolicyContext {
    pub cwd: PathBuf,
    pub tool_spec: Option<ToolSpec>,
    /// Turn-scoped гранты, выданные через approval-gated tool results
    /// (см. [`TurnPermissionGrants`]).
    pub granted_permissions: Vec<String>,
}

impl PolicyContext {
    pub fn new(cwd: PathBuf, tool_spec: Option<ToolSpec>) -> Self {
        Self {
            cwd,
            tool_spec,
            granted_permissions: Vec::new(),
        }
    }

    pub fn with_granted_permissions(mut self, granted_permissions: Vec<String>) -> Self {
        self.granted_permissions = granted_permissions;
        self
    }
}

#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct PolicyVisibilityContext {
    pub cwd: PathBuf,
    pub tool_spec: ToolSpec,
}

impl PolicyVisibilityContext {
    pub fn new(cwd: PathBuf, tool_spec: ToolSpec) -> Self {
        Self { cwd, tool_spec }
    }
}

pub trait ApprovalPolicy: Send + Sync {
    fn evaluate(&self, call: &ToolCall, ctx: &PolicyContext) -> PolicyDecision;

    fn evaluate_visibility(&self, ctx: &PolicyVisibilityContext) -> PolicyDecision;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grants_deduplicate_and_snapshot_sorted() {
        let grants = TurnPermissionGrants::default();
        grants.grant(["escalated_exec".to_owned(), "b".to_owned()]);
        grants.grant(["escalated_exec".to_owned(), "a".to_owned()]);

        assert_eq!(grants.snapshot(), vec!["a", "b", "escalated_exec"]);
    }
}
