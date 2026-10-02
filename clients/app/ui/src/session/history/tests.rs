use super::*;
#[test]
fn transcript_messages_merge_progress_subagent_into_preceding_task_card() {
    // Снапшот прогресса: карточка бегущего task + отдельное
    // subagent-сообщение сразу за ней — как шлёт turn_progress.
    let messages = transcript_messages(vec![
        TranscriptMessage {
            message_id: None,
            phase: None,
            role: "system".to_owned(),
            text: String::new(),
            tool: Some(TranscriptTool {
                call_id: "call-task".to_owned(),
                name: "task".to_owned(),
                args: serde_json::json!({"agent_type": "explore", "prompt": "look around"}),
                status: "running".to_owned(),
                result: None,
                metadata: Value::Null,
            }),
            subagent: None,
            streaming: false,
        },
        TranscriptMessage {
            message_id: None,
            phase: None,
            role: "system".to_owned(),
            text: String::new(),
            tool: None,
            subagent: Some(TranscriptSubagent {
                child_thread_id: "child-thread".to_owned(),
                role: "explore".to_owned(),
                description: None,
                status: "running".to_owned(),
                iterations: None,
                tools: Vec::new(),
            }),
            streaming: false,
        },
    ]);

    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].id, 1);
    let tool = messages[0].tool.as_ref().expect("task tool");
    assert_eq!(tool.call_id, "call-task");
    let subagent = messages[0].subagent.as_ref().expect("merged subagent");
    assert_eq!(subagent.child_thread_id, "child-thread");
    assert!(subagent.is_running());
}

#[test]
fn transcript_messages_merge_background_subagent_into_matching_spawn_card() {
    let messages = transcript_messages(vec![
        TranscriptMessage {
            message_id: None,
            phase: None,
            role: "system".to_owned(),
            text: String::new(),
            tool: Some(TranscriptTool {
                call_id: "call-spawn".to_owned(),
                name: SPAWN_AGENT_TOOL.to_owned(),
                args: serde_json::json!({
                    "task_name": "scan",
                    "message": "look around",
                    "agent_type": "explore"
                }),
                status: "done".to_owned(),
                result: Some("started".to_owned()),
                metadata: Value::Null,
            }),
            subagent: None,
            streaming: false,
        },
        TranscriptMessage {
            message_id: None,
            phase: None,
            role: "assistant".to_owned(),
            text: "Продолжаю основной ход".to_owned(),
            tool: None,
            subagent: None,
            streaming: true,
        },
        TranscriptMessage {
            message_id: None,
            phase: None,
            role: "system".to_owned(),
            text: String::new(),
            tool: None,
            subagent: Some(TranscriptSubagent {
                child_thread_id: "child-thread".to_owned(),
                role: "explore".to_owned(),
                description: Some("scan".to_owned()),
                status: "running".to_owned(),
                iterations: None,
                tools: Vec::new(),
            }),
            streaming: false,
        },
    ]);

    assert_eq!(messages.len(), 2);
    assert!(messages[0].subagent.is_some());
    assert!(messages[1].streaming);
    assert_eq!(messages[1].text, "Продолжаю основной ход");
}

#[test]
fn transcript_messages_merge_background_subagent_into_matching_followup_card() {
    let messages = transcript_messages(vec![
        TranscriptMessage {
            message_id: None,
            phase: None,
            role: "system".to_owned(),
            text: String::new(),
            tool: Some(TranscriptTool {
                call_id: "call-followup".to_owned(),
                name: FOLLOWUP_TASK_TOOL.to_owned(),
                args: serde_json::json!({
                    "target": "/root/scan",
                    "message": "continue"
                }),
                status: "done".to_owned(),
                result: Some("resumed".to_owned()),
                metadata: Value::Null,
            }),
            subagent: None,
            streaming: false,
        },
        TranscriptMessage {
            message_id: None,
            phase: None,
            role: "system".to_owned(),
            text: String::new(),
            tool: None,
            subagent: Some(TranscriptSubagent {
                child_thread_id: "child-thread".to_owned(),
                role: "explore".to_owned(),
                description: Some("scan".to_owned()),
                status: "running".to_owned(),
                iterations: None,
                tools: Vec::new(),
            }),
            streaming: false,
        },
    ]);

    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages[0].tool.as_ref().map(|tool| tool.name.as_str()),
        Some(FOLLOWUP_TASK_TOOL)
    );
    assert!(messages[0].subagent.is_some());
}

#[test]
fn transcript_messages_reconstruct_subagent_from_committed_task_result() {
    // Committed history: карточек субагента нет, но у результата task
    // есть metadata SubagentResult — карточка восстанавливается из неё.
    let messages = transcript_messages(vec![TranscriptMessage {
        message_id: None,
        phase: None,
        role: "system".to_owned(),
        text: String::new(),
        tool: Some(TranscriptTool {
            call_id: "call-task".to_owned(),
            name: "task".to_owned(),
            args: serde_json::json!({
                "agent_type": "explore",
                "description": "map the crate",
                "prompt": "look around"
            }),
            status: "done".to_owned(),
            result: Some("summary text".to_owned()),
            metadata: serde_json::json!({
                "status": "completed",
                "iterations": 3,
                "child_thread_id": "child-thread"
            }),
        }),
        subagent: None,
        streaming: false,
    }]);

    assert_eq!(messages.len(), 1);
    let subagent = messages[0].subagent.as_ref().expect("synthetic subagent");
    assert_eq!(subagent.role, "explore");
    assert_eq!(subagent.description.as_deref(), Some("map the crate"));
    assert_eq!(
        subagent.status,
        SubagentActivityStatus::Finished("completed".to_owned())
    );
    assert_eq!(subagent.iterations, Some(3));
    // Итог виден через result_preview слитой tool-карточки.
    assert_eq!(
        messages[0]
            .tool
            .as_ref()
            .and_then(|tool| tool.result_preview.as_deref()),
        Some("summary text")
    );
}

#[test]
fn transcript_messages_skip_reconstruction_for_failed_task_without_metadata() {
    let messages = transcript_messages(vec![TranscriptMessage {
        message_id: None,
        phase: None,
        role: "system".to_owned(),
        text: String::new(),
        tool: Some(TranscriptTool {
            call_id: "call-task".to_owned(),
            name: "task".to_owned(),
            args: serde_json::json!({"agent_type": "explore", "prompt": "look"}),
            status: "failed".to_owned(),
            result: Some("boom".to_owned()),
            metadata: serde_json::json!({"tool": "task"}),
        }),
        subagent: None,
        streaming: false,
    }]);

    // Без child_thread_id карточку не к чему привязать — остаётся
    // обычная tool-карточка с ошибкой.
    assert_eq!(messages.len(), 1);
    assert!(messages[0].subagent.is_none());
    assert!(messages[0].tool.is_some());
}

#[test]
fn transcript_messages_restore_tool_activity_cards() {
    let messages = transcript_messages(vec![TranscriptMessage {
        message_id: None,
        phase: None,
        role: "system".to_owned(),
        text: String::new(),
        tool: Some(TranscriptTool {
            call_id: "call-1".to_owned(),
            name: "read_file".to_owned(),
            args: serde_json::json!({"path": "src/lib.rs"}),
            status: "done".to_owned(),
            result: Some("line 1\nline 2".to_owned()),
            metadata: Value::Null,
        }),
        subagent: None,
        streaming: false,
    }]);

    assert_eq!(messages.len(), 1);
    let tool = messages[0].tool.as_ref().expect("tool activity");
    assert_eq!(tool.call_id, "call-1");
    assert_eq!(tool.name, "read_file");
    assert_eq!(tool.args_preview, "{\n  \"path\": \"src/lib.rs\"\n}");
    assert_eq!(tool.status, ToolActivityStatus::Done);
    assert_eq!(tool.result_preview.as_deref(), Some("line 1\nline 2"));
}

#[test]
fn transcript_messages_restore_subagent_activity_cards() {
    let messages = transcript_messages(vec![TranscriptMessage {
        message_id: None,
        phase: None,
        role: "system".to_owned(),
        text: String::new(),
        tool: None,
        subagent: Some(TranscriptSubagent {
            child_thread_id: "child-thread".to_owned(),
            role: "reviewer".to_owned(),
            description: Some("check patch".to_owned()),
            status: "completed".to_owned(),
            iterations: Some(2),
            tools: vec![TranscriptTool {
                call_id: "call-child".to_owned(),
                name: "read_file".to_owned(),
                args: serde_json::json!({"path": "src/lib.rs"}),
                status: "done".to_owned(),
                result: Some("contents".to_owned()),
                metadata: Value::Null,
            }],
        }),
        streaming: false,
    }]);

    assert_eq!(messages.len(), 1);
    assert!(messages[0].tool.is_none());
    let subagent = messages[0].subagent.as_ref().expect("subagent activity");
    assert_eq!(subagent.child_thread_id, "child-thread");
    assert_eq!(subagent.role, "reviewer");
    assert_eq!(subagent.description.as_deref(), Some("check patch"));
    assert_eq!(
        subagent.status,
        SubagentActivityStatus::Finished("completed".to_owned())
    );
    assert_eq!(subagent.iterations, Some(2));
    assert_eq!(subagent.tools.len(), 1);
    assert_eq!(subagent.tools[0].call_id, "call-child");
    assert_eq!(subagent.tools[0].status, ToolActivityStatus::Done);
    assert_eq!(
        subagent.tools[0].result_preview.as_deref(),
        Some("contents")
    );
}
