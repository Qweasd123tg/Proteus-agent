use crate::{
    contracts::CancellationToken,
    domain::{ExecutionId, new_execution_id},
};

/// Generic identity and cancellation boundary for one logical workload.
///
/// The scope deliberately owns no runtime services, chat identity, history or
/// process-broker lineage.
#[derive(Debug, Clone)]
pub struct ExecutionScope {
    pub execution_id: ExecutionId,
    pub cancellation: CancellationToken,
}

impl ExecutionScope {
    pub fn new(execution_id: ExecutionId, cancellation: CancellationToken) -> Self {
        Self {
            execution_id,
            cancellation,
        }
    }

    pub fn fresh(cancellation: CancellationToken) -> Self {
        Self::new(new_execution_id(), cancellation)
    }

    /// Creates a targeted cancellation view of the same logical execution.
    /// This is not a child execution and therefore preserves `ExecutionId`.
    pub fn child_cancellation_scope(&self) -> Self {
        Self::new(self.execution_id, self.cancellation.child_token())
    }
}

#[cfg(test)]
mod tests {
    use std::any::TypeId;

    use super::*;
    use crate::domain::TurnId;

    #[test]
    fn execution_scope_constructs_without_turn() {
        let scope = ExecutionScope::fresh(CancellationToken::new());

        assert!(!scope.cancellation.is_cancelled());
    }

    #[test]
    fn execution_id_and_turn_id_are_type_distinct() {
        assert_ne!(TypeId::of::<ExecutionId>(), TypeId::of::<TurnId>());
    }

    #[test]
    fn child_cancellation_scope_preserves_execution_identity() {
        let scope = ExecutionScope::fresh(CancellationToken::new());
        let child = scope.child_cancellation_scope();

        assert_eq!(scope.execution_id, child.execution_id);
        child.cancellation.cancel();
        assert!(!scope.cancellation.is_cancelled());
    }
}
