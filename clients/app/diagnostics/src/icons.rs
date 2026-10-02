use leptos::prelude::*;

#[component]
pub(crate) fn Icon(
    name: &'static str,
    #[prop(default = 18)] size: u32,
    #[prop(default = "")] class: &'static str,
) -> impl IntoView {
    view! {
        <svg class=class width=size height=size viewBox="0 0 20 20" fill="none"
            stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"
            style="display:block;flex:none;pointer-events:none" aria-hidden="true" focusable="false">
            <use href=format!("/assets/proteus-icons.svg#{name}")/>
        </svg>
    }
}
