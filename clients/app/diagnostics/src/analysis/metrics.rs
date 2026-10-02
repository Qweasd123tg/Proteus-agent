use leptos::prelude::*;
use proteus_contracts::app_protocol::analysis::{AppAnalysisStep, AppAnalysisStepData};
use std::sync::Arc;

#[component]
pub(super) fn TurnMetrics(steps: Arc<Vec<AppAnalysisStep>>) -> impl IntoView {
    let mut models = 0;
    let mut tools = 0;
    let mut errors = 0;
    let mut missing = 0;
    let mut usage_count = 0;
    let mut input = 0_u64;
    let mut output = 0_u64;
    for step in steps.iter() {
        errors += usize::from(super::presentation::step_has_error(step));
        match &step.data {
            AppAnalysisStepData::Model {
                response, failure, ..
            } => {
                models += 1;
                missing += usize::from(response.is_none() && failure.is_none());
                if let Some(usage) = response.as_ref().and_then(|r| r.usage.as_ref()) {
                    usage_count += 1;
                    input += u64::from(usage.input_tokens);
                    output += u64::from(usage.output_tokens);
                }
            }
            AppAnalysisStepData::Tool { result, .. } => {
                tools += 1;
                missing += usize::from(result.is_none());
            }
            AppAnalysisStepData::Compaction { .. } | AppAnalysisStepData::Hook { .. } => {}
        }
    }
    let tokens = if usage_count == 0 {
        "нет данных".to_owned()
    } else {
        format!("{input} / {output}")
    };
    view! {
        <dl class="analysis-metrics">
            <div><dt>"Запросы модели"</dt><dd>{models}</dd></div>
            <div><dt>"Инструменты"</dt><dd>{tools}</dd></div>
            <div><dt>"Ошибки шагов"</dt><dd>{errors}</dd></div>
            <div><dt>"Без результата"</dt><dd>{missing}</dd></div>
            <div><dt>"Токены: вход / выход"</dt><dd>{tokens}</dd></div>
        </dl>
        <p class="analysis-note">{format!("Usage записан для {usage_count} из {models} запросов модели, включая сжатие. Сумма учитывает только записанные значения.")}</p>
    }
}
