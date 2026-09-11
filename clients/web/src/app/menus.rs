use wasm_bindgen::JsCast;
use web_sys::{MouseEvent, window};
pub(super) fn close_menus_on_outside_click(ev: MouseEvent) {
    let Some(document) = window().and_then(|window| window.document()) else {
        return;
    };
    let target = ev
        .target()
        .and_then(|target| target.dyn_into::<web_sys::Node>().ok());
    for selector in [
        ".composer-access-menu[open]",
        ".composer-model-menu[open]",
        ".utility-menu[open]",
    ] {
        let Ok(Some(menu)) = document.query_selector(selector) else {
            continue;
        };
        if target
            .as_ref()
            .is_some_and(|target| menu.contains(Some(target)))
        {
            continue;
        }
        let _ = menu.remove_attribute("open");
    }
}

pub(crate) fn dismiss_top_menu() -> bool {
    let Some(document) = window().and_then(|window| window.document()) else {
        return false;
    };
    let Ok(Some(menu)) = document.query_selector(".composer-menu[open], .utility-menu[open]")
    else {
        return false;
    };
    let _ = menu.remove_attribute("open");
    if let Ok(Some(summary)) = menu.query_selector("summary") {
        if let Some(element) = summary.dyn_ref::<web_sys::HtmlElement>() {
            let _ = element.focus();
        }
    }
    true
}
