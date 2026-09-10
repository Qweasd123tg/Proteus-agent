use crate::{api::get_json, types::*};
use leptos::{prelude::*, task::spawn_local};
use wasm_bindgen::JsValue;

pub(super) fn refresh_pending_control_plane(
    set_pending_approvals: WriteSignal<Vec<ApprovalRequestInfo>>,
    set_pending_user_inputs: WriteSignal<Vec<UserInputRequestInfo>>,
    set_queued_prompts: WriteSignal<Vec<QueuedPromptInfo>>,
    generation: ReadSignal<u64>,
    expected: u64,
) {
    spawn_local(async move {
        let result = get_json::<PendingControlPlaneInfo>("/pending").await;
        if generation.try_get_untracked() != Some(expected) {
            return;
        }
        match result {
            Ok(pending) => {
                set_pending_approvals.set(pending.approvals);
                set_pending_user_inputs.set(pending.user_inputs);
                set_queued_prompts.set(pending.queued_user_messages);
            }
            Err(error) => web_sys::console::warn_1(&JsValue::from_str(&format!(
                "Pending control-plane refresh failed: {error}"
            ))),
        }
    });
}
