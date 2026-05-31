//! Wire protocol re-exports из proteus-contracts.
//!
//! Типы `StdioRequest` и `StdioOutput` определены в `proteus-contracts::app_protocol`,
//! чтобы web/desktop-клиенты могли depend на них без зависимости на ядро.
//! Здесь — только re-export для обратной совместимости внутреннего кода.

pub use proteus_contracts::app_protocol::{StdioOutput, StdioRequest};

#[cfg(test)]
mod tests {
    use super::*;
    use proteus_contracts::contracts::{ApprovalCacheScope, UserInputResponse};
    use proteus_contracts::domain::PermissionMode;

    #[test]
    fn request_id_is_extracted_for_all_commands() {
        assert_eq!(
            StdioRequest::Send {
                id: Some("send".to_owned()),
                text: "hi".to_owned(),
            }
            .id(),
            Some("send".to_owned())
        );
        assert_eq!(
            StdioRequest::ClearHistory {
                id: Some("clear".to_owned()),
            }
            .id(),
            Some("clear".to_owned())
        );
        assert_eq!(
            StdioRequest::Approval {
                id: Some("approval".to_owned()),
                approval_id: "a1".to_owned(),
                approved: true,
                note: None,
                cache: ApprovalCacheScope::None,
            }
            .id(),
            Some("approval".to_owned())
        );
        assert_eq!(
            StdioRequest::UserInput {
                id: Some("user-input".to_owned()),
                request_id: "u1".to_owned(),
                response: UserInputResponse::empty(),
            }
            .id(),
            Some("user-input".to_owned())
        );
        assert_eq!(
            StdioRequest::SetPermissionMode {
                id: Some("mode".to_owned()),
                mode: PermissionMode::Plan,
            }
            .id(),
            Some("mode".to_owned())
        );
        assert_eq!(
            StdioRequest::SetReasoningEffort {
                id: Some("effort".to_owned()),
                effort: Some("medium".to_owned()),
            }
            .id(),
            Some("effort".to_owned())
        );
        assert_eq!(
            StdioRequest::SetModel {
                id: Some("model".to_owned()),
                model: "deepseek-v4-pro".to_owned(),
            }
            .id(),
            Some("model".to_owned())
        );
        assert_eq!(
            StdioRequest::SetReasoningEnabled {
                id: Some("reasoning".to_owned()),
                enabled: true,
            }
            .id(),
            Some("reasoning".to_owned())
        );
        assert_eq!(
            StdioRequest::ConfigSummary {
                id: Some("configs".to_owned()),
            }
            .id(),
            Some("configs".to_owned())
        );
    }

    #[test]
    fn output_uses_tagged_json_shape() {
        let output = StdioOutput::Response {
            id: Some("1".to_owned()),
            ok: true,
            output: None,
            error: None,
        };
        let json = serde_json::to_value(output).expect("stdio output serializes");
        assert_eq!(json["type"], "response");
        assert_eq!(json["id"], "1");
        assert_eq!(json["ok"], true);
    }

    #[test]
    fn approval_request_accepts_optional_cache_scope() {
        let without_cache: StdioRequest = serde_json::from_value(serde_json::json!({
            "type": "approval",
            "approval_id": "a1",
            "approved": true,
            "note": null
        }))
        .expect("approval request without cache deserializes");
        let with_cache: StdioRequest = serde_json::from_value(serde_json::json!({
            "type": "approval",
            "approval_id": "a1",
            "approved": true,
            "note": null,
            "cache": "exact_call"
        }))
        .expect("approval request with cache deserializes");
        let with_tool_cache: StdioRequest = serde_json::from_value(serde_json::json!({
            "type": "approval",
            "approval_id": "a1",
            "approved": true,
            "note": null,
            "cache": "tool_in_cwd"
        }))
        .expect("approval request with tool cache deserializes");

        match without_cache {
            StdioRequest::Approval { cache, .. } => assert_eq!(cache, ApprovalCacheScope::None),
            other => panic!("expected approval request, got {other:?}"),
        }
        match with_cache {
            StdioRequest::Approval { cache, .. } => {
                assert_eq!(cache, ApprovalCacheScope::ExactCall)
            }
            other => panic!("expected approval request, got {other:?}"),
        }
        match with_tool_cache {
            StdioRequest::Approval { cache, .. } => {
                assert_eq!(cache, ApprovalCacheScope::ToolInCwd)
            }
            other => panic!("expected approval request, got {other:?}"),
        }
    }
}
