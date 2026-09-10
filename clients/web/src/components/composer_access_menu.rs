use crate::{actions::AppActions, types::PermissionMode};
use leptos::prelude::*;

fn mode_label(mode: PermissionMode) -> &'static str {
    match mode {
        PermissionMode::Plan => "Планирование",
        PermissionMode::Normal => "С подтверждениями",
        PermissionMode::Auto => "Без подтверждений",
    }
}

#[component]
pub(super) fn ComposerAccessMenu(
    mode: ReadSignal<PermissionMode>,
    actions: AppActions,
) -> impl IntoView {
    view! {
        <details class="composer-menu composer-access-menu">
            <summary class="composer-menu-trigger" aria-label="Режим доступа" title=move || mode.get().description()>
                <super::icons::ShieldIcon />
                <span class="composer-menu-model">{move || mode_label(mode.get())}</span>
                <super::icons::ChevronDownIcon />
            </summary>
            <div class="composer-menu-panel">
                <section class="composer-menu-section">
                    <span class="composer-menu-label">"Режим доступа"</span>
                    <div class="composer-menu-options stacked">
                        {[PermissionMode::Normal, PermissionMode::Auto, PermissionMode::Plan]
                            .into_iter().map(|option| view! {
                                <button type="button" class="menu-option menu-option-row"
                                    class:active=move || mode.get() == option
                                    on:click=move |_| actions.set_permission_mode(option)>
                                    <span class="menu-option-text">
                                        <span class="menu-option-title">{mode_label(option)}</span>
                                        <span class="menu-option-desc">{option.description()}</span>
                                    </span>
                                    <span class="menu-option-check" aria-hidden="true">"✓"</span>
                                </button>
                            }).collect_view()}
                    </div>
                </section>
            </div>
        </details>
    }
}
