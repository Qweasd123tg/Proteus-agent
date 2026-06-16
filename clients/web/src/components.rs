use std::collections::HashMap;

use leptos::{prelude::*, task::spawn_local};
use serde_json::Value;
use web_sys::{MouseEvent, window};

use crate::api::{get_json, post_json};
use crate::markdown::markdown_html;
use crate::types::*;
use crate::ui_utils::{compact_json, copy_to_clipboard, short_id, short_path};

#[component]
pub(crate) fn ResumeView() -> impl IntoView {
    let (sessions, set_sessions) = signal(Vec::<SessionSummary>::new());
    let (status, set_status) = signal("загружаю сессии".to_owned());

    load_sessions(set_sessions, set_status);

    let refresh = move |_| load_sessions(set_sessions, set_status);
    let resume = move |session_dir: String| {
        set_status.set("возвращаю сессию".to_owned());
        spawn_local(async move {
            match post_json(
                "/resume",
                &ResumeSessionRequest {
                    id: Some("resume".to_owned()),
                    session_dir,
                },
            )
            .await
            {
                Ok(StdioOutput::Response { ok: true, .. }) => {
                    set_status.set("сессия открыта".to_owned());
                    if let Some(window) = window() {
                        let _ = window.location().set_href("/");
                    }
                }
                Ok(StdioOutput::Response { error, .. }) => {
                    set_status.set(error.unwrap_or_else(|| "не удалось открыть сессию".to_owned()));
                }
                Ok(StdioOutput::Event { .. }) => {
                    set_status.set("неожиданное событие resume".to_owned());
                }
                Err(error) => set_status.set(format!("не удалось открыть сессию: {error}")),
            }
        });
    };

    view! {
        <section class="resume-page">
            <div class="resume-toolbar">
                <div>
                    <h2>"Прошлые сессии"</h2>
                    <p>{move || status.get()}</p>
                </div>
                <button type="button" class="secondary" on:click=refresh>"Обновить"</button>
            </div>
            {move || {
                let items = sessions.get();
                if items.is_empty() {
                    view! {
                        <div class="empty-state">
                            <div class="empty-state-title">"Сохранённых сессий нет"</div>
                        </div>
                    }.into_any()
                } else {
                    view! {
                        <div class="resume-list">
                            <For
                                each=move || sessions.get()
                                key=|session| session.session_dir.clone()
                                children=move |session| {
                                    let session_dir = session.session_dir.clone();
                                    let workspace = session.workspace_path.clone().unwrap_or_else(|| "неизвестный workspace".to_owned());
                                    let session_id = session
                                        .session_id
                                        .as_deref()
                                        .map(short_id)
                                        .unwrap_or("legacy")
                                        .to_owned();
                                    view! {
                                        <article class="resume-item">
                                            <div class="resume-item-main">
                                                <div class="resume-item-header">
                                                    <strong>{short_path(&workspace)}</strong>
                                                    <code>{session_id}</code>
                                                </div>
                                                <p>{session.preview.clone().unwrap_or_else(|| "Нет превью диалога".to_owned())}</p>
                                                <div class="resume-meta">
                                                    <span>{workspace}</span>
                                                    <span>{format!("{} сообщений", session.message_count)}</span>
                                                </div>
                                            </div>
                                            <button
                                                type="button"
                                                class="btn-primary"
                                                disabled=!session.resumable
                                                on:click=move |_| resume(session_dir.clone())
                                            >
                                                "Открыть"
                                            </button>
                                        </article>
                                    }
                                }
                            />
                        </div>
                    }.into_any()
                }
            }}
        </section>
    }
}

fn load_sessions(set_sessions: WriteSignal<Vec<SessionSummary>>, set_status: WriteSignal<String>) {
    spawn_local(async move {
        match get_json::<Vec<SessionSummary>>("/sessions").await {
            Ok(items) => {
                let count = items.len();
                set_sessions.set(items);
                set_status.set(format!("{count} сессий"));
            }
            Err(error) => set_status.set(format!("не удалось загрузить сессии: {error}")),
        }
    });
}

#[component]
pub(crate) fn ApprovalCard<F>(request: ApprovalRequestInfo, on_resolve: F) -> impl IntoView
where
    F: Fn(String, bool, ApprovalCacheScope) + Copy + 'static,
{
    let (cache, set_cache) = signal(ApprovalCacheScope::None);
    let approve_id = request.approval_id.clone();
    let deny_id = request.approval_id.clone();
    let args_preview = compact_json(&request.call.args);
    let is_safe_cache_target = approval_allows_tool_cwd_cache(&request);
    let spec_hint = request
        .tool_spec
        .as_ref()
        .and_then(|spec| spec.get("description"))
        .and_then(Value::as_str)
        .unwrap_or(&request.reason)
        .to_owned();

    view! {
        <article class="control-card approval-card">
            <div class="control-card-header">
                <span class="status-badge running">
                    <span class="dot"></span>
                    "Доступ"
                </span>
                <strong>{request.call.name}</strong>
                <code>{short_path(&request.cwd)}</code>
            </div>
            <p>{spec_hint}</p>
            <pre>{args_preview}</pre>
            <div class="control-row">
                <span class="control-label">"Кэш"</span>
                <div class="segmented">
                    <button
                        type="button"
                        class:active=move || cache.get() == ApprovalCacheScope::None
                        on:click=move |_| set_cache.set(ApprovalCacheScope::None)
                    >
                        {ApprovalCacheScope::None.label()}
                    </button>
                    <button
                        type="button"
                        class:active=move || cache.get() == ApprovalCacheScope::ExactCall
                        on:click=move |_| set_cache.set(ApprovalCacheScope::ExactCall)
                    >
                        {ApprovalCacheScope::ExactCall.label()}
                    </button>
                    {if is_safe_cache_target {
                        view! {
                            <button
                                type="button"
                                class:active=move || cache.get() == ApprovalCacheScope::ToolInCwd
                                on:click=move |_| set_cache.set(ApprovalCacheScope::ToolInCwd)
                            >
                                {ApprovalCacheScope::ToolInCwd.label()}
                            </button>
                        }.into_any()
                    } else {
                        view! { <></> }.into_any()
                    }}
                </div>
            </div>
            <div class="control-actions">
                <button
                    type="button"
                    class="secondary danger"
                    on:click=move |_| on_resolve(deny_id.clone(), false, ApprovalCacheScope::None)
                >
                    "Отклонить"
                </button>
                <button
                    type="button"
                    class="btn-primary"
                    on:click=move |_| on_resolve(approve_id.clone(), true, cache.get())
                >
                    "Разрешить"
                </button>
            </div>
        </article>
    }
}

fn approval_allows_tool_cwd_cache(request: &ApprovalRequestInfo) -> bool {
    request
        .tool_spec
        .as_ref()
        .and_then(|spec| spec.get("safety"))
        .and_then(Value::as_str)
        .is_some_and(|safety| matches!(safety, "ReadOnly" | "WritesFiles"))
}

#[component]
pub(crate) fn UserInputCard<F>(request: UserInputRequestInfo, on_submit: F) -> impl IntoView
where
    F: Fn(String, HashMap<String, Vec<String>>) + Copy + 'static,
{
    let (answers, set_answers) = signal(HashMap::<String, Vec<String>>::new());
    let (custom_answers, set_custom_answers) = signal(HashMap::<String, String>::new());
    let (current_question, set_current_question) = signal(0usize);
    let request_id = request.request_id.clone();
    let title = request
        .title
        .clone()
        .unwrap_or_else(|| "Нужен ответ".to_owned());
    let questions = request.questions.clone();
    let question_count = questions.len();
    let tabs = questions
        .iter()
        .enumerate()
        .map(|(index, question)| {
            let label = if question.header.trim().is_empty() {
                format!("Вопрос {}", index + 1)
            } else {
                question.header.clone()
            };
            (index, label)
        })
        .collect::<Vec<_>>();

    let submit_answers = move || {
        let mut merged = answers.get();
        for (question_id, value) in custom_answers.get() {
            let value = value.trim().to_owned();
            if !value.is_empty() {
                merged.entry(question_id).or_default().push(value);
            }
        }
        on_submit(request_id.clone(), merged);
    };
    let confirm = move |_| {
        if question_count == 0 || current_question.get() + 1 >= question_count {
            submit_answers();
        } else {
            set_current_question.update(|index| *index += 1);
        }
    };

    view! {
        <article class="task-card running input-chat-card">
            <div class="task-card-header input-chat-header">
                <span class="status-badge disconnected">
                    <span class="dot"></span>
                    "Вопрос"
                </span>
                <strong>{title}</strong>
                <span class="input-step">
                    {move || {
                        if question_count == 0 {
                            "0 / 0".to_owned()
                        } else {
                            format!("{} / {}", current_question.get() + 1, question_count)
                        }
                    }}
                </span>
            </div>
            <div class="input-tabs" role="tablist">
                <For
                    each=move || tabs.clone()
                    key=|(index, _)| *index
                    children=move |(index, label)| {
                        view! {
                            <button
                                type="button"
                                role="tab"
                                class=move || {
                                    if current_question.get() == index {
                                        "active"
                                    } else if current_question.get() > index {
                                        "done"
                                    } else {
                                        ""
                                    }
                                }
                                on:click=move |_| set_current_question.set(index)
                            >
                                <span>{label}</span>
                            </button>
                        }
                    }
                />
            </div>
            {move || {
                let Some(question) = questions.get(current_question.get()).cloned() else {
                    return view! {
                        <section class="input-question">
                            <div class="input-question-header">
                                <span>"Вопрос"</span>
                                <strong>"Вопросы не переданы"</strong>
                            </div>
                        </section>
                    }.into_any();
                };
                let question_id = question.id.clone();
                let custom_value_question_id = question.id.clone();
                let custom_write_question_id = question.id.clone();
                let header = question.header.clone();
                let question_text = question.question.clone();
                let options = question.options.clone();
                let multi_select = question.multi_select;
                let show_custom = question.is_other || question.options.is_empty();
                let input_type = if question.is_secret { "password" } else { "text" };

                view! {
                    <section class="input-question">
                        <div class="input-question-header">
                            <span>{header}</span>
                            <strong>{question_text}</strong>
                        </div>
                        <div class="choice-grid">
                            <For
                                each=move || options.clone()
                                key=|option| option.label.clone()
                                children=move |option| {
                                    let selected_question_id = question_id.clone();
                                    let selected_label = option.label.clone();
                                    let click_question_id = question_id.clone();
                                    let click_label = option.label.clone();
                                    view! {
                                        <button
                                            type="button"
                                            class="choice-button"
                                            class:active=move || {
                                                answers
                                                    .get()
                                                    .get(&selected_question_id)
                                                    .is_some_and(|values| values.contains(&selected_label))
                                            }
                                            on:click=move |_| {
                                                let question_id = click_question_id.clone();
                                                let label = click_label.clone();
                                                set_answers.update(|all| {
                                                    if multi_select {
                                                        let values = all.entry(question_id).or_default();
                                                        if let Some(index) = values.iter().position(|value| value == &label) {
                                                            values.remove(index);
                                                        } else {
                                                            values.push(label);
                                                        }
                                                    } else {
                                                        all.insert(question_id, vec![label]);
                                                    }
                                                });
                                            }
                                        >
                                            <span>{option.label}</span>
                                            <small>{option.description}</small>
                                        </button>
                                    }
                                }
                            />
                        </div>
                        {if show_custom {
                            view! {
                                <input
                                    type=input_type
                                    placeholder="Свой вариант"
                                    prop:value=move || {
                                        custom_answers
                                            .get()
                                            .get(&custom_value_question_id)
                                            .cloned()
                                            .unwrap_or_default()
                                    }
                                    on:input:target=move |ev| {
                                        set_custom_answers.update(|answers| {
                                            answers.insert(custom_write_question_id.clone(), ev.target().value());
                                        });
                                    }
                                />
                            }.into_any()
                        } else {
                            view! { <></> }.into_any()
                        }}
                    </section>
                }.into_any()
            }}
            <div class="input-chat-actions">
                <button
                    type="button"
                    class="secondary"
                    disabled=move || current_question.get() == 0
                    on:click=move |_| set_current_question.update(|index| *index = index.saturating_sub(1))
                >
                    "Назад"
                </button>
                <button type="button" class="btn-primary" on:click=confirm>
                    {move || {
                        if question_count == 0 || current_question.get() + 1 >= question_count {
                            "Отправить"
                        } else {
                            "Подтвердить"
                        }
                    }}
                </button>
            </div>
        </article>
    }
}

#[component]
pub(crate) fn ToastStack<F>(toasts: ReadSignal<Vec<ToastMessage>>, on_dismiss: F) -> impl IntoView
where
    F: Fn(u64) + Copy + Send + 'static,
{
    view! {
        <div class="toast-stack" aria-live="polite">
            <For
                each=move || toasts.get()
                key=|toast| toast.id
                children=move |toast| {
                    let toast_id = toast.id;
                    view! {
                        <div class="toast">
                            <span>{toast.text}</span>
                            <button
                                type="button"
                                class="secondary"
                                title="Закрыть"
                                on:click=move |_| on_dismiss(toast_id)
                            >
                                "×"
                            </button>
                        </div>
                    }
                }
            />
        </div>
    }
}

#[component]
pub(crate) fn QueuedPromptCard<S, C>(
    text: String,
    is_sending: ReadSignal<bool>,
    on_send: S,
    on_clear: C,
) -> impl IntoView
where
    S: Fn(MouseEvent) + 'static,
    C: Fn(MouseEvent) + 'static,
{
    let preview = text.clone();
    view! {
        <article class="task-card running queued-card">
            <div class="task-card-header">
                <span class="status-badge disconnected">
                    <span class="dot"></span>
                    "В очереди"
                </span>
            </div>
            <div class="message system-message queued-message">
                <p>{preview}</p>
                <div class="queued-actions">
                    <button
                        type="button"
                        class="btn-primary"
                        disabled=move || is_sending.get()
                        on:click=on_send
                    >
                        "Отправить"
                    </button>
                    <button type="button" class="secondary" on:click=on_clear>
                        "Убрать"
                    </button>
                </div>
            </div>
        </article>
    }
}

#[component]
pub(crate) fn PlanActionsCard<R, E, X>(on_revise: R, on_execute: E, on_exit: X) -> impl IntoView
where
    R: Fn(MouseEvent) + Copy + 'static,
    E: Fn(MouseEvent) + Copy + 'static,
    X: Fn(MouseEvent) + Copy + 'static,
{
    view! {
        <article class="task-card running plan-actions-card">
            <div class="task-card-header">
                <span class="status-badge running">
                    <span class="dot"></span>
                    "План готов"
                </span>
            </div>
            <div class="message system-message plan-actions-message">
                <button
                    type="button"
                    class="secondary"
                    on:click=on_revise
                    title="Уточнить последний план текстом из поля ввода"
                >
                    "Уточнить"
                </button>
                <button
                    type="button"
                    class="btn-primary"
                    on:click=on_execute
                    title="Переключиться в обычный режим и выполнить последний план"
                >
                    "Выполнить"
                </button>
                <button
                    type="button"
                    class="secondary"
                    on:click=on_exit
                    title="Вернуться в обычный режим"
                >
                    "Выйти"
                </button>
            </div>
        </article>
    }
}

#[component]
fn ToolActivityCard(message_id: u64, messages: ReadSignal<Vec<Message>>) -> impl IntoView {
    let (expanded, set_expanded) = signal(false);
    view! {
        <article class="tool-card">
            <button
                type="button"
                class="tool-card-summary"
                title="Показать детали tool"
                on:click=move |_| set_expanded.update(|value| *value = !*value)
            >
                <span class=move || current_tool(messages, message_id).map(|tool| tool.status.badge_class()).unwrap_or("status-badge idle")>
                    <span class=move || {
                        current_tool(messages, message_id)
                            .map(|tool| {
                                if matches!(tool.status, ToolActivityStatus::Running | ToolActivityStatus::WaitingApproval) {
                                    "spinner-dot"
                                } else {
                                    "dot"
                                }
                            })
                            .unwrap_or("dot")
                    }></span>
                    {move || current_tool(messages, message_id).map(|tool| tool.status.label()).unwrap_or("tool")}
                </span>
                <strong>{move || current_tool(messages, message_id).map(|tool| tool.name).unwrap_or_else(|| "tool".to_owned())}</strong>
                <code>{move || current_tool(messages, message_id).map(|tool| short_id(&tool.call_id).to_owned()).unwrap_or_default()}</code>
            </button>
            {move || {
                if expanded.get() {
                    view! {
                        <div class="tool-card-details">
                            <pre>{move || current_tool(messages, message_id).map(|tool| tool.args_preview).unwrap_or_default()}</pre>
                            {move || if let Some(result) = current_tool(messages, message_id).and_then(|tool| tool.result_preview) {
                                view! { <pre>{result}</pre> }.into_any()
                            } else {
                                view! { <></> }.into_any()
                            }}
                        </div>
                    }.into_any()
                } else {
                    view! { <></> }.into_any()
                }
            }}
        </article>
    }
}

#[component]
pub(crate) fn WorkingCard(status: ReadSignal<String>) -> impl IntoView {
    view! {
        <article class="task-card running working-card">
            <div class="task-card-header">
                <span class="status-badge running">
                    <span class="spinner-dot"></span>
                    {move || status.get()}
                </span>
            </div>
        </article>
    }
}

#[component]
pub(crate) fn MessageView(message_id: u64, messages: ReadSignal<Vec<Message>>) -> impl IntoView {
    let Some(initial_message) = current_message(messages, message_id) else {
        return view! { <></> }.into_any();
    };

    if initial_message.tool.is_some() {
        return view! {
            <article class=move || {
                current_tool(messages, message_id)
                    .map(|tool| tool_turn_card_class(tool.status))
                    .unwrap_or_else(|| "task-card agent-turn-item tool-turn-item".to_owned())
            }>
                <ToolActivityCard message_id messages />
            </article>
        }
        .into_any();
    }

    if initial_message.role == MessageRole::User {
        return user_message_view(message_id, messages);
    }

    if initial_message.role == MessageRole::Reasoning {
        return reasoning_message_view(message_id, messages);
    }

    let turn_class = match initial_message.role {
        MessageRole::Assistant => "task-card assistant-turn role-assistant",
        MessageRole::System => "task-card assistant-turn role-system",
        MessageRole::User | MessageRole::Reasoning => "task-card assistant-turn",
    };
    let (collapsed, set_collapsed) = signal(false);
    let toggle_title = move || {
        if collapsed.get() {
            "Развернуть"
        } else {
            "Свернуть"
        }
    };
    view! {
        <article class=turn_class>
            <div class="task-card-header">
                <span class="assistant-role">{move || current_message(messages, message_id).map(|message| message.role.label()).unwrap_or("Сообщение")}</span>
                <div class="message-actions">
                    <button
                        type="button"
                        class="icon-button"
                        title="Скопировать markdown"
                        on:click=move |_| copy_to_clipboard(current_message_text(messages, message_id))
                    >
                        "Копировать"
                    </button>
                    <button
                        type="button"
                        class="icon-button"
                        title=toggle_title
                        on:click=move |_| set_collapsed.update(|value| *value = !*value)
                    >
                        {move || if collapsed.get() { "Открыть" } else { "Скрыть" }}
                    </button>
                </div>
            </div>
            {move || {
                if collapsed.get() {
                    view! {
                        <div class="message collapsed-message">
                            "Сообщение скрыто"
                        </div>
                    }.into_any()
                } else {
                    view! {
                        <div
                            class=move || current_message_content_class(messages, message_id)
                            inner_html=move || current_message_html(messages, message_id)
                        ></div>
                    }.into_any()
                }
            }}
        </article>
    }
    .into_any()
}

/// Запрос пользователя: правый «пузырь», без тяжёлой шапки роли; copy
/// появляется по наведению (стиль в CSS).
fn user_message_view(message_id: u64, messages: ReadSignal<Vec<Message>>) -> AnyView {
    view! {
        <article class="user-turn">
            <div class="user-bubble">
                <button
                    type="button"
                    class="icon-button user-copy"
                    title="Скопировать"
                    on:click=move |_| copy_to_clipboard(current_message_text(messages, message_id))
                >
                    "Копировать"
                </button>
                <div class="message user-message" inner_html=move || current_message_html(messages, message_id)></div>
            </div>
        </article>
    }
    .into_any()
}

/// Reasoning-поток: пока стримится — развёрнут, после завершения сворачивается
/// в строку-переключатель «Размышления».
fn reasoning_message_view(message_id: u64, messages: ReadSignal<Vec<Message>>) -> AnyView {
    let streaming = current_message(messages, message_id).is_some_and(|message| message.streaming);
    let (expanded, set_expanded) = signal(streaming);
    let (last_streaming, set_last_streaming) = signal(streaming);
    Effect::new(move |_| {
        let streaming =
            current_message(messages, message_id).is_some_and(|message| message.streaming);
        if last_streaming.get() && !streaming {
            set_expanded.set(false);
        }
        set_last_streaming.set(streaming);
    });
    view! {
        <article class="task-card running agent-turn-item reasoning-turn">
            <button
                type="button"
                class="reasoning-toggle"
                on:click=move |_| set_expanded.update(|value| *value = !*value)
            >
                <span class=move || {
                    if current_message(messages, message_id).is_some_and(|message| message.streaming) {
                        "status-badge running"
                    } else {
                        "status-badge idle"
                    }
                }>
                    {move || {
                        if current_message(messages, message_id).is_some_and(|message| message.streaming) {
                            view! { <span class="spinner-dot"></span> }.into_any()
                        } else {
                            view! { <span class="dot"></span> }.into_any()
                        }
                    }}
                    "Размышления"
                </span>
                <span class="reasoning-caret">
                    {move || if expanded.get() { "−" } else { "+" }}
                </span>
            </button>
            {move || {
                if expanded.get() {
                    view! {
                        <div class="message reasoning-message" inner_html=move || current_message_html(messages, message_id)></div>
                    }.into_any()
                } else {
                    view! { <></> }.into_any()
                }
            }}
        </article>
    }
    .into_any()
}

fn current_message(messages: ReadSignal<Vec<Message>>, message_id: u64) -> Option<Message> {
    messages.with(|items| {
        items
            .iter()
            .find(|message| message.id == message_id)
            .cloned()
    })
}

fn current_tool(messages: ReadSignal<Vec<Message>>, message_id: u64) -> Option<ToolActivity> {
    current_message(messages, message_id).and_then(|message| message.tool)
}

fn current_message_text(messages: ReadSignal<Vec<Message>>, message_id: u64) -> String {
    current_message(messages, message_id)
        .map(|message| message.text)
        .unwrap_or_default()
}

fn current_message_html(messages: ReadSignal<Vec<Message>>, message_id: u64) -> String {
    markdown_html(&current_message_text(messages, message_id))
}

fn current_message_content_class(messages: ReadSignal<Vec<Message>>, message_id: u64) -> String {
    current_message(messages, message_id)
        .map(|message| {
            let message_class = message.role.message_class();
            if message.streaming {
                format!("{message_class} streaming-message")
            } else {
                message_class.to_owned()
            }
        })
        .unwrap_or_else(|| "message system-message".to_owned())
}

fn tool_turn_card_class(status: ToolActivityStatus) -> String {
    let state_class = match status {
        ToolActivityStatus::Running
        | ToolActivityStatus::WaitingApproval
        | ToolActivityStatus::Approved => "running",
        ToolActivityStatus::Done => "success",
        ToolActivityStatus::Denied | ToolActivityStatus::Failed => "error",
    };
    format!(
        "task-card {state_class} agent-turn-item tool-turn-item status-{}",
        status.key()
    )
}
