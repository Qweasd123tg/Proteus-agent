use leptos::prelude::*;

#[component]
fn Icon(name: &'static str, #[prop(default = 18)] size: u32) -> impl IntoView {
    view! {
        <svg width=size height=size viewBox="0 0 20 20" fill="none"
            stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"
            style="display:block;flex:none;pointer-events:none" aria-hidden="true" focusable="false">
            <use href=format!("/assets/proteus-icons.svg#{name}")/>
        </svg>
    }
}

macro_rules! icon {
    ($component:ident, $name:literal, $size:literal) => {
        #[component]
        pub(crate) fn $component() -> impl IntoView {
            view! { <Icon name=$name size=$size/> }
        }
    };
}

icon!(PlusIcon, "plus", 16);
icon!(RefreshIcon, "refresh", 16);
icon!(ArrowUpIcon, "arrow-up", 20);
icon!(ArrowDownIcon, "arrow-down", 16);
icon!(StopIcon, "stop", 20);
icon!(QueueIcon, "queue", 16);
icon!(EditIcon, "edit", 16);
icon!(TrashIcon, "trash", 16);
icon!(ShieldIcon, "shield", 16);
icon!(ChevronDownIcon, "chevron-down", 14);
icon!(SettingsIcon, "settings", 18);
icon!(AnalysisIcon, "analysis", 18);
icon!(HistoryIcon, "history", 18);
icon!(InspectorIcon, "inspector", 18);
icon!(CloseIcon, "close", 16);
icon!(MoreIcon, "more", 18);

#[component]
pub(crate) fn PanelIcon(#[prop(default = false)] right: bool) -> impl IntoView {
    view! { <Icon name=if right { "panel-right" } else { "panel-left" }/> }
}
