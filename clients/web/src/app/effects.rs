use super::{navigation::AppRouter, state::AppState};
use crate::{
    app_toasts::install_transport_toast_effect, chat_scroll::*, types::*, ui_preferences::*,
    ui_utils::set_timeout,
};
use leptos::prelude::*;
use wasm_bindgen::prelude::*;
#[wasm_bindgen]
unsafe extern "C" {
    #[wasm_bindgen(js_namespace = window, js_name = proteusTypesetMath)]
    fn proteus_typeset_math();
}
pub(super) fn install(state: AppState, router: AppRouter) {
    let super::state::ChatState {
        is_sending,
        active_stream_message_id,
        tool_activities,
        pending_user_inputs,
        messages,
        ..
    } = state.chat;
    let super::state::RequestState {
        draft,
        set_draft,
        queued_prompts,
        ..
    } = state.request;
    let super::state::SessionState {
        transport_status,
        active_session_dir,
        set_context_usage,
        ..
    } = state.session;
    let super::state::ViewState {
        set_toasts,
        next_toast_id,
        set_next_toast_id,
        last_error_toast,
        set_last_error_toast,
        stick_to_bottom,
        scroll_frame_pending,
        set_scroll_frame_pending,
        set_last_results_scroll_top,
        activity_now_ms,
        set_activity_now_ms,
        detach_baseline,
        set_detach_baseline,
        results_ref,
        resize,
        ..
    } = state.view;
    let is_chat_route = move || router.is_chat();
    let last_math_typeset_signature = StoredValue::new_local(None::<(u64, u64)>);
    let activity_tick_pending = StoredValue::new_local(false);
    Effect::new(move |_| {
        let _ = (
            messages.with(|_| ()),
            pending_user_inputs.with(|items| items.len()),
            queued_prompts.with(|items| items.len()),
            is_sending.get(),
            // Возврат на чат после SPA-перехода: лента смонтирована заново,
            // прилипание к низу надо восстановить.
            is_chat_route(),
        );
        if stick_to_bottom.get() {
            schedule_results_scroll(
                results_ref,
                stick_to_bottom,
                scroll_frame_pending,
                set_scroll_frame_pending,
                set_last_results_scroll_top,
            );
        }
    });

    Effect::new(move |_| {
        // SPA-переход пересобирает DOM чата: вне чата сбрасываем подпись,
        // чтобы по возвращении MathJax переверстал формулы заново.
        if !is_chat_route() {
            last_math_typeset_signature.set_value(None);
            return;
        }
        if active_stream_message_id.get().is_some() {
            return;
        }
        let signature = messages.with(|items| latest_math_signature(items));
        let mut unchanged = false;
        last_math_typeset_signature.with_value(|last| {
            unchanged = *last == signature;
        });
        if unchanged {
            return;
        }
        last_math_typeset_signature.set_value(signature);
        if signature.is_some() {
            proteus_typeset_math();
        }
    });

    Effect::new(move |_| {
        let _ = activity_now_ms.get();
        let active = is_sending.get()
            || tool_activities.with(|items| items.iter().any(tool_activity_is_active));
        if !active {
            activity_tick_pending.set_value(false);
            return;
        }
        let mut pending = false;
        activity_tick_pending.with_value(|value| {
            pending = *value;
        });
        if pending {
            return;
        }
        activity_tick_pending.set_value(true);
        set_timeout(1000, move || {
            activity_tick_pending.set_value(false);
            set_activity_now_ms.set(js_sys::Date::now().max(0.0) as u64);
        });
    });

    resize.install_persistence_effects();

    // Черновик композера привязан к сессии: восстанавливается после
    // переключения сессий и перезагрузки страницы.
    let draft_session = StoredValue::new_local(None::<Option<String>>);
    Effect::new(move |_| {
        let session = active_session_dir.get();
        let mut previous = None;
        draft_session.with_value(|value| previous = value.clone());
        draft_session.set_value(Some(session.clone()));
        match previous {
            // Первый прогон после монтирования: сессия ещё не резолвнулась.
            None => {}
            Some(previous) if previous == session => {}
            Some(previous) => match session.as_deref() {
                // Сессия только что получила dir (новая сессия или /config
                // после перезагрузки): набранный текст не затираем, а
                // записываем в черновик этой сессии.
                Some(dir) if previous.is_none() => {
                    let current = draft.get_untracked();
                    if current.trim().is_empty() {
                        set_draft.set(load_session_draft(dir).unwrap_or_default());
                    } else {
                        save_session_draft(dir, &current);
                    }
                }
                Some(dir) => set_draft.set(load_session_draft(dir).unwrap_or_default()),
                None => set_draft.set(String::new()),
            },
        }
    });
    // Каждое изменение черновика сохраняем под активной сессией.
    Effect::new(move |_| {
        let text = draft.get();
        if let Some(dir) = active_session_dir.get_untracked() {
            save_session_draft(&dir, &text);
        }
    });

    // Бублик контекста тоже привязан к сессии: при переключении показываем
    // её последний снимок (или ничего), а не хвост предыдущей. Живые
    // TokenUsageUpdated затем перезаписывают сигнал.
    Effect::new(move |_| {
        let session = active_session_dir.get();
        set_context_usage.set(load_context_usage(session.as_deref()));
    });

    // Счётчик для кнопки «вниз»: сколько сообщений добавилось с момента,
    // как лента отлипла от низа.
    Effect::new(move |_| {
        if stick_to_bottom.get() {
            if detach_baseline.get_untracked().is_some() {
                set_detach_baseline.set(None);
            }
        } else if detach_baseline.get_untracked().is_none() {
            set_detach_baseline.set(Some(messages.with_untracked(|items| items.len())));
        }
    });
    install_transport_toast_effect(
        transport_status,
        last_error_toast,
        set_last_error_toast,
        next_toast_id,
        set_next_toast_id,
        set_toasts,
    );
}

pub(crate) fn latest_math_signature(messages: &[Message]) -> Option<(u64, u64)> {
    messages
        .iter()
        .rev()
        .find(|message| {
            !message.streaming && message.tool.is_none() && message_may_contain_math(&message.text)
        })
        .map(|message| (message.id, message.version))
}

fn message_may_contain_math(text: &str) -> bool {
    text.contains('$') || text.contains("\\(") || text.contains("\\[")
}

pub(crate) fn tool_activity_is_active(tool: &ToolActivity) -> bool {
    matches!(
        tool.status,
        ToolActivityStatus::Running
            | ToolActivityStatus::WaitingApproval
            | ToolActivityStatus::Approved
    )
}
