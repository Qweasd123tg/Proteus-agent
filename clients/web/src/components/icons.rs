use leptos::prelude::*;

#[component]
pub(crate) fn SettingsIcon() -> impl IntoView {
    view! { <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
        <path d="M3 6h4m4 0h10M3 12h10m4 0h4M3 18h2m4 0h12" />
        <circle cx="9" cy="6" r="2" /><circle cx="15" cy="12" r="2" /><circle cx="7" cy="18" r="2" />
    </svg> }
}
