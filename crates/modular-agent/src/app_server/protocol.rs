//! Wire protocol re-exports из agent-contracts.
//!
//! Типы `StdioRequest` и `StdioOutput` определены в `agent-contracts::app_protocol`,
//! чтобы клиенты (TUI, GUI) могли depend на них без зависимости на ядро.
//! Здесь — только re-export для обратной совместимости внутреннего кода.

pub use agent_contracts::app_protocol::{StdioOutput, StdioRequest};

#[cfg(test)]
mod tests {
    use super::*;
    use agent_contracts::contracts::ApprovalCacheScope;

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
    }
}
