//! Проверки model response на границе дочернего агентского цикла.

use std::collections::HashSet;

use anyhow::{Result, bail};

use crate::{
    domain::{ToolCall, ToolSpec},
    model_standard::{CanonicalModelResponse, validate_model_response_structure},
};

/// Компактный снимок capability-набора конкретного model request. Храним
/// только имена, чтобы не клонировать полные JSON schemas на каждой итерации.
pub(super) struct RequestVisibleTools {
    names: HashSet<String>,
}

impl RequestVisibleTools {
    pub(super) fn capture(request_tools: &[ToolSpec]) -> Self {
        Self {
            names: request_tools.iter().map(|tool| tool.name.clone()).collect(),
        }
    }

    /// Structural validation всегда идёт раньше capability validation: ни
    /// malformed history projection, ни duplicate id, ни неуспешный
    /// finish_reason не должны дойти до history mutation/ToolOrchestrator.
    pub(super) fn validate_response(
        &self,
        role_name: &str,
        response: &CanonicalModelResponse,
    ) -> Result<()> {
        validate_model_response_structure(response).map_err(|error| {
            anyhow::anyhow!(
                "sequential subagent role '{role_name}' returned invalid model response: {error}"
            )
        })?;
        self.validate_tool_calls(role_name, &response.tool_calls)
    }

    /// Не даёт model response расширить capability-набор за пределы tools,
    /// которые были видимы в соответствующем model request.
    ///
    /// Проверяется весь batch до первого вызова `ToolOrchestrator`: иначе
    /// разрешённый call в начале ответа мог бы исполниться до обнаружения
    /// скрытого call-а позже в том же ответе.
    pub(super) fn validate_tool_calls(
        &self,
        role_name: &str,
        tool_calls: &[ToolCall],
    ) -> Result<()> {
        for call in tool_calls {
            if !self.names.contains(call.name.as_str()) {
                bail!(
                    "sequential subagent role '{role_name}' model requested tool '{}' that was not present in the model request",
                    call.name
                );
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::domain::{ToolSafety, new_call_id};

    #[test]
    fn request_visible_names_are_exact_and_case_sensitive() {
        let tools = vec![ToolSpec::new(
            "read_file",
            "Read",
            json!({}),
            ToolSafety::ReadOnly,
        )];

        let visible = RequestVisibleTools::capture(&tools);
        visible
            .validate_tool_calls(
                "explore",
                &[ToolCall::new(new_call_id(), "read_file", json!({}))],
            )
            .expect("visible tool is accepted");

        let error = visible
            .validate_tool_calls(
                "explore",
                &[ToolCall::new(new_call_id(), "Read_File", json!({}))],
            )
            .expect_err("different tool name must be rejected");
        assert!(
            error
                .to_string()
                .contains("tool 'Read_File' that was not present in the model request")
        );
    }
}
