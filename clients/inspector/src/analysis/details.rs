use super::presentation::*;
use leptos::prelude::*;
use proteus_contracts::app_protocol::analysis::*;

#[component]
pub(super) fn StepDetails(step: AppAnalysisStep) -> impl IntoView {
    let title = step_title(&step);
    let status = step_status(&step);
    let elapsed = duration(step.started_at_ms, step.finished_at_ms);
    let identity = format!(
        "Запись {} · execution {:?} · thread {:?}",
        step.sequence, step.execution_id, step.thread_id
    );
    view! {
        <article class="analysis-detail">
            <header><span class="analysis-kicker">{step_kind(&step)}</span><h2>{title}</h2>
                <p>{format!("{status} · {elapsed} · {}", timestamp(step.started_at_ms))}</p></header>
            {match step.data {
                AppAnalysisStepData::Model { exchange_id, request, response, failure, messages, .. } => {
                    let finish = response.as_ref().map(|r| format!("Причина остановки: {:?} · Конец хода: {}", r.finish_reason, match r.end_turn { Some(true) => "да", Some(false) => "нет", None => "не указан" }));
                    let usage = response.as_ref().and_then(|r| r.usage.as_ref());
                    let usage_summary = usage.map(|value| format!("Вход: {} · Выход: {} · Из кэша: {}", value.input_tokens, value.output_tokens, value.cached_input_tokens.map(|n| n.to_string()).unwrap_or_else(|| "нет данных".into()))).unwrap_or_else(|| "Расход токенов не записан.".into());
                    let usage_json = usage.map(pretty);
                    let output = response.as_ref().map(|r| r.messages.clone()).unwrap_or(messages);
                    view! {
                        <p class="analysis-note">"Сохранённый canonical-запрос после подготовки Core. Это не HTTP-пакет провайдера; чувствительные значения могут быть скрыты при записи журнала."</p>
                        <p class="analysis-usage">{usage_summary}</p>
                        {finish.map(|value| view! { <p class="analysis-note">{value}</p> })}<code>{exchange_id.to_string()}</code>
                        {failure.map(|error| view! { <div class="analysis-error"><strong>"Ошибка модели"</strong><pre>{error.message}</pre></div> })}
                        <section><h3>"Инструкции"</h3>
                            {request.instructions.iter().map(|instruction| view! { <pre class="analysis-text">{instruction.text.clone()}</pre> }).collect_view()}
                        </section>
                        <section><h3>{format!("Сообщения запроса · {}", request.messages.len())}</h3>
                            {request.messages.iter().map(|message| view! {
                                <details class="analysis-message"><summary>{format!("{:?} · {}", message.role, message_preview(message))}</summary>
                                    <pre class="analysis-text">{message_content(message)}</pre>
                                    <details><summary>"Структура сообщения"</summary><pre>{pretty(message)}</pre></details>
                                </details>
                            }).collect_view()}
                        </section>
                        <section><h3>{format!("Доступные инструменты · {}", request.tools.len())}</h3>
                            <p>{request.tools.iter().map(|tool| tool.name.as_str()).collect::<Vec<_>>().join(", ")}</p>
                        </section>
                        <section><h3>"Записанный ответ"</h3>
                            {output.iter().map(|message| view! { <pre class="analysis-text">{message_content(message)}</pre> }).collect_view()}
                            {output.is_empty().then(|| view! { <p class="analysis-note">"Сообщения ответа не записаны."</p> })}
                        </section>
                        <details><summary>"Токены от провайдера"</summary><pre>{usage_json.unwrap_or_else(|| "Данных нет; это не нулевой расход.".into())}</pre></details>
                        <details><summary>"Полный сохранённый запрос"</summary><pre>{pretty(&request)}</pre></details>
                        <details><summary>"Полный сохранённый ответ"</summary><pre>{pretty(&response)}</pre></details>
                    }.into_any()
                }
                AppAnalysisStepData::Tool { call, approval_reason, resolution, result } => {
                    view! {
                        <code>{call.id}</code>
                        <section><h3>"Аргументы"</h3><pre>{pretty(&call.args)}</pre></section>
                        {approval_reason.map(|reason| view! { <section><h3>"Запрос подтверждения"</h3><p>{reason}</p></section> })}
                        <details><summary>"Решение о выполнении"</summary><pre>{pretty(&resolution)}</pre></details>
                        {match result {
                            Some(result) => view! {
                                <section><h3>"Результат"</h3><pre class="analysis-text">{result.output.clone()}</pre>
                                    {result.error.clone().map(|error| view! { <pre class="analysis-error">{error}</pre> })}
                                </section>
                                <details><summary>"Полный результат"</summary><pre>{pretty(&result)}</pre></details>
                            }.into_any(),
                            None => view! { <p class="analysis-warning">"Результат не записан. По этому журналу нельзя установить, выполнилось ли внешнее действие. Повторный запуск может повторить его эффект."</p> }.into_any(),
                        }}
                    }.into_any()
                }
                AppAnalysisStepData::Compaction { report, history_revision } => view! {
                    <p>{format!("Сообщений: {} → {} · версия истории {}", report.input_messages, report.output_messages, history_revision)}</p>
                    <p>{report.reason.clone()}</p><pre class="analysis-text">{report.summary.clone()}</pre>
                    <details><summary>"Отчёт сжатия"</summary><pre>{pretty(&report)}</pre></details>
                }.into_any(),
                AppAnalysisStepData::Hook { trace } => view! {
                    <p class="analysis-note">"Обработчики выполнены в порядке конфигурации. Здесь сохранены вход, решения и результат цепочки."</p>
                    <section><h3>"Обработчики"</h3>
                        {trace.steps.iter().map(|step| view! { <details><summary>{step.module_id.clone()}</summary><pre>{pretty(&step.outcome)}</pre></details> }).collect_view()}
                    </section>
                    <details><summary>"Входные данные"</summary><pre>{pretty(&trace.input)}</pre></details>
                    <details><summary>"Результат цепочки"</summary><pre>{pretty(&trace.output)}</pre></details>
                }.into_any(),
            }}
            <details class="analysis-identity"><summary>"Идентификаторы шага"</summary><code>{identity}</code></details>
        </article>
    }
}
