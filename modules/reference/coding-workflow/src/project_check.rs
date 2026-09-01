use proteus_contracts::{
    domain::{AgentOutput, Event, ToolCall, ToolChoice, ToolResult},
    model_standard::{
        CanonicalMessage, CanonicalModelRequest, InstructionBlock, InstructionKind, MessageRole,
    },
    process_module::{
        ProcessModuleError, WorkflowModuleHostMut, WorkflowModuleInput, WorkflowModuleOutput,
    },
};
use serde_json::{Value, json};

use crate::{
    host::{complete_model, emit_event, execute_tool},
    metadata::with_workflow_phase,
    output_text::message_text,
    validation::validate_model_response,
};

pub(crate) const PROJECT_CHECK_MODULE_ID: &str = "coding.project_check";

const TEST_TIMEOUT_MS: u64 = 600_000;
const MAX_DIAGNOSTIC_CHARS: usize = 12_000;
const EXPLAIN_FAILURE_INSTRUCTIONS: &str = "\
You explain the result of a deterministic project check. The controller has already \
selected and executed the test command. Explain the likely cause of the failure and \
give concise next diagnostic steps. Do not call tools, do not claim that you changed \
files, and do not invent output that is absent from the supplied diagnostics.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ProjectDefinition {
    kind: &'static str,
    marker: &'static str,
    command: &'static str,
}

const PROJECTS: [ProjectDefinition; 4] = [
    ProjectDefinition {
        kind: "rust",
        marker: "Cargo.toml",
        command: "cargo test",
    },
    ProjectDefinition {
        kind: "node",
        marker: "package.json",
        command: "npm test",
    },
    ProjectDefinition {
        kind: "python",
        marker: "pyproject.toml",
        command: "python -m pytest",
    },
    ProjectDefinition {
        kind: "go",
        marker: "go.mod",
        command: "go test ./...",
    },
];

pub(crate) fn run_project_check(
    input: WorkflowModuleInput,
    host: &mut WorkflowModuleHostMut<'_>,
) -> Result<WorkflowModuleOutput, ProcessModuleError> {
    emit_event(
        host,
        &Event::TaskReceived {
            task: input.task.clone(),
        },
    )?;

    let git_status = run_tool(host, &input, "git-status", "git_status", json!({}))?;
    if !git_status.ok {
        return finish(
            host,
            &input,
            blocked_report("git_status", "git status", &git_status),
        );
    }
    let git_dirty = git_status_is_dirty(&git_status.output);

    let root_entries = run_tool(
        host,
        &input,
        "list-root",
        "list_dir",
        json!({ "path": "." }),
    )?;
    if !root_entries.ok {
        return finish(
            host,
            &input,
            blocked_report("project_detection", "project root listing", &root_entries),
        );
    }

    let Some(project) = detect_project(&root_entries.output) else {
        return finish(
            host,
            &input,
            CheckReport {
                text: format!(
                    "Тип проекта не определён. Поддерживаемые markers: {}.",
                    PROJECTS
                        .iter()
                        .map(|project| format!("`{}`", project.marker))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                status: "unsupported",
                stage: "project_detection",
                project: None,
                git_dirty: Some(git_dirty),
                exit_code: None,
                timed_out: None,
                model_calls: 0,
                model_finish_reason: None,
            },
        );
    };

    let test_result = run_tool(
        host,
        &input,
        "run-tests",
        "shell",
        json!({
            "command": project.command,
            "timeout_ms": TEST_TIMEOUT_MS,
        }),
    )?;
    let exit_code = test_result
        .metadata
        .get("exit_code")
        .and_then(Value::as_i64);
    let timed_out = test_result
        .metadata
        .get("timed_out")
        .and_then(Value::as_bool);

    if test_result.ok {
        return finish(
            host,
            &input,
            CheckReport {
                text: format!(
                    "Проверка проекта завершена успешно.\n\n- Тип: `{}` (`{}`)\n- Команда: `{}`\n- Рабочее дерево: {}",
                    project.kind,
                    project.marker,
                    project.command,
                    git_state(git_dirty),
                ),
                status: "passed",
                stage: "tests",
                project: Some(project),
                git_dirty: Some(git_dirty),
                exit_code,
                timed_out,
                model_calls: 0,
                model_finish_reason: None,
            },
        );
    }

    if exit_code.is_none() && timed_out.is_none() {
        return finish(
            host,
            &input,
            blocked_report("tests", "test command", &test_result).with_project(project, git_dirty),
        );
    }

    let diagnostic = diagnostic_prompt(&input, project, git_dirty, &test_result);
    let mut request = CanonicalModelRequest::new(
        input.runtime.model_ref.clone(),
        vec![CanonicalMessage::text(MessageRole::User, diagnostic)],
    )
    .with_instructions(vec![InstructionBlock::new(
        InstructionKind::System,
        EXPLAIN_FAILURE_INSTRUCTIONS,
        100,
    )])
    .with_tool_choice(ToolChoice::None)
    .with_reasoning(input.runtime.reasoning.clone())
    .with_metadata(json!({
        "workflow_module": PROJECT_CHECK_MODULE_ID,
        "workflow_phase": "explain_failure",
    }));
    request.limits.max_input_tokens = input.runtime.max_input_tokens;

    emit_event(
        host,
        &Event::ModelRequestPrepared {
            model: request.model.clone(),
        },
    )?;
    let response = complete_model(host, &request, "project_check_explain_failure")?;
    emit_event(
        host,
        &Event::ModelResponseReceived {
            finish_reason: response.finish_reason.clone(),
        },
    )?;
    validate_model_response("project_check_explain_failure", &request, &response)?;
    let explanation = message_text(&response.message);

    finish(
        host,
        &input,
        CheckReport {
            text: format!(
                "Проверка проекта завершилась ошибкой.\n\n- Тип: `{}` (`{}`)\n- Команда: `{}`\n- Exit code: {}\n- Рабочее дерево: {}\n\nОбъяснение модели:\n{}",
                project.kind,
                project.marker,
                project.command,
                exit_code
                    .map(|code| code.to_string())
                    .unwrap_or_else(|| "нет".to_owned()),
                git_state(git_dirty),
                explanation,
            ),
            status: "failed",
            stage: "tests",
            project: Some(project),
            git_dirty: Some(git_dirty),
            exit_code,
            timed_out,
            model_calls: 1,
            model_finish_reason: serde_json::to_value(&response.finish_reason)
                .ok()
                .and_then(|value| value.as_str().map(str::to_owned)),
        },
    )
}

fn run_tool(
    host: &mut WorkflowModuleHostMut<'_>,
    input: &WorkflowModuleInput,
    stage: &str,
    name: &str,
    args: Value,
) -> Result<ToolResult, ProcessModuleError> {
    let call_id = format!("project-check-{}-{stage}", input.runtime.turn_id);
    execute_tool(host, input, &ToolCall::new(call_id, name, args))
}

fn detect_project(listing: &str) -> Option<ProjectDefinition> {
    PROJECTS.iter().copied().find(|project| {
        listing.lines().any(|line| {
            let Some((kind, name)) = line.split_once('\t') else {
                return false;
            };
            matches!(kind, "file" | "symlink") && name == project.marker
        })
    })
}

fn git_status_is_dirty(output: &str) -> bool {
    output
        .lines()
        .map(str::trim)
        .any(|line| !line.is_empty() && !line.starts_with("##"))
}

fn git_state(dirty: bool) -> &'static str {
    if dirty {
        "есть изменения"
    } else {
        "чистое"
    }
}

fn diagnostic_prompt(
    input: &WorkflowModuleInput,
    project: ProjectDefinition,
    git_dirty: bool,
    result: &ToolResult,
) -> String {
    let error = result.error.as_deref().unwrap_or("<none>");
    let output = bounded_text(&result.output, MAX_DIAGNOSTIC_CHARS);
    format!(
        "Original task:\n{}\n\nProject kind: {}\nMarker: {}\nCommand: {}\nGit working tree: {}\nTool error: {}\n\nCommand output:\n{}",
        input.task.text,
        project.kind,
        project.marker,
        project.command,
        git_state(git_dirty),
        error,
        output,
    )
}

fn bounded_text(text: &str, max_chars: usize) -> String {
    let mut chars = text.chars();
    let mut bounded = chars.by_ref().take(max_chars).collect::<String>();
    if chars.next().is_some() {
        bounded.push_str("\n[diagnostics truncated]");
    }
    bounded
}

struct CheckReport {
    text: String,
    status: &'static str,
    stage: &'static str,
    project: Option<ProjectDefinition>,
    git_dirty: Option<bool>,
    exit_code: Option<i64>,
    timed_out: Option<bool>,
    model_calls: u8,
    model_finish_reason: Option<String>,
}

impl CheckReport {
    fn with_project(mut self, project: ProjectDefinition, git_dirty: bool) -> Self {
        self.project = Some(project);
        self.git_dirty = Some(git_dirty);
        self
    }
}

fn blocked_report(stage: &'static str, operation: &str, result: &ToolResult) -> CheckReport {
    CheckReport {
        text: format!(
            "Проверка проекта остановлена на шаге `{stage}`: не удалось выполнить {operation}.\n\n{}",
            result.text_or_status(),
        ),
        status: "blocked",
        stage,
        project: None,
        git_dirty: None,
        exit_code: None,
        timed_out: None,
        model_calls: 0,
        model_finish_reason: None,
    }
}

fn finish(
    host: &mut WorkflowModuleHostMut<'_>,
    input: &WorkflowModuleInput,
    report: CheckReport,
) -> Result<WorkflowModuleOutput, ProcessModuleError> {
    let project = report.project.map(|project| {
        json!({
            "kind": project.kind,
            "marker": project.marker,
            "command": project.command,
        })
    });
    let metadata = json!({
        "session_id": input.runtime.session_id,
        "thread_id": input.runtime.thread_id,
        "turn_id": input.runtime.turn_id,
        "workflow": {
            "source": "process",
            "module_id": PROJECT_CHECK_MODULE_ID,
            "controller": "deterministic",
        },
        "project_check": {
            "status": report.status,
            "stage": report.stage,
            "project": project,
            "git_dirty": report.git_dirty,
            "exit_code": report.exit_code,
            "timed_out": report.timed_out,
            "model_calls": report.model_calls,
            "model_finish_reason": report.model_finish_reason,
        },
    });
    let output = AgentOutput::new(report.text.clone(), metadata);
    let message = with_workflow_phase(
        CanonicalMessage::text(MessageRole::Assistant, report.text),
        PROJECT_CHECK_MODULE_ID,
        report.stage,
    );
    emit_event(
        host,
        &Event::TurnFinished {
            output: output.clone(),
        },
    )?;
    Ok(WorkflowModuleOutput {
        output,
        new_messages: vec![message],
        history_replacement: None,
        compactions: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_detection_has_a_stable_priority() {
        let project = detect_project("file\tpackage.json\nfile\tCargo.toml\n")
            .expect("supported project marker");

        assert_eq!(project.kind, "rust");
        assert_eq!(project.command, "cargo test");
    }

    #[test]
    fn project_detection_requires_an_exact_root_entry() {
        assert!(detect_project("file\tCargo.toml.backup\ndir\tCargo.toml\n").is_none());
    }

    #[test]
    fn diagnostic_bounding_preserves_utf8() {
        assert_eq!(bounded_text("абвг", 3), "абв\n[diagnostics truncated]");
    }
}
