mod commands;
mod connection;
mod effects;
mod extension_state;
pub(crate) mod menus;
mod navigation;
mod shell;
mod state;

use crate::{components::ToolCardsCollapsed, session::settings::load_web_settings};
use leptos::prelude::*;

#[component]
pub(crate) fn App() -> impl IntoView {
    let state = state::AppState::new();
    let router = navigation::AppRouter::new(state.session.active_session_dir);
    provide_context(ToolCardsCollapsed(state.view.tool_cards_collapsed));
    load_web_settings(
        state.session.active_session_dir,
        state.chat.transcript_generation,
        state.view.set_tool_cards_collapsed,
    );
    effects::install(state, router);
    let connection = connection::connect(state);
    let commands = commands::commands(state, connection);
    view! { <shell::AppShell state connection commands router /> }
}
