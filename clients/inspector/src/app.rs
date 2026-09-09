use leptos::prelude::*;
use web_sys::window;

use crate::{
    api::{app_server_origin, chat_link_url, has_session_token, load_session_token},
    architecture::ArchitectureView,
    configs::ConfigsView,
};

#[component]
pub(crate) fn App() -> impl IntoView {
    let is_architecture = window()
        .and_then(|window| window.location().pathname().ok())
        .is_some_and(|path| path == "/architecture");
    let token_error = load_session_token().err();
    let origin = app_server_origin();
    let endpoint = origin
        .trim_start_matches("http://")
        .trim_start_matches("https://")
        .to_owned();
    let access_label = match token_error {
        Some(_) => "Хранилище сессии недоступно",
        None if has_session_token() => "Токен сессии настроен",
        None => "Локальный сервер",
    };

    view! {
        <div class="inspector-shell">
            <a class="skip-link" href="#inspector-content">"Перейти к содержимому"</a>
            <aside class="inspector-sidebar">
                <a class="inspector-brand" href="/configs" aria-label="Proteus — сборка агента">
                    <span class="brand-mark" aria-hidden="true"><i></i><i></i><i></i></span>
                    <span><strong>"proteus"</strong><small>"INSPECTOR"</small></span>
                </a>
                <div class="sidebar-section-label">"РАБОЧЕЕ ПРОСТРАНСТВО"</div>
                <nav class="inspector-nav" aria-label="Разделы Inspector">
                    <a class="inspector-nav-item" class:active=!is_architecture
                        aria-current=if !is_architecture { Some("page") } else { None }
                        href="/configs">
                        <NavIcon kind="assembly"/>
                        <span>"Сборка агента"</span>
                        <span class="nav-indicator" aria-hidden="true"></span>
                    </a>
                    <a class="inspector-nav-item" class:active=is_architecture
                        aria-current=if is_architecture { Some("page") } else { None }
                        href="/architecture">
                        <NavIcon kind="architecture"/>
                        <span>"Архитектура"</span>
                        <span class="nav-indicator" aria-hidden="true"></span>
                    </a>
                </nav>
                <div class="sidebar-note">
                    <span class="sidebar-note-glyph" aria-hidden="true">"◈"</span>
                    <strong>"Агент из ваших модулей"</strong>
                    <p>"Выберите поведение. Настройте инструменты. Посмотрите, как всё связано."</p>
                </div>
                <div class="sidebar-connection">
                    <span class="connection-label"><span class="connection-dot"></span>{access_label}</span>
                    <code title=origin>{endpoint}</code>
                </div>
            </aside>
            <main class="inspector-main">
                <header class="inspector-topbar">
                    <div class="inspector-breadcrumb"><span>"Рабочее пространство"</span><span aria-hidden="true">"/"</span><strong>"Inspector"</strong></div>
                    <a class="inspector-chat-link" href=chat_link_url()>"Открыть чат"<span aria-hidden="true">"↗"</span></a>
                </header>
                <div class="inspector-content" id="inspector-content" tabindex="-1">
                    {if is_architecture {
                        view! { <ArchitectureView/> }.into_any()
                    } else {
                        view! { <ConfigsView/> }.into_any()
                    }}
                </div>
            </main>
        </div>
    }
}

#[component]
fn NavIcon(kind: &'static str) -> impl IntoView {
    let path = if kind == "assembly" {
        "M4 4h6v6H4z M14 4h6v6h-6z M4 14h6v6H4z M17 14v6 M14 17h6"
    } else {
        "M9 3h6v5H9z M3 16h6v5H3z M15 16h6v5h-6z M12 8v4 M6 16v-4h12v4"
    };
    view! {
        <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d=path/></svg>
    }
}
