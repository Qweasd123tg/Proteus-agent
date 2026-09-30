use proteus_contracts::app_protocol::analysis::*;

pub(super) fn turn_status(status: AppAnalysisTurnStatus) -> &'static str {
    match status {
        AppAnalysisTurnStatus::Unsettled => "Нет завершения",
        AppAnalysisTurnStatus::Success => "Завершён",
        AppAnalysisTurnStatus::Error => "Ошибка",
        AppAnalysisTurnStatus::Canceled => "Отменён",
        AppAnalysisTurnStatus::Timeout => "Таймаут",
    }
}

pub(super) fn step_title(step: &AppAnalysisStep) -> String {
    match &step.data {
        AppAnalysisStepData::Model {
            request, origin, ..
        } => format!(
            "{} · {}",
            request.model.model,
            if matches!(
                origin,
                proteus_contracts::contracts::ModelCallOrigin::Compactor
            ) {
                "сжатие"
            } else {
                "основной запрос"
            }
        ),
        AppAnalysisStepData::Tool { call, .. } => call.name.clone(),
        AppAnalysisStepData::Compaction { .. } => "Сжатие истории".into(),
        AppAnalysisStepData::Hook { trace } => match &trace.input.event {
            proteus_contracts::contracts::HookEvent::TurnStarted { .. } => "Начало хода",
            proteus_contracts::contracts::HookEvent::BeforeModel { .. } => "Перед запросом модели",
            proteus_contracts::contracts::HookEvent::BeforeTool { .. } => "Перед инструментом",
            proteus_contracts::contracts::HookEvent::AfterTool { .. } => "После инструмента",
            proteus_contracts::contracts::HookEvent::TurnSettled { .. } => "Завершение хода",
        }
        .into(),
    }
}

pub(super) fn step_kind(step: &AppAnalysisStep) -> &'static str {
    match step.data {
        AppAnalysisStepData::Model { .. } => "Модель",
        AppAnalysisStepData::Tool { .. } => "Инструмент",
        AppAnalysisStepData::Compaction { .. } => "Контекст",
        AppAnalysisStepData::Hook { .. } => "Хуки",
    }
}

pub(super) fn step_has_error(step: &AppAnalysisStep) -> bool {
    match &step.data {
        AppAnalysisStepData::Model {
            failure, response, ..
        } => {
            failure.is_some()
                || response.as_ref().is_some_and(|r| {
                    matches!(
                        r.finish_reason,
                        proteus_contracts::model_standard::FinishReason::Error
                    )
                })
        }
        AppAnalysisStepData::Tool { result, .. } => result.as_ref().is_some_and(|r| !r.ok),
        AppAnalysisStepData::Hook { trace } => trace.steps.iter().any(|step| {
            matches!(
                step.outcome,
                proteus_contracts::contracts::HookStepOutcome::Failed { .. }
            )
        }),
        _ => false,
    }
}

pub(super) fn step_status(step: &AppAnalysisStep) -> &'static str {
    if step_has_error(step) {
        return "Ошибка";
    }
    match &step.data {
        AppAnalysisStepData::Model { response: None, .. } => "Нет результата",
        AppAnalysisStepData::Tool { result: None, .. } => "Нет результата",
        AppAnalysisStepData::Compaction { report, .. } if !report.changed => "Без изменений",
        _ => "Готово",
    }
}

pub(super) fn duration(start: i64, end: Option<i64>) -> String {
    match end {
        Some(end) => format!("{:.2} с", (end - start).max(0) as f64 / 1000.0),
        None => "—".into(),
    }
}

pub(super) fn timestamp(ms: i64) -> String {
    js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(ms as f64))
        .to_locale_string("ru-RU", &js_sys::Object::new())
        .into()
}

pub(super) fn message_content(
    message: &proteus_contracts::model_standard::CanonicalMessage,
) -> String {
    let text = message.display_text();
    if text.is_empty() {
        pretty(message)
    } else {
        text
    }
}

pub(super) fn message_preview(
    message: &proteus_contracts::model_standard::CanonicalMessage,
) -> String {
    let text = message.display_text();
    if text.is_empty() {
        "структурированное содержимое".into()
    } else {
        text.chars().take(90).collect()
    }
}

pub(super) fn pretty(value: &impl serde::Serialize) -> String {
    serde_json::to_string_pretty(value).unwrap_or_default()
}

pub(super) fn persist(session: &str, turn: &str) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let Ok(href) = window.location().href() else {
        return;
    };
    let Ok(url) = web_sys::Url::new(&href) else {
        return;
    };
    let mut parts: Vec<String> = url
        .search()
        .trim_start_matches('?')
        .split('&')
        .filter(|part| {
            !part.is_empty()
                && !part.starts_with("analysis_session=")
                && !part.starts_with("analysis_turn=")
        })
        .map(str::to_owned)
        .collect();
    for (key, value) in [("analysis_session", session), ("analysis_turn", turn)] {
        if !value.is_empty() {
            parts.push(format!("{key}={}", js_sys::encode_uri_component(value)));
        }
    }
    url.set_search(&parts.join("&"));
    if let Ok(history) = window.history() {
        let _ = history.replace_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some(&url.href()));
    }
}
