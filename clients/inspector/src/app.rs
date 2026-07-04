use leptos::prelude::*;
use web_sys::window;

use crate::{
    api::{app_server_origin, chat_link_url, has_session_token, load_session_token},
    architecture::ArchitectureView,
    configs::ConfigsView,
};

#[component]
pub(crate) fn App() -> impl IntoView {
    let route = current_path();
    let is_configs_route = route == "/configs";
    let token_error = load_session_token().err();

    // Бейдж показывает фактический app-server origin (из ?server= или
    // sessionStorage) и наличие session token, а не захардкоженный текст.
    let origin = app_server_origin();
    let badge_label = match token_error {
        Some(error) => format!("session storage failed: {error}"),
        None if has_session_token() => format!("{} · token", short_origin(&origin)),
        None => short_origin(&origin),
    };

    view! {
        <div class="app-layout">
            <main class="workspace-main">
                <header class="topbar">
                    <div class="topbar-left">
                        <a class="brand" href="/architecture">"Proteus Inspector"</a>
                        <span class="status-badge idle" title=origin>
                            <span class="dot"></span>
                            {badge_label}
                        </span>
                    </div>
                    <nav class="topnav">
                        <a class="topnav-link" class:active=!is_configs_route href="/architecture">"Architecture"</a>
                        <a class="topnav-link" class:active=is_configs_route href="/configs">"Configs"</a>
                        <a class="topnav-link" href=chat_link_url()>"Чат"</a>
                    </nav>
                </header>
                <section class="session-workspace">
                    {if is_configs_route {
                        view! { <ConfigsView /> }.into_any()
                    } else {
                        view! { <ArchitectureView /> }.into_any()
                    }}
                </section>
            </main>
        </div>
    }
}

fn current_path() -> String {
    window()
        .and_then(|window| window.location().pathname().ok())
        .unwrap_or_else(|| "/architecture".to_owned())
}

/// `http://127.0.0.1:8787` -> `127.0.0.1:8787` — компактный текст бейджа.
fn short_origin(origin: &str) -> String {
    origin
        .trim_start_matches("http://")
        .trim_start_matches("https://")
        .to_owned()
}
