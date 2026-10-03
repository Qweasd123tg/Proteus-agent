mod client_modules;
mod commands;
mod connection;
mod effects;
mod extension_state;
pub(crate) mod menus;
mod navigation;
mod notifications;
mod shell;
mod state;

use crate::components::ToolCardsCollapsed;
use leptos::prelude::*;

#[component]
pub(crate) fn App() -> impl IntoView {
    provide_context(crate::interface_settings::InterfaceSettings::new());
    let state = state::AppState::new();
    let transcript_views = crate::components::transcript_state::TranscriptViewState::new();
    provide_context(transcript_views);
    Effect::new(move |_| {
        state.session.active_session_dir.track();
        transcript_views.clear();
    });
    let router = navigation::AppRouter::new(state.session.active_session_dir);
    provide_context(ToolCardsCollapsed(state.view.tool_cards_collapsed));
    effects::install(state, router);
    let connection = connection::connect(state);
    client_modules::install(state, connection, router);
    let commands = commands::commands(state, connection, router);
    view! { <shell::AppShell state connection commands router /> }
}
