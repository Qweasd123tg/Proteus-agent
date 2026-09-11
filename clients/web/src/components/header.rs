use crate::{types::*, ui_utils::short_path};
use leptos::prelude::*;
use web_sys::{MouseEvent, window};
/// Закрывает меню дополнительных действий в топбаре (нативный <details>).
fn close_topbar_menu() {
    if let Some(document) = window().and_then(|window| window.document())
        && let Ok(Some(menu)) = document.query_selector(".topbar-menu[open]")
    {
        let _ = menu.remove_attribute("open");
    }
}

#[component]
#[allow(clippy::too_many_arguments)]
pub(crate) fn HeaderView<N, R, O, C, T>(
    route: ReadSignal<String>,
    workspace_label: ReadSignal<String>,
    transport_status: ReadSignal<TransportStatus>,
    waiting_background_sessions: Memo<Vec<SessionSummary>>,
    active_session_dir: ReadSignal<Option<String>>,
    active_run_id: ReadSignal<Option<String>>,
    event_count: ReadSignal<u64>,
    tool_activities: ReadSignal<Vec<ToolActivity>>,
    info_panel_open: ReadSignal<bool>,
    on_navigate: N,
    on_reconnect: R,
    on_open_session: O,
    on_cancel: C,
    on_toggle_info: T,
) -> impl IntoView
where
    N: Fn(MouseEvent, &'static str) + Copy + Send + Sync + 'static,
    R: Fn(MouseEvent) + Copy + Send + Sync + 'static,
    O: Fn(SessionSummary) + Copy + Send + Sync + 'static,
    C: Fn(MouseEvent) + Copy + Send + Sync + 'static,
    T: Fn(MouseEvent) + Copy + Send + Sync + 'static,
{
    let is_chat_route =
        move || !matches!(route.get().as_str(), "/resume" | "/context" | "/settings");
    let transport_badge_class = move || match transport_status.get() {
        TransportStatus::Connecting | TransportStatus::Reconnecting => "status-badge disconnected",
        TransportStatus::Connected => "status-badge completed",
        TransportStatus::Error(_) | TransportStatus::Shutdown => "status-badge failed",
    };
    view! {
                <header class="topbar">
                    <div class="topbar-left">
                        <a
                            class="brand"
                            title=move || workspace_label.get()
                            href="/"
                            on:click=move |ev| on_navigate(ev, "/")
                        >
                            {move || short_path(&workspace_label.get())}
                        </a>
                        <button
                            type="button"
                            class=move || format!("connection-badge {}", transport_badge_class())
                            title=move || format!("{} · нажмите для переподключения", transport_status.get().label()) aria-label=move || format!("Соединение: {}", transport_status.get().label())
                            on:click=on_reconnect
                        >
                            <span class="dot"></span>
                            <span class="connection-text">{move || transport_status.get().label()}</span>
                        </button>
                    </div>
                    <nav class="topnav">
                        {move || {
                            let waiting = waiting_background_sessions.get();
                            if waiting.is_empty() {
                                ().into_any()
                            } else {
                                let count = waiting.len();
                                let first = waiting[0].clone();
                                view! {
                                    <button
                                        type="button"
                                        class="status-badge attention"
                                        title="Другие сессии ждут доступа или ответа — открыть"
                                        on:click=move |_| on_open_session(first.clone())
                                    >
                                        <span class="dot"></span>
                                        {format!("ждёт: {count}")}
                                    </button>
                                }.into_any()
                            }
                        }}
                        <a
                            class="topnav-link"
                            class:active=move || is_chat_route()
                            href="/"
                            on:click=move |ev| on_navigate(ev, "/")
                        >
                            "Чат"
                        </a>
                        <a
                            class="topnav-link"
                            class:active=move || route.get() == "/context"
                            href="/context"
                            on:click=move |ev| on_navigate(ev, "/context")
                        >
                            "Анализ"
                        </a>
                        <a
                            class="topnav-link"
                            class:active=move || route.get() == "/resume"
                            href="/resume"
                            on:click=move |ev| on_navigate(ev, "/resume")
                        >
                            "Сессии"
                        </a>
                        // Резервный тумблер инфо-панели для узких экранов:
                        // там свёрнутая рейка спрятана целиком, и своей кнопки
                        // у панели не видно. На десктопе скрыт (см. CSS).
                        {move || if is_chat_route() {
                            view! {
                                <button
                                    type="button"
                                    class="sidebar-toggle info-panel-mobile-toggle"
                                    class:active=move || info_panel_open.get()
                                    title="Инфо по чату"
                                    aria-label="Инфо по чату"
                                    on:click=on_toggle_info
                                >
                                    <super::icons::PanelIcon right=true />
                                </button>
                            }.into_any()
                        } else {
                            ().into_any()
                        }}
                        // Настройки доступны напрямую; редкие действия и счётчики — в меню.
                        <a class="topnav-link settings-link" class:active=move || route.get() == "/settings" href="/settings" title="Настройки" aria-label="Настройки" on:click=move |ev| on_navigate(ev, "/settings")><super::icons::SettingsIcon /></a>
                        <details class="topbar-menu">
                            <summary title="Другие действия" aria-label="Другие действия">"···"</summary>
                            <div class="topbar-menu-panel">
                                <a
                                    class="topbar-menu-item"
                                    href=move || crate::api::inspector_link_url(
                                        active_session_dir.get().as_deref()
                                    )
                                    on:click=move |_| close_topbar_menu()
                                >
                                    "Inspector"
                                </a>
                                <button
                                    type="button"
                                    class="topbar-menu-item danger"
                                    disabled=move || active_run_id.get().is_none()
                                    on:click=move |ev| {
                                        close_topbar_menu();
                                        on_cancel(ev);
                                    }
                                >
                                    "Остановить ход"
                                </button>
                                <div class="topbar-menu-footer">
                                    {move || format!(
                                        "{} events · {} tools",
                                        event_count.get(),
                                        tool_activities.with(|items| items.len()),
                                    )}
                                </div>
                            </div>
                        </details>
                    </nav>
                </header>
    }
}
