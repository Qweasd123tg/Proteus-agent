use std::collections::HashMap;

use leptos::{prelude::*, task::spawn_local};
use serde_json::Value;
use web_sys::{MouseEvent, window};

use crate::api::{get_json, post_json};
use crate::markdown::{markdown_html, plain_text_html};
use crate::types::*;
use crate::ui_utils::{compact_json, compact_text, copy_to_clipboard, set_timeout, short_id, short_path};

const REASONING_RENDER_LIMIT: usize = 8000;
const APPROVAL_PREVIEW_RENDER_LIMIT: usize = 12000;
const COPY_FEEDBACK_MS: i32 = 1200;
/// Пороги (в процентах) для смены цвета дуги: норма → внимание → критично.
const CONTEXT_RING_WARN_PERCENT: u8 = 70;
const CONTEXT_RING_CRIT_PERCENT: u8 = 90;

#[derive(Clone)]
struct RenderedMessageCache {
    id: u64,
    version: u64,
    html: String,
}

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
                                                <p>{resume_session_preview(&session)}</p>
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

fn resume_session_preview(session: &SessionSummary) -> String {
    if let Some(preview) = session
        .preview
        .as_deref()
        .filter(|text| !text.trim().is_empty())
    {
        preview.to_owned()
    } else if session.message_count == 0 {
        "Новый чат".to_owned()
    } else {
        "Сессия".to_owned()
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
    let exact_scope = if approval_is_command(&request) {
        ApprovalCacheScope::ExactCommand
    } else {
        ApprovalCacheScope::ExactCall
    };
    let allows_workspace_write_cache = approval_allows_workspace_write_cache(&request);
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
            {approval_preview(request.preview.clone())}
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
                        class:active=move || cache.get() == exact_scope
                        on:click=move |_| set_cache.set(exact_scope)
                    >
                        {exact_scope.label()}
                    </button>
                    {if allows_workspace_write_cache {
                        view! {
                            <button
                                type="button"
                                class:active=move || cache.get() == ApprovalCacheScope::WorkspaceWrite
                                on:click=move |_| set_cache.set(ApprovalCacheScope::WorkspaceWrite)
                            >
                                {ApprovalCacheScope::WorkspaceWrite.label()}
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

fn approval_preview(preview: Option<ApprovalPreviewInfo>) -> impl IntoView {
    let Some(preview) = preview else {
        return view! { <></> }.into_any();
    };
    let ApprovalPreviewInfo {
        kind,
        title,
        summary,
        affected_files,
        body,
        language,
        metadata: _,
    } = preview;
    let kind = approval_preview_kind(&kind);
    let files = approval_preview_files(&affected_files);
    let body_label = language
        .as_deref()
        .filter(|language| !language.trim().is_empty())
        .unwrap_or("preview")
        .to_owned();
    let body = body
        .filter(|body| !body.trim().is_empty())
        .map(|body| compact_text(&body, APPROVAL_PREVIEW_RENDER_LIMIT));

    view! {
        <section>
            <div class="control-card-header">
                <span class="status-badge completed">
                    <span class="dot"></span>
                    {kind}
                </span>
                <strong>{title}</strong>
            </div>
            <p>{summary}</p>
            {if let Some(files) = files {
                view! {
                    <div class="control-row">
                        <span class="control-label">"Файлы"</span>
                        <code>{files}</code>
                    </div>
                }.into_any()
            } else {
                view! { <></> }.into_any()
            }}
            {if let Some(body) = body {
                view! {
                    <div>
                        <div class="control-card-header">
                            <span class="status-badge idle">
                                <span class="dot"></span>
                                {body_label}
                            </span>
                        </div>
                        <pre><code>{body}</code></pre>
                    </div>
                }.into_any()
            } else {
                view! { <></> }.into_any()
            }}
        </section>
    }
    .into_any()
}

fn approval_preview_kind(kind: &str) -> String {
    match kind {
        "command" => "Команда".to_owned(),
        "patch" => "Diff".to_owned(),
        "write_file" => "Файл".to_owned(),
        kind if !kind.trim().is_empty() => kind.to_owned(),
        _ => "Preview".to_owned(),
    }
}

fn approval_preview_files(files: &[String]) -> Option<String> {
    if files.is_empty() {
        return None;
    }
    let visible = files.iter().take(3).cloned().collect::<Vec<_>>().join(", ");
    if files.len() > 3 {
        Some(format!("{visible}, +{}", files.len() - 3))
    } else {
        Some(visible)
    }
}

fn approval_is_command(request: &ApprovalRequestInfo) -> bool {
    request.call.name.eq_ignore_ascii_case("shell")
        || tool_safety(request).is_some_and(|safety| safety == "RunsCommands")
}

fn approval_allows_workspace_write_cache(request: &ApprovalRequestInfo) -> bool {
    if !tool_safety(request).is_some_and(|safety| safety == "WritesFiles") {
        return false;
    }
    request.tool_spec.as_ref().is_some_and(|spec| {
        let Some(approval) = spec
            .get("metadata")
            .and_then(|metadata| metadata.get("approval"))
        else {
            return false;
        };
        if approval
            .get("cache")
            .and_then(|cache| cache.get("workspace_write"))
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            return true;
        }
        ["cache", "cache_scopes"].into_iter().any(|field| {
            approval
                .get(field)
                .and_then(Value::as_array)
                .is_some_and(|scopes| {
                    scopes
                        .iter()
                        .any(|scope| scope.as_str() == Some("workspace_write"))
                })
        })
    })
}

fn tool_safety(request: &ApprovalRequestInfo) -> Option<&str> {
    request
        .tool_spec
        .as_ref()
        .and_then(|spec| spec.get("safety"))
        .and_then(Value::as_str)
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
fn ToolActivityCard(
    message: Memo<Option<Message>>,
    activity_now_ms: ReadSignal<u64>,
) -> impl IntoView {
    let (expanded, set_expanded) = signal(false);
    view! {
        <article class="tool-card">
            <button
                type="button"
                class="tool-card-summary"
                title="Показать детали tool"
                on:click=move |_| set_expanded.update(|value| *value = !*value)
            >
                <span class=move || current_tool(message).map(|tool| tool.status.badge_class()).unwrap_or("status-badge idle")>
                    <span class=move || {
                        current_tool(message)
                            .map(|tool| {
                                if matches!(tool.status, ToolActivityStatus::Running | ToolActivityStatus::WaitingApproval) {
                                    "spinner-dot"
                                } else {
                                    "dot"
                                }
                            })
                            .unwrap_or("dot")
                    }></span>
                    {move || current_tool_status_label(message, activity_now_ms)}
                </span>
                <strong>{move || current_tool(message).map(|tool| tool.name).unwrap_or_else(|| "tool".to_owned())}</strong>
                <code>{move || current_tool(message).map(|tool| short_id(&tool.call_id).to_owned()).unwrap_or_default()}</code>
            </button>
            {move || {
                if expanded.get() {
                    view! {
                        <div class="tool-card-details">
                            <pre>{move || current_tool(message).map(|tool| tool.args_preview).unwrap_or_default()}</pre>
                            {move || if let Some(result) = current_tool(message).and_then(|tool| tool.result_preview) {
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

/// Маленький бублик в строке ввода: показывает, насколько заполнено
/// контекстное окно. Скрыт, пока не пришёл первый снимок `TokenUsageUpdated`
/// с известным потолком окна.
#[component]
pub(crate) fn ContextRing(usage: ReadSignal<Option<ContextUsage>>) -> impl IntoView {
    move || {
        let Some(context) = usage.get() else {
            return view! { <></> }.into_any();
        };
        let percent = context.percent();
        let degrees = f64::from(percent) / 100.0 * 360.0;
        // Метку автокомпакта рисуем только когда сервер прислал порог.
        let compaction_percent = context.compaction_percent();
        let level = if percent >= CONTEXT_RING_CRIT_PERCENT {
            "crit"
        } else if percent >= CONTEXT_RING_WARN_PERCENT {
            "warn"
        } else {
            "ok"
        };
        let mut style = format!("--context-ring-deg: {degrees:.1}deg");
        let mut title = format!(
            "Контекст: {percent}% · {} / {} токенов",
            format_token_count(context.used_tokens),
            format_token_count(context.max_tokens),
        );
        if let (Some(mark_percent), Some(trigger_tokens)) =
            (compaction_percent, context.compaction_trigger_tokens)
        {
            let mark_degrees = f64::from(mark_percent) / 100.0 * 360.0;
            style.push_str(&format!("; --context-ring-mark-deg: {mark_degrees:.1}deg"));
            title.push_str(&format!(
                " · автокомпакт при {mark_percent}% (~{})",
                format_token_count(trigger_tokens),
            ));
        }
        let class = if compaction_percent.is_some() {
            format!("context-ring context-ring-{level} context-ring-has-mark")
        } else {
            format!("context-ring context-ring-{level}")
        };
        view! {
            <div
                class=class
                style=style
                title=title.clone()
                aria-label=title
            >
                <span class="context-ring-label">{percent.to_string()}</span>
            </div>
        }
        .into_any()
    }
}

/// Компактная запись числа токенов: «90.5k», «200k», «512».
fn format_token_count(tokens: u32) -> String {
    if tokens < 1000 {
        return tokens.to_string();
    }
    let thousands = f64::from(tokens) / 1000.0;
    let formatted = format!("{thousands:.1}");
    format!("{}k", formatted.trim_end_matches(".0"))
}

/// Кнопка копирования с короткой обратной связью: после клика подсвечивается
/// и меняет ярлык на «Скопировано», затем сама сбрасывается.
#[component]
fn CopyButton<F>(
    text: F,
    #[prop(into)] class: String,
    #[prop(into)] title: String,
) -> impl IntoView
where
    F: Fn() -> String + 'static,
{
    let (copied, set_copied) = signal(false);
    view! {
        <button
            type="button"
            class=class
            class:copied=move || copied.get()
            title=title
            on:click=move |_| {
                copy_to_clipboard(text());
                set_copied.set(true);
                set_timeout(COPY_FEEDBACK_MS, move || set_copied.set(false));
            }
        >
            {move || if copied.get() { "Скопировано" } else { "Копировать" }}
        </button>
    }
}

#[component]
pub(crate) fn MessageView(
    message_id: u64,
    messages: Memo<HashMap<u64, Message>>,
    activity_now_ms: ReadSignal<u64>,
) -> impl IntoView {
    let message = Memo::new(move |_| current_message(messages, message_id));
    let Some(initial_message) = message.get_untracked() else {
        return view! { <></> }.into_any();
    };

    if initial_message.tool.is_some() {
        return view! {
            <article class=move || {
                current_tool(message)
                    .map(|tool| tool_turn_card_class(tool.status))
                    .unwrap_or_else(|| "task-card agent-turn-item tool-turn-item".to_owned())
            }>
                <ToolActivityCard message activity_now_ms />
            </article>
        }
        .into_any();
    }

    if initial_message.role == MessageRole::User {
        return user_message_view(message);
    }

    if initial_message.role == MessageRole::Reasoning {
        return reasoning_message_view(message);
    }

    let turn_class = match initial_message.role {
        MessageRole::Assistant => "task-card assistant-turn role-assistant",
        MessageRole::System => "task-card assistant-turn role-system",
        MessageRole::User | MessageRole::Reasoning => "task-card assistant-turn",
    };
    let rendered_html = cached_message_html(message);
    view! {
        <article class=turn_class>
            <div class="task-card-header">
                <span class="assistant-role">{move || message.get().map(|message| message.role.label()).unwrap_or("Сообщение")}</span>
                <div class="message-actions">
                    <CopyButton
                        text=move || current_message_text(message)
                        class="icon-button"
                        title="Скопировать markdown"
                    />
                </div>
            </div>
            <div
                class=move || current_message_content_class(message)
                inner_html=move || rendered_html.get()
            ></div>
        </article>
    }
    .into_any()
}

/// Запрос пользователя: правый «пузырь», без тяжёлой шапки роли; copy
/// появляется по наведению (стиль в CSS).
fn user_message_view(message: Memo<Option<Message>>) -> AnyView {
    let rendered_html = cached_message_html(message);
    view! {
        <article class="user-turn">
            <div class="user-bubble">
                <CopyButton
                    text=move || current_message_text(message)
                    class="icon-button user-copy"
                    title="Скопировать"
                />
                <div class="message user-message" inner_html=move || rendered_html.get()></div>
            </div>
        </article>
    }
    .into_any()
}

/// Reasoning-поток всегда начинается свёрнутым: длинное thinking-содержимое не
/// должно блокировать scroll/render основного ответа.
fn reasoning_message_view(message: Memo<Option<Message>>) -> AnyView {
    let streaming = message
        .get_untracked()
        .is_some_and(|message| message.streaming);
    let (expanded, set_expanded) = signal(false);
    let (last_streaming, set_last_streaming) = signal(streaming);
    Effect::new(move |_| {
        let streaming = message.get().is_some_and(|message| message.streaming);
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
                    if message.get().is_some_and(|message| message.streaming) {
                        "status-badge running"
                    } else {
                        "status-badge idle"
                    }
                }>
                    {move || {
                        if message.get().is_some_and(|message| message.streaming) {
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
                        <div class="message reasoning-message" inner_html=move || current_reasoning_html(message)></div>
                    }.into_any()
                } else {
                    view! { <></> }.into_any()
                }
            }}
        </article>
    }
    .into_any()
}

fn current_message(messages: Memo<HashMap<u64, Message>>, message_id: u64) -> Option<Message> {
    messages.with(|items| items.get(&message_id).cloned())
}

fn current_tool(message: Memo<Option<Message>>) -> Option<ToolActivity> {
    message.get().and_then(|message| message.tool)
}

fn current_tool_status_label(
    message: Memo<Option<Message>>,
    activity_now_ms: ReadSignal<u64>,
) -> String {
    let Some(tool) = current_tool(message) else {
        return "tool".to_owned();
    };
    if matches!(
        tool.status,
        ToolActivityStatus::Running
            | ToolActivityStatus::WaitingApproval
            | ToolActivityStatus::Approved
    ) {
        let elapsed_seconds = activity_now_ms
            .get()
            .saturating_sub(tool.started_at_ms)
            .saturating_div(1000);
        format!(
            "{} · {}",
            tool.status.label(),
            format_elapsed_seconds(elapsed_seconds)
        )
    } else {
        tool.status.label().to_owned()
    }
}

fn format_elapsed_seconds(seconds: u64) -> String {
    if seconds < 60 {
        format!("{seconds}s")
    } else {
        format!("{}m {:02}s", seconds / 60, seconds % 60)
    }
}

fn current_message_text(message: Memo<Option<Message>>) -> String {
    message
        .get()
        .map(|message| message.text)
        .unwrap_or_default()
}

fn cached_message_html(message: Memo<Option<Message>>) -> Memo<String> {
    let cache = StoredValue::new_local(None::<RenderedMessageCache>);
    Memo::new(move |_| {
        let Some(message) = message.get() else {
            return String::new();
        };
        let mut cached = None;
        cache.with_value(|slot| {
            if let Some(slot) = slot.as_ref()
                && slot.id == message.id
                && slot.version == message.version
            {
                cached = Some(slot.html.clone());
            }
        });
        if let Some(html) = cached {
            return html;
        }
        let html = render_message_html(&message);
        cache.set_value(Some(RenderedMessageCache {
            id: message.id,
            version: message.version,
            html: html.clone(),
        }));
        html
    })
}

fn render_message_html(message: &Message) -> String {
    markdown_html(&message.text)
}

fn current_reasoning_html(message: Memo<Option<Message>>) -> String {
    let Some(message) = message.get() else {
        return String::new();
    };
    plain_text_html(&compact_text(&message.text, REASONING_RENDER_LIMIT))
}

fn current_message_content_class(message: Memo<Option<Message>>) -> String {
    message
        .get()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_elapsed_seconds_keeps_short_and_minute_forms_compact() {
        assert_eq!(format_elapsed_seconds(9), "9s");
        assert_eq!(format_elapsed_seconds(65), "1m 05s");
    }

    #[test]
    fn render_message_html_formats_markdown_while_streaming() {
        let html = render_message_html(&Message {
            id: 1,
            version: 0,
            role: MessageRole::Assistant,
            text: "**live** markdown".to_owned(),
            tool: None,
            streaming: true,
        });

        assert!(html.contains("<strong>live</strong>"));
    }
}
