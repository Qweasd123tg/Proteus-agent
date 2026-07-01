use leptos::{html, prelude::*};
use web_sys::{KeyboardEvent, MouseEvent, SubmitEvent};

use super::controls::ContextRing;
use crate::actions::AppActions;
use crate::types::*;

#[component]
#[allow(clippy::too_many_arguments)]
pub(crate) fn ComposerView<S, K, R, C, P, T, SS, DE>(
    composer_ref: NodeRef<html::Textarea>,
    composer_height: ReadSignal<i32>,
    draft: ReadSignal<String>,
    set_draft: WriteSignal<String>,
    mode: ReadSignal<PermissionMode>,
    model_name: ReadSignal<String>,
    model_options: ReadSignal<Vec<String>>,
    reasoning_enabled: ReadSignal<bool>,
    effort: ReadSignal<ReasoningEffort>,
    effort_options: ReadSignal<Vec<String>>,
    is_sending: ReadSignal<bool>,
    active_turn_id: ReadSignal<Option<String>>,
    stick_to_bottom: ReadSignal<bool>,
    set_stick_to_bottom: WriteSignal<bool>,
    context_usage: ReadSignal<Option<ContextUsage>>,
    actions: AppActions,
    settings_summary: SS,
    draft_is_empty: DE,
    on_submit: S,
    on_keydown: K,
    on_begin_resize: R,
    on_clear: C,
    on_send_plan: P,
    on_cancel_turn: T,
) -> impl IntoView
where
    S: Fn(SubmitEvent) + 'static,
    K: Fn(KeyboardEvent) + 'static,
    R: Fn(MouseEvent) + 'static,
    C: Fn(MouseEvent) + Copy + 'static,
    P: Fn(MouseEvent) + Copy + Send + 'static,
    T: Fn(MouseEvent) + Copy + 'static,
    SS: Fn() -> String + Copy + Send + 'static,
    DE: Fn() -> bool + Copy + Send + 'static,
{
    view! {
        <form
            class="composer"
            style=move || format!("--input-min-height: {}px", composer_height.get())
            on:submit=on_submit
        >
            {move || {
                if stick_to_bottom.get() {
                    ().into_any()
                } else {
                    view! {
                        <button
                            type="button"
                            class="jump-to-bottom"
                            title="К последнему сообщению"
                            aria-label="К последнему сообщению"
                            on:click=move |_| set_stick_to_bottom.set(true)
                        >
                            "↓"
                        </button>
                    }.into_any()
                }
            }}
            <div class="composer-shell">
                <div
                    class="composer-resize-handle"
                    aria-hidden="true"
                    on:mousedown=on_begin_resize
                ></div>
                <textarea
                    node_ref=composer_ref
                    prop:value=move || draft.get()
                    placeholder=move || {
                        if mode.get() == PermissionMode::Plan {
                            "Опиши тему; агент задаст уточняющие вопросы"
                        } else {
                            "Попроси Proteus посмотреть, изменить или объяснить код"
                        }
                    }
                    on:input:target=move |ev| set_draft.set(ev.target().value())
                    on:keydown=on_keydown
                />
                <div class="composer-actions">
                    <ContextRing usage=context_usage />
                    <div class="composer-buttons">
                        <button type="button" class="secondary" on:click=on_clear>
                            "Очистить"
                        </button>
                        {move || {
                            if mode.get() == PermissionMode::Plan {
                                ().into_any()
                            } else {
                                view! {
                                    <button
                                        type="button"
                                        class="secondary"
                                        disabled=move || draft_is_empty() || is_sending.get()
                                        on:click=on_send_plan
                                        title="Переключиться в план и задать уточняющие вопросы"
                                    >
                                        "План"
                                    </button>
                                }.into_any()
                            }
                        }}
                        <button
                            type="button"
                            class="secondary danger"
                            disabled=move || active_turn_id.get().is_none()
                            on:click=on_cancel_turn
                        >
                            "Стоп"
                        </button>
                        <details class="composer-menu">
                            <summary class="composer-menu-trigger" aria-label="Настройки запроса">
                                <span class="composer-menu-summary">{settings_summary}</span>
                            </summary>
                            <div class="composer-menu-panel">
                                <section class="composer-menu-section">
                                    <span class="composer-menu-label">"model"</span>
                                    <div class="composer-menu-options model-options">
                                        {move || {
                                            let options = model_options.get();
                                            let current = model_name.get();
                                            if options.is_empty() {
                                                let label = if current.trim().is_empty() {
                                                    "default".to_owned()
                                                } else {
                                                    current
                                                };
                                                view! {
                                                    <button type="button" class="menu-option active" disabled=true>
                                                        {label}
                                                    </button>
                                                }.into_any()
                                            } else {
                                                view! {
                                                    <For
                                                        each=move || model_options.get()
                                                        key=|model| model.clone()
                                                        children=move |model| {
                                                            let active_model = model.clone();
                                                            let click_model = model.clone();
                                                            view! {
                                                                <button
                                                                    type="button"
                                                                    class="menu-option"
                                                                    class:active=move || model_name.get() == active_model
                                                                    on:click=move |_| actions.set_model_name(click_model.clone())
                                                                >
                                                                    {model}
                                                                </button>
                                                            }
                                                        }
                                                    />
                                                }.into_any()
                                            }
                                        }}
                                    </div>
                                </section>

                                <section class="composer-menu-section">
                                    <span class="composer-menu-label">"mode"</span>
                                    <div class="composer-menu-options">
                                        <button
                                            type="button"
                                            class="menu-option"
                                            class:active=move || mode.get() == PermissionMode::Plan
                                            title=PermissionMode::Plan.description()
                                            on:click=move |_| actions.set_permission_mode(PermissionMode::Plan)
                                        >
                                            {PermissionMode::Plan.label()}
                                        </button>
                                        <button
                                            type="button"
                                            class="menu-option"
                                            class:active=move || mode.get() == PermissionMode::Normal
                                            title=PermissionMode::Normal.description()
                                            on:click=move |_| actions.set_permission_mode(PermissionMode::Normal)
                                        >
                                            {PermissionMode::Normal.label()}
                                        </button>
                                        <button
                                            type="button"
                                            class="menu-option"
                                            class:active=move || mode.get() == PermissionMode::Auto
                                            title=PermissionMode::Auto.description()
                                            on:click=move |_| actions.set_permission_mode(PermissionMode::Auto)
                                        >
                                            {PermissionMode::Auto.label()}
                                        </button>
                                    </div>
                                </section>

                                <section class="composer-menu-section compact">
                                    <span class="composer-menu-label">"reasoning"</span>
                                    <div class="composer-menu-options">
                                        <button
                                            type="button"
                                            class="menu-option"
                                            class:active=move || reasoning_enabled.get()
                                            on:click=move |_| actions.set_reasoning_enabled(true)
                                        >
                                            "on"
                                        </button>
                                        <button
                                            type="button"
                                            class="menu-option"
                                            class:active=move || !reasoning_enabled.get()
                                            on:click=move |_| actions.set_reasoning_enabled(false)
                                        >
                                            "off"
                                        </button>
                                    </div>
                                </section>

                                <section class="composer-menu-section compact">
                                    <span class="composer-menu-label">"effort"</span>
                                    <div class="composer-menu-options">
                                        <button
                                            type="button"
                                            class="menu-option"
                                            class:active=move || effort.get() == ReasoningEffort::Config
                                            disabled=move || !reasoning_enabled.get()
                                            on:click=move |_| actions.set_reasoning_effort(ReasoningEffort::Config)
                                        >
                                            "auto"
                                        </button>
                                        <For
                                            each=move || effort_options.get()
                                            key=|option| option.clone()
                                            children=move |option| {
                                                let active_effort = option.clone();
                                                let click_effort = ReasoningEffort::from_value(&option);
                                                view! {
                                                    <button
                                                        type="button"
                                                        class="menu-option"
                                                        class:active=move || effort.get().value() == active_effort
                                                        disabled=move || !reasoning_enabled.get()
                                                        on:click=move |_| actions.set_reasoning_effort(click_effort.clone())
                                                    >
                                                        {option}
                                                    </button>
                                                }
                                            }
                                        />
                                    </div>
                                </section>
                            </div>
                        </details>
                        <button type="submit" class="btn-primary" disabled=draft_is_empty>
                            {move || {
                                if is_sending.get() {
                                    "В очередь"
                                } else if mode.get() == PermissionMode::Plan {
                                    "Спросить план"
                                } else {
                                    "Отправить"
                                }
                            }}
                        </button>
                    </div>
                </div>
            </div>
        </form>
    }
}
