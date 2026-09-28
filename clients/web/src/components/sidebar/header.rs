use super::super::{
    icons::{EditIcon, RefreshIcon},
    panel::PanelToggle,
};
use leptos::prelude::*;
use web_sys::MouseEvent;

#[component]
pub(super) fn SidebarHeader<T, R, N>(
    collapsed: ReadSignal<bool>,
    on_toggle: T,
    on_refresh: R,
    on_new_session: N,
) -> impl IntoView
where
    T: Fn(MouseEvent) + Copy + 'static,
    R: Fn(MouseEvent) + Copy + 'static,
    N: Fn(MouseEvent) + Copy + 'static,
{
    view! {
        <div class="sidebar-header">
            <div class="sidebar-brand-row">
                <span class="sidebar-brand">"Proteus"</span>
                <PanelToggle expanded=Signal::derive(move || !collapsed.get()) on_toggle />
            </div>
            <div class="sidebar-header-actions">
                <button type="button" title="Обновить сессии" aria-label="Обновить сессии" on:click=on_refresh><RefreshIcon /></button>
                <button type="button" title="Новая сессия" aria-label="Новая сессия" on:click=on_new_session><EditIcon /><span class="new-session-label">"Новый чат"</span></button>
            </div>
        </div>
    }
}
