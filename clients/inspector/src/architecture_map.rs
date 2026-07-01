use leptos::prelude::*;
use wasm_bindgen::{JsCast, closure::Closure, prelude::wasm_bindgen};
use web_sys::{MouseEvent, WheelEvent};

use crate::ui_utils::set_timeout;

pub(crate) const TOPOLOGY_MAP_ELEMENT_ID: &str = "topology-map-canvas";
pub(crate) const TOPOLOGY_MAP_VIEWPORT_ID: &str = "topology-map-viewport";
const MAP_SCALE_MIN: f64 = 0.2;
const MAP_SCALE_MAX: f64 = 4.0;

#[wasm_bindgen]
unsafe extern "C" {
    #[wasm_bindgen(js_namespace = window, js_name = proteusRenderMermaid, catch)]
    fn proteus_render_mermaid(element_id: &str, code: &str) -> Result<(), wasm_bindgen::JsValue>;
}

/// Пан/зум состояния карты: translate+scale на stage-обёртке вокруг SVG.
#[derive(Clone, Copy)]
pub(crate) struct MapViewState {
    pub(crate) scale: ReadSignal<f64>,
    pub(crate) set_scale: WriteSignal<f64>,
    pub(crate) offset: ReadSignal<(f64, f64)>,
    pub(crate) set_offset: WriteSignal<(f64, f64)>,
    pub(crate) dragging: ReadSignal<bool>,
    pub(crate) set_dragging: WriteSignal<bool>,
    // (pointer_x, pointer_y, offset_x, offset_y) на момент mousedown
    pub(crate) drag_start: ReadSignal<(f64, f64, f64, f64)>,
    pub(crate) set_drag_start: WriteSignal<(f64, f64, f64, f64)>,
}

impl MapViewState {
    pub(crate) fn new() -> Self {
        let (scale, set_scale) = signal(1.0_f64);
        let (offset, set_offset) = signal((16.0_f64, 16.0_f64));
        let (dragging, set_dragging) = signal(false);
        let (drag_start, set_drag_start) = signal((0.0, 0.0, 0.0, 0.0));
        Self {
            scale,
            set_scale,
            offset,
            set_offset,
            dragging,
            set_dragging,
            drag_start,
            set_drag_start,
        }
    }

    /// Зум с фиксированной точкой `anchor` (координаты внутри viewport).
    pub(crate) fn zoom(self, factor: f64, anchor: (f64, f64)) {
        let old_scale = self.scale.get_untracked();
        let new_scale = (old_scale * factor).clamp(MAP_SCALE_MIN, MAP_SCALE_MAX);
        if (new_scale - old_scale).abs() < 1e-9 {
            return;
        }
        let (anchor_x, anchor_y) = anchor;
        let (offset_x, offset_y) = self.offset.get_untracked();
        let ratio = new_scale / old_scale;
        self.set_offset.set((
            anchor_x - (anchor_x - offset_x) * ratio,
            anchor_y - (anchor_y - offset_y) * ratio,
        ));
        self.set_scale.set(new_scale);
    }

    pub(crate) fn zoom_at_center(self, factor: f64) {
        let anchor = map_viewport_size()
            .map(|(width, height)| (width / 2.0, height / 2.0))
            .unwrap_or((0.0, 0.0));
        self.zoom(factor, anchor);
    }

    /// Вписать SVG в viewport и отцентрировать.
    pub(crate) fn fit(self) {
        let Some((viewport_w, viewport_h)) = map_viewport_size() else {
            return;
        };
        let Some(svg) = map_svg_element() else {
            return;
        };
        let current_scale = self.scale.get_untracked();
        if current_scale <= 0.0 {
            return;
        }
        let svg_rect = svg.get_bounding_client_rect();
        let natural_w = svg_rect.width() / current_scale;
        let natural_h = svg_rect.height() / current_scale;
        if natural_w <= 0.0 || natural_h <= 0.0 {
            return;
        }
        let margin = 24.0;
        let scale = (((viewport_w - margin) / natural_w).min((viewport_h - margin) / natural_h))
            .clamp(MAP_SCALE_MIN, 1.5);
        self.set_scale.set(scale);
        self.set_offset.set((
            (viewport_w - natural_w * scale) / 2.0,
            (viewport_h - natural_h * scale) / 2.0,
        ));
    }

    pub(crate) fn reset(self) {
        self.set_scale.set(1.0);
        self.set_offset.set((16.0, 16.0));
    }
}

pub(crate) fn render_mermaid_map(code: &str) -> Result<(), wasm_bindgen::JsValue> {
    proteus_render_mermaid(TOPOLOGY_MAP_ELEMENT_ID, code)
}

pub(crate) fn install_mermaid_rendered_fit(map_view: MapViewState) {
    let on_mermaid_rendered = Closure::<dyn FnMut(web_sys::Event)>::wrap(Box::new(move |_| {
        // SVG только что вставлен; даём браузеру кадр на layout.
        set_timeout(50, move || map_view.fit());
    }));
    if let Some(window) = web_sys::window() {
        let _ = window.add_event_listener_with_callback(
            "proteus-mermaid-rendered",
            on_mermaid_rendered.as_ref().unchecked_ref(),
        );
    }
    on_mermaid_rendered.forget();
}

#[component]
pub(crate) fn TopologyMapView(map: MapViewState) -> impl IntoView {
    view! {
        <section class="config-section">
            <div class="config-section-header">
                <h3>"Map"</h3>
                <div class="mermaid-summary-actions">
                    <span class="map-zoom-label" title="Колесо — зум, мышью — перемещение, двойной клик — вписать">
                        {move || format!("{:.0}%", map.scale.get() * 100.0)}
                    </span>
                    <button
                        type="button"
                        class="secondary mermaid-copy-button"
                        title="Уменьшить"
                        on:click=move |_| map.zoom_at_center(1.0 / 1.25)
                    >
                        "−"
                    </button>
                    <button
                        type="button"
                        class="secondary mermaid-copy-button"
                        title="Увеличить"
                        on:click=move |_| map.zoom_at_center(1.25)
                    >
                        "+"
                    </button>
                    <button
                        type="button"
                        class="secondary mermaid-copy-button"
                        title="Вписать карту в окно"
                        on:click=move |_| map.fit()
                    >
                        "fit"
                    </button>
                    <button
                        type="button"
                        class="secondary mermaid-copy-button"
                        title="Масштаб 100%"
                        on:click=move |_| map.reset()
                    >
                        "1:1"
                    </button>
                </div>
            </div>
            <div
                id=TOPOLOGY_MAP_VIEWPORT_ID
                class="mermaid-map-viewport"
                class:dragging=move || map.dragging.get()
                on:wheel=move |ev: WheelEvent| {
                    ev.prevent_default();
                    let factor = if ev.delta_y() < 0.0 { 1.15 } else { 1.0 / 1.15 };
                    map.zoom(factor, map_event_anchor(&ev));
                }
                on:mousedown=move |ev: MouseEvent| {
                    if ev.button() != 0 {
                        return;
                    }
                    ev.prevent_default();
                    let (offset_x, offset_y) = map.offset.get();
                    map.set_drag_start.set((
                        f64::from(ev.client_x()),
                        f64::from(ev.client_y()),
                        offset_x,
                        offset_y,
                    ));
                    map.set_dragging.set(true);
                }
                on:mousemove=move |ev: MouseEvent| {
                    if !map.dragging.get() {
                        return;
                    }
                    let (start_x, start_y, offset_x, offset_y) = map.drag_start.get();
                    map.set_offset.set((
                        offset_x + f64::from(ev.client_x()) - start_x,
                        offset_y + f64::from(ev.client_y()) - start_y,
                    ));
                }
                on:mouseup=move |_| map.set_dragging.set(false)
                on:mouseleave=move |_| map.set_dragging.set(false)
                on:dblclick=move |_| map.fit()
            >
                <div
                    class="mermaid-map-stage"
                    style=move || {
                        let (offset_x, offset_y) = map.offset.get();
                        format!(
                            "transform: translate({offset_x}px, {offset_y}px) scale({});",
                            map.scale.get()
                        )
                    }
                >
                    <div id=TOPOLOGY_MAP_ELEMENT_ID class="mermaid-map">
                        "Карта рендерится через mermaid.js (нужен CDN); без сети используйте кнопку Mermaid для копирования источника."
                    </div>
                </div>
            </div>
        </section>
    }
}

fn map_viewport_size() -> Option<(f64, f64)> {
    let viewport = web_sys::window()?
        .document()?
        .get_element_by_id(TOPOLOGY_MAP_VIEWPORT_ID)?;
    Some((
        f64::from(viewport.client_width()),
        f64::from(viewport.client_height()),
    ))
}

fn map_svg_element() -> Option<web_sys::Element> {
    web_sys::window()?
        .document()?
        .get_element_by_id(TOPOLOGY_MAP_ELEMENT_ID)?
        .query_selector("svg")
        .ok()
        .flatten()
}

pub(crate) fn map_event_anchor(ev: &WheelEvent) -> (f64, f64) {
    let Some(viewport) = ev
        .current_target()
        .and_then(|target| target.dyn_into::<web_sys::Element>().ok())
    else {
        return (0.0, 0.0);
    };
    let rect = viewport.get_bounding_client_rect();
    (
        f64::from(ev.client_x()) - rect.left(),
        f64::from(ev.client_y()) - rect.top(),
    )
}
