use leptos::prelude::*;
use web_sys::MouseEvent;

#[component]
pub(crate) fn InfoPanelView<T, R>(
    open: ReadSignal<bool>,
    width: ReadSignal<i32>,
    on_toggle: T,
    on_begin_resize: R,
) -> impl IntoView
where
    T: Fn(MouseEvent) + Copy + Send + Sync + 'static,
    R: Fn(MouseEvent) + Copy + Send + Sync + 'static,
{
    view! {
        <button type="button" class="panel-backdrop" class:open=open inert=move || (!open.get()).then_some("") aria-label="Закрыть обзор" on:click=on_toggle></button>
        <aside class="info-panel" class:open=open style=move || format!("--info-panel-width: {}px", width.get()) aria-label="Панели справа">
            <div class="info-panel-resize-handle" aria-hidden="true" on:mousedown=on_begin_resize></div>
            <div class="info-panel-surface" inert=move || (!open.get()).then_some("")>
                <div class="info-panel-header"><h2>"Обзор"</h2><super::panel::PanelToggle expanded=open.into() right=true on_toggle /></div>
            </div>
            <div class="info-panel-rail-surface" inert=move || open.get().then_some("")>
                <div class="info-panel-header"><super::panel::PanelToggle expanded=open.into() right=true on_toggle /></div>
            </div>
            <div class="extension-host extension-dock-right" data-extension-location="right"></div>
        </aside>
    }
}
