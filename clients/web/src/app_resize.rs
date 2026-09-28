use leptos::prelude::*;
use web_sys::MouseEvent;

use crate::ui_preferences::{
    load_bool_setting, load_i32_setting, save_bool_setting, save_i32_setting,
};

const MIN_CHAT_WIDTH_PX: i32 = 420;
const DEFAULT_CHAT_WIDTH_PX: i32 = 820;
const MAX_CHAT_WIDTH_PX: i32 = 1600;
const MIN_SIDEBAR_WIDTH_PX: i32 = 210;
const MAX_SIDEBAR_WIDTH_PX: i32 = 360;
/// Ширина свёрнутых реек (см. CSS .sidebar-collapsed).
const COLLAPSED_RAIL_WIDTH_PX: i32 = 40;
/// Утащили край уже этого порога — панель сворачивается; шире — раскрывается.
const PANEL_COLLAPSE_AT_PX: i32 = 150;

#[derive(Clone, Copy)]
pub(crate) struct AppResizeState {
    pub(crate) sidebar_width: ReadSignal<i32>,
    pub(crate) sidebar_collapsed: ReadSignal<bool>,
    pub(crate) chat_width: ReadSignal<i32>,
    set_sidebar_width: WriteSignal<i32>,
    set_sidebar_collapsed: WriteSignal<bool>,
    set_chat_width: WriteSignal<i32>,
    dragging_sidebar: ReadSignal<bool>,
    set_dragging_sidebar: WriteSignal<bool>,
    dragging_chat: ReadSignal<bool>,
    set_dragging_chat: WriteSignal<bool>,
    resize_start_x: ReadSignal<i32>,
    set_resize_start_x: WriteSignal<i32>,
    resize_start_sidebar: ReadSignal<i32>,
    set_resize_start_sidebar: WriteSignal<i32>,
    resize_start_chat: ReadSignal<i32>,
    set_resize_start_chat: WriteSignal<i32>,
}

impl AppResizeState {
    pub(crate) fn new() -> Self {
        let (sidebar_width, set_sidebar_width) = signal(
            load_i32_setting("proteus.sidebarWidth", 280)
                .clamp(MIN_SIDEBAR_WIDTH_PX, MAX_SIDEBAR_WIDTH_PX),
        );
        let (sidebar_collapsed, set_sidebar_collapsed) =
            signal(load_bool_setting("proteus.sidebarCollapsed", false));
        let (chat_width, set_chat_width) = signal(
            load_i32_setting("proteus.chatWidth", DEFAULT_CHAT_WIDTH_PX)
                .clamp(MIN_CHAT_WIDTH_PX, MAX_CHAT_WIDTH_PX),
        );
        let (dragging_sidebar, set_dragging_sidebar) = signal(false);
        let (dragging_chat, set_dragging_chat) = signal(false);
        let (resize_start_x, set_resize_start_x) = signal(0_i32);
        let (resize_start_sidebar, set_resize_start_sidebar) = signal(280_i32);
        let (resize_start_chat, set_resize_start_chat) = signal(DEFAULT_CHAT_WIDTH_PX);

        Self {
            sidebar_width,
            sidebar_collapsed,
            chat_width,
            set_sidebar_width,
            set_sidebar_collapsed,
            set_chat_width,
            dragging_sidebar,
            set_dragging_sidebar,
            dragging_chat,
            set_dragging_chat,
            resize_start_x,
            set_resize_start_x,
            resize_start_sidebar,
            set_resize_start_sidebar,
            resize_start_chat,
            set_resize_start_chat,
        }
    }

    pub(crate) fn install_persistence_effects(self) {
        Effect::new(move |_| {
            if self.is_resizing() {
                return;
            }
            save_i32_setting("proteus.sidebarWidth", self.sidebar_width.get());
        });

        Effect::new(move |_| {
            if self.is_resizing() {
                return;
            }
            save_bool_setting("proteus.sidebarCollapsed", self.sidebar_collapsed.get());
        });

        Effect::new(move |_| {
            if self.is_resizing() {
                return;
            }
            save_i32_setting("proteus.chatWidth", self.chat_width.get());
        });
    }

    pub(crate) fn begin_sidebar_resize(self, ev: MouseEvent) {
        ev.prevent_default();
        self.set_dragging_sidebar.set(true);
        self.set_resize_start_x.set(ev.client_x());
        self.set_resize_start_sidebar
            .set(if self.sidebar_collapsed.get() {
                COLLAPSED_RAIL_WIDTH_PX
            } else {
                self.sidebar_width.get()
            });
    }

    pub(crate) fn begin_chat_resize(self, ev: MouseEvent) {
        ev.prevent_default();
        self.set_dragging_chat.set(true);
        self.set_resize_start_x.set(ev.client_x());
        self.set_resize_start_chat.set(self.chat_width.get());
    }

    pub(crate) fn drag(self, ev: MouseEvent) {
        // Обе боковые панели сворачиваются и раскрываются тем же жестом, что
        // и ресайзятся: утащили край за порог — схлопнулись, вытащили обратно
        // — раскрылись.
        if self.dragging_sidebar.get() {
            let delta = ev.client_x() - self.resize_start_x.get();
            let target = self.resize_start_sidebar.get() + delta;
            if target < PANEL_COLLAPSE_AT_PX {
                self.set_sidebar_collapsed.set(true);
            } else {
                self.set_sidebar_collapsed.set(false);
                self.set_sidebar_width
                    .set(target.clamp(MIN_SIDEBAR_WIDTH_PX, MAX_SIDEBAR_WIDTH_PX));
            }
        }
        if self.dragging_chat.get() {
            let delta = ev.client_x() - self.resize_start_x.get();
            self.set_chat_width.set(
                (self.resize_start_chat.get() + delta * 2)
                    .clamp(MIN_CHAT_WIDTH_PX, MAX_CHAT_WIDTH_PX),
            );
        }
    }

    pub(crate) fn stop(self) {
        self.set_dragging_sidebar.set(false);
        self.set_dragging_chat.set(false);
    }

    pub(crate) fn is_resizing(self) -> bool {
        self.dragging_sidebar.get() || self.dragging_chat.get()
    }

    pub(crate) fn toggle_sidebar(self) {
        crate::ui_layout::prepare_panel_focus(".sidebar");
        self.set_sidebar_collapsed.update(|value| *value = !*value);
    }
}
