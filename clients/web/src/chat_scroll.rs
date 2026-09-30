use leptos::{html, prelude::*};
use web_sys::HtmlElement;
pub(crate) const CHAT_REATTACH_THRESHOLD_PX: i32 = 4;

pub(crate) fn is_at_bottom(results: &HtmlElement) -> bool {
    let distance = results.scroll_height() - results.scroll_top() - results.client_height();
    distance <= CHAT_REATTACH_THRESHOLD_PX
}

pub(crate) fn schedule_results_scroll(
    results_ref: NodeRef<html::Section>,
    stick_to_bottom: ReadSignal<bool>,
) {
    #[cfg(target_arch = "wasm32")]
    if stick_to_bottom.get_untracked()
        && let Some(root) = results_ref.get_untracked()
    {
        use wasm_bindgen::prelude::*;
        #[wasm_bindgen(raw_module = "/ui/transcript-scroll.js")]
        extern "C" {
            #[wasm_bindgen(js_name = requestBottom)]
            fn request_bottom(root: &web_sys::Element);
        }
        request_bottom(root.as_ref());
    }
    #[cfg(not(target_arch = "wasm32"))]
    let _ = (results_ref, stick_to_bottom);
}
