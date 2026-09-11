use crate::{types::*, ui_utils::short_path};
use leptos::prelude::*;
use web_sys::MouseEvent;
#[component]
#[allow(clippy::too_many_arguments)]
pub(crate) fn HeaderView<N, O, T>(
    route: ReadSignal<String>,
    workspace_label: ReadSignal<String>,
    waiting_background_sessions: Memo<Vec<SessionSummary>>,
    info_panel_open: ReadSignal<bool>,
    on_navigate: N,
    on_open_session: O,
    on_toggle_info: T,
) -> impl IntoView
where
    N: Fn(MouseEvent, &'static str) + Copy + Send + Sync + 'static,
    O: Fn(SessionSummary) + Copy + Send + Sync + 'static,
    T: Fn(MouseEvent) + Copy + Send + Sync + 'static,
{
    let is_chat_route =
        move || !matches!(route.get().as_str(), "/resume" | "/context" | "/settings");
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
                    </nav>
                </header>
    }
}
