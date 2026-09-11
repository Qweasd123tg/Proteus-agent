use crate::{components::icons::*, types::*};
use leptos::prelude::*;
use web_sys::{MouseEvent, window};

#[component]
#[allow(clippy::too_many_arguments)]
pub(crate) fn SidebarFooter<N, R, C>(
    route: ReadSignal<String>,
    transport_status: ReadSignal<TransportStatus>,
    active_session_dir: ReadSignal<Option<String>>,
    active_run_id: ReadSignal<Option<String>>,
    event_count: ReadSignal<u64>,
    tool_activities: ReadSignal<Vec<ToolActivity>>,
    on_navigate: N,
    on_reconnect: R,
    on_cancel: C,
) -> impl IntoView
where
    N: Fn(MouseEvent, &'static str) + Copy + Send + Sync + 'static,
    R: Fn(MouseEvent) + Copy + 'static,
    C: Fn(MouseEvent) + Copy + 'static,
{
    let connection_class = move || match transport_status.get() {
        TransportStatus::Connected => "completed",
        TransportStatus::Connecting | TransportStatus::Reconnecting => "disconnected",
        TransportStatus::Error(_) | TransportStatus::Shutdown => "failed",
    };
    view! {
        <nav class="sidebar-footer" aria-label="Инструменты и настройки">
            <a href="/context" class:active=move || route.get() == "/context" title="Анализ" aria-label="Анализ" on:click=move |ev| on_navigate(ev, "/context")>
                <AnalysisIcon/><span class="sidebar-footer-label">"Анализ"</span>
            </a>
            <a href="/resume" class:active=move || route.get() == "/resume" title="Сессии" aria-label="Сессии" on:click=move |ev| on_navigate(ev, "/resume")>
                <HistoryIcon/><span class="sidebar-footer-label">"Сессии"</span>
            </a>
            <a href="/settings" class="settings-link" class:active=move || route.get() == "/settings" title="Настройки" aria-label="Настройки" on:click=move |ev| on_navigate(ev, "/settings")>
                <SettingsIcon/><span class="sidebar-footer-label">"Настройки"</span>
            </a>
            <a href=move || crate::api::inspector_link_url(active_session_dir.get().as_deref()) title="Inspector" aria-label="Inspector">
                <InspectorIcon/><span class="sidebar-footer-label">"Inspector"</span>
            </a>
            <div class="sidebar-footer-status">
                <button type="button" class=move || format!("connection-badge status-badge {}", connection_class())
                    title=move || format!("{} · нажмите для переподключения", transport_status.get().label())
                    aria-label=move || format!("Соединение: {}", transport_status.get().label()) on:click=on_reconnect>
                    <span class="dot"></span><span class="connection-text">{move || transport_status.get().label()}</span>
                </button>
                <details class="utility-menu">
                    <summary title="Диагностика" aria-label="Диагностика">"···"</summary>
                    <div class="utility-menu-panel">
                        <button type="button" class="utility-menu-item danger" disabled=move || active_run_id.get().is_none()
                            on:click=move |ev| {
                                if let Some(document) = window().and_then(|window| window.document())
                                    && let Ok(Some(menu)) = document.query_selector(".utility-menu[open]") {
                                    let _ = menu.remove_attribute("open");
                                }
                                on_cancel(ev);
                            }>"Остановить ход"</button>
                        <div class="utility-menu-footer">{move || format!("События: {} · инструменты: {}", event_count.get(), tool_activities.with(|items| items.len()))}</div>
                    </div>
                </details>
            </div>
        </nav>
    }
}
