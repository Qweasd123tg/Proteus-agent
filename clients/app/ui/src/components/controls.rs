use leptos::prelude::*;
use web_sys::MouseEvent;

use crate::types::*;

#[component]
pub(crate) fn ToastStack<F>(toasts: ReadSignal<Vec<ToastMessage>>, on_dismiss: F) -> impl IntoView
where
    F: Fn(u64) + Copy + Send + 'static,
{
    view! {
        <div class="toast-stack" aria-live="polite">
            <For
                each=move || toasts.get()
                key=|toast| toast.id
                children=move |toast| {
                    let toast_id = toast.id;
                    view! {
                        <div class="toast">
                            <span>{toast.text}</span>
                            <button
                                type="button"
                                class="secondary"
                                title="Закрыть"
                                aria-label="Закрыть"
                                on:click=move |_| on_dismiss(toast_id)
                            >
                                <super::icons::CloseIcon/>
                            </button>
                        </div>
                    }
                }
            />
        </div>
    }
}

#[component]
pub(crate) fn PlanActionsCard<R, E, X>(on_revise: R, on_execute: E, on_exit: X) -> impl IntoView
where
    R: Fn(MouseEvent) + Copy + 'static,
    E: Fn(MouseEvent) + Copy + 'static,
    X: Fn(MouseEvent) + Copy + 'static,
{
    view! {
        <article class="task-card running plan-actions-card">
            <div class="task-card-header">
                <span class="status-badge running">
                    <span class="dot"></span>
                    "План готов"
                </span>
            </div>
            <div class="message system-message plan-actions-message">
                <button
                    type="button"
                    class="secondary"
                    on:click=on_revise
                    title="Уточнить последний план текстом из поля ввода"
                >
                    "Уточнить"
                </button>
                <button
                    type="button"
                    class="btn-primary"
                    on:click=on_execute
                    title="Переключиться в обычный режим и выполнить последний план"
                >
                    "Выполнить"
                </button>
                <button
                    type="button"
                    class="secondary"
                    on:click=on_exit
                    title="Вернуться в обычный режим"
                >
                    "Выйти"
                </button>
            </div>
        </article>
    }
}

#[component]
pub(crate) fn WorkingCard(status: ReadSignal<AgentStatus>) -> impl IntoView {
    // Waiting for the user is not progress: no spinner, attention colour.
    let waiting = Memo::new(move |_| status.with(AgentStatus::is_waiting));
    view! {
        <article class="task-card running working-card">
            <div class="task-card-header">
                <span class=move || if waiting.get() { "status-badge attention" } else { "status-badge running" }>
                    {move || (!waiting.get()).then(|| view! { <span class="spinner-dot"></span> })}
                    {move || status.with(AgentStatus::label)}
                </span>
            </div>
        </article>
    }
}
