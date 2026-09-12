//! Вкладка Architecture: рендер `TopologySnapshot` из `/inspect/topology`.
//!
//! Карта и каталог используют один server-owned snapshot; Mermaid — отдельный export.

use leptos::{prelude::*, task::spawn_local};

use crate::api::get_text;
use crate::types::*;
use crate::ui_utils::copy_to_clipboard;

mod snapshot;
use snapshot::TopologySnapshotView;

#[component]
pub(crate) fn ArchitectureView() -> impl IntoView {
    let (snapshot, set_snapshot) = signal(None::<TopologySnapshot>);
    let (source, set_source) = signal(String::new());
    let (status, set_status) = signal("Загружаем активную сборку…".to_owned());

    load_topology_snapshot(set_snapshot, set_source, set_status);

    let refresh = move |_| load_topology_snapshot(set_snapshot, set_source, set_status);
    let copy_mermaid = move |_| {
        spawn_local(async move {
            match get_text("/inspect/topology.mmd").await {
                Ok(text) => {
                    copy_to_clipboard(text);
                    set_status.set("Mermaid скопирован".to_owned());
                }
                Err(error) => set_status.set(format!("Не удалось экспортировать: {error}")),
            }
        });
    };

    view! {
        <section class="configs-page architecture-page">
            <div class="resume-toolbar">
                <div>
                    <h2>"Архитектура сборки"</h2>
                    <p>{move || status.get()}</p>
                </div>
                <div class="toolbar-actions">
                    <button type="button" class="secondary" on:click=copy_mermaid>"Mermaid"</button>
                    <button type="button" class="secondary" on:click=refresh>"Обновить"</button>
                </div>
            </div>
            {move || {
                snapshot
                    .get()
                    .map(|snapshot| view! { <TopologySnapshotView snapshot source=source.get_untracked() /> }.into_any())
                    .unwrap_or_else(|| {
                        view! {
                            <div class="empty-state">
                                <div class="empty-state-title">"Нет данных о сборке"</div>
                            </div>
                        }
                        .into_any()
                    })
            }}
        </section>
    }
}

fn load_topology_snapshot(
    set_snapshot: WriteSignal<Option<TopologySnapshot>>,
    set_source: WriteSignal<String>,
    set_status: WriteSignal<String>,
) {
    spawn_local(async move {
        match get_text("/inspect/topology").await.and_then(|text| {
            serde_json::from_str::<TopologySnapshot>(&text)
                .map(|snapshot| (snapshot, text))
                .map_err(|error| error.to_string())
        }) {
            Ok((snapshot, text)) => {
                let slot_count = snapshot.slots.len();
                let tool_count = snapshot.tools.iter().filter(|tool| tool.registered).count();
                let process_count = snapshot
                    .modules
                    .iter()
                    .filter(|module| module.source.kind() == "process")
                    .count();
                let warning_count = snapshot.warnings.len();
                set_source.set(text);
                set_snapshot.set(Some(snapshot));
                set_status.set(format!(
                    "{slot_count} слотов · {tool_count} инструментов · {process_count} модулей · предупреждений: {warning_count}"
                ));
            }
            Err(error) => {
                set_snapshot.set(None);
                set_source.set(String::new());
                set_status.set(format!("Не удалось подключиться к серверу: {error}"));
            }
        }
    });
}
