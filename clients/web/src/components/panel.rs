use super::icons::PanelIcon;
use leptos::prelude::*;
use web_sys::MouseEvent;

/// Один toggle contract для открытой поверхности и компактной рейки обеих панелей.
#[component]
pub(crate) fn PanelToggle<T>(
    expanded: Signal<bool>,
    #[prop(default = false)] right: bool,
    on_toggle: T,
) -> impl IntoView
where
    T: Fn(MouseEvent) + Copy + 'static,
{
    view! {
        <button
            type="button"
            class=if right { "panel-toggle" } else { "panel-toggle sidebar-collapse-toggle" }
            data-panel-toggle=if right { "info" } else { "sidebar" }
            aria-label=if right { "Панель обзора" } else { "Панель сессий" }
            aria-expanded=move || expanded.get().to_string()
            title=move || if expanded.get() { "Свернуть панель" } else { "Развернуть панель" }
            on:click=on_toggle
        ><PanelIcon right /></button>
    }
}
