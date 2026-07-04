use leptos::prelude::*;
use web_sys::MouseEvent;

use crate::app_helpers::{
    load_bool_setting, load_i32_setting, save_bool_setting, save_i32_setting,
};

const MIN_COMPOSER_HEIGHT_PX: i32 = 56;
const DEFAULT_COMPOSER_HEIGHT_PX: i32 = 88;
const MAX_COMPOSER_HEIGHT_PX: i32 = 240;
const MIN_CHAT_WIDTH_PX: i32 = 420;
const DEFAULT_CHAT_WIDTH_PX: i32 = 768;
const MAX_CHAT_WIDTH_PX: i32 = 1600;
const MIN_SIDEBAR_WIDTH_PX: i32 = 210;
const MAX_SIDEBAR_WIDTH_PX: i32 = 360;
const MIN_INFO_WIDTH_PX: i32 = 240;
const DEFAULT_INFO_WIDTH_PX: i32 = 300;
const MAX_INFO_WIDTH_PX: i32 = 420;
/// Ширина свёрнутых реек (см. CSS .sidebar-collapsed / .info-panel).
const COLLAPSED_RAIL_WIDTH_PX: i32 = 40;
/// Утащили край уже этого порога — панель сворачивается; шире — раскрывается.
const PANEL_COLLAPSE_AT_PX: i32 = 150;

#[derive(Clone, Copy)]
pub(crate) struct AppResizeState {
    pub(crate) sidebar_width: ReadSignal<i32>,
    pub(crate) sidebar_collapsed: ReadSignal<bool>,
    pub(crate) info_width: ReadSignal<i32>,
    pub(crate) info_open: ReadSignal<bool>,
    pub(crate) composer_height: ReadSignal<i32>,
    pub(crate) chat_width: ReadSignal<i32>,
    set_sidebar_width: WriteSignal<i32>,
    set_sidebar_collapsed: WriteSignal<bool>,
    set_info_width: WriteSignal<i32>,
    set_info_open: WriteSignal<bool>,
    set_composer_height: WriteSignal<i32>,
    set_chat_width: WriteSignal<i32>,
    dragging_sidebar: ReadSignal<bool>,
    set_dragging_sidebar: WriteSignal<bool>,
    dragging_info: ReadSignal<bool>,
    set_dragging_info: WriteSignal<bool>,
    dragging_composer: ReadSignal<bool>,
    set_dragging_composer: WriteSignal<bool>,
    dragging_chat: ReadSignal<bool>,
    set_dragging_chat: WriteSignal<bool>,
    resize_start_x: ReadSignal<i32>,
    set_resize_start_x: WriteSignal<i32>,
    resize_start_y: ReadSignal<i32>,
    set_resize_start_y: WriteSignal<i32>,
    resize_start_sidebar: ReadSignal<i32>,
    set_resize_start_sidebar: WriteSignal<i32>,
    resize_start_info: ReadSignal<i32>,
    set_resize_start_info: WriteSignal<i32>,
    resize_start_composer: ReadSignal<i32>,
    set_resize_start_composer: WriteSignal<i32>,
    resize_start_chat: ReadSignal<i32>,
    set_resize_start_chat: WriteSignal<i32>,
}

impl AppResizeState {
    pub(crate) fn new() -> Self {
        let (sidebar_width, set_sidebar_width) =
            signal(load_i32_setting("proteus.sidebarWidth", 260));
        let (sidebar_collapsed, set_sidebar_collapsed) =
            signal(load_bool_setting("proteus.sidebarCollapsed", false));
        let (info_width, set_info_width) = signal(
            load_i32_setting("proteus.infoPanelWidth", DEFAULT_INFO_WIDTH_PX)
                .clamp(MIN_INFO_WIDTH_PX, MAX_INFO_WIDTH_PX),
        );
        // Ключ исторический: состояние панели раньше хранил app.rs.
        let (info_open, set_info_open) = signal(load_bool_setting("proteus.infoPanelOpen", false));
        let (composer_height, set_composer_height) = signal(
            load_i32_setting("proteus.composerHeight", DEFAULT_COMPOSER_HEIGHT_PX)
                .clamp(MIN_COMPOSER_HEIGHT_PX, MAX_COMPOSER_HEIGHT_PX),
        );
        let (chat_width, set_chat_width) = signal(
            load_i32_setting("proteus.chatWidth", DEFAULT_CHAT_WIDTH_PX)
                .clamp(MIN_CHAT_WIDTH_PX, MAX_CHAT_WIDTH_PX),
        );
        let (dragging_sidebar, set_dragging_sidebar) = signal(false);
        let (dragging_info, set_dragging_info) = signal(false);
        let (dragging_composer, set_dragging_composer) = signal(false);
        let (dragging_chat, set_dragging_chat) = signal(false);
        let (resize_start_x, set_resize_start_x) = signal(0_i32);
        let (resize_start_y, set_resize_start_y) = signal(0_i32);
        let (resize_start_sidebar, set_resize_start_sidebar) = signal(260_i32);
        let (resize_start_info, set_resize_start_info) = signal(DEFAULT_INFO_WIDTH_PX);
        let (resize_start_composer, set_resize_start_composer) = signal(DEFAULT_COMPOSER_HEIGHT_PX);
        let (resize_start_chat, set_resize_start_chat) = signal(DEFAULT_CHAT_WIDTH_PX);

        Self {
            sidebar_width,
            sidebar_collapsed,
            info_width,
            info_open,
            composer_height,
            chat_width,
            set_sidebar_width,
            set_sidebar_collapsed,
            set_info_width,
            set_info_open,
            set_composer_height,
            set_chat_width,
            dragging_sidebar,
            set_dragging_sidebar,
            dragging_info,
            set_dragging_info,
            dragging_composer,
            set_dragging_composer,
            dragging_chat,
            set_dragging_chat,
            resize_start_x,
            set_resize_start_x,
            resize_start_y,
            set_resize_start_y,
            resize_start_sidebar,
            set_resize_start_sidebar,
            resize_start_info,
            set_resize_start_info,
            resize_start_composer,
            set_resize_start_composer,
            resize_start_chat,
            set_resize_start_chat,
        }
    }

    pub(crate) fn install_persistence_effects(self) {
        Effect::new(move |_| {
            save_i32_setting("proteus.sidebarWidth", self.sidebar_width.get());
        });

        Effect::new(move |_| {
            save_bool_setting("proteus.sidebarCollapsed", self.sidebar_collapsed.get());
        });

        Effect::new(move |_| {
            save_i32_setting("proteus.infoPanelWidth", self.info_width.get());
        });

        Effect::new(move |_| {
            save_bool_setting("proteus.infoPanelOpen", self.info_open.get());
        });

        Effect::new(move |_| {
            save_i32_setting("proteus.composerHeight", self.composer_height.get());
        });

        Effect::new(move |_| {
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

    pub(crate) fn begin_info_resize(self, ev: MouseEvent) {
        ev.prevent_default();
        self.set_dragging_info.set(true);
        self.set_resize_start_x.set(ev.client_x());
        self.set_resize_start_info.set(if self.info_open.get() {
            self.info_width.get()
        } else {
            COLLAPSED_RAIL_WIDTH_PX
        });
    }

    pub(crate) fn begin_composer_resize(self, ev: MouseEvent) {
        ev.prevent_default();
        self.set_dragging_composer.set(true);
        self.set_resize_start_y.set(ev.client_y());
        self.set_resize_start_composer
            .set(self.composer_height.get());
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
        // — раскрылись. Логика зеркальная: у сайдбара тянется правый край,
        // у инфо-панели — левый.
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
        if self.dragging_info.get() {
            let delta = self.resize_start_x.get() - ev.client_x();
            let target = self.resize_start_info.get() + delta;
            if target < PANEL_COLLAPSE_AT_PX {
                self.set_info_open.set(false);
            } else {
                self.set_info_open.set(true);
                self.set_info_width
                    .set(target.clamp(MIN_INFO_WIDTH_PX, MAX_INFO_WIDTH_PX));
            }
        }
        if self.dragging_composer.get() {
            let delta = ev.client_y() - self.resize_start_y.get();
            self.set_composer_height.set(
                (self.resize_start_composer.get() - delta)
                    .clamp(MIN_COMPOSER_HEIGHT_PX, MAX_COMPOSER_HEIGHT_PX),
            );
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
        self.set_dragging_info.set(false);
        self.set_dragging_composer.set(false);
        self.set_dragging_chat.set(false);
    }

    pub(crate) fn is_resizing(self) -> bool {
        self.dragging_sidebar.get()
            || self.dragging_info.get()
            || self.dragging_composer.get()
            || self.dragging_chat.get()
    }

    pub(crate) fn toggle_sidebar(self) {
        self.set_sidebar_collapsed.update(|value| *value = !*value);
    }

    pub(crate) fn toggle_info_panel(self) {
        self.set_info_open.update(|value| *value = !*value);
    }
}
