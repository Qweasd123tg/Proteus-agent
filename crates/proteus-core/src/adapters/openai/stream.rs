use serde_json::Value;

use super::response::from_openai_response;
use crate::model_standard::ModelStreamEvent;

/// Трансляция одного SSE event'а от OpenAI Responses API в наши
/// `ModelStreamEvent`. Вариантов много; всё что не распознали —
/// игнорируем (возвращаем пустой вектор), это безопасно потому что
/// финальный `Response` приходит на `response.completed`.
pub(super) fn translate_sse_event(event_type: &str, data: &str) -> Vec<ModelStreamEvent> {
    // [DONE] sentinel у OpenAI не используется в Responses API, но на
    // всякий случай — безопасный фаст-path.
    if data == "[DONE]" {
        return Vec::new();
    }
    let Ok(parsed) = serde_json::from_str::<Value>(data) else {
        return Vec::new();
    };
    match event_type {
        "response.output_text.delta" => {
            if let Some(delta) = parsed.get("delta").and_then(Value::as_str) {
                return vec![ModelStreamEvent::TextDelta {
                    text: delta.to_owned(),
                }];
            }
            Vec::new()
        }
        "response.reasoning_summary_text.delta" | "response.reasoning_summary.delta" => {
            if let Some(delta) = parsed.get("delta").and_then(Value::as_str) {
                return vec![ModelStreamEvent::ReasoningSummaryDelta {
                    text: delta.to_owned(),
                }];
            }
            Vec::new()
        }
        "response.function_call_arguments.delta" => {
            let call_id = parsed
                .get("item_id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned();
            let args_delta = parsed
                .get("delta")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned();
            if call_id.is_empty() || args_delta.is_empty() {
                return Vec::new();
            }
            vec![ModelStreamEvent::ToolCallDelta {
                call_id,
                name: None,
                args_delta,
            }]
        }
        "response.completed" => {
            // В payload-е объект полного `response`, парсим через
            // существующий `from_openai_response`. Если парсинг упал —
            // эмитим Error, чтобы drain-loop не ждал вечно.
            let response_value = parsed.get("response").cloned().unwrap_or(parsed);
            match from_openai_response(response_value) {
                Ok(response) => vec![ModelStreamEvent::Response { response }],
                Err(error) => vec![ModelStreamEvent::Error {
                    message: format!("failed to parse final response: {error}"),
                }],
            }
        }
        "response.error" | "error" => {
            let message = parsed
                .get("error")
                .and_then(|e| e.get("message"))
                .and_then(Value::as_str)
                .map(str::to_owned)
                .or_else(|| {
                    parsed
                        .get("message")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                })
                .unwrap_or_else(|| "unknown openai error".to_owned());
            vec![ModelStreamEvent::Error { message }]
        }
        _ => Vec::new(),
    }
}

/// Строит финальный Response из события response.completed. Если прокси
/// прислал пустой `output` (хотя item'ы были доставлены через
/// response.output_item.done) — подставляем накопленные item'ы, иначе
/// настоящий ответ модели теряется как <empty model response>.
pub(super) fn finalize_completed_event(
    data: &str,
    fallback_items: &[Value],
) -> Vec<ModelStreamEvent> {
    let Ok(parsed) = serde_json::from_str::<Value>(data) else {
        return Vec::new();
    };
    let mut response_value = parsed.get("response").cloned().unwrap_or(parsed);
    let output_is_empty = response_value
        .get("output")
        .and_then(Value::as_array)
        .map(|items| items.is_empty())
        .unwrap_or(true);
    if output_is_empty && !fallback_items.is_empty() {
        response_value["output"] = Value::Array(fallback_items.to_vec());
    }
    match from_openai_response(response_value) {
        Ok(response) => vec![ModelStreamEvent::Response { response }],
        Err(error) => vec![ModelStreamEvent::Error {
            message: format!("failed to parse final response: {error}"),
        }],
    }
}
