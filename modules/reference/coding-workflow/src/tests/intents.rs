use super::*;
use proteus_contracts::domain::PermissionMode;

#[test]
fn planning_intents_are_workflow_instructions_and_preserve_user_text() {
    for (intent, mode, instruction) in [
        ("planning.start", PermissionMode::Plan, "planning interview"),
        (
            "planning.revise",
            PermissionMode::Plan,
            "feedback on the latest plan",
        ),
        (
            "planning.execute",
            PermissionMode::Normal,
            "latest approved plan",
        ),
    ] {
        for codex in [false, true] {
            let mut input = workflow_input("Точный текст пользователя");
            input.runtime.intent = Some(intent.into());
            input.runtime.permission_mode = mode;
            let mut host = FakeHost::default();
            let json = serde_json::to_string(&input).unwrap();
            let result = if codex {
                CodingCodexLoopWorkflow.run_json(json, &mut host)
            } else {
                CodingSingleLoopWorkflow::default().run_json(json, &mut host)
            };
            result.unwrap();
            let requests = host.requests.lock().unwrap();
            assert_eq!(requests.len(), 1);
            assert!(
                requests[0]
                    .instructions
                    .iter()
                    .any(|i| i.text.contains(instruction))
            );
            assert_eq!(host.context_builds.lock().unwrap()[0].text, input.task.text);
            assert_eq!(
                requests[0].messages.last().unwrap(),
                input.history.last().unwrap()
            );
        }
    }
}

#[test]
fn invalid_or_unsupported_intents_fail_before_context_model_or_tools() {
    for (intent, mode) in [
        ("planning.start", PermissionMode::Auto),
        ("planning.revise", PermissionMode::Normal),
        ("planning.execute", PermissionMode::Plan),
        ("unknown.action", PermissionMode::Normal),
    ] {
        let mut input = workflow_input("must not execute");
        input.runtime.intent = Some(intent.into());
        input.runtime.permission_mode = mode;
        let mut host = FakeHost::default();
        assert!(
            CodingCodexLoopWorkflow
                .run_json(serde_json::to_string(&input).unwrap(), &mut host)
                .is_err()
        );
        assert!(host.context_builds.lock().unwrap().is_empty());
        assert!(host.requests.lock().unwrap().is_empty());
        assert!(host.executed_calls.lock().unwrap().is_empty());
    }
    let mut input = workflow_input("no silent fallback to project checks");
    input.runtime.intent = Some("planning.start".into());
    input.runtime.permission_mode = PermissionMode::Plan;
    let mut host = FakeHost::default();
    let json = serde_json::to_string(&input).unwrap();
    assert!(
        CodingProjectCheckWorkflow
            .run_json(json.clone(), &mut host)
            .is_err()
    );
    assert!(
        CodingPlanExecuteReviewWorkflow
            .run_json(json, &mut host)
            .is_err()
    );
    assert!(host.executed_calls.lock().unwrap().is_empty());
}
