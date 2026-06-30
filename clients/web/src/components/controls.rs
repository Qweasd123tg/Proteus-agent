use leptos::prelude::*;
use web_sys::MouseEvent;

use crate::types::*;

/// Пороги (в процентах) для смены цвета дуги: норма → внимание → критично.
const CONTEXT_RING_WARN_PERCENT: u8 = 70;
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

/// Миникарта пользовательских сообщений у правого края ленты: тонкие штрихи
/// (по одному на сообщение), при наведении раскрывается список с короткими
/// текстами; клик по пункту прокручивает к сообщению. Скрыта, пока сообщений
/// меньше двух.
#[component]
pub(crate) fn MessageNav<J>(
    items: Memo<Vec<(u64, String)>>,
    active: ReadSignal<Option<u64>>,
    on_jump: J,
) -> impl IntoView
where
    J: Fn(u64) + Copy + Send + 'static,
{
    move || {
        if items.with(|items| items.len() < 2) {
            return ().into_any();
        }
        view! {
            <nav class="msg-nav" aria-label="Переход к моим сообщениям">
                <div class="msg-nav-ticks">
                    <For
                        each=move || items.get()
                        key=|(id, _)| *id
                        children=move |(id, _)| {
                            view! {
                                <button
                                    type="button"
                                    class="msg-nav-tick"
                                    class:active=move || active.get() == Some(id)
                                    aria-label="К сообщению"
                                    on:click=move |_| on_jump(id)
                                ></button>
                            }
                        }
                    />
                </div>
                <div class="msg-nav-list">
                    <For
                        each=move || items.get()
                        key=|(id, _)| *id
                        children=move |(id, text)| {
                            view! {
                                <button
                                    type="button"
                                    class="msg-nav-item"
                                    class:active=move || active.get() == Some(id)
                                    on:click=move |_| on_jump(id)
                                >
                                    {text}
                                </button>
                            }
                        }
                    />
                </div>
            </nav>
        }
        .into_any()
    }
}

#[component]
pub(crate) fn QueuedPromptCard<S, C>(
    text: String,
    is_sending: ReadSignal<bool>,
    on_send: S,
    on_clear: C,
) -> impl IntoView
where
    S: Fn(MouseEvent) + 'static,
    C: Fn(MouseEvent) + 'static,
{
    let preview = text.clone();
    view! {
        <article class="task-card running queued-card">
            <div class="task-card-header">
                <span class="status-badge disconnected">
                    <span class="dot"></span>
                    "В очереди"
                </span>
            </div>
            <div class="message system-message queued-message">
                <p>{preview}</p>
                <div class="queued-actions">
                    <button
                        type="button"
                        class="btn-primary"
                        disabled=move || is_sending.get()
                        on:click=on_send
                    >
                        "Отправить"
                    </button>
                    <button type="button" class="secondary" on:click=on_clear>
                        "Убрать"
                    </button>
                </div>
            </div>
        </article>
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

/// Маленький бублик в строке ввода: показывает, насколько заполнено
/// контекстное окно. На старте использует последний сохранённый снимок,
/// если текущая сессия ещё не прислала свежий `TokenUsageUpdated`.
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
        let level = if percent >= CONTEXT_RING_CRIT_PERCENT {
            "crit"
        } else if percent >= CONTEXT_RING_WARN_PERCENT {
            "warn"
        } else {
            "ok"
        };
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
        let class = if compaction_percent.is_some() {
            format!("context-ring context-ring-{level} context-ring-has-mark")
        } else {
            format!("context-ring context-ring-{level}")
        };
        view! {
            <div
                class=class
                style=style
                title=title.clone()
                aria-label=title
            >
                <span class="context-ring-label">{percent.to_string()}</span>
            </div>
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
