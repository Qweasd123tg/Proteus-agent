use super::super::icons::{AppLogo, PlusIcon, RefreshIcon};
use leptos::prelude::*;
use web_sys::MouseEvent;

#[component]
pub(super) fn SidebarHeader<R, N>(on_refresh: R, on_new_session: N) -> impl IntoView
where
    R: Fn(MouseEvent) + Copy + 'static,
    N: Fn(MouseEvent) + Copy + 'static,
{
    view! {
        <div class="sidebar-header">
            <div class="sidebar-brand-row">
                <button type="button" class="sidebar-brand" data-app-menu="" aria-label="Меню Proteus" aria-haspopup="menu">
                    <AppLogo/>
                    <span class="sidebar-brand-label">"Proteus"</span>
                    <span class="sidebar-brand-chevron"><super::super::icons::ChevronDownIcon/></span>
                </button>
            </div>
            <div class="sidebar-header-actions">
                <button type="button" title="Обновить сессии" aria-label="Обновить сессии" on:click=on_refresh><RefreshIcon /></button>
                <button type="button" data-shortcut="new-chat" title="Новая сессия" aria-label="Новая сессия" on:click=on_new_session><PlusIcon /><span class="new-session-label">"Новый чат"</span></button>
            </div>
        </div>
    }
}
