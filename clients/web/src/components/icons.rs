use leptos::prelude::*;

#[component]
pub(crate) fn PlusIcon() -> impl IntoView {
    view! { <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" aria-hidden="true"><path d="M12 5v14M5 12h14" /></svg> }
}

#[component]
pub(crate) fn RefreshIcon() -> impl IntoView {
    view! { <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M20 7v5h-5M20 12a8 8 0 1 0-2 5M20 12a8 8 0 0 0-2-5" /></svg> }
}

#[component]
pub(crate) fn PanelIcon(#[prop(default = false)] right: bool) -> impl IntoView {
    view! { <svg width="17" height="17" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="3" y="4" width="18" height="16" rx="3" /><path d=if right { "M15 4v16" } else { "M9 4v16" } /></svg> }
}

#[component]
pub(crate) fn ArrowUpIcon() -> impl IntoView {
    view! { <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M12 19V5m-6 6 6-6 6 6" /></svg> }
}

#[component]
pub(crate) fn StopIcon() -> impl IntoView {
    view! { <svg width="18" height="18" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><rect x="6" y="6" width="12" height="12" rx="2" /></svg> }
}

#[component]
pub(crate) fn QueueIcon() -> impl IntoView {
    view! { <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M5 4v10a4 4 0 0 0 4 4h10m-4-4 4 4-4 4M10 5h7M10 9h5" /></svg> }
}

#[component]
pub(crate) fn EditIcon() -> impl IntoView {
    view! { <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m15 5 4 4M4 20l5-1L20 8a2.8 2.8 0 0 0-4-4L5 15l-1 5Z" /></svg> }
}

#[component]
pub(crate) fn TrashIcon() -> impl IntoView {
    view! { <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M4 7h16M9 7V4h6v3M6 7l1 13h10l1-13M10 11v5m4-5v5" /></svg> }
}

#[component]
pub(crate) fn ShieldIcon() -> impl IntoView {
    view! { <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m12 3 8 3v6c0 5-8 9-8 9s-8-4-8-9V6l8-3Z" /><path d="m8.5 12 2.5 2.5 4.5-5" /></svg> }
}

#[component]
pub(crate) fn ChevronDownIcon() -> impl IntoView {
    view! { <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m6 9 6 6 6-6" /></svg> }
}

#[component]
pub(crate) fn SettingsIcon() -> impl IntoView {
    view! { <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
        <path d="M3 6h4m4 0h10M3 12h10m4 0h4M3 18h2m4 0h12" />
        <circle cx="9" cy="6" r="2" /><circle cx="15" cy="12" r="2" /><circle cx="7" cy="18" r="2" />
    </svg> }
}
