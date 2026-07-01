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

#[derive(Clone, Copy)]
pub(crate) struct AppResizeState {
    pub(crate) sidebar_width: ReadSignal<i32>,
    pub(crate) sidebar_collapsed: ReadSignal<bool>,
    pub(crate) composer_height: ReadSignal<i32>,
    pub(crate) chat_width: ReadSignal<i32>,
    set_sidebar_width: WriteSignal<i32>,
    set_sidebar_collapsed: WriteSignal<bool>,
    set_composer_height: WriteSignal<i32>,
    set_chat_width: WriteSignal<i32>,
    dragging_sidebar: ReadSignal<bool>,
    set_dragging_sidebar: WriteSignal<bool>,
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
        let (composer_height, set_composer_height) = signal(
            load_i32_setting("proteus.composerHeight", DEFAULT_COMPOSER_HEIGHT_PX)
                .clamp(MIN_COMPOSER_HEIGHT_PX, MAX_COMPOSER_HEIGHT_PX),
        );
        let (chat_width, set_chat_width) = signal(
            load_i32_setting("proteus.chatWidth", DEFAULT_CHAT_WIDTH_PX)
                .clamp(MIN_CHAT_WIDTH_PX, MAX_CHAT_WIDTH_PX),
        );
        let (dragging_sidebar, set_dragging_sidebar) = signal(false);
        let (dragging_composer, set_dragging_composer) = signal(false);
        let (dragging_chat, set_dragging_chat) = signal(false);
        let (resize_start_x, set_resize_start_x) = signal(0_i32);
        let (resize_start_y, set_resize_start_y) = signal(0_i32);
        let (resize_start_sidebar, set_resize_start_sidebar) = signal(260_i32);
        let (resize_start_composer, set_resize_start_composer) = signal(DEFAULT_COMPOSER_HEIGHT_PX);
        let (resize_start_chat, set_resize_start_chat) = signal(DEFAULT_CHAT_WIDTH_PX);

        Self {
            sidebar_width,
            sidebar_collapsed,
            composer_height,
            chat_width,
            set_sidebar_width,
            set_sidebar_collapsed,
            set_composer_height,
            set_chat_width,
            dragging_sidebar,
            set_dragging_sidebar,
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
            save_i32_setting("proteus.composerHeight", self.composer_height.get());
        });

        Effect::new(move |_| {
            save_i32_setting("proteus.chatWidth", self.chat_width.get());
        });
    }

    pub(crate) fn begin_sidebar_resize(self, ev: MouseEvent) {
        ev.prevent_default();
        if self.sidebar_collapsed.get() {
            return;
        }
        self.set_dragging_sidebar.set(true);
        self.set_resize_start_x.set(ev.client_x());
        self.set_resize_start_sidebar.set(self.sidebar_width.get());
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
        if self.dragging_sidebar.get() {
            let delta = ev.client_x() - self.resize_start_x.get();
            self.set_sidebar_width
                .set((self.resize_start_sidebar.get() + delta).clamp(210, 360));
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
        self.set_dragging_composer.set(false);
        self.set_dragging_chat.set(false);
    }

    pub(crate) fn is_resizing(self) -> bool {
        self.dragging_sidebar.get() || self.dragging_composer.get() || self.dragging_chat.get()
    }

    pub(crate) fn toggle_sidebar(self) {
        self.set_sidebar_collapsed.update(|value| *value = !*value);
    }
}
