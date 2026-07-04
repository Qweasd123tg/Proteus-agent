use leptos::{html, prelude::*};
use web_sys::{KeyboardEvent, MouseEvent, SubmitEvent};

use crate::actions::AppActions;
use crate::types::*;
use crate::ui_utils::compact_text;

#[component]
#[allow(clippy::too_many_arguments)]
pub(crate) fn ComposerView<S, K, R, T, DE, NB>(
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
    actions: AppActions,
    draft_is_empty: DE,
    new_below_count: NB,
    on_submit: S,
    on_keydown: K,
    on_begin_resize: R,
    on_cancel_turn: T,
) -> impl IntoView
where
    S: Fn(SubmitEvent) + 'static,
    K: Fn(KeyboardEvent) + 'static,
    R: Fn(MouseEvent) + 'static,
    T: Fn(MouseEvent) + Copy + Send + 'static,
    DE: Fn() -> bool + Copy + Send + 'static,
    NB: Fn() -> usize + Copy + Send + Sync + 'static,
{
    // Чип настроек: модель — ярко, режим и effort — приглушённой сноской.
    let trigger_model = move || {
        let model = model_name.get();
        if model.trim().is_empty() {
            "модель".to_owned()
        } else {
            compact_text(&model, 24)
        }
    };
    let trigger_meta = move || {
        let reasoning = if reasoning_enabled.get() {
            effort.get().label()
        } else {
            "none".to_owned()
        };
        format!("{} · {}", mode.get().label(), reasoning)
    };
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
                            class=move || {
                                if new_below_count() > 0 {
                                    "jump-to-bottom has-count"
                                } else {
                                    "jump-to-bottom"
                                }
                            }
                            title="К последнему сообщению"
                            aria-label="К последнему сообщению"
                            on:click=move |_| set_stick_to_bottom.set(true)
                        >
                            {move || {
                                let count = new_below_count();
                                if count > 0 {
                                    format!("↓ {count}")
                                } else {
                                    "↓".to_owned()
                                }
                            }}
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
                    <div class="composer-buttons">
                        // Стоп появляется только пока идёт ход — в покое не
                        // держим мёртвую серую кнопку.
                        {move || {
                            if active_turn_id.get().is_some() {
                                view! {
                                    <button
                                        type="button"
                                        class="secondary danger"
                                        on:click=on_cancel_turn
                                    >
                                        "Стоп"
                                    </button>
                                }.into_any()
                            } else {
                                ().into_any()
                            }
                        }}
                        <details class="composer-menu">
                            <summary class="composer-menu-trigger" aria-label="Настройки запроса">
                                <span class="composer-menu-model">{trigger_model}</span>
                                <span class="composer-menu-meta">{trigger_meta}</span>
                            </summary>
                            <div class="composer-menu-panel">
                                <section class="composer-menu-section">
                                    <span class="composer-menu-label">"model"</span>
                                    <div class="composer-menu-options stacked">
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
                                                    <button type="button" class="menu-option menu-option-row active" disabled=true>
                                                        <span class="menu-option-title">{label}</span>
                                                        <span class="menu-option-check" aria-hidden="true">"✓"</span>
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
                                                                    class="menu-option menu-option-row"
                                                                    class:active=move || model_name.get() == active_model
                                                                    on:click=move |_| actions.set_model_name(click_model.clone())
                                                                >
                                                                    <span class="menu-option-title">{model}</span>
                                                                    <span class="menu-option-check" aria-hidden="true">"✓"</span>
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
                                    <div class="composer-menu-options stacked">
                                        <button
                                            type="button"
                                            class="menu-option menu-option-row"
                                            class:active=move || mode.get() == PermissionMode::Plan
                                            on:click=move |_| actions.set_permission_mode(PermissionMode::Plan)
                                        >
                                            <span class="menu-option-text">
                                                <span class="menu-option-title">{PermissionMode::Plan.label()}</span>
                                                <span class="menu-option-desc">{PermissionMode::Plan.description()}</span>
                                            </span>
                                            <span class="menu-option-check" aria-hidden="true">"✓"</span>
                                        </button>
                                        <button
                                            type="button"
                                            class="menu-option menu-option-row"
                                            class:active=move || mode.get() == PermissionMode::Normal
                                            on:click=move |_| actions.set_permission_mode(PermissionMode::Normal)
                                        >
                                            <span class="menu-option-text">
                                                <span class="menu-option-title">{PermissionMode::Normal.label()}</span>
                                                <span class="menu-option-desc">{PermissionMode::Normal.description()}</span>
                                            </span>
                                            <span class="menu-option-check" aria-hidden="true">"✓"</span>
                                        </button>
                                        <button
                                            type="button"
                                            class="menu-option menu-option-row"
                                            class:active=move || mode.get() == PermissionMode::Auto
                                            on:click=move |_| actions.set_permission_mode(PermissionMode::Auto)
                                        >
                                            <span class="menu-option-text">
                                                <span class="menu-option-title">{PermissionMode::Auto.label()}</span>
                                                <span class="menu-option-desc">{PermissionMode::Auto.description()}</span>
                                            </span>
                                            <span class="menu-option-check" aria-hidden="true">"✓"</span>
                                        </button>
                                    </div>
                                </section>

                                // Единственная ручка рассуждений: «none»
                                // выключает их целиком, любой effort включает.
                                <section class="composer-menu-section compact">
                                    <span class="composer-menu-label">"effort"</span>
                                    <div class="composer-menu-options">
                                        <button
                                            type="button"
                                            class="menu-option"
                                            class:active=move || !reasoning_enabled.get()
                                            title="Без рассуждений"
                                            on:click=move |_| actions.set_reasoning_effort(ReasoningEffort::None)
                                        >
                                            "none"
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
                                                        class:active=move || {
                                                            reasoning_enabled.get()
                                                                && effort.get().value() == active_effort
                                                        }
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
