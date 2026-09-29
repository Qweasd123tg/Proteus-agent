use super::Toggle;
use crate::{
    app_resize::AppResizeState,
    interface_settings::{save_number, settings},
};
use leptos::prelude::*;

#[component]
pub(super) fn Appearance(resize: AppResizeState) -> impl IntoView {
    let prefs = settings();
    let status = RwSignal::new(String::new());
    view! {
        <p class="settings-section-description">"Размер текста и ширина применяются к диалогу и полю ввода. Настройки сохраняются на этом устройстве."</p>
        <label class="settings-row">
            <span class="settings-label"><strong>"Размер текста"</strong><span class="settings-hint">"От 12 до 22 пикселей"</span></span>
            <span class="settings-range"><input aria-label="Размер текста" type="range" min="12" max="22" step="1" prop:value=move || prefs.font_size.get() on:input:target=move |ev| {
                if let Ok(value) = ev.target().value().parse::<i32>() {
                    match save_number("proteus.fontSize",value) { Ok(())=>{prefs.font_size.set(value);status.set(String::new());},Err(e)=>{ev.target().set_value(&prefs.font_size.get_untracked().to_string());status.set(e);} }
                }
            }/><output>{move || format!("{} px",prefs.font_size.get())}</output></span>
        </label>
        <label class="settings-row">
            <span class="settings-label"><strong>"Ширина диалога"</strong><span class="settings-hint">"Максимальная ширина. Её также можно менять перетаскиванием края чата."</span></span>
            <span class="settings-range"><input aria-label="Ширина диалога" type="range" min="420" max="1600" step="10" prop:value=move || resize.chat_width.get() on:input:target=move |ev| {
                if let Ok(value)=ev.target().value().parse::<i32>() {
                    match resize.set_width(value) { Ok(())=>status.set(String::new()),Err(e)=>{ev.target().set_value(&resize.chat_width.get_untracked().to_string());status.set(e);} }
                }
            }/><output>{move || format!("{} px",resize.chat_width.get())}</output></span>
        </label>
        <Toggle label="Компактный интерфейс" hint="Меньше отступы в диалоге, списке чатов и настройках." storage_key="proteus.compactInterface" value=prefs.compact/>
        <Toggle label="Анимации" hint="Плавные переходы и раскрытие панелей. Системное уменьшение движения также учитывается." storage_key="proteus.animations" value=RwSignal::new(crate::ui_preferences::load_bool_setting("proteus.animations",true)) animation=true/>
        <p class="settings-status" role="status">{move || status.get()}</p>
        <div class="appearance-preview" style=move || format!("font-size:{}px",prefs.font_size.get())><strong>"Пример сообщения"</strong><p>"Так будет выглядеть текст в диалоге. Размер можно подобрать под свой экран."</p><code>"let result = agent.run(task);"</code></div>
    }
}
