use leptos::{html, prelude::*};
use wasm_bindgen::{JsCast, closure::Closure, prelude::wasm_bindgen};
use web_sys::HtmlElement;
pub(crate) const CHAT_REATTACH_THRESHOLD_PX: i32 = 4;
#[wasm_bindgen]
unsafe extern "C" {
    #[wasm_bindgen(js_namespace = window, js_name = requestAnimationFrame)]
    fn request_animation_frame(callback: &js_sys::Function) -> i32;
}

pub(crate) fn is_at_bottom(results: &HtmlElement) -> bool {
    let distance = results.scroll_height() - results.scroll_top() - results.client_height();
    distance <= CHAT_REATTACH_THRESHOLD_PX
}

pub(crate) fn schedule_results_scroll(
    results_ref: NodeRef<html::Section>,
    stick_to_bottom: ReadSignal<bool>,
    scroll_frame_pending: ReadSignal<bool>,
    set_scroll_frame_pending: WriteSignal<bool>,
    set_last_results_scroll_top: WriteSignal<i32>,
) {
    // Untracked: это управляющий флаг «кадр уже запланирован», а не
    // зависимость. Tracked-чтение подписывало вызывающий эффект автоскролла
    // на сам флаг: set(true) → rerun, set(false) в rAF → rerun → новый кадр →
    // set(true) → ... — вечный 60fps-цикл с принудительным reflow всей ленты
    // (scroll_height) на каждом кадре, который и вешал вкладку на длинных
    // транскриптах во время стрима.
    if scroll_frame_pending.get_untracked() {
        return;
    }
    set_scroll_frame_pending.set(true);

    let callback = Closure::once_into_js(move || {
        scroll_results_to_bottom(results_ref, stick_to_bottom, set_last_results_scroll_top);
        set_scroll_frame_pending.set(false);
    });
    request_animation_frame(callback.unchecked_ref());
}

fn scroll_results_to_bottom(
    results_ref: NodeRef<html::Section>,
    stick_to_bottom: ReadSignal<bool>,
    set_last_results_scroll_top: WriteSignal<i32>,
) {
    // rAF-колбэк — не реактивный контекст: tracked-чтения здесь бессмысленны
    // и в dev-сборке заваливают консоль предупреждениями reactive_graph.
    if let Some(results) = results_ref.get_untracked()
        && stick_to_bottom.get_untracked()
    {
        results.set_scroll_top(results.scroll_height());
        set_last_results_scroll_top.set(results.scroll_top());
    }
}
