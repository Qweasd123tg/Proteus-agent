use leptos::prelude::Owner;
use serde_json::Value;

use super::*;

#[test]
fn finish_active_streaming_assistant_message_marks_message_done() {
    let owner = Owner::new();
    owner.with(|| {
        let (messages, set_messages) = signal(vec![Message {
            message_id: None,
            phase: None,
            id: 1,
            version: 0,
            text_offset: 0,
            role: MessageRole::Assistant,
            text: "**ready**".to_owned(),
            tool: None,
            subagent: None,
            streaming: true,
        }]);
        let (active_stream_message_id, set_active_stream_message_id) = signal(Some(1));

        finish_active_streaming_assistant_message(
            set_messages,
            active_stream_message_id,
            set_active_stream_message_id,
        );

        let items = messages.get_untracked();
        assert!(!items[0].streaming);
        assert_eq!(items[0].version, 1);
        assert_eq!(active_stream_message_id.get_untracked(), None);
    });
}

#[test]
fn push_assistant_message_if_missing_appends_new_final_output() {
    let owner = Owner::new();
    owner.with(|| {
        let (messages, set_messages) = signal(vec![Message {
            message_id: None,
            phase: None,
            id: 1,
            version: 0,
            text_offset: 0,
            role: MessageRole::Assistant,
            text: "pre-tool note".to_owned(),
            tool: None,
            subagent: None,
            streaming: false,
        }]);
        let (next_message_id, set_next_message_id) = signal(2);

        push_assistant_message_if_missing(
            set_messages,
            next_message_id,
            set_next_message_id,
            "final answer".to_owned(),
        );

        let items = messages.get_untracked();
        assert_eq!(items.len(), 2);
        assert_eq!(items[1].id, 2);
        assert_eq!(items[1].text, "final answer");
        assert_eq!(next_message_id.get_untracked(), 3);
    });
}

fn history_message(id: u64, role: MessageRole, text: &str) -> Message {
    Message {
        message_id: None,
        phase: None,
        id,
        version: 0,
        text_offset: 0,
        role,
        text: text.to_owned(),
        tool: None,
        subagent: None,
        streaming: false,
    }
}

fn tool_activity(call_id: &str, status: ToolActivityStatus) -> ToolActivity {
    ToolActivity {
        call_id: call_id.to_owned(),
        name: "shell".to_owned(),
        args: Value::Null,
        args_preview: String::new(),
        started_at_ms: 0,
        finished_at_ms: None,
        status,
        result_preview: None,
    }
}

fn subagent_activity(child_thread_id: &str, status: SubagentActivityStatus) -> SubagentActivity {
    SubagentActivity {
        child_thread_id: child_thread_id.to_owned(),
        role: "reviewer".to_owned(),
        description: Some("check the implementation".to_owned()),
        status,
        iterations: None,
        started_at_ms: 10,
        finished_at_ms: None,
        tools: Vec::new(),
    }
}

fn subagent_message(id: u64, activity: SubagentActivity) -> Message {
    Message {
        message_id: None,
        phase: None,
        id,
        version: 0,
        text_offset: 0,
        role: MessageRole::System,
        text: String::new(),
        tool: None,
        subagent: Some(activity),
        streaming: false,
    }
}

#[test]
fn prepend_history_messages_keeps_live_tail_after_history() {
    let owner = Owner::new();
    owner.with(|| {
        // Живой хвост: стрим-сообщение, прилетевшее по SSE раньше /history.
        let (messages, set_messages) = signal(vec![Message {
            message_id: None,
            phase: None,
            id: 1,
            version: 0,
            text_offset: 0,
            role: MessageRole::Assistant,
            text: "хвост стрима".to_owned(),
            tool: None,
            subagent: None,
            streaming: true,
        }]);
        let (next_message_id, set_next_message_id) = signal(2_u64);
        let (active_stream_message_id, set_active_stream_message_id) = signal(Some(1_u64));
        let (streamed_this_turn, set_streamed_this_turn) = signal(true);
        let transcript = vec![
            history_message(1, MessageRole::User, "первый вопрос"),
            history_message(2, MessageRole::Assistant, "первый ответ"),
        ];

        prepend_history_messages(
            set_messages,
            next_message_id,
            set_next_message_id,
            set_active_stream_message_id,
            set_streamed_this_turn,
            transcript,
        );

        let items = messages.get_untracked();
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].text, "первый вопрос");
        assert_eq!(items[1].text, "первый ответ");
        // Хвост остаётся живым и последним; его id не меняется.
        assert_eq!(items[2].id, 1);
        assert!(items[2].streaming);
        // Id истории выделены поверх счётчика и не пересекаются с живыми.
        assert_eq!(items[0].id, 2);
        assert_eq!(items[1].id, 3);
        assert_eq!(next_message_id.get_untracked(), 4);
        // История без стрим-хвоста — живой стрим остаётся целью дельт.
        assert_eq!(active_stream_message_id.get_untracked(), Some(1));
        assert!(streamed_this_turn.get_untracked());
    });
}

#[test]
fn push_subagent_tool_nests_by_thread_id_and_reports_miss() {
    let owner = Owner::new();
    owner.with(|| {
        let (messages, set_messages) = signal(vec![subagent_message(
            1,
            subagent_activity("child-thread", SubagentActivityStatus::Running),
        )]);

        let nested = push_subagent_tool(
            set_messages,
            "child-thread",
            tool_activity("call-1", ToolActivityStatus::Running),
        );
        let missing = push_subagent_tool(
            set_messages,
            "other-thread",
            tool_activity("call-2", ToolActivityStatus::Running),
        );

        let items = messages.get_untracked();
        assert!(nested);
        assert!(!missing);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].version, 1);
        let subagent = items[0].subagent.as_ref().expect("subagent card");
        assert_eq!(subagent.tools.len(), 1);
        assert_eq!(subagent.tools[0].call_id, "call-1");
    });
}

#[test]
fn update_tool_status_updates_nested_subagent_tool() {
    let owner = Owner::new();
    owner.with(|| {
        let mut activity = subagent_activity("child-thread", SubagentActivityStatus::Running);
        activity
            .tools
            .push(tool_activity("call-1", ToolActivityStatus::Running));
        let (tool_activities, set_tool_activities) =
            signal(vec![tool_activity("call-1", ToolActivityStatus::Running)]);
        let (messages, set_messages) = signal(vec![subagent_message(1, activity)]);

        let nested = update_tool_status(
            set_tool_activities,
            set_messages,
            "call-1",
            ToolActivityStatus::Done,
            Some("ok".to_owned()),
            42,
        );

        let items = messages.get_untracked();
        assert!(nested);
        assert_eq!(items[0].version, 1);
        let tool = &items[0].subagent.as_ref().expect("subagent card").tools[0];
        assert_eq!(tool.status, ToolActivityStatus::Done);
        assert_eq!(tool.result_preview.as_deref(), Some("ok"));
        // Терминальный статус фиксирует момент завершения для duration.
        assert_eq!(tool.finished_at_ms, Some(42));

        let rail_items = tool_activities.get_untracked();
        assert_eq!(rail_items[0].status, ToolActivityStatus::Done);
        assert_eq!(rail_items[0].result_preview.as_deref(), Some("ok"));
    });
}

#[test]
fn finish_subagent_message_closes_running_card() {
    let owner = Owner::new();
    owner.with(|| {
        let (messages, set_messages) = signal(vec![subagent_message(
            1,
            subagent_activity("child-thread", SubagentActivityStatus::Running),
        )]);

        finish_subagent_message(
            set_messages,
            "child-thread",
            SubagentActivityStatus::Finished("completed".to_owned()),
            Some(3),
            110,
        );

        let items = messages.get_untracked();
        assert_eq!(items[0].version, 1);
        let subagent = items[0].subagent.as_ref().expect("subagent card");
        assert_eq!(
            subagent.status,
            SubagentActivityStatus::Finished("completed".to_owned())
        );
        assert_eq!(subagent.iterations, Some(3));
        // started_at_ms = 10 в хелпере: длительность 100ms.
        assert_eq!(subagent.duration_ms(), Some(100));
    });
}

#[test]
fn push_subagent_message_attaches_to_running_task_tool_card() {
    let owner = Owner::new();
    owner.with(|| {
        let mut task_tool = tool_activity("call-task", ToolActivityStatus::Running);
        task_tool.name = TASK_TOOL.to_owned();
        let mut task_message = history_message(1, MessageRole::System, "");
        task_message.tool = Some(task_tool);
        let (messages, set_messages) = signal(vec![task_message]);
        let (next_message_id, set_next_message_id) = signal(2);

        push_subagent_message(
            set_messages,
            next_message_id,
            set_next_message_id,
            subagent_activity("child-thread", SubagentActivityStatus::Running),
        );

        // Активность прикрепилась к task-карточке: дубль не создан.
        let items = messages.get_untracked();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].version, 1);
        assert!(items[0].tool.is_some());
        let subagent = items[0].subagent.as_ref().expect("attached subagent");
        assert_eq!(subagent.child_thread_id, "child-thread");
        assert_eq!(next_message_id.get_untracked(), 2);
    });
}

#[test]
fn push_subagent_message_attaches_to_running_spawn_agent_card() {
    let owner = Owner::new();
    owner.with(|| {
        let mut spawn_tool = tool_activity("call-spawn", ToolActivityStatus::Running);
        spawn_tool.name = SPAWN_AGENT_TOOL.to_owned();
        let mut spawn_message = history_message(1, MessageRole::System, "");
        spawn_message.tool = Some(spawn_tool);
        let (messages, set_messages) = signal(vec![spawn_message]);
        let (next_message_id, set_next_message_id) = signal(2);

        push_subagent_message(
            set_messages,
            next_message_id,
            set_next_message_id,
            subagent_activity("child-thread", SubagentActivityStatus::Running),
        );

        let items = messages.get_untracked();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].version, 1);
        assert_eq!(
            items[0].tool.as_ref().map(|tool| tool.name.as_str()),
            Some(SPAWN_AGENT_TOOL)
        );
        assert!(items[0].subagent.is_some());
        assert_eq!(next_message_id.get_untracked(), 2);
    });
}

#[test]
fn push_subagent_message_attaches_to_running_followup_card() {
    let owner = Owner::new();
    owner.with(|| {
        let mut followup_tool = tool_activity("call-followup", ToolActivityStatus::Running);
        followup_tool.name = FOLLOWUP_TASK_TOOL.to_owned();
        let mut followup_message = history_message(1, MessageRole::System, "");
        followup_message.tool = Some(followup_tool);
        let (messages, set_messages) = signal(vec![followup_message]);
        let (next_message_id, set_next_message_id) = signal(2);

        push_subagent_message(
            set_messages,
            next_message_id,
            set_next_message_id,
            subagent_activity("same-child-thread", SubagentActivityStatus::Running),
        );

        let items = messages.get_untracked();
        assert_eq!(items.len(), 1);
        assert_eq!(
            items[0].tool.as_ref().map(|tool| tool.name.as_str()),
            Some(FOLLOWUP_TASK_TOOL)
        );
        assert_eq!(
            items[0]
                .subagent
                .as_ref()
                .map(|subagent| subagent.child_thread_id.as_str()),
            Some("same-child-thread")
        );
        assert_eq!(next_message_id.get_untracked(), 2);
    });
}

#[test]
fn push_subagent_message_skips_finished_task_card_and_falls_back_to_standalone() {
    let owner = Owner::new();
    owner.with(|| {
        // Завершённый прошлый task не должен получить чужую активность.
        let mut task_tool = tool_activity("call-task", ToolActivityStatus::Done);
        task_tool.name = TASK_TOOL.to_owned();
        let mut task_message = history_message(1, MessageRole::System, "");
        task_message.tool = Some(task_tool);
        let (messages, set_messages) = signal(vec![task_message]);
        let (next_message_id, set_next_message_id) = signal(2);

        push_subagent_message(
            set_messages,
            next_message_id,
            set_next_message_id,
            subagent_activity("child-thread", SubagentActivityStatus::Running),
        );

        let items = messages.get_untracked();
        assert_eq!(items.len(), 2);
        assert!(items[0].subagent.is_none());
        assert!(items[1].subagent.is_some());
        assert_eq!(next_message_id.get_untracked(), 3);
    });
}

#[test]
fn push_subagent_message_dedups_running_child_thread_id() {
    let owner = Owner::new();
    owner.with(|| {
        let (messages, set_messages) = signal(Vec::<Message>::new());
        let (next_message_id, set_next_message_id) = signal(1);

        push_subagent_message(
            set_messages,
            next_message_id,
            set_next_message_id,
            subagent_activity("child-thread", SubagentActivityStatus::Running),
        );
        push_subagent_message(
            set_messages,
            next_message_id,
            set_next_message_id,
            subagent_activity("child-thread", SubagentActivityStatus::Running),
        );

        assert_eq!(messages.get_untracked().len(), 1);
        assert_eq!(next_message_id.get_untracked(), 2);

        finish_subagent_message(
            set_messages,
            "child-thread",
            SubagentActivityStatus::Finished("completed".to_owned()),
            Some(1),
            50,
        );
        push_subagent_message(
            set_messages,
            next_message_id,
            set_next_message_id,
            subagent_activity("child-thread", SubagentActivityStatus::Running),
        );

        assert_eq!(messages.get_untracked().len(), 2);
        assert_eq!(next_message_id.get_untracked(), 3);
    });
}

#[test]
fn prepend_history_messages_adopts_history_streaming_tail() {
    let owner = Owner::new();
    owner.with(|| {
        // Живой хвост из SSE и стрим-хвост в снапшоте прогресса: снапшот
        // авторитетнее (содержит настриманное до перезагрузки), локальный
        // дубль выбрасывается, дельты переключаются на сообщение истории.
        let (messages, set_messages) = signal(vec![Message {
            message_id: None,
            phase: None,
            id: 1,
            version: 0,
            text_offset: 0,
            role: MessageRole::Assistant,
            text: "хвост после переподключения".to_owned(),
            tool: None,
            subagent: None,
            streaming: true,
        }]);
        let (next_message_id, set_next_message_id) = signal(2_u64);
        let (active_stream_message_id, set_active_stream_message_id) = signal(Some(1_u64));
        let (streamed_this_turn, set_streamed_this_turn) = signal(true);
        let mut partial = history_message(3, MessageRole::Assistant, "первые сто слов");
        partial.streaming = true;
        let transcript = vec![
            history_message(1, MessageRole::User, "вопрос"),
            history_message(2, MessageRole::Assistant, "прошлый ответ"),
            partial,
        ];

        prepend_history_messages(
            set_messages,
            next_message_id,
            set_next_message_id,
            set_active_stream_message_id,
            set_streamed_this_turn,
            transcript,
        );

        let items = messages.get_untracked();
        assert_eq!(items.len(), 3);
        assert_eq!(items[2].text, "первые сто слов");
        assert!(items[2].streaming);
        // Цель для дельт — усыновлённый хвост истории (id 2 + 2 = 4).
        assert_eq!(active_stream_message_id.get_untracked(), Some(4));
        assert!(streamed_this_turn.get_untracked());
    });
}

#[test]
fn prepend_history_messages_drops_live_duplicates_of_history_tail() {
    let owner = Owner::new();
    owner.with(|| {
        // Ход успел завершиться, пока /history был в пути: финальный ответ
        // уже лежит в живых сообщениях и продублирован историей.
        let (messages, set_messages) = signal(vec![history_message(
            1,
            MessageRole::Assistant,
            "финальный ответ",
        )]);
        let (next_message_id, set_next_message_id) = signal(2_u64);
        let (_, set_active_stream_message_id) = signal(None::<u64>);
        let (_, set_streamed_this_turn) = signal(false);
        let transcript = vec![
            history_message(1, MessageRole::User, "вопрос"),
            history_message(2, MessageRole::Assistant, "финальный ответ"),
        ];

        prepend_history_messages(
            set_messages,
            next_message_id,
            set_next_message_id,
            set_active_stream_message_id,
            set_streamed_this_turn,
            transcript,
        );

        let items = messages.get_untracked();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].text, "вопрос");
        assert_eq!(items[1].text, "финальный ответ");
    });
}

#[test]
fn prepend_history_messages_drops_live_tool_duplicated_by_call_id() {
    let owner = Owner::new();
    owner.with(|| {
        let live_tool = ToolActivity {
            call_id: "call-7".to_owned(),
            name: "shell".to_owned(),
            args: serde_json::Value::Null,
            args_preview: String::new(),
            started_at_ms: 0,
            finished_at_ms: None,
            status: ToolActivityStatus::Done,
            result_preview: None,
        };
        let (messages, set_messages) = signal(vec![Message {
            message_id: None,
            phase: None,
            id: 1,
            version: 0,
            text_offset: 0,
            role: MessageRole::System,
            text: String::new(),
            tool: Some(live_tool.clone()),
            subagent: None,
            streaming: false,
        }]);
        let (next_message_id, set_next_message_id) = signal(2_u64);
        let (_, set_active_stream_message_id) = signal(None::<u64>);
        let (_, set_streamed_this_turn) = signal(false);
        let mut history_tool_message = history_message(1, MessageRole::System, "");
        history_tool_message.tool = Some(live_tool);
        let transcript = vec![
            history_message(1, MessageRole::User, "вопрос"),
            history_tool_message,
        ];

        prepend_history_messages(
            set_messages,
            next_message_id,
            set_next_message_id,
            set_active_stream_message_id,
            set_streamed_this_turn,
            transcript,
        );

        let items = messages.get_untracked();
        assert_eq!(items.len(), 2);
        assert!(items[1].tool.is_some());
    });
}

#[test]
fn prepend_history_messages_drops_flat_history_tool_duplicated_by_nested_subagent_tool() {
    let owner = Owner::new();
    owner.with(|| {
        let nested_tool = tool_activity("call-7", ToolActivityStatus::Running);
        let mut activity = subagent_activity("child-thread", SubagentActivityStatus::Running);
        activity.tools.push(nested_tool.clone());
        let (messages, set_messages) = signal(vec![subagent_message(1, activity)]);
        let (next_message_id, set_next_message_id) = signal(2_u64);
        let (_, set_active_stream_message_id) = signal(None::<u64>);
        let (_, set_streamed_this_turn) = signal(false);
        let mut history_tool_message = history_message(1, MessageRole::System, "");
        history_tool_message.tool = Some(nested_tool);
        let transcript = vec![
            history_message(1, MessageRole::User, "вопрос"),
            history_tool_message,
        ];

        prepend_history_messages(
            set_messages,
            next_message_id,
            set_next_message_id,
            set_active_stream_message_id,
            set_streamed_this_turn,
            transcript,
        );

        let items = messages.get_untracked();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].text, "вопрос");
        assert!(items[1].tool.is_none());
        let subagent = items[1].subagent.as_ref().expect("subagent card");
        assert_eq!(subagent.tools.len(), 1);
        assert_eq!(subagent.tools[0].call_id, "call-7");
    });
}

#[test]
fn prepend_history_messages_drops_live_subagent_duplicated_by_history_snapshot() {
    let owner = Owner::new();
    owner.with(|| {
        let live = subagent_message(
            1,
            subagent_activity("child-thread", SubagentActivityStatus::Running),
        );
        let (messages, set_messages) = signal(vec![live]);
        let (next_message_id, set_next_message_id) = signal(2_u64);
        let (_, set_active_stream_message_id) = signal(None::<u64>);
        let (_, set_streamed_this_turn) = signal(false);
        let history_subagent = subagent_message(
            1,
            subagent_activity("child-thread", SubagentActivityStatus::Running),
        );
        let transcript = vec![
            history_message(1, MessageRole::User, "вопрос"),
            history_subagent,
        ];

        prepend_history_messages(
            set_messages,
            next_message_id,
            set_next_message_id,
            set_active_stream_message_id,
            set_streamed_this_turn,
            transcript,
        );

        let items = messages.get_untracked();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].text, "вопрос");
        assert!(items[1].subagent.is_some());
    });
}

#[test]
fn finalize_running_activity_interrupts_tools_and_subagents() {
    let owner = Owner::new();
    owner.with(|| {
        let mut running_subagent =
            subagent_activity("child-thread", SubagentActivityStatus::Running);
        running_subagent
            .tools
            .push(tool_activity("call-nested", ToolActivityStatus::Running));
        let mut flat_tool_message = history_message(2, MessageRole::System, "");
        flat_tool_message.tool = Some(tool_activity(
            "call-flat",
            ToolActivityStatus::WaitingApproval,
        ));
        let mut done_tool_message = history_message(3, MessageRole::System, "");
        done_tool_message.tool = Some(tool_activity("call-done", ToolActivityStatus::Done));
        let (messages, set_messages) = signal(vec![
            subagent_message(1, running_subagent),
            flat_tool_message,
            done_tool_message,
        ]);
        let (tool_activities, set_tool_activities) = signal(vec![
            tool_activity("call-flat", ToolActivityStatus::Running),
            tool_activity("call-done", ToolActivityStatus::Done),
        ]);

        finalize_running_activity(set_tool_activities, set_messages, 99);

        let items = messages.get_untracked();
        let subagent = items[0].subagent.as_ref().expect("subagent card");
        assert_eq!(
            subagent.status,
            SubagentActivityStatus::Finished("interrupted".to_owned())
        );
        assert_eq!(subagent.finished_at_ms, Some(99));
        assert_eq!(subagent.tools[0].status, ToolActivityStatus::Interrupted);
        assert_eq!(items[0].version, 1);
        assert_eq!(
            items[1].tool.as_ref().expect("flat tool").status,
            ToolActivityStatus::Interrupted
        );
        assert_eq!(items[1].version, 1);
        // Уже терминальная карточка не трогается и не будит подписчиков.
        assert_eq!(
            items[2].tool.as_ref().expect("done tool").status,
            ToolActivityStatus::Done
        );
        assert_eq!(items[2].version, 0);

        let rail = tool_activities.get_untracked();
        assert_eq!(rail[0].status, ToolActivityStatus::Interrupted);
        assert_eq!(rail[1].status, ToolActivityStatus::Done);
    });
}

#[test]
fn finalize_running_activity_preserves_spawned_background_subagent() {
    let owner = Owner::new();
    owner.with(|| {
        let mut activity = subagent_activity("background-thread", SubagentActivityStatus::Running);
        activity
            .tools
            .push(tool_activity("nested", ToolActivityStatus::Running));
        let mut message = subagent_message(1, activity);
        let mut spawn_tool = tool_activity("spawn", ToolActivityStatus::Done);
        spawn_tool.name = SPAWN_AGENT_TOOL.to_owned();
        message.tool = Some(spawn_tool);
        let (messages, set_messages) = signal(vec![message]);
        let (tool_activities, set_tool_activities) = signal(Vec::new());

        finalize_running_activity(set_tool_activities, set_messages, 99);

        let items = messages.get_untracked();
        let subagent = items[0].subagent.as_ref().expect("background subagent");
        assert_eq!(subagent.status, SubagentActivityStatus::Running);
        assert_eq!(subagent.finished_at_ms, None);
        assert_eq!(subagent.tools[0].status, ToolActivityStatus::Running);
        assert_eq!(items[0].version, 0);
        assert!(tool_activities.get_untracked().is_empty());
    });
}

#[test]
fn finalize_running_activity_preserves_followup_background_subagent() {
    let owner = Owner::new();
    owner.with(|| {
        let mut message = subagent_message(
            1,
            subagent_activity("followup-thread", SubagentActivityStatus::Running),
        );
        let mut followup_tool = tool_activity("followup", ToolActivityStatus::Done);
        followup_tool.name = FOLLOWUP_TASK_TOOL.to_owned();
        message.tool = Some(followup_tool);
        let (messages, set_messages) = signal(vec![message]);
        let (_tool_activities, set_tool_activities) = signal(Vec::new());

        finalize_running_activity(set_tool_activities, set_messages, 99);

        let items = messages.get_untracked();
        let subagent = items[0].subagent.as_ref().expect("follow-up subagent");
        assert_eq!(subagent.status, SubagentActivityStatus::Running);
        assert_eq!(subagent.finished_at_ms, None);
        assert_eq!(items[0].version, 0);
    });
}

#[test]
fn push_assistant_message_if_missing_skips_existing_final_output() {
    let owner = Owner::new();
    owner.with(|| {
        let (messages, set_messages) = signal(vec![Message {
            message_id: None,
            phase: None,
            id: 1,
            version: 0,
            text_offset: 0,
            role: MessageRole::Assistant,
            text: "final answer".to_owned(),
            tool: None,
            subagent: None,
            streaming: false,
        }]);
        let (next_message_id, set_next_message_id) = signal(2);

        push_assistant_message_if_missing(
            set_messages,
            next_message_id,
            set_next_message_id,
            "final answer".to_owned(),
        );

        assert_eq!(messages.get_untracked().len(), 1);
        assert_eq!(next_message_id.get_untracked(), 2);
    });
}
