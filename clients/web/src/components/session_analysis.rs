use leptos::prelude::*;

use super::{context_map::ContextMapView, extensions::UsageDetailsView};
use crate::{session::summaries::sidebar_session_title, types::SessionSummary, ui_utils::short_id};

mod location;

#[component]
pub(crate) fn SessionAnalysisView<O>(
    sessions: ReadSignal<Vec<SessionSummary>>,
    active_session_dir: ReadSignal<Option<String>>,
    on_open: O,
) -> impl IntoView
where
    O: Fn(SessionSummary) + Copy + Send + Sync + 'static,
{
    let (requested, context) = location::read();
    let (selected, set_selected) = signal(requested.or_else(|| active_session_dir.get_untracked()));
    let (context_tab, set_context_tab) = signal(context);
    let summary = Memo::new(move |_| {
        let selected = selected.get();
        sessions.with(|items| {
            items
                .iter()
                .find(|item| item.session_dir.to_str() == selected.as_deref())
                .cloned()
        })
    });
    Effect::new(move |_| {
        let active = active_session_dir.get();
        if selected.get_untracked().is_none() {
            set_selected.set(active);
        }
    });
    Effect::new(move |_| location::persist(selected.get().as_deref(), context_tab.get()));

    view! {
        <section class="context-page analysis-page">
            <div class="analysis-heading">
                <div class="analysis-title">
                    <span class="panel-kicker">"Анализ сессии"</span>
                    <h1>{move || summary.get().map(|item| sidebar_session_title(&item)).unwrap_or_else(|| if selected.get().is_some() { "Сохранённая сессия" } else { "Выберите сессию" }.to_owned())}</h1>
                    <p>{move || summary.get().map(|item| format!("{} · {} сообщений", item.workspace_path.display(), item.message_count))}</p>
                    <details class="analysis-identity">
                        <summary>"Идентификаторы сессии"</summary>
                        <code>{move || summary.get().map(|item| format!("ID: {}", item.session_id))}</code>
                        <code>{move || selected.get()}</code>
                    </details>
                </div>
                <div class="analysis-session-actions">
                    <label for="analysis-session">"Сессия для анализа"</label>
                    <select id="analysis-session" class="context-session-select"
                        prop:value=move || selected.get().unwrap_or_default()
                        on:change:target=move |event| {
                            let value = event.target().value();
                            set_selected.set((!value.is_empty()).then_some(value));
                        }>
                        {move || if summary.get().is_none() {
                            let value = selected.get().unwrap_or_default();
                            let label = if value.is_empty() { "Выберите сессию" } else { "Выбранная сессия" };
                            Some(view! { <option value=value>{label}</option> })
                        } else { None }}
                        <For each=move || sessions.get() key=|item| item.session_dir.to_string_lossy().into_owned()
                            children=move |item| {
                                let label = format!("{} · {}", sidebar_session_title(&item), short_id(&item.session_id));
                                let option_session = item.session_dir.to_string_lossy().into_owned();
                                view! {
                                    <option value=item.session_dir.to_string_lossy().into_owned()
                                        prop:selected=move || selected.get().as_deref() == Some(option_session.as_str())>
                                        {label}
                                    </option>
                                }
                            } />
                    </select>
                    <button type="button" class="secondary analysis-open-chat"
                        disabled=move || summary.get().is_none()
                        on:click=move |_| { if let Some(item) = summary.get_untracked() { on_open(item); } }>
                        "Открыть диалог"
                    </button>
                </div>
            </div>
            <nav class="analysis-tabs" aria-label="Раздел анализа">
                <button type="button" aria-pressed=move || (!context_tab.get()).to_string()
                    class:active=move || !context_tab.get() on:click=move |_| set_context_tab.set(false)>
                    "Запросы и расход"
                </button>
                <button type="button" aria-pressed=move || context_tab.get().to_string()
                    class:active=move || context_tab.get() on:click=move |_| set_context_tab.set(true)>
                    "Контекст и инструменты"
                </button>
            </nav>
            <div class="analysis-scroll">
                <div hidden=move || context_tab.get()>
                    <UsageDetailsView session_dir=selected />
                </div>
                <Show when=move || context_tab.get()>
                    <ContextMapView session_dir=selected />
                </Show>
            </div>
        </section>
    }
}
