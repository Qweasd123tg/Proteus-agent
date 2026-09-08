use leptos::prelude::*;

use crate::types::*;
use crate::ui_utils::{short_path, shorten_home};

#[component]
pub(super) fn ConfigOverview(summary: ConfigSummary) -> impl IntoView {
    let config_path_full = summary
        .config_path
        .as_deref()
        .unwrap_or("(не выбран)")
        .to_owned();
    let config_path = shorten_home(&config_path_full);
    let model = non_empty(&summary.model.label, "Модель не выбрана");
    view! {
        <section class="config-overview cfg-overview" aria-label="Текущая сборка">
            <article class="config-panel">
                <span class="panel-kicker">"Профиль"</span>
                <strong>{non_empty(&summary.profile, "default")}</strong>
                <p><code title=config_path_full>{config_path}</code></p>
            </article>
            <article class="config-panel">
                <span class="panel-kicker">"Модель"</span>
                <strong>{model}</strong>
                <p>{format!("{} · {}", non_empty(&summary.model.provider, "provider не задан"), non_empty(&summary.model.name, "model не задан"))}</p>
            </article>
            <article class="config-panel">
                <span class="panel-kicker">"Режим"</span>
                <strong>{permission_label(&summary.permission_mode)}</strong>
                <p>{format!("{} модулей · {} инструментов", summary.modules.len(), summary.registered_tools.len())}</p>
            </article>
        </section>
    }
}

#[component]
pub(super) fn ConfigSections(summary: ConfigSummary) -> impl IntoView {
    let components = summary.components.clone();
    let files = summary.config_files.clone();
    view! {
        <div class="cfg-details-sections">
            <RuntimeFacts summary=summary.clone()/>
            <details class="cfg-details-group">
                <summary>{format!("Компоненты · {}", components.len())}</summary>
                <div class="config-list">
                    <For each=move || components.clone() key=|component| component.id.clone() children=move |component| {
                        let exports = component.exports.iter().map(|export| format!("{}/{}", export.slot, export.module_id)).collect::<Vec<_>>().join(", ");
                        view! { <article class="config-list-item">
                            <div class="config-list-main"><div class="config-list-title"><strong>{component.id}</strong></div><p>{exports}</p></div>
                            <span class="status-badge completed"><span class="dot"></span>"Настроен"</span>
                        </article> }
                    }/>
                </div>
            </details>
            <details class="cfg-details-group">
                <summary>{format!("Файлы профиля · {}", files.len())}</summary>
                <div class="config-chip-row">
                    <For each=move || files.clone() key=|path| path.clone() children=move |path| {
                        let full = path.clone(); view! { <span class="config-chip" title=full>{short_path(&path)}</span> }
                    }/>
                </div>
            </details>
        </div>
    }
}

fn non_empty(value: &str, fallback: &str) -> String {
    if value.trim().is_empty() {
        fallback.to_owned()
    } else {
        value.to_owned()
    }
}

fn permission_label(mode: &str) -> String {
    match mode {
        "Plan" => "Только чтение".into(),
        "Normal" => "С подтверждениями".into(),
        "Auto" => "Автоматический".into(),
        _ => non_empty(mode, "Не задан"),
    }
}

#[component]
fn RuntimeFacts(summary: ConfigSummary) -> impl IntoView {
    let cwd = shorten_home(&summary.cwd);
    let session_dir = summary
        .session_dir
        .as_deref()
        .map(shorten_home)
        .unwrap_or_else(|| "Не задан".to_owned());
    let effort = summary
        .reasoning
        .effort
        .clone()
        .unwrap_or_else(|| "Автоматически".to_owned());
    let budget = summary
        .reasoning
        .budget_tokens
        .map(|tokens| format!("{tokens} токенов"))
        .unwrap_or_else(|| "Не задан".to_owned());
    view! {
        <section class="config-section">
            <div class="config-section-header"><h3>"Среда и параметры модели"</h3><span>"Текущая сборка"</span></div>
            <div class="config-row"><span>"Рабочий каталог"</span><code title=summary.cwd>{cwd}</code></div>
            <div class="config-row"><span>"Каталог сессий"</span><code title=summary.session_dir.unwrap_or_default()>{session_dir}</code></div>
            <div class="config-row"><span>"Рассуждение модели"</span><code>{if summary.reasoning.enabled { "Включено" } else { "Отключено" }}</code></div>
            <div class="config-row"><span>"Уровень рассуждения"</span><code>{effort}</code></div>
            <div class="config-row"><span>"Сводка рассуждения"</span><code>{if summary.reasoning.summary { "Включена" } else { "Отключена" }}</code></div>
            <div class="config-row"><span>"Бюджет рассуждения"</span><code>{budget}</code></div>
        </section>
    }
}
