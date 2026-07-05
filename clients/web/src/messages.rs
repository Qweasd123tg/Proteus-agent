use leptos::prelude::*;

use crate::tool_names::TASK_TOOL;
use crate::types::{
    Message, MessageRole, SubagentActivity, SubagentActivityStatus, ToolActivity,
    ToolActivityStatus, TransportStatus,
};
use crate::ui_utils::compact_text;

/// Потолок превью результата для вызовов внутри карточки субагента. Вложенные
/// карточки живут в одном Message: каждый его апдейт клонируется и глубоко
/// сравнивается реактивной лентой, поэтому полные выводы child tools раздували
/// карточку до мегабайтов и подвешивали браузер. Полный вывод всё равно уходит
/// только дочерней модели — в UI хватает усечённого превью.
pub(crate) const NESTED_TOOL_PREVIEW_CHAR_LIMIT: usize = 10_000;

pub(crate) fn report_error(
    set_messages: WriteSignal<Vec<Message>>,
    next_message_id: ReadSignal<u64>,
    set_next_message_id: WriteSignal<u64>,
    set_transport_status: WriteSignal<TransportStatus>,
    prefix: &str,
    error: String,
) {
    let message = format!("{prefix}: {error}");
    set_transport_status.set(TransportStatus::Error(message.clone()));
    push_message(
        set_messages,
        next_message_id,
        set_next_message_id,
        MessageRole::System,
        message,
    );
}

pub(crate) fn push_message(
    set_messages: WriteSignal<Vec<Message>>,
    next_message_id: ReadSignal<u64>,
    set_next_message_id: WriteSignal<u64>,
    role: MessageRole,
    text: impl Into<String>,
) {
    let id = next_message_id.get();
    set_next_message_id.set(id + 1);
    set_messages.update(|items| {
        items.push(Message {
            id,
            version: 0,
            role,
            text: text.into(),
            tool: None,
            subagent: None,
            streaming: false,
        });
    });
}

pub(crate) fn push_user_message_once(
    set_messages: WriteSignal<Vec<Message>>,
    next_message_id: ReadSignal<u64>,
    set_next_message_id: WriteSignal<u64>,
    text: impl Into<String>,
) {
    let text = text.into();
    let id = next_message_id.get();
    let mut pushed = false;
    set_messages.update(|items| {
        if items
            .last()
            .is_some_and(|message| message.role == MessageRole::User && message.text == text)
        {
            return;
        }
        items.push(Message {
            id,
            version: 0,
            role: MessageRole::User,
            text,
            tool: None,
            subagent: None,
            streaming: false,
        });
        pushed = true;
    });
    if pushed {
        set_next_message_id.set(id + 1);
    }
}

pub(crate) fn push_assistant_message_once(
    set_messages: WriteSignal<Vec<Message>>,
    next_message_id: ReadSignal<u64>,
    set_next_message_id: WriteSignal<u64>,
    text: impl Into<String>,
) {
    let text = text.into();
    let id = next_message_id.get();
    let mut pushed = false;
    set_messages.update(|items| {
        if items
            .last()
            .is_some_and(|message| message.role == MessageRole::Assistant && message.text == text)
        {
            return;
        }
        items.push(Message {
            id,
            version: 0,
            role: MessageRole::Assistant,
            text,
            tool: None,
            subagent: None,
            streaming: false,
        });
        pushed = true;
    });
    if pushed {
        set_next_message_id.set(id + 1);
    }
}

pub(crate) fn push_assistant_message_if_missing(
    set_messages: WriteSignal<Vec<Message>>,
    next_message_id: ReadSignal<u64>,
    set_next_message_id: WriteSignal<u64>,
    text: String,
) {
    if text.trim().is_empty() {
        return;
    }

    let id = next_message_id.get();
    let mut pushed = false;
    set_messages.update(|items| {
        if items
            .iter()
            .any(|message| message.role == MessageRole::Assistant && message.text == text)
        {
            return;
        }
        items.push(Message {
            id,
            version: 0,
            role: MessageRole::Assistant,
            text,
            tool: None,
            subagent: None,
            streaming: false,
        });
        pushed = true;
    });
    if pushed {
        set_next_message_id.set(id + 1);
    }
}

/// Если хвост загруженного транскрипта — незавершённое streaming-сообщение
/// ассистента (сервер отдал прогресс бегущего хода), делаем его целью для
/// последующих SSE-дельт: текст продолжит дописываться в него, а TurnOutput
/// в конце перезапишет его финальным текстом.
pub(crate) fn adopt_streaming_tail(
    transcript: &[Message],
    set_active_stream_message_id: WriteSignal<Option<u64>>,
    set_streamed_this_turn: WriteSignal<bool>,
) {
    let Some(last) = transcript.last() else {
        return;
    };
    if last.role == MessageRole::Assistant && last.streaming && last.tool.is_none() {
        set_active_stream_message_id.set(Some(last.id));
        set_streamed_this_turn.set(true);
    }
}

/// Подкладывает историю с сервера перед уже накопленными живыми сообщениями:
/// при старте посреди активного хода SSE успевает доставить стрим-дельты
/// раньше, чем приходит ответ /history, и историю нельзя ни выбросить, ни
/// поставить после хвоста. Id истории выделяются поверх текущего счётчика,
/// чтобы не столкнуться с id живых сообщений (порядок ленты задаёт Vec, не id).
pub(crate) fn prepend_history_messages(
    set_messages: WriteSignal<Vec<Message>>,
    next_message_id: ReadSignal<u64>,
    set_next_message_id: WriteSignal<u64>,
    set_active_stream_message_id: WriteSignal<Option<u64>>,
    set_streamed_this_turn: WriteSignal<bool>,
    mut transcript: Vec<Message>,
) {
    let base = next_message_id.get();
    for (index, message) in transcript.iter_mut().enumerate() {
        message.id = base + index as u64;
    }
    set_next_message_id.set(base + transcript.len() as u64);
    let history_has_streaming_tail = transcript.last().is_some_and(|message| {
        message.role == MessageRole::Assistant && message.streaming && message.tool.is_none()
    });
    let streaming_tail_id = history_has_streaming_tail
        .then(|| transcript.last().map(|message| message.id))
        .flatten();
    set_messages.update(|items| {
        // Стрим-хвост из снапшота прогресса уже содержит текст, который SSE
        // успел доставить живьём после подключения, — локальный стрим-дубль
        // убираем, дальше дельты пойдут в усыновлённое сообщение истории.
        if history_has_streaming_tail {
            items.retain(|live| {
                !(live.role == MessageRole::Assistant && live.streaming && live.tool.is_none())
            });
        }
        // Если /history пришёл после live SSE дочернего цикла, TurnProgress
        // может отдать тот же child tool как плоскую карточку. Live-карточка
        // субагента информативнее: сохраняем её и выкидываем плоский дубль.
        let nested_live_call_ids = items
            .iter()
            .filter_map(|message| message.subagent.as_ref())
            .flat_map(|subagent| subagent.tools.iter().map(|tool| tool.call_id.clone()))
            .collect::<Vec<_>>();
        if !nested_live_call_ids.is_empty() {
            transcript.retain(|hist| {
                !hist.tool.as_ref().is_some_and(|tool| {
                    nested_live_call_ids
                        .iter()
                        .any(|call_id| call_id == &tool.call_id)
                })
            });
        }
        // Если ход успел завершиться, пока /history был в пути, живые
        // сообщения могут дублировать хвост истории — локальный дубль убираем.
        items.retain(|live| !history_duplicates_live(&transcript, live));
        let live = std::mem::take(items);
        *items = transcript;
        items.extend(live);
    });
    if let Some(id) = streaming_tail_id {
        set_active_stream_message_id.set(Some(id));
        set_streamed_this_turn.set(true);
    }
}

fn history_duplicates_live(transcript: &[Message], live: &Message) -> bool {
    if let Some(live_tool) = &live.tool {
        return transcript.iter().any(|hist| {
            hist.tool
                .as_ref()
                .is_some_and(|tool| tool.call_id == live_tool.call_id)
        });
    }
    if let Some(live_subagent) = &live.subagent {
        return transcript.iter().any(|hist| {
            hist.subagent
                .as_ref()
                .is_some_and(|subagent| subagent.child_thread_id == live_subagent.child_thread_id)
        });
    }
    if live.text.trim().is_empty() || live.streaming {
        return false;
    }
    transcript
        .iter()
        .rev()
        .take(4)
        .any(|hist| hist.tool.is_none() && hist.role == live.role && hist.text == live.text)
}

/// Завершить активный reasoning-блок (сворачивается в UI). Вызывается, когда
/// начинается текст ответа, tool call или ход завершается.
pub(crate) fn finish_streaming_reasoning(set_messages: WriteSignal<Vec<Message>>) {
    set_messages.update(|items| {
        for message in items.iter_mut() {
            if message.role == MessageRole::Reasoning && message.streaming {
                message.streaming = false;
                message.version += 1;
            }
        }
    });
}

pub(crate) fn push_tool_message(
    set_messages: WriteSignal<Vec<Message>>,
    next_message_id: ReadSignal<u64>,
    set_next_message_id: WriteSignal<u64>,
    tool: ToolActivity,
) {
    let id = next_message_id.get();
    set_next_message_id.set(id + 1);
    set_messages.update(|items| {
        items.push(Message {
            id,
            version: 0,
            role: MessageRole::System,
            text: String::new(),
            tool: Some(tool),
            subagent: None,
            streaming: false,
        });
    });
}

pub(crate) fn append_streaming_assistant_delta(
    set_messages: WriteSignal<Vec<Message>>,
    next_message_id: ReadSignal<u64>,
    set_next_message_id: WriteSignal<u64>,
    active_stream_message_id: ReadSignal<Option<u64>>,
    set_active_stream_message_id: WriteSignal<Option<u64>>,
    text: &str,
) {
    if text.is_empty() {
        return;
    }

    if let Some(message_id) = active_stream_message_id.get() {
        set_messages.update(|items| {
            if let Some(message) = items.iter_mut().find(|message| message.id == message_id) {
                message.text.push_str(text);
                message.version += 1;
            }
        });
    } else {
        let id = next_message_id.get();
        set_next_message_id.set(id + 1);
        set_active_stream_message_id.set(Some(id));
        set_messages.update(|items| {
            items.push(Message {
                id,
                version: 0,
                role: MessageRole::Assistant,
                text: text.to_owned(),
                tool: None,
                subagent: None,
                streaming: true,
            });
        });
    }
}

pub(crate) fn finish_active_streaming_assistant_message(
    set_messages: WriteSignal<Vec<Message>>,
    active_stream_message_id: ReadSignal<Option<u64>>,
    set_active_stream_message_id: WriteSignal<Option<u64>>,
) {
    if let Some(message_id) = active_stream_message_id.get() {
        set_messages.update(|items| {
            if let Some(message) = items.iter_mut().find(|message| message.id == message_id) {
                message.streaming = false;
                message.version += 1;
            }
        });
        set_active_stream_message_id.set(None);
    }
}

pub(crate) fn finish_all_streaming_assistant_messages(set_messages: WriteSignal<Vec<Message>>) {
    set_messages.update(|items| {
        for message in items {
            if message.role == MessageRole::Assistant && message.streaming {
                message.streaming = false;
                message.version += 1;
            }
        }
    });
}

pub(crate) fn finish_streaming_assistant_message(
    set_messages: WriteSignal<Vec<Message>>,
    next_message_id: ReadSignal<u64>,
    set_next_message_id: WriteSignal<u64>,
    active_stream_message_id: ReadSignal<Option<u64>>,
    set_active_stream_message_id: WriteSignal<Option<u64>>,
    final_text: String,
) {
    if let Some(message_id) = active_stream_message_id.get() {
        set_messages.update(|items| {
            if let Some(message) = items.iter_mut().find(|message| message.id == message_id) {
                message.text = final_text.clone();
                message.streaming = false;
                message.version += 1;
            }
        });
        set_active_stream_message_id.set(None);
    } else {
        push_assistant_message_once(
            set_messages,
            next_message_id,
            set_next_message_id,
            final_text,
        );
    }
}

/// Обновляет статус tool-вызова в рейке активности и в ленте (плоская
/// карточка или вложенный в субагента вызов). Терминальный статус фиксирует
/// `finished_at_ms = now_ms` для duration. Возвращает true, если вызов найден
/// внутри карточки субагента — статусная строка говорит о субагенте, а не о
/// родителе.
pub(crate) fn update_tool_status(
    set_tool_activities: WriteSignal<Vec<ToolActivity>>,
    set_messages: WriteSignal<Vec<Message>>,
    call_id: &str,
    status: ToolActivityStatus,
    result_preview: Option<String>,
    now_ms: u64,
) -> bool {
    let finished_at_ms = status.is_terminal().then_some(now_ms);
    set_tool_activities.update(|items| {
        if let Some(item) = items.iter_mut().find(|item| item.call_id == call_id) {
            item.status = status;
            if let Some(finished_at_ms) = finished_at_ms {
                item.finished_at_ms = Some(finished_at_ms);
            }
            if let Some(result_preview) = result_preview.clone() {
                item.result_preview = Some(result_preview);
            }
        }
    });
    let mut nested = false;
    set_messages.update(|items| {
        for message in items.iter_mut() {
            if let Some(tool) = message.tool.as_mut().filter(|tool| tool.call_id == call_id) {
                tool.status = status;
                if let Some(finished_at_ms) = finished_at_ms {
                    tool.finished_at_ms = Some(finished_at_ms);
                }
                if let Some(result_preview) = result_preview.clone() {
                    tool.result_preview = Some(result_preview);
                }
                message.version += 1;
                return;
            }
            // Tool-вызовы дочернего цикла лежат внутри карточки субагента —
            // Approval*/ToolFinished находят их по тому же call_id. Превью
            // результата усечено (см. NESTED_TOOL_PREVIEW_CHAR_LIMIT).
            if let Some(subagent) = message.subagent.as_mut()
                && let Some(tool) = subagent
                    .tools
                    .iter_mut()
                    .find(|tool| tool.call_id == call_id)
            {
                tool.status = status;
                if let Some(finished_at_ms) = finished_at_ms {
                    tool.finished_at_ms = Some(finished_at_ms);
                }
                if let Some(result_preview) = result_preview.as_deref() {
                    tool.result_preview =
                        Some(compact_text(result_preview, NESTED_TOOL_PREVIEW_CHAR_LIMIT));
                }
                message.version += 1;
                nested = true;
                return;
            }
        }
    });
    nested
}

/// Финализация на границе хода (TurnOutput/Error/Shutdown): все ещё бегущие
/// tool- и subagent-карточки принудительно закрываются статусом «прервано».
/// Терминальное событие после конца хода уже не придёт (пропущенный
/// ToolFinished, обрыв SSE, упавший ход) — без этого спиннеры и таймеры
/// крутились бы вечно.
pub(crate) fn finalize_running_activity(
    set_tool_activities: WriteSignal<Vec<ToolActivity>>,
    set_messages: WriteSignal<Vec<Message>>,
    now_ms: u64,
) {
    set_tool_activities.update(|items| {
        for tool in items.iter_mut() {
            interrupt_tool(tool, now_ms);
        }
    });
    set_messages.update(|items| {
        for message in items.iter_mut() {
            let mut changed = false;
            if let Some(tool) = message.tool.as_mut() {
                changed |= interrupt_tool(tool, now_ms);
            }
            if let Some(subagent) = message.subagent.as_mut() {
                if subagent.is_running() {
                    subagent.status = SubagentActivityStatus::Finished("interrupted".to_owned());
                    subagent.finished_at_ms = Some(now_ms);
                    changed = true;
                }
                for tool in subagent.tools.iter_mut() {
                    changed |= interrupt_tool(tool, now_ms);
                }
            }
            if changed {
                message.version += 1;
            }
        }
    });
}

fn interrupt_tool(tool: &mut ToolActivity, now_ms: u64) -> bool {
    if tool.status.is_terminal() {
        return false;
    }
    tool.status = ToolActivityStatus::Interrupted;
    tool.finished_at_ms = Some(now_ms);
    true
}

/// Карточка субагента по `SubagentStarted`. Если в ленте бежит вызов `task`
/// (workflow эмитит его ToolCallRequested перед запуском субагента),
/// активность прикрепляется к нему — одна карточка вместо дубля «task +
/// субагент», как и в снапшоте turn progress. Иначе — отдельная карточка
/// (другой workflow может звать SubagentRunner без tool `task`). Повтор
/// события для уже бегущего child_thread_id игнорируется; resume завершённой
/// задачи (тот же thread, новый вызов task) — новая карточка.
pub(crate) fn push_subagent_message(
    set_messages: WriteSignal<Vec<Message>>,
    next_message_id: ReadSignal<u64>,
    set_next_message_id: WriteSignal<u64>,
    activity: SubagentActivity,
) {
    let id = next_message_id.get();
    let mut pushed = false;
    set_messages.update(|items| {
        if items.iter().any(|message| {
            message.subagent.as_ref().is_some_and(|subagent| {
                subagent.child_thread_id == activity.child_thread_id && subagent.is_running()
            })
        }) {
            return;
        }
        if let Some(message) = items.iter_mut().rev().find(|message| {
            message.subagent.is_none()
                && message
                    .tool
                    .as_ref()
                    .is_some_and(|tool| tool.name == TASK_TOOL && !tool.status.is_terminal())
        }) {
            message.subagent = Some(activity);
            message.version += 1;
            return;
        }
        items.push(Message {
            id,
            version: 0,
            role: MessageRole::System,
            text: String::new(),
            tool: None,
            subagent: Some(activity),
            streaming: false,
        });
        pushed = true;
    });
    if pushed {
        set_next_message_id.set(id + 1);
    }
}

/// Закрывает карточку субагента по `SubagentFinished`; `now_ms` фиксирует
/// длительность. Если карточки нет (страница открылась посреди работы
/// субагента), событие игнорируется — итог всё равно виден в summary
/// tool-вызова `task` из истории.
pub(crate) fn finish_subagent_message(
    set_messages: WriteSignal<Vec<Message>>,
    child_thread_id: &str,
    status: SubagentActivityStatus,
    iterations: Option<u32>,
    now_ms: u64,
) {
    set_messages.update(|items| {
        if let Some(message) = items.iter_mut().rev().find(|message| {
            message.subagent.as_ref().is_some_and(|subagent| {
                subagent.child_thread_id == child_thread_id && subagent.is_running()
            })
        }) {
            let Some(subagent) = message.subagent.as_mut() else {
                return;
            };
            subagent.status = status;
            subagent.iterations = iterations;
            subagent.finished_at_ms = Some(now_ms);
            message.version += 1;
        }
    });
}

/// Вкладывает tool-вызов дочернего цикла в бегущую карточку субагента с тем
/// же thread_id. Возвращает false, если подходящей карточки нет — вызывающий
/// рисует обычную плоскую карточку.
pub(crate) fn push_subagent_tool(
    set_messages: WriteSignal<Vec<Message>>,
    thread_id: &str,
    tool: ToolActivity,
) -> bool {
    let mut nested = false;
    set_messages.update(|items| {
        if let Some(message) = items.iter_mut().rev().find(|message| {
            message.subagent.as_ref().is_some_and(|subagent| {
                subagent.child_thread_id == thread_id && subagent.is_running()
            })
        }) {
            let Some(subagent) = message.subagent.as_mut() else {
                return;
            };
            if !subagent
                .tools
                .iter()
                .any(|item| item.call_id == tool.call_id)
            {
                let mut tool = tool;
                if let Some(result_preview) = tool.result_preview.as_deref() {
                    tool.result_preview =
                        Some(compact_text(result_preview, NESTED_TOOL_PREVIEW_CHAR_LIMIT));
                }
                subagent.tools.push(tool);
            }
            message.version += 1;
            nested = true;
        }
    });
    nested
}

#[cfg(test)]
mod tests {
    use leptos::prelude::Owner;
    use serde_json::Value;

    use super::*;

    #[test]
    fn finish_active_streaming_assistant_message_marks_message_done() {
        let owner = Owner::new();
        owner.with(|| {
            let (messages, set_messages) = signal(vec![Message {
                id: 1,
                version: 0,
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
                id: 1,
                version: 0,
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
            id,
            version: 0,
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

    fn subagent_activity(
        child_thread_id: &str,
        status: SubagentActivityStatus,
    ) -> SubagentActivity {
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
            id,
            version: 0,
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
                id: 1,
                version: 0,
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
                id: 1,
                version: 0,
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
                id: 1,
                version: 0,
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
    fn push_assistant_message_if_missing_skips_existing_final_output() {
        let owner = Owner::new();
        owner.with(|| {
            let (messages, set_messages) = signal(vec![Message {
                id: 1,
                version: 0,
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
}
