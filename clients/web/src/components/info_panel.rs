use leptos::prelude::*;
use web_sys::MouseEvent;

use super::controls::ContextRing;
use super::format_token_count;
use super::tool_activity::{PlanStepPreview, parse_plan_steps};
use crate::tool_names::UPDATE_PLAN_TOOL;
use crate::types::*;
use crate::ui_utils::short_path;

/// Список пунктов плана с маркерами статусов; используется и в теле панели,
/// и в поповере свёрнутой рейки.
#[component]
fn PlanStepList(plan_steps: Memo<Vec<PlanStepPreview>>) -> impl IntoView {
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
    }
}

/// Инфо-панель справа от ленты: статус плана текущей задачи (последний
/// update_plan), заполнение контекста и параметры сессии. Сворачивается своей
/// кнопкой в шапке (зеркально левому сайдбару) до рейки с кружком контекста.
#[component]
#[allow(clippy::too_many_arguments)]
pub(crate) fn InfoPanelView<T, R>(
    open: ReadSignal<bool>,
    width: ReadSignal<i32>,
    on_toggle: T,
    on_begin_resize: R,
    messages: ReadSignal<Vec<Message>>,
    model_name: ReadSignal<String>,
    reasoning_enabled: ReadSignal<bool>,
    effort: ReadSignal<ReasoningEffort>,
    context_usage: ReadSignal<Option<ContextUsage>>,
    agent_status: ReadSignal<String>,
    event_count: ReadSignal<u64>,
    tool_activities: ReadSignal<Vec<ToolActivity>>,
    pending_user_inputs: ReadSignal<Vec<UserInputRequestInfo>>,
    workspace_label: ReadSignal<String>,
) -> impl IntoView
where
    T: Fn(MouseEvent) + Copy + 'static,
    R: Fn(MouseEvent) + Copy + 'static,
{
    // Последний update_plan в ленте — актуальное состояние плана задачи.
    let plan_steps = Memo::new(move |_| {
        messages.with(|items| {
            items
                .iter()
                .rev()
                .filter_map(|message| message.tool.as_ref())
                .find(|tool| tool.name == UPDATE_PLAN_TOOL)
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
    let pending_total = Memo::new(move |_| pending_user_inputs.with(|items| items.len()));

    view! {
        <aside
            class="info-panel"
            class:open=move || open.get()
            style=move || format!("--info-panel-width: {}px", width.get())
            aria-label="Инфо по чату"
        >
            // Левый край панели тянется: ресайз, а за порогом — схлопывание
            // (зеркально левому сайдбару).
            <div
                class="info-panel-resize-handle"
                aria-hidden="true"
                on:mousedown=on_begin_resize
            ></div>
            <div class="info-panel-header">
                <h2>"Инфо"</h2>
                <button
                    type="button"
                    title=move || if open.get() { "Свернуть панель" } else { "Развернуть панель" }
                    on:click=on_toggle
                >
                    {move || if open.get() { "›" } else { "‹" }}
                </button>
            </div>

            // Рейка свёрнутого состояния: бублик контекста и мини-этапы
            // плана точками; у обоих при наведении — поповер с деталями
            // в едином стиле (.rail-popover).
            <div class="info-panel-rail">
                <div class="info-rail-ring rail-popover-host">
                    <ContextRing usage=context_usage />
                    {move || {
                        let Some(usage) = context_usage.get() else {
                            return ().into_any();
                        };
                        let percent = usage.percent();
                        view! {
                            <div class="rail-popover rail-popover-left">
                                <div class="rail-popover-head">
                                    <span class="panel-kicker">"Контекст"</span>
                                    <code>{format!("{percent}%")}</code>
                                </div>
                                <div class="info-row">
                                    <span>"Токены"</span>
                                    <code>
                                        {format!(
                                            "{} / {}",
                                            format_token_count(usage.used_tokens),
                                            format_token_count(usage.max_tokens),
                                        )}
                                    </code>
                                </div>
                                {usage
                                    .compaction_trigger_tokens
                                    .map(|trigger| {
                                        view! {
                                            <div class="info-row">
                                                <span>"Автокомпакт"</span>
                                                <code>{format!("~{}", format_token_count(trigger))}</code>
                                            </div>
                                        }
                                        .into_any()
                                    })
                                    .unwrap_or_else(|| ().into_any())}
                            </div>
                        }
                        .into_any()
                    }}
                </div>
                {move || {
                    if plan_steps.with(|steps| steps.is_empty()) {
                        return ().into_any();
                    }
                    view! {
                        <div class="info-rail-plan rail-popover-host">
                            <For
                                each=move || plan_steps.get().into_iter().enumerate()
                                key=|(index, step)| format!("{index}:{}", step.status)
                                children=move |(_, step): (usize, PlanStepPreview)| {
                                    let class = format!("info-rail-step {}", step.status);
                                    view! { <span class=class></span> }
                                }
                            />
                            <div class="rail-popover rail-popover-left">
                                <div class="rail-popover-head">
                                    <span class="panel-kicker">"План"</span>
                                    {move || {
                                        plan_progress
                                            .get()
                                            .map(|progress| view! { <code>{progress}</code> }.into_any())
                                            .unwrap_or_else(|| ().into_any())
                                    }}
                                </div>
                                <PlanStepList plan_steps />
                            </div>
                        </div>
                    }
                    .into_any()
                }}
            </div>

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
                            view! { <PlanStepList plan_steps /> }.into_any()
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
                        // Отметка порога автокомпакта — как штрих на кружке.
                        let mark = usage
                            .compaction_trigger_tokens
                            .filter(|_| usage.max_tokens > 0)
                            .map(|trigger| {
                                (f64::from(trigger) / f64::from(usage.max_tokens) * 100.0)
                                    .clamp(0.0, 100.0)
                            });
                        view! {
                            <div class="context-cache-bar info-usage-bar">
                                <span style=format!("width: {percent:.1}%")></span>
                                {mark
                                    .map(|mark| {
                                        view! {
                                            <span
                                                class="info-usage-mark"
                                                title="Порог автокомпакта"
                                                style=format!("left: {mark:.1}%")
                                            ></span>
                                        }
                                        .into_any()
                                    })
                                    .unwrap_or_else(|| ().into_any())}
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
                    <div class="info-row">
                        <span>"Ожидают"</span>
                        <code>{move || pending_total.get().to_string()}</code>
                    </div>
                </section>
            </div>
        </aside>
    }
}
