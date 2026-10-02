use leptos::{html, prelude::*};
#[component]
pub(crate) fn SettingsView() -> impl IntoView {
    let root = NodeRef::<html::Div>::new();
    super::client_module::mount(root, None);
    view! { <div node_ref=root class="settings-page"/> }
}
