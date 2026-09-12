use super::extensions::ExtensionSettingsView;
use crate::ui_preferences::{TOOL_CARDS_COLLAPSED_KEY, try_save_bool_setting};
use leptos::prelude::*;

#[component]
pub(crate) fn SettingsView(
    tool_cards_collapsed: ReadSignal<bool>,
    set_tool_cards_collapsed: WriteSignal<bool>,
) -> impl IntoView {
    let (status, set_status) = signal(String::new());
    let toggle = move |_| {
        let next = !tool_cards_collapsed.get_untracked();
        match try_save_bool_setting(TOOL_CARDS_COLLAPSED_KEY, next) {
            Ok(()) => {
                set_tool_cards_collapsed.set(next);
                set_status.set("Сохранено на этом устройстве".into());
            }
            Err(error) => {
                set_tool_cards_collapsed.set(tool_cards_collapsed.get_untracked());
                set_status.set(format!("Не сохранено: {error}"));
            }
        }
    };
    view! {
        <section class="settings-page">
            <header class="settings-toolbar">
                <h1>"Настройки"</h1>
                <p>"Настройте рабочее пространство под себя."</p>
            </header>
            <nav class="settings-nav" aria-label="Разделы настроек">
                <a href="#general">"Чат"</a><a href="#extensions">"Расширения"</a>
            </nav>
            <section class="settings-section" id="general">
                <h2>"Чат"</h2>
                <label class="settings-row">
                    <span class="settings-label"><strong>"Компактные карточки инструментов"</strong>
                        <span class="settings-hint">"Показывать подробности выполнения только при раскрытии карточки. Настройка действует для всех чатов на этом устройстве."</span>
                    </span>
                    <input type="checkbox" class="settings-toggle" prop:checked=move || tool_cards_collapsed.get() on:change=toggle />
                </label>
                <p class="settings-status" role="status">{move || status.get()}</p>
            </section>
            <section class="settings-section" id="extensions">
                <h2>"Расширения"</h2>
                <p class="settings-section-description">"Боковые панели чата. Включайте нужные и меняйте их порядок."</p>
                <ExtensionSettingsView />
            </section>
        </section>
    }
}
