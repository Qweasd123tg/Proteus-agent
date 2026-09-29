use crate::types::*;
use leptos::prelude::*;
use web_sys::MouseEvent;
#[component]
#[allow(clippy::too_many_arguments)]
pub(crate) fn HeaderView<N, O, S>(
    sidebar_collapsed: ReadSignal<bool>,
    on_toggle_sidebar: S,
    workspace_label: ReadSignal<String>,
    waiting_background_sessions: Memo<Vec<SessionSummary>>,
    on_navigate: N,
    on_open_session: O,
) -> impl IntoView
where
    S: Fn(MouseEvent) + Copy + Send + Sync + 'static,
    N: Fn(MouseEvent, &'static str) + Copy + Send + Sync + 'static,
    O: Fn(SessionSummary) + Copy + Send + Sync + 'static,
{
    view! {
                <header class="topbar">
                    <div class="topbar-left">
                        <super::panel::PanelToggle expanded=Signal::derive(move || !sidebar_collapsed.get()) on_toggle=on_toggle_sidebar />
                        <button class="sidebar-toggle" title="Назад" aria-label="Назад" on:click=move |_| { if let Some(w)=web_sys::window() { if let Ok(h)=w.history() { let _=h.back(); } } }><super::icons::BackIcon /></button>
                        <button class="sidebar-toggle" title="Вперёд" aria-label="Вперёд" on:click=move |_| { if let Some(w)=web_sys::window() { if let Ok(h)=w.history() { let _=h.forward(); } } }><super::icons::ForwardIcon /></button>
                        <a
                            class="brand"
                            title=move || workspace_label.get()
                            href="/"
                            on:click=move |ev| on_navigate(ev, "/")
                        >
                            <super::icons::FolderIcon />
                            <span>{move || crate::ui_utils::short_path(&workspace_label.get())}</span>
                        </a>
                    </div>
                    <nav class="topnav" aria-label="Действия рабочей области">
                        <div class="module-dock" data-module-zone="header"></div>
                        <div class="widget-slot" data-widget-slot="header"></div>
                        <button type="button" class="sidebar-toggle" data-workspace-split="" data-shortcut="workspace" title="Разделить область" aria-label="Разделить область" aria-pressed="false"><super::icons::PanelIcon right=true /></button>
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
                    </nav>
                </header>
    }
}
