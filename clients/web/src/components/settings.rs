use super::extensions::ExtensionSettingsView;
use crate::api::{get_json, post_json};
use crate::types::*;
use leptos::{prelude::*, task::spawn_local};
use serde_json::Value;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

#[component]
pub(crate) fn SettingsView(
    tool_cards_collapsed: ReadSignal<bool>,
    set_tool_cards_collapsed: WriteSignal<bool>,
) -> impl IntoView {
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
                <ChatSettings tool_cards_collapsed set_tool_cards_collapsed />
            </section>
            <section class="settings-section" id="extensions">
                <h2>"Расширения"</h2>
                <p class="settings-section-description">"Панели в правой области чата. Включайте нужные и меняйте их порядок."</p>
                <ExtensionSettingsView />
            </section>
        </section>
    }
}

#[component]
fn ChatSettings(
    tool_cards_collapsed: ReadSignal<bool>,
    set_tool_cards_collapsed: WriteSignal<bool>,
) -> impl IntoView {
    let (value, set_value) = signal(tool_cards_collapsed.get_untracked());
    let (pending, set_pending) = signal(true);
    let (ready, set_ready) = signal(false);
    let (status, set_status) = signal("Загрузка…".to_owned());
    let alive = Arc::new(AtomicBool::new(true));
    let cleanup = alive.clone();
    on_cleanup(move || cleanup.store(false, Ordering::Relaxed));
    let load = move || {
        let alive = alive.clone();
        set_pending.set(true);
        set_status.set("Загрузка…".into());
        spawn_local(async move {
            let result = get_json::<Value>("/config").await;
            if !alive.load(Ordering::Relaxed) {
                return;
            }
            match result {
                Ok(config) => {
                    if let Some(value) = config
                        .pointer("/web/tool_cards_collapsed")
                        .and_then(Value::as_bool)
                    {
                        set_value.set(value);
                        set_tool_cards_collapsed.set(value);
                        set_status.set(String::new());
                        set_ready.set(true);
                    } else {
                        set_status.set(
                            "Сервер не предоставил настройки чата. Попробуйте загрузить снова."
                                .into(),
                        );
                    }
                }
                Err(error) => set_status.set(format!(
                    "Не удалось загрузить настройки: {error}. Попробуйте загрузить снова."
                )),
            }
            set_pending.set(false);
        });
    };
    load();
    let toggle = move |_| {
        if pending.get_untracked() {
            return;
        }
        let previous = value.get_untracked();
        let next = !previous;
        set_value.set(next);
        set_pending.set(true);
        set_status.set("Сохранение…".into());
        spawn_local(async move {
            let result = post_json(
                "/config/web",
                &serde_json::json!({"id":"web","tool_cards_collapsed":next}),
            )
            .await;
            let error = match result {
                Ok(StdioOutput::Response { ok: true, .. }) => None,
                Ok(StdioOutput::Response { error, .. }) => {
                    Some(error.unwrap_or_else(|| "Не удалось сохранить".into()))
                }
                Ok(_) => Some("Неожиданный ответ сервера".into()),
                Err(error) => Some(error),
            };
            if let Some(error) = error {
                set_value.try_set(previous);
                set_status.try_set(format!("Не сохранено: {error}"));
            } else {
                set_tool_cards_collapsed.set(next);
                set_status.try_set("Сохранено".into());
            }
            set_pending.try_set(false);
        });
    };
    view! {
        <label class="settings-row">
            <span class="settings-label"><strong>"Компактные карточки инструментов"</strong>
                <span class="settings-hint">"Показывать подробности выполнения только при раскрытии карточки."</span>
            </span>
            <input type="checkbox" class="settings-toggle" prop:checked=move || value.get() disabled=move || pending.get() || !ready.get() on:change=toggle />
        </label>
        <p class="settings-status" role="status">{move || status.get()}</p>
        <button type="button" class="secondary settings-retry" hidden=move || ready.get() disabled=move || pending.get() on:click=move |_| load()>"Повторить загрузку"</button>
    }
}
