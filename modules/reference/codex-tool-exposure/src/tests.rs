use super::*;
use proteus_contracts::{
    contracts::ToolExposureRequest,
    domain::{
        AgentTask, HostedToolConfig, ToolSafety, ToolSpec, ToolSurface, WebSearchHostedToolConfig,
    },
};

#[test]
fn selector_rejects_unknown_or_malformed_export_config() {
    for config in [
        json!({"max_hot_tools":"oops"}),
        json!({"always_include":[42]}),
        json!({"typo":true}),
        json!({"max_hot_tools":0}),
    ] {
        assert!(CodexDynamicConfig::from_value(&config).is_err(), "{config}");
    }
}

fn spec(name: &str, description: &str, safety: ToolSafety) -> ToolSpec {
    ToolSpec::new(name, description, json!({ "type": "object" }), safety)
}

fn control_spec(name: &str, safety: ToolSafety) -> ToolSpec {
    spec(name, "Collaboration control", safety).with_metadata(json!({
        "hot": true,
        "category": "proteus_agent_control"
    }))
}

fn hosted_web_search_spec() -> ToolSpec {
    spec("web_search", "Search the web", ToolSafety::Network).with_surface(
        ToolSurface::provider_hosted(HostedToolConfig::WebSearch {
            config: WebSearchHostedToolConfig::default(),
        }),
    )
}

fn select(query: &str, max_tools: usize, candidates: Vec<ToolSpec>) -> ToolExposureOutput {
    let task = AgentTask::new(query.to_owned(), std::env::current_dir().unwrap());
    let request = ToolExposureRequest::new(task)
        .with_query(query)
        .with_max_tools(max_tools);
    let input = ToolExposureInput::new(request, candidates);
    select_with_input(input)
}

fn select_with_input(input: ToolExposureInput) -> ToolExposureOutput {
    select_with_config(input, json!({}))
}

fn select_with_config(input: ToolExposureInput, config: Value) -> ToolExposureOutput {
    let input_json = serde_json::to_string(&input).unwrap();
    let module = CodexDynamicToolExposureModule {
        config: CodexDynamicConfig::from_value(&config).unwrap(),
    };
    let output_json = match module.select_json(input_json) {
        Ok(output) => output,
        Err(error) => panic!("{error}"),
    };
    serde_json::from_str(&output_json).unwrap()
}

#[test]
fn codex_selector_keeps_user_input_and_boosts_intent_tools() {
    let output = select(
        "fix code and run tests",
        5,
        vec![
            spec("request_user_input", "Ask user", ToolSafety::ReadOnly),
            spec("shell", "Run commands", ToolSafety::RunsCommands),
            spec("git_diff", "Show git diff", ToolSafety::ReadOnly),
            spec("read_file", "Read a file", ToolSafety::ReadOnly),
            spec("grep", "Search files", ToolSafety::ReadOnly),
            spec("apply_patch", "Apply patch", ToolSafety::WritesFiles),
            spec("remember_fact", "Remember fact", ToolSafety::ReadOnly),
        ],
    );

    let names = output
        .tools
        .iter()
        .map(|tool| tool.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        [
            "request_user_input",
            "shell",
            "apply_patch",
            "read_file",
            "grep"
        ]
    );
    assert_eq!(output.metadata["selector"], "codex_dynamic");
    assert_eq!(
        output.metadata["selected_tool_reasons"]["request_user_input"],
        "always_include"
    );
    assert_eq!(
        output.metadata["selected_tool_reasons"]["shell"],
        "intent_match"
    );
    assert_eq!(output.metadata["hidden_count"], 2);
}

#[test]
fn codex_selector_penalizes_non_read_only_tools_in_plan_phase() {
    let task = AgentTask::new(
        "fix code and run tests".to_owned(),
        std::env::current_dir().unwrap(),
    );
    let request = ToolExposureRequest::new(task)
        .with_query("fix code and run tests")
        .with_max_tools(3)
        .with_phase("plan");
    // Пустой always_include, чтобы проверить именно скоринг.
    let input = ToolExposureInput::new(
        request,
        vec![
            spec("shell", "Run commands", ToolSafety::RunsCommands),
            spec("apply_patch", "Apply patch", ToolSafety::WritesFiles),
            spec("read_file", "Read a file", ToolSafety::ReadOnly),
            spec("grep", "Search files", ToolSafety::ReadOnly),
            spec("git_diff", "Show git diff", ToolSafety::ReadOnly),
            spec("list_dir", "List directory", ToolSafety::ReadOnly),
        ],
    );

    let output = select_with_config(input, json!({ "always_include": ["read_file"] }));

    let names = output
        .tools
        .iter()
        .map(|tool| tool.name.as_str())
        .collect::<Vec<_>>();
    assert!(!names.contains(&"shell"), "{names:?}");
    assert!(!names.contains(&"apply_patch"), "{names:?}");
    assert_eq!(output.metadata["phase"], json!("plan"));
}

#[test]
fn codex_selector_never_invents_tools_when_all_candidates_fit() {
    let output = select(
        "read files",
        10,
        vec![
            spec("read_file", "Read a file", ToolSafety::ReadOnly),
            spec("grep", "Search files", ToolSafety::ReadOnly),
        ],
    );

    let names = output
        .tools
        .iter()
        .map(|tool| tool.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(names, ["read_file", "grep"]);
    assert_eq!(
        output.metadata["selected_tool_reasons"]["read_file"],
        "all_candidates_fit"
    );
    assert_eq!(output.metadata["hidden_count"], 0);
}

#[test]
fn codex_selector_uses_export_config() {
    let task = AgentTask::new("read files".to_owned(), std::env::current_dir().unwrap());
    let request = ToolExposureRequest::new(task);
    let input = ToolExposureInput::new(
        request,
        vec![
            spec("git_status", "Show git status", ToolSafety::ReadOnly),
            spec("read_file", "Read a file", ToolSafety::ReadOnly),
            spec("grep", "Search files", ToolSafety::ReadOnly),
        ],
    );

    let output = select_with_config(
        input,
        json!({
            "max_hot_tools": 2,
            "always_include": ["git_status"],
        }),
    );
    let names = output
        .tools
        .iter()
        .map(|tool| tool.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(names, ["git_status", "read_file"]);
    assert_eq!(output.metadata["max_tools"], 2);
    assert_eq!(
        output.metadata["selected_tool_reasons"]["git_status"],
        "always_include"
    );
}

#[test]
fn collaboration_control_group_is_atomic_and_preserves_hot_tool_budget() {
    let task = AgentTask::new("stable", std::env::current_dir().unwrap());
    let request = ToolExposureRequest::new(task).with_max_tools(3);
    let input = ToolExposureInput::new(
        request,
        vec![
            spec("request_user_input", "Ask", ToolSafety::ReadOnly),
            control_spec("spawn_agent", ToolSafety::WritesFiles),
            control_spec("list_agents", ToolSafety::ReadOnly),
            control_spec("wait_agent", ToolSafety::ReadOnly),
            control_spec("interrupt_agent", ToolSafety::ReadOnly),
            control_spec("send_message", ToolSafety::WritesFiles),
            control_spec("followup_task", ToolSafety::WritesFiles),
            spec("read_file", "Read", ToolSafety::ReadOnly),
            spec("grep", "Search", ToolSafety::ReadOnly),
            spec("remember_fact", "Remember", ToolSafety::ReadOnly),
        ],
    );

    let output = select_with_input(input);
    let names = output
        .tools
        .iter()
        .map(|tool| tool.name.as_str())
        .collect::<HashSet<_>>();
    for required in [
        "request_user_input",
        "spawn_agent",
        "list_agents",
        "wait_agent",
        "interrupt_agent",
        "send_message",
        "followup_task",
        "read_file",
        "grep",
    ] {
        assert!(names.contains(required), "missing {required}: {names:?}");
    }
    assert_eq!(output.metadata["max_tools"], 9);
    assert_eq!(
        output.metadata["selected_tool_reasons"]["followup_task"],
        "control_group"
    );
}

#[test]
fn implicit_task_text_keeps_cache_stable_hot_set() {
    let candidates = vec![
        spec("shell", "Run commands", ToolSafety::RunsCommands),
        spec("apply_patch", "Apply patch", ToolSafety::WritesFiles),
        spec("read_file", "Read a file", ToolSafety::ReadOnly),
        spec("grep", "Search files", ToolSafety::ReadOnly),
    ];
    let select_task = |text: &str| {
        let task = AgentTask::new(text, std::env::current_dir().unwrap());
        let request = ToolExposureRequest::new(task).with_max_tools(2);
        select_with_input(ToolExposureInput::new(request, candidates.clone()))
    };

    let run = select_task("run the tests in a shell");
    let edit = select_task("apply a patch to the code");
    let names = |output: &ToolExposureOutput| {
        output
            .tools
            .iter()
            .map(|tool| tool.name.clone())
            .collect::<Vec<_>>()
    };

    assert_eq!(names(&run), names(&edit));
    assert_eq!(run.metadata["query"], "");
    assert_eq!(run.metadata["query_source"], "stable_hot_set");
}

#[test]
fn plan_phase_filters_writes_even_when_all_candidates_fit() {
    let task = AgentTask::new("plan changes", std::env::current_dir().unwrap());
    let request = ToolExposureRequest::new(task)
        .with_max_tools(10)
        .with_phase("plan");
    let output = select_with_input(ToolExposureInput::new(
        request,
        vec![
            spec("read_file", "Read", ToolSafety::ReadOnly),
            spec("write_file", "Write", ToolSafety::WritesFiles),
            spec("shell", "Run", ToolSafety::RunsCommands),
        ],
    ));

    assert_eq!(
        output
            .tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>(),
        ["read_file"]
    );
}

#[test]
fn always_include_is_deduplicated_and_can_clear_defaults() {
    let task = AgentTask::new("stable", std::env::current_dir().unwrap());
    let request = ToolExposureRequest::new(task).with_max_tools(2);
    let candidates = vec![
        spec("request_user_input", "Ask", ToolSafety::ReadOnly),
        spec("read_file", "Read", ToolSafety::ReadOnly),
        spec("grep", "Search", ToolSafety::ReadOnly),
    ];
    let deduplicated = select_with_config(
        ToolExposureInput::new(request.clone(), candidates.clone()),
        json!({
            "always_include": ["request_user_input", "request_user_input"]
        }),
    );
    assert_eq!(deduplicated.tools.len(), 2);
    assert_eq!(deduplicated.tools[0].name, "request_user_input");

    let cleared = select_with_config(
        ToolExposureInput::new(request, candidates),
        json!({ "always_include": [] }),
    );
    assert_eq!(cleared.tools[0].name, "read_file");
    assert!(
        !cleared
            .tools
            .iter()
            .any(|tool| tool.name == "request_user_input")
    );
}

#[test]
fn provider_hosted_tool_stays_direct_outside_hot_tool_budget() {
    let output = select(
        "stable",
        1,
        vec![
            spec("read_file", "Read", ToolSafety::ReadOnly),
            hosted_web_search_spec(),
            spec("remember_fact", "Remember", ToolSafety::WritesFiles),
        ],
    );
    let names = output
        .tools
        .iter()
        .map(|tool| tool.name.as_str())
        .collect::<Vec<_>>();

    assert!(names.contains(&"web_search"), "{names:?}");
    assert_eq!(output.metadata["max_tools"], 2);
    assert_eq!(
        output.metadata["selected_tool_reasons"]["web_search"],
        "provider_hosted_direct"
    );
}
