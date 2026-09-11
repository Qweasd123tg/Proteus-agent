use super::state::AppState;

pub(super) fn publish(state: AppState) {
    #[cfg(not(target_arch = "wasm32"))]
    let _ = state;
    #[cfg(target_arch = "wasm32")]
    {
        use crate::components::tool_activity::parse_plan_steps;
        use leptos::prelude::*;
        use wasm_bindgen::prelude::*;
        #[wasm_bindgen(raw_module = "/extensions/web-adapter.js")]
        extern "C" {
            #[wasm_bindgen(js_name = publishSessionState)]
            fn publish_snapshot(value: &str);
        }
        let plan = Memo::new(move |_| {
            state.chat.messages.with(|items| {
                items
                    .iter()
                    .rev()
                    .filter_map(|message| message.tool.as_ref())
                    .find(|tool| tool.name == crate::tool_names::UPDATE_PLAN_TOOL)
                    .map(|tool| parse_plan_steps(&tool.args))
                    .unwrap_or_default()
                    .iter()
                    .map(|step| serde_json::json!({"step": step.step, "status": step.status}))
                    .collect::<Vec<_>>()
            })
        });
        Effect::new(move |_| {
            let context = state.session.context_usage.get().map(|usage| serde_json::json!({
                "used": usage.used_tokens, "max": usage.max_tokens, "trigger": usage.compaction_trigger_tokens,
            }));
            let value = serde_json::json!({
                "session_dir": state.session.active_session_dir.get(),
                "workspace": state.session.workspace_label.get(),
                "model": state.request.model_name.get(),
                "mode": state.request.mode.get().label(),
                "reasoning": if state.request.reasoning_enabled.get() { state.request.effort.get().label() } else { "выкл".to_owned() },
                "status": state.chat.agent_status.get(),
                "events": state.session.event_count.get(),
                "tools": state.chat.tool_activities.with(Vec::len),
                "pending": state.chat.pending_approvals.with(Vec::len) + state.chat.pending_user_inputs.with(Vec::len),
                "plan": plan.get(), "context": context,
            });
            publish_snapshot(&value.to_string());
        });
    }
}
