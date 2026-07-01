use leptos::prelude::*;

use crate::types::{ToastMessage, TransportStatus};
use crate::ui_utils::set_timeout;

const TOAST_DISMISS_MS: i32 = 6000;

pub(crate) fn install_transport_toast_effect(
    transport_status: ReadSignal<TransportStatus>,
    last_error_toast: ReadSignal<Option<String>>,
    set_last_error_toast: WriteSignal<Option<String>>,
    next_toast_id: ReadSignal<u64>,
    set_next_toast_id: WriteSignal<u64>,
    set_toasts: WriteSignal<Vec<ToastMessage>>,
) {
    Effect::new(move |_| match transport_status.get() {
        TransportStatus::Error(message) => {
            if last_error_toast.get_untracked().as_deref() != Some(message.as_str()) {
                let id = next_toast_id.get_untracked();
                set_next_toast_id.set(id + 1);
                set_toasts.update(|items| {
                    items.push(ToastMessage {
                        id,
                        text: message.clone(),
                    });
                });
                set_last_error_toast.set(Some(message));
                set_timeout(TOAST_DISMISS_MS, move || {
                    set_toasts.update(|items| items.retain(|toast| toast.id != id));
                });
            }
        }
        TransportStatus::Connected => {
            if last_error_toast.get_untracked().is_some() {
                set_last_error_toast.set(None);
            }
        }
        TransportStatus::Connecting | TransportStatus::Shutdown => {}
    });
}
