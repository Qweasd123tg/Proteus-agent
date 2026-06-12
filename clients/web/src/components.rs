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
pub(crate) fn ConfigsView() -> impl IntoView {
    let (summary, set_summary) = signal(None::<ConfigSummary>);
    let (status, set_status) = signal("загружаю конфигурацию".to_owned());

    load_config_summary(set_summary, set_status);

    let refresh = move |_| load_config_summary(set_summary, set_status);

    view! {
        <section class="configs-page">
            <div class="resume-toolbar">
                <div>
                    <h2>"Configs"</h2>
                    <p>{move || status.get()}</p>
                </div>
                <button type="button" class="secondary" on:click=refresh>"Обновить"</button>
            </div>
            {move || {
                summary
                    .get()
                    .map(|summary| view! { <ConfigSummaryView summary /> }.into_any())
                    .unwrap_or_else(|| {
                        view! {
                            <div class="empty-state">
                                <div class="empty-state-title">"Config summary недоступен"</div>
                            </div>
                        }
                        .into_any()
                    })
            }}
        </section>
    }
}

fn load_config_summary(
    set_summary: WriteSignal<Option<ConfigSummary>>,
    set_status: WriteSignal<String>,
) {
    spawn_local(async move {
        match get_json::<ConfigSummary>("/config").await {
            Ok(summary) => {
                let module_count = summary.modules.len();
                let tool_count = summary.registered_tools.len();
                let plugin_count = summary.plugins.len();
                set_summary.set(Some(summary));
                set_status.set(format!(
                    "{module_count} modules · {tool_count} tools · {plugin_count} plugins"
                ));
            }
            Err(error) => {
                set_summary.set(None);
                set_status.set(format!("не удалось загрузить config: {error}"));
            }
        }
    });
}

#[component]
fn ConfigSummaryView(summary: ConfigSummary) -> impl IntoView {
    let model_label = non_empty(summary.model.label.as_str(), "model не выбран");
    let config_path = summary
        .config_path
        .as_deref()
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| "(default discovery / none)".to_owned());
    let reasoning_effort = summary
        .reasoning
        .effort
        .as_deref()
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| "auto".to_owned());
    let reasoning_budget = summary
        .reasoning
        .budget_tokens
        .map(|tokens| tokens.to_string())
        .unwrap_or_else(|| "-".to_owned());
    let reasoning_enabled = if summary.reasoning.enabled {
        "on"
    } else {
        "off"
    };
    let modules = summary.modules.clone();
    let enabled_tools = summary.tools_enabled.clone();
    let registered_tools = summary.registered_tools.clone();
    let plugins = summary.plugins.clone();
    let config_files = summary.config_files.clone();
    let model_options = summary.model_options.clone();

    view! {
        <div class="configs-scroll">
            <section class="config-overview">
                <article class="config-panel">
                    <div class="config-panel-header">
                        <span class="panel-kicker">"runtime"</span>
                        <strong>{non_empty(summary.profile.as_str(), "default")}</strong>
                    </div>
                    <div class="config-kv">
                        <span>"cwd"</span>
                        <code>{non_empty(summary.cwd.as_str(), "-")}</code>
                    </div>
                    <div class="config-kv">
                        <span>"config"</span>
                        <code>{config_path}</code>
                    </div>
                    <div class="config-kv">
                        <span>"mode"</span>
                        <code>{non_empty(summary.permission_mode.as_str(), "-")}</code>
                    </div>
                </article>
                <article class="config-panel">
                    <div class="config-panel-header">
                        <span class="panel-kicker">"model"</span>
                        <strong>{model_label}</strong>
                    </div>
                    <div class="config-kv">
                        <span>"provider"</span>
                        <code>{non_empty(summary.model.provider.as_str(), "-")}</code>
                    </div>
                    <div class="config-kv">
                        <span>"name"</span>
                        <code>{non_empty(summary.model.name.as_str(), "-")}</code>
                    </div>
                    <div class="config-chip-row">
                        <For
                            each=move || model_options.clone()
                            key=|model| model.label.clone()
                            children=move |model| view! { <span class="config-chip">{non_empty(model.label.as_str(), model.name.as_str())}</span> }
                        />
                    </div>
                </article>
                <article class="config-panel">
                    <div class="config-panel-header">
                        <span class="panel-kicker">"reasoning"</span>
                        <strong>{reasoning_enabled}</strong>
                    </div>
                    <div class="config-kv">
                        <span>"effort"</span>
                        <code>{reasoning_effort}</code>
                    </div>
                    <div class="config-kv">
                        <span>"summary"</span>
                        <code>{if summary.reasoning.summary { "true" } else { "false" }}</code>
                    </div>
                    <div class="config-kv">
                        <span>"budget"</span>
                        <code>{reasoning_budget}</code>
                    </div>
                    <div class="config-chip-row">
                        <For
                            each=move || summary.reasoning.effort_options.clone()
                            key=|effort| effort.clone()
                            children=move |effort| view! { <span class="config-chip">{effort}</span> }
                        />
                    </div>
                </article>
            </section>

            <section class="config-section">
                <div class="config-section-header">
                    <h3>"Modules"</h3>
                    <span>{modules.len()}</span>
                </div>
                <div class="config-table">
                    <For
                        each=move || modules.clone()
                        key=|module| format!("{}:{}", module.slot, module.id)
                        children=move |module| {
                            view! {
                                <div class="config-row">
                                    <span>{module.slot}</span>
                                    <code>{module.id}</code>
                                </div>
                            }
                        }
                    />
                </div>
            </section>

            <section class="config-section">
                <div class="config-section-header">
                    <h3>"Enabled tools"</h3>
                    <span>{enabled_tools.len()}</span>
                </div>
                {if enabled_tools.is_empty() {
                    view! { <div class="config-empty">"(none)"</div> }.into_any()
                } else {
                    view! {
                        <div class="config-chip-row">
                            <For
                                each=move || enabled_tools.clone()
                                key=|tool| tool.clone()
                                children=move |tool| view! { <span class="config-chip">{tool}</span> }
                            />
                        </div>
                    }.into_any()
                }}
            </section>

            <section class="config-section">
                <div class="config-section-header">
                    <h3>"Registered tools"</h3>
                    <span>{registered_tools.len()}</span>
                </div>
                <div class="config-list">
                    <For
                        each=move || registered_tools.clone()
                        key=|tool| format!("{}:{}", tool.name, tool.source)
                        children=move |tool| {
                            view! {
                                <article class="config-list-item">
                                    <div class="config-list-main">
                                        <div class="config-list-title">
                                            <strong>{tool.name}</strong>
                                            <code>{tool.source}</code>
                                        </div>
                                        <p>{tool.description}</p>
                                    </div>
                                    <span class="status-badge idle">{tool.safety}</span>
                                </article>
                            }
                        }
                    />
                </div>
            </section>

            <section class="config-section">
                <div class="config-section-header">
                    <h3>"Plugins"</h3>
                    <span>{plugins.len()}</span>
                </div>
                <div class="config-list">
                    <For
                        each=move || plugins.clone()
                        key=|plugin| format!("{}:{}", plugin.name, plugin.version)
                        children=move |plugin| {
                            let badge_class = if plugin.status.starts_with("error") {
                                "status-badge failed"
                            } else {
                                "status-badge completed"
                            };
                            view! {
                                <article class="config-list-item">
                                    <div class="config-list-main">
                                        <div class="config-list-title">
                                            <strong>{plugin.name}</strong>
                                            <code>{plugin.version}</code>
                                        </div>
                                        <p>{plugin.description}</p>
                                    </div>
                                    <span class=badge_class>
                                        <span class="dot"></span>
                                        {plugin.status}
                                    </span>
                                </article>
                            }
                        }
                    />
                </div>
            </section>

            <section class="config-section">
                <div class="config-section-header">
                    <h3>"Config files"</h3>
                    <span>{config_files.len()}</span>
                </div>
                {if config_files.is_empty() {
                    view! { <div class="config-empty">"(none)"</div> }.into_any()
                } else {
                    view! {
                        <div class="config-table">
                            <For
                                each=move || config_files.clone()
                                key=|path| path.clone()
                                children=move |path| {
                                    view! {
                                        <div class="config-row">
                                            <span>{short_path(&path)}</span>
                                            <code>{path}</code>
                                        </div>
                                    }
                                }
                            />
                        </div>
                    }.into_any()
                }}
            </section>
        </div>
    }
}

fn non_empty(value: &str, fallback: &str) -> String {
    if value.trim().is_empty() {
        fallback.to_owned()
    } else {
        value.to_owned()
    }
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
fn ToolActivityCard(tool: ToolActivity) -> impl IntoView {
    let (expanded, set_expanded) = signal(false);
    let args = tool.args_preview.clone();
    let result = tool.result_preview.clone();
    view! {
        <article class="tool-card">
            <button
                type="button"
                class="tool-card-summary"
                title="Показать детали tool"
                on:click=move |_| set_expanded.update(|value| *value = !*value)
            >
                <span class=tool.status.badge_class()>
                    <span class=if matches!(tool.status, ToolActivityStatus::Running | ToolActivityStatus::WaitingApproval) { "spinner-dot" } else { "dot" }></span>
                    {tool.status.label()}
                </span>
                <strong>{tool.name}</strong>
                <code>{short_id(&tool.call_id).to_owned()}</code>
            </button>
            {move || {
                if expanded.get() {
                    view! {
                        <div class="tool-card-details">
                            <pre>{args.clone()}</pre>
                            {if let Some(result) = result.clone() {
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
pub(crate) fn MessageView(message: Message) -> impl IntoView {
    if message.tool.is_some() {
        let tool = message.tool.expect("checked above");
        let card_class = tool_turn_card_class(tool.status);
        return view! {
            <article class=card_class>
                <ToolActivityCard tool />
            </article>
        }
        .into_any();
    }

    let card_class = message.role.card_class();
    let message_class = message.role.message_class();
    let badge_class = message.role.badge_class();
    let text = message.text.clone();
    let html = markdown_html(&text);
    let content_class = if message.streaming {
        format!("{message_class} streaming-message")
    } else {
        message_class.to_owned()
    };
    let (collapsed, set_collapsed) = signal(false);
    let copy_text = text.clone();
    let toggle_title = move || {
        if collapsed.get() {
            "Развернуть"
        } else {
            "Свернуть"
        }
    };
    view! {
        <article class=card_class>
            <div class="task-card-header">
                <span class=badge_class>
                    <span class="dot"></span>
                    {message.role.label()}
                </span>
                <div class="message-actions">
                    <button
                        type="button"
                        class="icon-button"
                        title="Скопировать markdown"
                        on:click=move |_| copy_to_clipboard(copy_text.clone())
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
                        <div class=content_class.clone() inner_html=html.clone()></div>
                    }.into_any()
                }
            }}
        </article>
    }
    .into_any()
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
