use leptos::prelude::*;

mod text_range;
pub(crate) use text_range::merge_text;

use crate::tool_names::{FOLLOWUP_TASK_TOOL, SPAWN_AGENT_TOOL, TASK_TOOL};
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
    set_messages: crate::transcript::TranscriptWriter,
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
    set_messages: crate::transcript::TranscriptWriter,
    next_message_id: ReadSignal<u64>,
    set_next_message_id: WriteSignal<u64>,
    role: MessageRole,
    text: impl Into<String>,
) {
    let id = next_message_id.get();
    set_next_message_id.set(id + 1);
    set_messages.update(|items| {
        items.push(Message {
            message_id: None,
            phase: None,
            id,
            version: 0,
            text_offset: 0,
            role,
            text: text.into(),
            tool: None,
            subagent: None,
            streaming: false,
        });
    });
}

pub(crate) fn push_user_message_once(
    set_messages: crate::transcript::TranscriptWriter,
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
            message_id: None,
            phase: None,
            id,
            version: 0,
            text_offset: 0,
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
    set_messages: crate::transcript::TranscriptWriter,
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
            message_id: None,
            phase: None,
            id,
            version: 0,
            text_offset: 0,
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

pub(crate) fn adopt_streaming_tail(
    transcript: &[Message],
    set_active_stream_message_id: WriteSignal<Option<u64>>,
    set_streamed_this_turn: WriteSignal<bool>,
) {
    let Some(last) = transcript.iter().rev().find(|message| {
        message.role == MessageRole::Assistant && message.streaming && message.tool.is_none()
    }) else {
        return;
    };
    set_active_stream_message_id.set(Some(last.id));
    set_streamed_this_turn.set(true);
}

pub(crate) fn finish_streaming_reasoning(set_messages: crate::transcript::TranscriptWriter) {
    set_messages.update_where(
        |message| message.role == MessageRole::Reasoning && message.streaming,
        |message| {
            message.streaming = false;
            message.version += 1;
        },
    );
}

pub(crate) fn push_tool_message(
    set_messages: crate::transcript::TranscriptWriter,
    next_message_id: ReadSignal<u64>,
    set_next_message_id: WriteSignal<u64>,
    tool: ToolActivity,
) {
    let id = next_message_id.get();
    set_next_message_id.set(id + 1);
    set_messages.update(|items| {
        items.push(Message {
            message_id: None,
            phase: None,
            id,
            version: 0,
            text_offset: 0,
            role: MessageRole::System,
            text: String::new(),
            tool: Some(tool),
            subagent: None,
            streaming: false,
        });
    });
}

pub(crate) fn finish_active_streaming_assistant_message(
    set_messages: crate::transcript::TranscriptWriter,
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

pub(crate) fn finish_streaming_assistant_message(
    set_messages: crate::transcript::TranscriptWriter,
    next_message_id: ReadSignal<u64>,
    set_next_message_id: WriteSignal<u64>,
    active_stream_message_id: ReadSignal<Option<u64>>,
    set_active_stream_message_id: WriteSignal<Option<u64>>,
    final_text: String,
) {
    if let Some(message_id) = active_stream_message_id.get() {
        set_messages.update(|items| {
            if let Some(message) = items.iter_mut().find(|message| message.id == message_id) {
                // A turn output is not allowed to replace another canonical item.
                if message.message_id.is_none() {
                    message.text = final_text.clone();
                }
                message.streaming = false;
                message.version += 1;
            }
        });
        set_active_stream_message_id.set(None);
        push_assistant_message_once(
            set_messages,
            next_message_id,
            set_next_message_id,
            final_text,
        );
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
    set_messages: crate::transcript::TranscriptWriter,
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
    set_messages: crate::transcript::TranscriptWriter,
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
                // `spawn_agent`/`followup_task` возвращают управление сразу,
                // а ребёнок продолжает жить между родительскими turn-ами. Его карточку
                // и вложенные tools закроет настоящий SubagentFinished;
                // граница TurnOutput родителя для них не терминальна.
                let background = message.tool.as_ref().is_some_and(|tool| {
                    matches!(tool.name.as_str(), SPAWN_AGENT_TOOL | FOLLOWUP_TASK_TOOL)
                });
                if background {
                    if changed {
                        message.version += 1;
                    }
                    continue;
                }
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

/// Карточка субагента по `SubagentStarted`. Если в ленте бежит вызов одного из
/// subagent facade tools (он эмитит ToolCallRequested перед запуском ребёнка),
/// активность прикрепляется к нему — одна карточка вместо дубля «task +
/// субагент», как и в снапшоте turn progress. Иначе — отдельная карточка
/// (другой workflow может звать AgentControl без tool `task`). Повтор
/// события для уже бегущего child_thread_id игнорируется; resume завершённой
/// задачи (тот же thread, новый вызов task) — новая карточка.
pub(crate) fn push_subagent_message(
    set_messages: crate::transcript::TranscriptWriter,
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
                && message.tool.as_ref().is_some_and(|tool| {
                    matches!(
                        tool.name.as_str(),
                        TASK_TOOL | SPAWN_AGENT_TOOL | FOLLOWUP_TASK_TOOL
                    ) && !tool.status.is_terminal()
                })
        }) {
            message.subagent = Some(activity);
            message.version += 1;
            return;
        }
        items.push(Message {
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
    set_messages: crate::transcript::TranscriptWriter,
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
    set_messages: crate::transcript::TranscriptWriter,
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
mod tests;
