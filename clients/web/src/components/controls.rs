use leptos::prelude::*;
use web_sys::MouseEvent;

use crate::types::*;

/// Пороги (в процентах) для смены цвета дуги: норма → внимание → критично.
const CONTEXT_RING_CRIT_PERCENT: u8 = 90;

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
                                on:click=move |_| on_dismiss(toast_id)
                            >
                                "×"
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
pub(crate) fn WorkingCard(status: ReadSignal<String>) -> impl IntoView {
    view! {
        <article class="task-card running working-card">
            <div class="task-card-header">
                <span class="status-badge running">
                    <span class="spinner-dot"></span>
                    {move || status.get()}
                </span>
            </div>
        </article>
    }
}

/// Бублик заполнения контекстного окна (рейка инфо-панели): дуга открывает
/// круговой градиент зелёный → жёлтый → красный по мере наполнения, метка
/// порога автокомпакта — приглушённый штрих на дуге; процент и токены — в
/// title при наведении, в красной зоне дуга подсвечивается.
/// На старте использует последний сохранённый снимок, если текущая сессия
/// ещё не прислала свежий `TokenUsageUpdated`.
#[component]
pub(crate) fn ContextRing(usage: ReadSignal<Option<ContextUsage>>) -> impl IntoView {
    move || {
        let Some(context) = usage.get() else {
            return ().into_any();
        };
        let percent = context.percent();
        let degrees = f64::from(percent) / 100.0 * 360.0;
        // Метку автокомпакта рисуем только когда сервер прислал порог.
        let compaction_percent = context.compaction_percent();
        let mut style = format!("--context-ring-deg: {degrees:.1}deg");
        let mut title = format!(
            "Контекст: {percent}% · {} / {} токенов",
            format_token_count(context.used_tokens),
            format_token_count(context.max_tokens),
        );
        if let (Some(mark_percent), Some(trigger_tokens)) =
            (compaction_percent, context.compaction_trigger_tokens)
        {
            let mark_degrees = f64::from(mark_percent) / 100.0 * 360.0;
            style.push_str(&format!("; --context-ring-mark-deg: {mark_degrees:.1}deg"));
            title.push_str(&format!(
                " · автокомпакт при {mark_percent}% (~{})",
                format_token_count(trigger_tokens),
            ));
        }
        let mut class = "context-ring".to_owned();
        if compaction_percent.is_some() {
            class.push_str(" context-ring-has-mark");
        }
        if percent >= CONTEXT_RING_CRIT_PERCENT {
            class.push_str(" context-ring-crit");
        }
        view! {
            <div
                class=class
                style=style
                aria-label=title
            ></div>
        }
        .into_any()
    }
}

/// Компактная запись числа токенов: «90.5k», «200k», «512».
pub(crate) fn format_token_count(tokens: u32) -> String {
    if tokens < 1000 {
        return tokens.to_string();
    }
    let thousands = f64::from(tokens) / 1000.0;
    let formatted = format!("{thousands:.1}");
    format!("{}k", formatted.trim_end_matches(".0"))
}
