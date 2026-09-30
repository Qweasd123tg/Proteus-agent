use std::collections::HashMap;

use leptos::prelude::*;
use serde_json::Value;

use super::{ToolPreview, tool_args_preview};
use crate::types::*;

#[component]
pub(crate) fn ApprovalCard<F>(request: ApprovalRequestInfo, on_resolve: F) -> impl IntoView
where
    F: Fn(String, bool, ApprovalCacheScope) + Copy + 'static,
{
    let (cache, set_cache) = signal(ApprovalCacheScope::None);
    let approve_id = request.approval_id.clone();
    let deny_id = request.approval_id.clone();
    let args_preview = tool_args_preview(&request.call.name, &request.call.args);
    let exact_scope = if approval_is_command(&request) {
        ApprovalCacheScope::ExactCommand
    } else {
        ApprovalCacheScope::ExactCall
    };
    let allows_workspace_write_cache = approval_allows_workspace_write_cache(&request);
    let tool_description = request
        .tool_spec
        .as_ref()
        .map(|spec| spec.description.as_str())
        .unwrap_or(&request.reason)
        .to_owned();
    let has_preview_body = request
        .preview
        .as_ref()
        .and_then(|preview| preview.body.as_ref())
        .is_some_and(|body| !body.trim().is_empty());
    let cache_id = format!("approval-scope-{}", request.approval_id);
    let origin_label = request
        .origin
        .as_ref()
        .and_then(|origin| origin.label.clone());

    view! {
        <article class="control-card approval-card">
            <header class="approval-heading">
                <div class="control-card-header">
                    <span class="status-badge attention">"Требуется разрешение"</span>
                    <strong title=tool_description>{request.call.name}</strong>
                    {origin_label.map(|label| view! { <span class="status-badge subagent">{format!("субагент: {label}")}</span> })}
                </div>
                <code class="approval-cwd">{request.cwd.to_string_lossy().into_owned()}</code>
            </header>
            <p>{request.reason}</p>
            {approval_preview(request.preview.clone())}
            {if has_preview_body {
                view! {
                    <details class="approval-arguments">
                        <summary>"Параметры вызова"</summary>
                        <ToolPreview text=Signal::derive(move || args_preview.clone()) />
                    </details>
                }.into_any()
            } else {
                view! { <ToolPreview text=Signal::derive(move || args_preview.clone()) caption="параметры вызова" /> }.into_any()
            }}
            <div class="approval-scope">
                <label for=cache_id.clone()>"Разрешение действует"</label>
                <select id=cache_id prop:value=move || match cache.get() {
                    ApprovalCacheScope::None => "none",
                    ApprovalCacheScope::ExactCall | ApprovalCacheScope::ExactCommand => "exact",
                    ApprovalCacheScope::WorkspaceWrite => "workspace_write",
                } on:change:target=move |event| {
                    let scope = match event.target().value().as_str() {
                        "exact" => exact_scope,
                        "workspace_write" if allows_workspace_write_cache => ApprovalCacheScope::WorkspaceWrite,
                        _ => ApprovalCacheScope::None,
                    };
                    set_cache.set(scope);
                }>
                    <option value="none" data-description=ApprovalCacheScope::None.description()>{ApprovalCacheScope::None.label()}</option>
                    <option value="exact" data-description=exact_scope.description()>{exact_scope.label()}</option>
                    {allows_workspace_write_cache.then(|| view! {
                        <option value="workspace_write" data-description=ApprovalCacheScope::WorkspaceWrite.description()>{ApprovalCacheScope::WorkspaceWrite.label()}</option>
                    })}
                </select>
                <p>{move || cache.get().description()}</p>
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
        return ().into_any();
    };
    let ApprovalPreviewInfo {
        kind,
        title,
        summary,
        affected_files,
        body,
        language: _,
        metadata: _,
    } = preview;
    let kind = approval_preview_kind(&kind);
    let body = body
        .filter(|body| !body.trim().is_empty())
        .map(|body| body.to_owned());

    view! {
        <section class="approval-preview">
            <div class="control-card-header">
                <span class="control-label">{kind}</span>
                <strong>{title}</strong>
            </div>
            <p>{summary}</p>
            {(!affected_files.is_empty()).then(|| view! {
                <ul class="approval-files">{affected_files.into_iter().map(|path| view! { <li>{path}</li> }).collect_view()}</ul>
            })}
            {if let Some(body) = body {
                view! {
                    <ToolPreview text=Signal::derive(move || body.clone()) />
                }.into_any()
            } else {
                ().into_any()
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

fn approval_is_command(request: &ApprovalRequestInfo) -> bool {
    request.call.name.eq_ignore_ascii_case("shell")
        || tool_safety(request).is_some_and(|safety| safety == "RunsCommands")
}

fn approval_allows_workspace_write_cache(request: &ApprovalRequestInfo) -> bool {
    if tool_safety(request).is_none_or(|safety| safety != "WritesFiles") {
        return false;
    }
    request.tool_spec.as_ref().is_some_and(|spec| {
        let Some(approval) = spec.metadata.get("approval") else {
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
    use proteus_contracts::domain::ToolSafety;
    request.tool_spec.as_ref().map(|spec| match spec.safety {
        ToolSafety::ReadOnly => "ReadOnly",
        ToolSafety::WritesFiles => "WritesFiles",
        ToolSafety::RunsCommands => "RunsCommands",
        ToolSafety::Network => "Network",
        ToolSafety::Dangerous => "Dangerous",
    })
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
    let origin_label = request
        .origin
        .as_ref()
        .and_then(|origin| origin.label.clone());
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
                {origin_label
                    .map(|label| {
                        view! {
                            <span class="status-badge subagent">
                                {format!("субагент: {label}")}
                            </span>
                        }
                    })}
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
                            ().into_any()
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
