use leptos::{html, prelude::*};
use web_sys::{KeyboardEvent, MouseEvent, SubmitEvent};

use super::{
    composer_access_menu::ComposerAccessMenu,
    composer_model_menu::ComposerModelMenu,
    icons::{ArrowUpIcon, StopIcon},
    queued_prompts::QueuedPrompts,
};
use crate::{actions::AppActions, types::*};

#[component]
#[allow(clippy::too_many_arguments)]
pub(crate) fn ComposerView<S, K, T, DE, NB>(
    composer_ref: NodeRef<html::Textarea>,
    draft: ReadSignal<String>,
    set_draft: WriteSignal<String>,
    mode: ReadSignal<PermissionMode>,
    model_name: ReadSignal<String>,
    model_options: ReadSignal<Vec<ModelOption>>,
    reasoning_enabled: ReadSignal<bool>,
    effort: ReadSignal<ReasoningEffort>,
    effort_options: ReadSignal<Vec<String>>,
    is_sending: ReadSignal<bool>,
    active_run_id: ReadSignal<Option<String>>,
    queued_prompts: ReadSignal<Vec<QueuedPromptInfo>>,
    stick_to_bottom: ReadSignal<bool>,
    set_stick_to_bottom: WriteSignal<bool>,
    actions: AppActions,
    draft_is_empty: DE,
    new_below_count: NB,
    on_submit: S,
    on_keydown: K,
    on_cancel_turn: T,
) -> impl IntoView
where
    S: Fn(SubmitEvent) + 'static,
    K: Fn(KeyboardEvent) + 'static,
    T: Fn(MouseEvent) + Copy + Send + 'static,
    DE: Fn() -> bool + Copy + Send + 'static,
    NB: Fn() -> usize + Copy + Send + Sync + 'static,
{
    let dock_ref = NodeRef::<html::Form>::new();
    #[cfg(target_arch = "wasm32")]
    crate::ui_layout::attach_composer(dock_ref);
    let submit_label = move || {
        if is_sending.get() {
            "Добавить в очередь"
        } else if mode.get() == PermissionMode::Plan {
            "Запросить план"
        } else {
            "Отправить сообщение"
        }
    };
    view! {
        <form class="composer" node_ref=dock_ref on:submit=on_submit>
            <QueuedPrompts items=queued_prompts actions />
            <Show when=move || !stick_to_bottom.get()>
                <button type="button"
                    class="jump-to-bottom" class:has-count=move || { new_below_count() > 0 }
                    title="К последнему сообщению" aria-label="К последнему сообщению"
                    on:click=move |_| set_stick_to_bottom.set(true)>
                    {move || if new_below_count() > 0 { format!("↓ {}", new_below_count()) } else { "↓".to_owned() }}
                </button>
            </Show>
            <div class="composer-shell">
                <div class="composer-input">
                    // Зеркало текста задаёт высоту средствами layout, без JS-измерений
                    // на каждом вводе. Пробел сохраняет последнюю пустую строку.
                    <div class="composer-measure" aria-hidden="true">{move || format!("{} ", draft.get())}</div>
                    <textarea node_ref=composer_ref rows="1" aria-label="Сообщение агенту"
                        prop:value=move || draft.get()
                        placeholder=move || if mode.get() == PermissionMode::Plan { "Что нужно спланировать?" } else { "Напишите задачу…" }
                        on:input:target=move |ev| set_draft.set(ev.target().value())
                        on:keydown=on_keydown />
                </div>
                <div class="composer-toolbar">
                    <div class="composer-options">
                        <ComposerAccessMenu mode actions />
                    </div>
                    <div class="composer-actions">
                        <ComposerModelMenu model_name model_options reasoning_enabled effort effort_options actions />
                        {move || active_run_id.get().is_some().then(|| view! {
                            <button type="button" class="composer-stop" title="Остановить ход · Esc" aria-label="Остановить ход" on:click=on_cancel_turn><StopIcon /></button>
                        })}
                        <button type="submit" class="composer-submit" disabled=draft_is_empty
                            hidden=move || active_run_id.get().is_some() && draft_is_empty()
                            aria-label=submit_label title=move || format!("{} · Enter", submit_label())><ArrowUpIcon /></button>
                    </div>
                </div>
            </div>
        </form>
    }
}
