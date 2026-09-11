use leptos::{prelude::*, task::spawn_local};
use std::time::Duration;

use super::format_token_count;
use crate::api::{get_json, session_path};
use crate::types::*;
use crate::ui_utils::short_path;

mod cache;
mod map;
mod snapshot;
#[cfg(test)]
mod tests;

/// A snapshot of the selected session. Polling belongs to this mounted view.
#[component]
pub(crate) fn ContextMapView(session_dir: ReadSignal<Option<String>>) -> impl IntoView {
    let snapshot = RwSignal::new(None::<ContextMapSnapshot>);
    let snapshot_session = RwSignal::new(None::<String>);
    let status = RwSignal::new(String::new());
    let pending = RwSignal::new(false);
    let revision = RwSignal::new(0u64);
    let generation = RwSignal::new(0u64);

    Effect::new(move |_| {
        let selected = session_dir.get();
        revision.get();
        generation.update(|value| *value += 1);
        let request_generation = generation.get_untracked();
        // Never show another session's snapshot under the new selection.
        if snapshot_session.get_untracked() != selected {
            snapshot.set(None);
            snapshot_session.set(selected.clone());
        }
        let Some(selected) = selected else {
            status.set("Сессия ещё не выбрана".to_owned());
            pending.set(false);
            return;
        };
        pending.set(true);
        status.set("Загружаю контекст…".to_owned());
        spawn_local(async move {
            let result = get_json::<ContextMapSnapshot>(&session_path("/context", &selected)).await;
            if generation.try_get_untracked() != Some(request_generation) {
                return;
            }
            pending.set(false);
            match result {
                Ok(value) => {
                    snapshot.set(Some(value));
                    status.set("Последний сохранённый снимок контекста".to_owned());
                }
                Err(error) => {
                    snapshot.set(None);
                    status.set(format!("Не удалось загрузить контекст: {error}"));
                }
            }
        });
    });
    if let Ok(timer) = set_interval_with_handle(
        move || {
            if !pending.get_untracked() {
                revision.update(|value| *value += 1);
            }
        },
        Duration::from_secs(7),
    ) {
        on_cleanup(move || timer.clear());
    }

    view! {
        <section class="analysis-context">
            <div class="analysis-section-heading">
                <div>
                    <h2>"Контекст и инструменты"</h2>
                    <p>"Последнее состояние сессии. Выбор хода в отчёте расхода не меняет этот снимок."</p>
                </div>
                <button type="button" class="secondary" disabled=move || pending.get()
                    on:click=move |_| revision.update(|value| *value += 1)>"Обновить"</button>
            </div>
            <p class="analysis-status" role="status">{move || status.get()}</p>
            {move || snapshot.get().map(snapshot::context_snapshot_view)}
        </section>
    }
}
