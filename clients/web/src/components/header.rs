use crate::types::*;
use leptos::prelude::*;
use web_sys::MouseEvent;
#[component]
#[allow(clippy::too_many_arguments)]
pub(crate) fn HeaderView<N, O, T>(
    route: ReadSignal<String>,
    workspace_label: ReadSignal<String>,
    session_title: Memo<String>,
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
                            <super::icons::FolderIcon />
                            <span>{move || match route.get().as_str() {
                                "/settings" => "Настройки".to_owned(),
                                "/resume" => "История сессий".to_owned(),
                                "/context" => "Анализ сессии".to_owned(),
                                _ => session_title.get(),
                            }}</span>
                        </a>
                    </div>
                    <nav class="topnav" aria-label="Действия чата">
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
                            class="topnav-link return-to-chat"
                            class:active=move || is_chat_route()
                            hidden=move || is_chat_route()
                            href="/"
                            on:click=move |ev| on_navigate(ev, "/")
                        >
                            "Чат"
                        </a>
                        // На узком экране рейка справа скрыта: переключатель остаётся в шапке.
                        {move || if is_chat_route() {
                            view! {
                                <button
                                    type="button"
                                    class="sidebar-toggle info-panel-mobile-toggle"
                                    class:active=move || info_panel_open.get()
                                    title="Инфо по чату"
                                    aria-label="Инфо по чату"
                                    aria-expanded=move || info_panel_open.get().to_string()
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
