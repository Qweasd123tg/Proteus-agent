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
        <path d="M9.81,5.25 L9.98,2.51 L14.02,2.51 L14.19,5.25 L15.22,5.67 L17.28,3.86 L20.14,6.72 L18.33,8.78 L18.75,9.81 L21.49,9.98 L21.49,14.02 L18.75,14.19 L18.33,15.22 L20.14,17.28 L17.28,20.14 L15.22,18.33 L14.19,18.75 L14.02,21.49 L9.98,21.49 L9.81,18.75 L8.78,18.33 L6.72,20.14 L3.86,17.28 L5.67,15.22 L5.25,14.19 L2.51,14.02 L2.51,9.98 L5.25,9.81 L5.67,8.78 L3.86,6.72 L6.72,3.86 L8.78,5.67 Z" /><circle cx="12" cy="12" r="3.1" />
    </svg> }
}

#[component]
pub(crate) fn AnalysisIcon() -> impl IntoView {
    view! { <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" aria-hidden="true"><path d="M4 4v16h16M8 15v-4M12 15V7M16 15v-6" /></svg> }
}
#[component]
pub(crate) fn HistoryIcon() -> impl IntoView {
    view! { <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M3 5v5h5M3 10a9 9 0 1 1 2 8M12 7v5l3 2" /></svg> }
}
#[component]
pub(crate) fn InspectorIcon() -> impl IntoView {
    view! { <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="9" y="3" width="6" height="5" rx="1"/><rect x="3" y="16" width="6" height="5" rx="1"/><rect x="15" y="16" width="6" height="5" rx="1"/><path d="M12 8v4M6 16v-4h12v4"/></svg> }
}
