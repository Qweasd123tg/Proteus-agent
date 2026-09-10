use crate::{actions::AppActions, types::*, ui_utils::compact_text};
use leptos::prelude::*;

#[component]
pub(super) fn ComposerModelMenu(
    model_name: ReadSignal<String>,
    model_options: ReadSignal<Vec<ModelOption>>,
    reasoning_enabled: ReadSignal<bool>,
    effort: ReadSignal<ReasoningEffort>,
    effort_options: ReadSignal<Vec<String>>,
    actions: AppActions,
) -> impl IntoView {
    let trigger_model = move || {
        let model = model_name.get();
        if model.trim().is_empty() {
            "Модель из профиля".to_owned()
        } else {
            let label = model_options
                .get()
                .into_iter()
                .find(|option| option.name == model)
                .map(|option| option.label)
                .unwrap_or(model);
            compact_text(&label, 32)
        }
    };
    view! {
    <details class="composer-menu composer-model-menu">
        <summary class="composer-menu-trigger" aria-label="Модель и рассуждение" title=move || model_name.get()>
            <span class="composer-menu-model">{trigger_model}</span>
            <Show when=move || reasoning_enabled.get()><span class="composer-menu-meta">{move || effort.get().label()}</span></Show>
            <super::icons::ChevronDownIcon />
        </summary>
        <div class="composer-menu-panel">
            <section class="composer-menu-section">
                <span class="composer-menu-label">"Модель"</span>
                <div class="composer-menu-options stacked">
                    {move || {
                        let options = model_options.get();
                        let current = model_name.get();
                        if options.is_empty() {
                            let label = if current.trim().is_empty() {
                                "Из профиля".to_owned()
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
                                    key=|model| model.name.clone()
                                    children=move |model| {
                                        let active_model = model.name.clone();
                                        let click_model = model.name.clone();
                                        let label = if model.hidden { format!("{} (скрытая)", model.label) } else { model.label };
                                        view! {
                                            <button
                                                type="button"
                                                class="menu-option menu-option-row"
                                                class:active=move || model_name.get() == active_model
                                                on:click=move |_| actions.set_model_name(click_model.clone())
                                            >
                                                <span class="menu-option-title">{label}</span>
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

            <Show when=move || !effort_options.get().is_empty()>
            <section class="composer-menu-section compact">
                <span class="composer-menu-label">"Рассуждение"</span>
                <div class="composer-menu-options">
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
                                        effort.get().value() == active_effort
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
            </Show>
        </div>
    </details>
    }
}
