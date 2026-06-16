use leptos::prelude::*;
use web_sys::window;

use crate::{api::load_session_token, architecture::ArchitectureView, configs::ConfigsView};

#[component]
pub(crate) fn App() -> impl IntoView {
    let route = current_path();
    let is_configs_route = route == "/configs";
    let (status, set_status) = signal("local app-server".to_owned());

    if let Err(error) = load_session_token() {
        set_status.set(format!("session token storage failed: {error}"));
    }

    view! {
        <div class="app-layout">
            <main class="workspace-main">
                <header class="topbar">
                    <div class="topbar-left">
                        <a class="brand" href="/architecture">"Proteus Inspector"</a>
                        <span class="status-badge idle">
                            <span class="dot"></span>
                            {move || status.get()}
                        </span>
                    </div>
                    <nav class="topnav">
                        <a class="topnav-link" href="/architecture">"Architecture"</a>
                        <a class="topnav-link" href="/configs">"Configs"</a>
                        <a class="topnav-link" href="http://127.0.0.1:1420/">"Чат"</a>
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
