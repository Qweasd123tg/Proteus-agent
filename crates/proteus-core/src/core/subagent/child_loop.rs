//! Дочерний агентский цикл (модель → tools → модель) и его helpers.

use std::time::Duration;

use anyhow::{Result, anyhow};
use serde_json::{Value, json};
use tokio::time::timeout;

use crate::{
    contracts::{
        BudgetTracker, RuntimeContext, SubagentRequest, SubagentRoleSpec, SubagentStatus,
        ToolExposureInput, ToolExposureRequest,
    },
    core::ToolOrchestrator,
    domain::{CacheHints, ModelRef, ThreadId, ToolSpec},
    model_standard::{
        CanonicalMessage, CanonicalModelRequest, CanonicalModelResponse, ContentPart, MessageRole,
        TokenUsage,
    },
};

use super::TASK_TOOL_NAME;

pub(super) struct ChildLoopState {
    pub history: Vec<CanonicalMessage>,
    pub iterations: u32,
    pub last_text: Option<String>,
    pub usage: Option<TokenUsage>,
}

/// Отбор tools ребёнка: сперва policy-видимость (тот же гейт, что
/// `visible_tool_specs` у workflow host), затем `ToolExposure::select` с
/// фазой роли. Tool `task` выкидывается из итогового списка.
pub(super) async fn select_child_tools(
    ctx: &RuntimeContext,
    orchestrator: &ToolOrchestrator,
    request: &SubagentRequest,
    role: &SubagentRoleSpec,
) -> Result<Vec<ToolSpec>> {
    let candidates = orchestrator.visible_tool_specs(ctx, &request.task.cwd);
    let exposure_request =
        ToolExposureRequest::new(request.task.clone()).with_phase(role.effective_exposure_phase());
    let output = ctx
        .tool_exposure
        .select(ToolExposureInput::new(exposure_request, candidates))
        .await?;
    Ok(apply_child_tool_filters(output.tools, role))
}

pub(super) fn apply_child_tool_filters(
    tools: Vec<ToolSpec>,
    role: &SubagentRoleSpec,
) -> Vec<ToolSpec> {
    let allowlist = role
        .config
        .get("tools")
        .and_then(Value::as_array)
        .map(|tools| tools.iter().filter_map(Value::as_str).collect::<Vec<_>>());

    tools
        .into_iter()
        .filter(|spec| spec.name != TASK_TOOL_NAME)
        .filter(|spec| match &allowlist {
            Some(allowed) => allowed.iter().any(|name| *name == spec.name),
            None => true,
        })
        .collect()
}

pub(super) async fn run_child_loop(
    role: &SubagentRoleSpec,
    request: &SubagentRequest,
    ctx: &RuntimeContext,
    orchestrator: &ToolOrchestrator,
    tools: &[ToolSpec],
    state: &mut ChildLoopState,
) -> Result<SubagentStatus> {
    // Бюджет скоупится на текущий запуск (fresh или resume), а не на весь
    // task_id: усечённый по бюджету ребёнок продолжается через resume с
    // новым окном — иначе продолжение упиралось бы в потолок сразу.
    let mut budget = BudgetTracker::new(role.limits.max_total_tokens);
    for _ in 0..role.limits.max_iterations {
        if ctx.is_cancelled() {
            return Ok(SubagentStatus::Cancelled);
        }

        // Delta-события ModelService эмитятся с контекстом родительского
        // хода (set_event_context ставит runtime до запуска workflow), то
        // есть стрим ребёнка утёк бы в родительский транскрипт как обычный
        // AssistantTextDelta и «переписался» финальным текстом родителя в
        // конце хода. Пока у карточки субагента нет собственного stream-slot,
        // дельты ребёнка глушим — итог приходит через SubagentResult.
        let model_request =
            CanonicalModelRequest::new(ctx.model_ref.clone(), state.history.clone())
                .with_tools(tools.to_vec())
                .with_reasoning(ctx.reasoning.clone())
                .with_cache(CacheHints::new(true, true))
                .with_metadata(json!({
                    "suppress_stream_deltas": true,
                    "prompt_cache_key": child_prompt_cache_key(&ctx.model_ref, ctx.thread_id),
                }));
        let response = match complete_model(ctx, model_request).await {
            Ok(response) => response,
            Err(_) if ctx.is_cancelled() => return Ok(SubagentStatus::Cancelled),
            Err(error) => return Err(error),
        };
        state.iterations += 1;
        accumulate_usage(&mut state.usage, response.usage.as_ref());
        budget.record(response.usage.as_ref());
        if let Some(text) = message_text(&response.message) {
            state.last_text = Some(text);
        }
        state.history.push(response.message.clone());

        if response.tool_calls.is_empty() {
            return Ok(SubagentStatus::Completed);
        }
        // Проверка по факту ответа (первый запрос всегда разрешён): при
        // превышении цикл останавливается до исполнения tool calls — они
        // не исполняются, история остаётся с незакрытыми calls, их закрывает
        // штатный snapshot-механизм терминальных статусов.
        if budget.exceeded() {
            return Ok(SubagentStatus::TokenBudgetExceeded);
        }

        for call in response.tool_calls {
            if ctx.is_cancelled() {
                return Ok(SubagentStatus::Cancelled);
            }
            let result = match orchestrator.execute(ctx, &request.task, call).await {
                Ok(result) => result,
                Err(_) if ctx.is_cancelled() => return Ok(SubagentStatus::Cancelled),
                Err(error) => return Err(error),
            };
            let call_id = result.call_id.clone();
            state.history.push(
                CanonicalMessage::new(MessageRole::Tool, vec![ContentPart::ToolResult { result }])
                    .with_tool_call_id(call_id),
            );
        }
    }
    Ok(SubagentStatus::MaxIterationsReached)
}

/// Стабильный prompt-cache ключ дочернего цикла. История ребёнка растёт
/// append-only, поэтому ключа на `(provider, model, child_thread_id)`
/// достаточно для консистентного prefix-cache routing между итерациями;
/// resume по `task_id` переиспользует тот же `child_thread_id`, так что кеш
/// продолжается и после resume. Схема согласована с workflow-ключом
/// (`proteus:{provider}:{model}:...`), но с явным пространством `subagent`,
/// чтобы дочерние префиксы не смешивались с родительскими.
pub(super) fn child_prompt_cache_key(model_ref: &ModelRef, thread_id: ThreadId) -> String {
    format!(
        "proteus:subagent:{}:{}:{}",
        sanitize_cache_key_component(&model_ref.provider),
        sanitize_cache_key_component(&model_ref.model),
        thread_id
    )
}

/// Копия sanitize-правила workflow-ключа: ASCII alnum/-/_/. остаются,
/// остальное заменяется на `_`, компонент обрезается до 64 символов.
fn sanitize_cache_key_component(value: &str) -> String {
    let mut out = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    out.truncate(64);
    if out.is_empty() {
        "model".to_owned()
    } else {
        out
    }
}

/// Model call с таймаутом родительского runtime и отменой через
/// `ctx.cancellation` — тот же контур, что у workflow plugin host.
async fn complete_model(
    ctx: &RuntimeContext,
    request: CanonicalModelRequest,
) -> Result<CanonicalModelResponse> {
    let completion = async {
        if ctx.model_timeout_ms == 0 {
            ctx.model.complete(request).await
        } else {
            timeout(
                Duration::from_millis(ctx.model_timeout_ms),
                ctx.model.complete(request),
            )
            .await
            .map_err(|_| anyhow!("model request timed out after {}ms", ctx.model_timeout_ms))?
        }
    };
    tokio::select! {
        result = completion => result,
        _ = ctx.cancellation.cancelled() => Err(anyhow!("turn canceled by client")),
    }
}

fn message_text(message: &CanonicalMessage) -> Option<String> {
    let text = message
        .parts
        .iter()
        .filter_map(|part| match part {
            ContentPart::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    (!text.trim().is_empty()).then_some(text)
}

/// Сумматор usage поверх `TokenUsage::accumulate` для Option-аккумуляторов
/// раннеров (`ChildLoopState.usage`, `TurnTracker.usage`).
pub(super) fn accumulate_usage(total: &mut Option<TokenUsage>, usage: Option<&TokenUsage>) {
    let Some(usage) = usage else {
        return;
    };
    match total {
        None => *total = Some(usage.clone()),
        Some(total) => total.accumulate(usage),
    }
}

/// Обрезает строку по границе char так, чтобы результат был <= `max_bytes`.
pub(super) fn truncate_at_char_boundary(mut text: String, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text;
    }
    let mut end = max_bytes;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
    text
}

/// snake_case строка статуса для `Event::SubagentFinished` — через serde,
/// чтобы не дублировать rename_all руками.
pub(super) fn subagent_status_label(status: SubagentStatus) -> String {
    serde_json::to_value(status)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| format!("{status:?}"))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::domain::{ToolSafety, new_thread_id};

    #[test]
    fn summary_truncation_respects_char_boundaries() {
        // "й" — 2 байта; лимит 5 байт должен резать по границе char (4 байта).
        let text = "й".repeat(4);
        let truncated = truncate_at_char_boundary(text, 5);
        assert_eq!(truncated, "йй");
        assert!(truncated.len() <= 5);
        assert!(truncated.is_char_boundary(truncated.len()));

        // Строка короче лимита не меняется.
        assert_eq!(truncate_at_char_boundary("abc".to_owned(), 5), "abc");
    }

    #[test]
    fn usage_accumulation_sums_option_fields() {
        let mut total = None;
        accumulate_usage(&mut total, Some(&TokenUsage::new(10, 2)));
        accumulate_usage(
            &mut total,
            Some(
                &TokenUsage::new(5, 3)
                    .with_cached_input_tokens(Some(4))
                    .with_reasoning_output_tokens(Some(7)),
            ),
        );
        accumulate_usage(&mut total, None);

        let total = total.expect("usage accumulated");
        assert_eq!(total.input_tokens, 15);
        assert_eq!(total.output_tokens, 5);
        assert_eq!(total.cached_input_tokens, Some(4));
        assert_eq!(total.cache_creation_input_tokens, None);
        assert_eq!(total.reasoning_output_tokens, Some(7));
    }

    #[test]
    fn status_label_is_snake_case() {
        assert_eq!(
            subagent_status_label(SubagentStatus::Completed),
            "completed"
        );
        assert_eq!(
            subagent_status_label(SubagentStatus::MaxIterationsReached),
            "max_iterations_reached"
        );
        assert_eq!(subagent_status_label(SubagentStatus::TimedOut), "timed_out");
        assert_eq!(
            subagent_status_label(SubagentStatus::Cancelled),
            "cancelled"
        );
        assert_eq!(
            subagent_status_label(SubagentStatus::TokenBudgetExceeded),
            "token_budget_exceeded"
        );
    }

    #[test]
    fn role_tools_allowlist_filters_exposed_tools() {
        let role = crate::contracts::SubagentRoleSpec::new("explore", "Explore", "prompt")
            .with_config(json!({ "tools": ["remember_fact"] }));
        let tools = apply_child_tool_filters(
            vec![
                ToolSpec::new("remember_fact", "Remember", json!({}), ToolSafety::ReadOnly),
                ToolSpec::new("search", "Search", json!({}), ToolSafety::ReadOnly),
                ToolSpec::new(TASK_TOOL_NAME, "Delegate", json!({}), ToolSafety::ReadOnly),
            ],
            &role,
        );

        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "remember_fact");
    }

    #[test]
    fn child_prompt_cache_key_is_stable_and_sanitized() {
        let model_ref = ModelRef::new("open ai", "gpt/5.5");
        let thread_id = new_thread_id();
        let key = child_prompt_cache_key(&model_ref, thread_id);
        assert_eq!(key, format!("proteus:subagent:open_ai:gpt_5.5:{thread_id}"));
        assert_eq!(key, child_prompt_cache_key(&model_ref, thread_id));
    }
}
