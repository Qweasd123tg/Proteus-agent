use leptos::prelude::*;

use super::format_token_count;
use super::tool_activity::{PlanStepPreview, parse_plan_steps};
use crate::types::*;
use crate::ui_utils::short_path;

/// Выдвижная инфо-панель справа от ленты: статус плана текущей задачи
/// (последний update_plan), заполнение контекста и параметры сессии.
#[component]
#[allow(clippy::too_many_arguments)]
pub(crate) fn InfoPanelView(
    open: ReadSignal<bool>,
    messages: ReadSignal<Vec<Message>>,
    model_name: ReadSignal<String>,
    mode: ReadSignal<PermissionMode>,
    reasoning_enabled: ReadSignal<bool>,
    effort: ReadSignal<ReasoningEffort>,
    context_usage: ReadSignal<Option<ContextUsage>>,
    agent_status: ReadSignal<String>,
    event_count: ReadSignal<u64>,
    tool_activities: ReadSignal<Vec<ToolActivity>>,
    workspace_label: ReadSignal<String>,
) -> impl IntoView {
    // Последний update_plan в ленте — актуальное состояние плана задачи.
    let plan_steps = Memo::new(move |_| {
        messages.with(|items| {
            items
                .iter()
                .rev()
                .filter_map(|message| message.tool.as_ref())
                .find(|tool| tool.name == "update_plan")
                .map(|tool| parse_plan_steps(&tool.args))
                .unwrap_or_default()
        })
    });
    let plan_progress = Memo::new(move |_| {
        plan_steps.with(|steps| {
            if steps.is_empty() {
                return None;
            }
            let completed = steps
                .iter()
                .filter(|step| step.status == "completed")
                .count();
            Some(format!("{completed}/{}", steps.len()))
        })
    });

    view! {
        <aside class="info-panel" class:open=move || open.get() aria-label="Инфо по чату">
            <div class="info-panel-inner">
                <div class="info-panel-body">
                    <section class="info-panel-section">
                        <div class="info-panel-section-head">
                            <span class="panel-kicker">"План"</span>
                            {move || {
                                plan_progress
                                    .get()
                                    .map(|progress| view! { <code>{progress}</code> }.into_any())
                                    .unwrap_or_else(|| ().into_any())
                            }}
                        </div>
                        {move || {
                            let steps = plan_steps.get();
                            if steps.is_empty() {
                                view! { <p class="info-panel-empty">"Агент ещё не составил план"</p> }.into_any()
                            } else {
                                view! {
                                    <div class="plan-step-list">
                                        <For
                                            each=move || plan_steps.get().into_iter().enumerate()
                                            key=|(index, step)| format!("{index}:{}:{}", step.step, step.status)
                                            children=move |(_, step): (usize, PlanStepPreview)| {
                                                let marker = match step.status.as_str() {
                                                    "completed" => "✓",
                                                    "in_progress" => "▸",
                                                    _ => "○",
                                                };
                                                let row_class = format!("plan-step-row {}", step.status);
                                                view! {
                                                    <div class=row_class>
                                                        <span class="plan-step-marker">{marker}</span>
                                                        <span class="plan-step-text">{step.step}</span>
                                                    </div>
                                                }
                                            }
                                        />
                                    </div>
                                }.into_any()
                            }
                        }}
                    </section>

                    <section class="info-panel-section">
                        <div class="info-panel-section-head">
                            <span class="panel-kicker">"Контекст"</span>
                            {move || {
                                context_usage
                                    .get()
                                    .map(|usage| {
                                        view! {
                                            <code>
                                                {format!(
                                                    "{} / {}",
                                                    format_token_count(usage.used_tokens),
                                                    format_token_count(usage.max_tokens),
                                                )}
                                            </code>
                                        }
                                        .into_any()
                                    })
                                    .unwrap_or_else(|| ().into_any())
                            }}
                        </div>
                        {move || {
                            let Some(usage) = context_usage.get() else {
                                return view! { <p class="info-panel-empty">"Замеров ещё нет"</p> }.into_any();
                            };
                            let percent = if usage.max_tokens == 0 {
                                0.0
                            } else {
                                (f64::from(usage.used_tokens) / f64::from(usage.max_tokens) * 100.0)
                                    .clamp(0.0, 100.0)
                            };
                            view! {
                                <div class="context-cache-bar info-usage-bar">
                                    <span style=format!("width: {percent:.1}%")></span>
                                </div>
                            }
                            .into_any()
                        }}
                    </section>

                    <section class="info-panel-section">
                        <div class="info-panel-section-head">
                            <span class="panel-kicker">"Сессия"</span>
                        </div>
                        <div class="info-row">
                            <span>"Статус"</span>
                            <code>{move || agent_status.get()}</code>
                        </div>
                        <div class="info-row">
                            <span>"Модель"</span>
                            <code>{move || model_name.get()}</code>
                        </div>
                        <div class="info-row">
                            <span>"Режим"</span>
                            <code>{move || mode.get().label()}</code>
                        </div>
                        <div class="info-row">
                            <span>"Reasoning"</span>
                            <code>
                                {move || {
                                    if reasoning_enabled.get() {
                                        effort.get().label()
                                    } else {
                                        "выкл".to_owned()
                                    }
                                }}
                            </code>
                        </div>
                        <div class="info-row">
                            <span>"Workspace"</span>
                            <code title=move || workspace_label.get()>
                                {move || short_path(&workspace_label.get())}
                            </code>
                        </div>
                        <div class="info-row">
                            <span>"Активность"</span>
                            <code>
                                {move || {
                                    format!(
                                        "{} events · {} tools",
                                        event_count.get(),
                                        tool_activities.with(|items| items.len()),
                                    )
                                }}
                            </code>
                        </div>
                    </section>
                </div>
            </div>
        </aside>
    }
}
