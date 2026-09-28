use super::extensions::ExtensionSettingsView;
use crate::ui_preferences::{TOOL_CARDS_COLLAPSED_KEY, try_save_bool_setting};
use leptos::prelude::*;

#[component]
pub(crate) fn SettingsView<N>(
    active_session_dir: ReadSignal<Option<String>>,
    on_navigate: N,
    tool_cards_collapsed: ReadSignal<bool>,
    set_tool_cards_collapsed: WriteSignal<bool>,
) -> impl IntoView
where
    N: Fn(web_sys::MouseEvent, &'static str) + Copy + Send + Sync + 'static,
{
    let section = RwSignal::new("general");
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
            <nav class="settings-nav" aria-label="Разделы настроек">
                <a class="settings-back" href="/" on:click=move |ev| on_navigate(ev, "/")><super::icons::BackIcon/>"Вернуться в чат"</a>
                <span class="settings-nav-label">"Настройки"</span>
                <button type="button" class:active=move || section.get()=="general" aria-pressed=move || (section.get()=="general").to_string() on:click=move |_| section.set("general")><super::icons::SettingsIcon/>"Общее"</button>
                <button type="button" data-settings-section="extensions" class:active=move || section.get()=="extensions" aria-pressed=move || (section.get()=="extensions").to_string() on:click=move |_| section.set("extensions")><super::icons::ExtensionsIcon/>"Расширения"</button>
                <button type="button" data-settings-section="diagnostics" class:active=move || section.get()=="diagnostics" aria-pressed=move || (section.get()=="diagnostics").to_string() on:click=move |_| section.set("diagnostics")><super::icons::InspectorIcon/>"Диагностика"</button>
            </nav>
            <div class="settings-content">
            <header class="settings-toolbar">
                <h1>{move || match section.get() { "extensions" => "Расширения", "diagnostics" => "Диагностика", _ => "Общее" }}</h1>

            </header>
            <section class="settings-section" id="general" hidden=move || section.get()!="general">
                <h2>"Чат"</h2>
                <label class="settings-row">
                    <span class="settings-label"><strong>"Компактные карточки инструментов"</strong>
                        <span class="settings-hint">"Показывать подробности выполнения только при раскрытии карточки. Настройка действует для всех чатов на этом устройстве."</span>
                    </span>
                    <input type="checkbox" class="settings-toggle" prop:checked=move || tool_cards_collapsed.get() on:change=toggle />
                </label>
                <p class="settings-status" role="status">{move || status.get()}</p>
            </section>
            <section class="settings-section" id="extensions" hidden=move || section.get()!="extensions">

                <p class="settings-section-description">"Выберите инструменты для меню «+» боковой области. Здесь можно включить расширение, настроить его и изменить порядок вкладок."</p>
                <ExtensionSettingsView />
            </section>
            <section class="settings-section" hidden=move || section.get()!="diagnostics">
                <h2>"Инструменты разработчика"</h2>
                <a class="settings-row" href=move || crate::api::inspector_link_url(active_session_dir.get().as_deref())>
                    <span class="settings-label"><strong>"Inspector"</strong><span class="settings-hint">"Расход, контекст, разбор ходов агента, сборка и архитектура модулей."</span></span><super::icons::InspectorIcon/>
                </a>

            </section>
            </div>
        </section>
    }
}
