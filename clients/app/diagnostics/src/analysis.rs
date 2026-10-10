use crate::api::{get_analysis_json, query_value};
use leptos::{prelude::*, task::spawn_local};
use proteus_contracts::app_protocol::{AppBootstrap, AppSessionSummary, analysis::*};
use std::sync::Arc;

mod details;
mod metrics;
mod presentation;
use details::StepDetails;
use presentation::*;

#[component]
pub(crate) fn AnalysisView() -> impl IntoView {
    let sessions = RwSignal::new(Vec::<AppSessionSummary>::new());
    let sessions_loaded = RwSignal::new(false);
    let selected = RwSignal::new(
        query_value("analysis_session")
            .or_else(|| query_value("session_dir"))
            .unwrap_or_default(),
    );
    let turn = RwSignal::new(query_value("analysis_turn").unwrap_or_default());
    let step = RwSignal::new(None::<u64>);
    let snapshot = RwSignal::new(None::<Arc<AppSessionAnalysis>>);
    let error = RwSignal::new(None::<String>);
    let loading = RwSignal::new(false);
    let refresh = RwSignal::new(0_u64);
    let generation = RwSignal::new(0_u64);
    let search = RwSignal::new(String::new());
    let failures_only = RwSignal::new(false);

    spawn_local(async move {
        match get_analysis_json::<Vec<AppSessionSummary>>("/sessions").await {
            Ok(items) => {
                if selected.try_get_untracked().is_none() {
                    return;
                }
                if selected.get_untracked().is_empty() {
                    let preferred = get_analysis_json::<AppBootstrap>("/bootstrap")
                        .await
                        .ok()
                        .and_then(|b| b.session_dir)
                        .filter(|path| items.iter().any(|item| &item.session_dir == path))
                        .or_else(|| items.first().map(|s| s.session_dir.clone()));
                    if selected.try_get_untracked().is_none() {
                        return;
                    }
                    if let Some(path) = preferred {
                        selected.set(path.to_string_lossy().into_owned());
                    }
                }
                sessions.set(items);
                sessions_loaded.set(true);
            }
            Err(message) => {
                error.try_set(Some(message));
            }
        }
    });

    Effect::new(move |_| {
        let session = selected.get();
        let turn_id = turn.get();
        refresh.get();
        let version = generation.get_untracked() + 1;
        generation.set(version);
        snapshot.set(None);
        if session.is_empty() {
            loading.set(false);
            return;
        }
        persist(&session, &turn_id);
        loading.set(true);
        error.set(None);
        let mut path = format!(
            "/analysis?session_dir={}",
            js_sys::encode_uri_component(&session)
        );
        if !turn_id.is_empty() {
            path.push_str(&format!(
                "&turn_id={}",
                js_sys::encode_uri_component(&turn_id)
            ));
        }
        spawn_local(async move {
            let result = get_analysis_json::<AppSessionAnalysis>(&path).await;
            if generation.try_get_untracked() != Some(version) {
                return;
            }
            loading.set(false);
            match result {
                Ok(value) => {
                    let steps = value
                        .selected
                        .as_ref()
                        .map(|detail| detail.steps.as_slice())
                        .unwrap_or_default();
                    if !steps
                        .iter()
                        .any(|item| Some(item.sequence) == step.get_untracked())
                    {
                        step.set(
                            steps
                                .iter()
                                .find(|item| step_status(item) == "Ошибка")
                                .or_else(|| steps.first())
                                .map(|item| item.sequence),
                        );
                    }
                    snapshot.set(Some(Arc::new(value)));
                }
                Err(message) => error.set(Some(message)),
            }
        });
    });

    view! {
        <section class="turn-analysis">
            <header class="analysis-heading">
                <div><h1>"Анализ ходов"</h1><p>"От задачи до результата — по сохранённым фактам."</p></div>
                <button class="secondary" disabled=move || loading.get() || selected.get().is_empty()
                    on:click=move |_| refresh.update(|value| *value += 1)>"Обновить"</button>
            </header>
            <div class="analysis-toolbar">
                <label for="inspector-analysis-session">"Сессия"</label>
                <select id="inspector-analysis-session" prop:value=move || selected.get()
                    on:change:target=move |event| {
                        turn.set(String::new()); step.set(None); selected.set(event.target().value());
                    }>
                    <option value="" disabled>"Выберите сохранённую сессию"</option>
                    {move || {
                        let path = selected.get();
                        (!path.is_empty() && !sessions.get().iter().any(|item| item.session_dir.to_string_lossy() == path))
                            .then(|| view! { <option value=path.clone() selected>{path.clone()}</option> })
                    }}
                    {move || sessions.get().into_iter().map(|item| {
                        let path = item.session_dir.to_string_lossy().into_owned();
                        let selected_path = path.clone();
                        let label = item.preview.unwrap_or_else(|| "Новый чат".into());
                        let description = item.workspace_path.to_string_lossy().into_owned();
                        view! { <option value=path data-description=description prop:selected=move || selected.get() == selected_path>{label}</option> }
                    }).collect_view()}
                </select>
            </div>
            <p class="analysis-note">"Просмотр не запускает сессию и не меняет активный чат. Запросы, результаты и конфигурация относятся к выбранному ходу."</p>
            <Show when=move || sessions_loaded.get() && sessions.get().is_empty() && selected.get().is_empty()><p class="analysis-note">"Сохранённых сессий пока нет. Выполните ход в чате, затем откройте анализ."</p></Show>
            {move || error.get().map(|message| view! { <div class="analysis-error" role="alert"><strong>"Не удалось загрузить анализ"</strong><pre>{message}</pre></div> })}
            <Show when=move || loading.get()><p role="status">"Читаю журнал…"</p></Show>
            {move || snapshot.get().map(|data| {
                let selected_id = data.selected.as_ref().map(|detail| detail.turn_id);
                let summary = data.turns.iter().find(|item| Some(item.turn_id) == selected_id).cloned();
                let summary_text = summary.as_ref().map(|item| format!("Ход {} · {} · {}", item.number, turn_status(item.status), duration(item.started_at_ms, item.finished_at_ms))).unwrap_or_default();
                let turn_list = data.turns.clone();
                let shared = data.clone();
                view! {
                    <div class="analysis-snapshot-meta"><span>{format!("Сессия {}", data.session_id)}</span><span>{format!("Версия журнала {}", data.revision)}</span></div>
                    <div class="analysis-workbench">
                        <aside class="analysis-turns" aria-label="Ходы сессии"><h2>"Ходы"</h2>
                            {turn_list.is_empty().then(|| view! { <p class="analysis-note">"В журнале пока нет ходов."</p> })}
                            {turn_list.into_iter().rev().map(|item| {
                                let id = item.turn_id.to_string();
                                let active = Some(item.turn_id) == selected_id;
                                view! { <button class="analysis-turn" class:active=active aria-pressed=active.to_string()
                                    on:click=move |_| { step.set(None); turn.set(id.clone()); }>
                                    <strong>{format!("Ход {}", item.number)}</strong><span>{turn_status(item.status)}</span>
                                    <p>{item.prompt_preview}</p><small>{timestamp(item.started_at_ms)}</small>
                                </button> }
                            }).collect_view()}
                        </aside>
                        <div class="analysis-turn-content">
                            {data.selected.as_ref().map(|detail| {
                                let steps = Arc::new(detail.steps.clone());
                                let steps_for_list = steps.clone();
                                view! {
                                    <header class="analysis-turn-summary"><h2>{summary_text}</h2><p>{detail.task.text.clone()}</p>
                                        <metrics::TurnMetrics steps=steps.clone()/>
                                        {detail.error.clone().map(|message| view! { <div class="analysis-error"><strong>"Причина завершения"</strong><pre>{message}</pre></div> })}
                                        {summary.as_ref().is_some_and(|s| s.status == AppAnalysisTurnStatus::Unsettled).then(|| view! { <p class="analysis-warning">"Завершение хода не записано. Он может ещё выполняться или быть прерван; этот снимок не устанавливает причину."</p> })}
                                        <details><summary>"Итоговый ответ"</summary><pre class="analysis-text">{detail.output.as_ref().map(|output| output.text.clone()).unwrap_or_else(|| "Итоговый ответ не записан.".into())}</pre></details>
                                        <details><summary>{format!("Конфигурация при запуске · epoch {}", detail.module_epoch)}</summary><pre>{pretty(&detail.config_snapshot)}</pre></details>
                                    </header>
                                    <div class="analysis-step-layout">
                                        <section class="analysis-steps" aria-label="Последовательность шагов">
                                            <h3>{format!("Шаги · {}", steps.len())}</h3>
                                            <input type="search" aria-label="Поиск по названию шага" placeholder="Название шага…"
                                                prop:value=move || search.get() on:input:target=move |event| search.set(event.target().value())/>
                                            <label class="analysis-filter"><input type="checkbox" prop:checked=move || failures_only.get()
                                                on:change:target=move |event| failures_only.set(event.target().checked())/>"Ошибки и без результата"</label>
                                            {move || {
                                                let term = search.get().to_lowercase();
                                                let visible = steps_for_list.iter().filter(|item| {
                                                    (!failures_only.get() || matches!(step_status(item), "Ошибка" | "Нет результата"))
                                                        && step_title(item).to_lowercase().contains(&term)
                                                }).collect::<Vec<_>>();
                                                if visible.is_empty() { return view! { <p class="analysis-note">"Подходящих шагов нет."</p> }.into_any(); }
                                                visible.into_iter().map(|item| {
                                                    let seq = item.sequence;
                                                    view! { <button class="analysis-step" class:active=move || step.get() == Some(seq)
                                                        aria-pressed=move || (step.get() == Some(seq)).to_string() on:click=move |_| step.set(Some(seq))>
                                                        <small>{format!("{} · #{}", step_kind(item), seq)}</small><strong>{step_title(item)}</strong>
                                                        <span class:error=step_status(item) == "Ошибка">{format!("{} · {}", step_status(item), duration(item.started_at_ms, item.finished_at_ms))}</span>
                                                    </button> }
                                                }).collect_view().into_any()
                                            }}
                                        </section>
                                        <div class="analysis-step-detail">
                                            {move || steps.iter().find(|item| Some(item.sequence) == step.get()).cloned()
                                                .map(|item| view! { <StepDetails step=item/> })}
                                        </div>
                                    </div>
                                }
                            })}
                        </div>
                    </div>
                    <details class="analysis-identity"><summary>"Граница отчёта"</summary><p>"Порядок шагов задан журналом; это не граф причинных зависимостей. Внутренние HTTP-повторы провайдера и отдельные журналы дочерних агентов здесь не объединяются. Времена инструментов включают ожидание подтверждения."</p><code>{shared.session_id.to_string()}</code></details>
                }
            })}
        </section>
    }
}
