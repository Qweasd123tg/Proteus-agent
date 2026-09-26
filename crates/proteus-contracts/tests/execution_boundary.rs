use proteus_contracts::contracts::{CancellationToken, ExecutionScope};

#[test]
fn child_cancellation_scope_preserves_execution_identity() {
    let scope = ExecutionScope::fresh(CancellationToken::new());
    let child = scope.child_cancellation_scope();

    assert_eq!(scope.execution_id, child.execution_id);
    child.cancellation.cancel();
    assert!(!scope.cancellation.is_cancelled());
}
