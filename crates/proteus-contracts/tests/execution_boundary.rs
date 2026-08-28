use std::any::TypeId;

use proteus_contracts::{
    contracts::{CancellationToken, ExecutionScope},
    domain::{ExecutionId, TurnId},
};

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

#[test]
fn generic_execution_contract_has_no_chat_domain_imports() {
    let source = include_str!("../src/contracts/execution.rs");
    for forbidden in ["TurnId", "AgentTask", "AgentOutput", "CanonicalMessage"] {
        assert!(
            !source.contains(forbidden),
            "generic execution contract imports chat-specific type {forbidden}"
        );
    }
}
