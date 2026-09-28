use crate::{components::icons::*, types::*};
use leptos::prelude::*;
use web_sys::MouseEvent;

#[component]
pub(crate) fn SidebarFooter<N, R>(
    route: ReadSignal<String>,
    transport_status: ReadSignal<TransportStatus>,
    on_navigate: N,
    on_reconnect: R,
) -> impl IntoView
where
    N: Fn(MouseEvent, &'static str) + Copy + Send + Sync + 'static,
    R: Fn(MouseEvent) + Copy + 'static,
{
    let connection_class = move || match transport_status.get() {
        TransportStatus::Connected => "completed",
        TransportStatus::Connecting | TransportStatus::Reconnecting => "disconnected",
        TransportStatus::Error(_) | TransportStatus::Shutdown => "failed",
    };
    view! {
        <nav class="sidebar-footer" aria-label="Инструменты и настройки">
            <a href="/settings" class="settings-link" class:active=move || route.get() == "/settings" title="Настройки" aria-label="Настройки" on:click=move |ev| on_navigate(ev, "/settings")>
                <SettingsIcon/><span class="sidebar-footer-label">"Настройки"</span>
            </a>
            <div class="sidebar-footer-status">
                <button type="button" class=move || format!("connection-badge status-badge {}", connection_class())
                    title=move || format!("{} · нажмите для переподключения", transport_status.get().label())
                    aria-label=move || format!("Соединение: {}", transport_status.get().label()) on:click=on_reconnect>
                    <span class="dot"></span>
                </button>
            </div>
        </nav>
    }
}
