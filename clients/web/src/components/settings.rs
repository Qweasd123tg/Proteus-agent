use leptos::{prelude::*, task::spawn_local};
use serde_json::Value;

use crate::api::{get_json, post_json};
use crate::types::*;

/// Страница настроек веб-клиента. Читает текущие значения из /config (секция
/// [web]) и пишет изменения в бэкенд (POST /config/web), который сохраняет их в
/// файл конфига — переход между роутами это полноценная перезагрузка, поэтому
/// чат подхватит новое значение при следующем заходе.
#[component]
pub(crate) fn SettingsView() -> impl IntoView {
    let (tool_cards_collapsed, set_tool_cards_collapsed) = signal(false);
    let (status, set_status) = signal("загружаю настройки".to_owned());

    spawn_local(async move {
        match get_json::<Value>("/config").await {
            Ok(config) => {
                if let Some(value) = config
                    .pointer("/web/tool_cards_collapsed")
                    .and_then(Value::as_bool)
                {
                    set_tool_cards_collapsed.set(value);
                }
                set_status.set(String::new());
            }
            Err(error) => set_status.set(format!("не удалось загрузить: {error}")),
        }
    });

    let toggle_tool_cards = move |_| {
        let value = !tool_cards_collapsed.get();
        set_tool_cards_collapsed.set(value);
        set_status.set("сохраняю…".to_owned());
        spawn_local(async move {
            match post_json(
                "/config/web",
                &serde_json::json!({ "id": "web", "tool_cards_collapsed": value }),
            )
            .await
            {
                Ok(StdioOutput::Response { ok: true, .. }) => {
                    set_status.set("сохранено".to_owned())
                }
                Ok(StdioOutput::Response { error, .. }) => {
                    set_status.set(error.unwrap_or_else(|| "не удалось сохранить".to_owned()));
                }
                Ok(StdioOutput::Event { .. }) => set_status.set("неожиданный ответ".to_owned()),
                Err(error) => set_status.set(format!("ошибка: {error}")),
            }
        });
    };

    view! {
        <section class="settings-page">
            <div class="settings-toolbar">
                <div>
                    <h2>"Настройки веба"</h2>
                    <p>{move || status.get()}</p>
                </div>
            </div>
            <div class="settings-list">
                <label class="settings-row">
                    <span class="settings-label">
                        <strong>"Сворачивать карточки тулов"</strong>
                        <span class="settings-hint">
                            "Карточки тулов открываются свёрнутыми по умолчанию."
                        </span>
                    </span>
                    <input
                        type="checkbox"
                        class="settings-toggle"
                        prop:checked=move || tool_cards_collapsed.get()
                        on:change=toggle_tool_cards
                    />
                </label>
            </div>
        </section>
    }
}
