use super::Toggle;
use crate::interface_settings::settings;
use leptos::prelude::*;

#[component]
pub(super) fn ChatSettings(
    tool_cards_collapsed: ReadSignal<bool>,
    set_tool_cards_collapsed: WriteSignal<bool>,
) -> impl IntoView {
    let prefs = settings();
    let status = RwSignal::new(String::new());
    view! {
        <label class="settings-row"><span class="settings-label"><strong>"Отправка сообщения"</strong><span class="settings-hint">"Shift+Enter всегда переносит строку. Во время ответа отправка добавляет сообщение в очередь."</span></span>
            <select aria-label="Отправка сообщения" prop:value=move || if prefs.ctrl_enter.get(){"ctrl-enter"}else{"enter"} on:change:target=move |ev| {
                let next=ev.target().value()=="ctrl-enter";
                match crate::ui_preferences::try_save_bool_setting("proteus.ctrlEnter",next) {Ok(())=>{prefs.ctrl_enter.set(next);status.set("Сохранено на этом устройстве".into());},Err(e)=>{ev.target().set_value(if prefs.ctrl_enter.get_untracked(){"ctrl-enter"}else{"enter"});status.set(format!("Не сохранено: {e}"));}}
            }><option value="enter">"Enter"</option><option value="ctrl-enter">"Ctrl / Cmd + Enter"</option></select>
        </label>
        <label class="settings-row"><span class="settings-label"><strong>"Компактные цепочки инструментов"</strong><span class="settings-hint">"Сворачивать новые цепочки в одну строку. Уже раскрытые цепочки сохраняют свой вид."</span></span>
            <input type="checkbox" class="settings-toggle" aria-label="Компактные цепочки инструментов" prop:checked=move || tool_cards_collapsed.get() on:change:target=move |ev| {
                let next=ev.target().checked();
                match crate::ui_preferences::try_save_bool_setting(crate::ui_preferences::TOOL_CARDS_COLLAPSED_KEY,next) {Ok(())=>{set_tool_cards_collapsed.set(next);status.set("Сохранено на этом устройстве".into());},Err(e)=>{ev.target().set_checked(tool_cards_collapsed.get_untracked());status.set(format!("Не сохранено: {e}"));}}
            }/>
        </label>
        <Toggle label="Автопрокрутка" hint="Следовать за новыми сообщениями и ответом. Прокрутка вверх приостанавливает следование. При отключении кнопка «К последнему сообщению» перемещает вниз один раз." storage_key="proteus.autoScroll" value=prefs.auto_scroll/>
        <p class="settings-status" role="status">{move || status.get()}</p>
    }
}
