mod appearance;
mod chat;
mod shortcuts;
use super::extensions::ExtensionSettingsView;
use crate::app_resize::AppResizeState;
use leptos::prelude::*;

#[component]
pub(crate) fn SettingsView<N>(
    active_session_dir: ReadSignal<Option<String>>,
    on_navigate: N,
    tool_cards_collapsed: ReadSignal<bool>,
    set_tool_cards_collapsed: WriteSignal<bool>,
    resize: AppResizeState,
) -> impl IntoView
where
    N: Fn(web_sys::MouseEvent, &'static str) + Copy + Send + Sync + 'static,
{
    let section = RwSignal::new("appearance");
    view! {
        <section class="settings-page">
            <nav class="settings-nav" aria-label="Разделы настроек">
                <a class="settings-back" href="/" on:click=move |ev| on_navigate(ev,"/")><super::icons::BackIcon/>"Вернуться в чат"</a>
                <span class="settings-nav-label">"Настройки"</span>
                <button type="button" data-settings-section="appearance" class:active=move || section.get()=="appearance" aria-pressed=move || (section.get()=="appearance").to_string() on:click=move |_| section.set("appearance")><super::icons::SettingsIcon/>"Внешний вид"</button>
                <button type="button" data-settings-section="chat" class:active=move || section.get()=="chat" aria-pressed=move || (section.get()=="chat").to_string() on:click=move |_| section.set("chat")><super::icons::ChatIcon/>"Чат"</button>
                <button type="button" data-settings-section="shortcuts" class:active=move || section.get()=="shortcuts" aria-pressed=move || (section.get()=="shortcuts").to_string() on:click=move |_| section.set("shortcuts")><super::icons::KeyboardIcon/>"Сочетания клавиш"</button>
                <button type="button" data-settings-section="extensions" class:active=move || section.get()=="extensions" aria-pressed=move || (section.get()=="extensions").to_string() on:click=move |_| section.set("extensions")><super::icons::ExtensionsIcon/>"Расширения"</button>
                <button type="button" data-settings-section="diagnostics" class:active=move || section.get()=="diagnostics" aria-pressed=move || (section.get()=="diagnostics").to_string() on:click=move |_| section.set("diagnostics")><super::icons::InspectorIcon/>"Диагностика"</button>
            </nav>
            <div class="settings-content">
                <header class="settings-toolbar"><h1>{move || match section.get(){"chat"=>"Чат","shortcuts"=>"Сочетания клавиш","extensions"=>"Расширения","diagnostics"=>"Диагностика",_=>"Внешний вид"}}</h1></header>
                <section class="settings-section" hidden=move || section.get()!="appearance"><appearance::Appearance resize/></section>
                <section class="settings-section" id="chat-settings" hidden=move || section.get()!="chat"><chat::ChatSettings tool_cards_collapsed set_tool_cards_collapsed/></section>
                <section class="settings-section" hidden=move || section.get()!="shortcuts"><Show when=move || section.get()=="shortcuts"><shortcuts::Shortcuts/></Show></section>
                <section class="settings-section" id="extensions" hidden=move || section.get()!="extensions">
                    <p class="settings-section-description">"Выберите инструменты для меню «+» боковой области. Здесь можно включить расширение, настроить его и изменить порядок вкладок."</p><ExtensionSettingsView/>
                </section>
                <section class="settings-section" hidden=move || section.get()!="diagnostics"><h2>"Инструменты разработчика"</h2><a class="settings-row" href=move || crate::api::inspector_link_url(active_session_dir.get().as_deref())><span class="settings-label"><strong>"Inspector"</strong><span class="settings-hint">"Расход, контекст, разбор ходов агента, сборка и архитектура модулей."</span></span><super::icons::InspectorIcon/></a></section>
            </div>
        </section>
    }
}

#[component]
fn Toggle(
    label: &'static str,
    hint: &'static str,
    storage_key: &'static str,
    value: RwSignal<bool>,
    #[prop(default = false)] animation: bool,
) -> impl IntoView {
    let status = RwSignal::new(String::new());
    view! {
        <label class="settings-row"><span class="settings-label"><strong>{label}</strong><span class="settings-hint">{hint}</span></span>
            <input type="checkbox" class="settings-toggle" aria-label=label data-animation-toggle=animation.then_some("") prop:checked=move || value.get() on:change:target=move |ev| {
                let next=ev.target().checked();
                match crate::ui_preferences::try_save_bool_setting(storage_key,next) {Ok(())=>{value.set(next);status.set(String::new());},Err(e)=>{ev.target().set_checked(value.get_untracked());status.set(format!("Не сохранено: {e}"));}}
            }/>
        </label><Show when=move || !status.get().is_empty()><p class="settings-status" role="status">{move || status.get()}</p></Show>
    }
}
