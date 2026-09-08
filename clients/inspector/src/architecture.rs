//! Вкладка Architecture: рендер `TopologySnapshot` из `/inspect/topology`.
//!
//! Принципы: каждый факт показывается один раз; группировка и порядок slots
//! приходят с сервера (`slot.category`/`slot.order`); никакой абсолютной
//! графики — pipeline рисуется потоком карточек, а source process modules
//! берётся из server-owned topology snapshot.

use leptos::{prelude::*, task::spawn_local};

use crate::api::{get_json, get_text};
use crate::architecture_map::{MapViewState, install_mermaid_rendered_fit, render_mermaid_map};
use crate::types::*;
use crate::ui_utils::copy_to_clipboard;

mod snapshot;
use snapshot::TopologySnapshotView;

#[component]
pub(crate) fn ArchitectureView() -> impl IntoView {
    let (snapshot, set_snapshot) = signal(None::<TopologySnapshot>);
    let (mermaid, set_mermaid) = signal(String::new());
    let (status, set_status) = signal("Загружаем активную сборку…".to_owned());

    load_topology_snapshot(set_snapshot, set_mermaid, set_status);

    let refresh = move |_| load_topology_snapshot(set_snapshot, set_mermaid, set_status);
    let copy_mermaid = move |_| {
        let text = mermaid.get();
        if text.trim().is_empty() {
            set_status.set("Mermaid недоступен".to_owned());
        } else {
            copy_to_clipboard(text);
            set_status.set("Mermaid скопирован".to_owned());
        }
    };

    let map_view = MapViewState::new();

    // Карта рендерится после того, как и snapshot (DOM-секция), и mermaid
    // source загружены; повторная загрузка перерисовывает карту. Рендер в
    // mermaid.js асинхронный (включая загрузку ESM-модуля), поэтому auto-fit
    // вызывается по событию proteus-mermaid-rendered из index.html.
    Effect::new(move |_| {
        let code = mermaid.get();
        if code.trim().is_empty() || snapshot.with(Option::is_none) {
            return;
        }
        let _ = render_mermaid_map(&code);
    });

    install_mermaid_rendered_fit(map_view);

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
                    .map(|snapshot| view! { <TopologySnapshotView snapshot mermaid=mermaid.get() map=map_view /> }.into_any())
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
    set_mermaid: WriteSignal<String>,
    set_status: WriteSignal<String>,
) {
    spawn_local(async move {
        match get_json::<TopologySnapshot>("/inspect/topology").await {
            Ok(snapshot) => {
                let slot_count = snapshot.slots.len();
                let tool_count = snapshot.tools.iter().filter(|tool| tool.registered).count();
                let process_count = snapshot
                    .modules
                    .iter()
                    .filter(|module| module.source.kind == "process")
                    .count();
                let warning_count = snapshot.warnings.len();
                set_snapshot.set(Some(snapshot));
                set_status.set(format!(
                    "{slot_count} слотов · {tool_count} инструментов · {process_count} модулей · предупреждений: {warning_count}"
                ));
                match get_text("/inspect/topology.mmd").await {
                    Ok(mermaid) => set_mermaid.set(mermaid),
                    Err(error) => {
                        set_mermaid.set(String::new());
                        set_status.set(format!("Mermaid недоступен: {error}"));
                    }
                }
            }
            Err(error) => {
                set_snapshot.set(None);
                set_mermaid.set(String::new());
                set_status.set(format!("Не удалось подключиться к серверу: {error}"));
            }
        }
    });
}
