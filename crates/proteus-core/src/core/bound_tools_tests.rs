use super::*;
use crate::domain::{HostedToolConfig, ToolSafety, ToolSurface, WebSearchHostedToolConfig};

#[test]
fn truncate_utf8_adds_visible_notice_within_limit() {
    // The second truncation of an already bounded tool result must retain its
    // final diagnostic, including under the real Core output budget.
    for (middle_bytes, limit) in [(112, 80), (250_000, DEFAULT_MAX_OUTPUT_BYTES)] {
        let original = format!("HEAD{}TAIL", "a".repeat(middle_bytes));
        let expected_bytes = original.len();
        let (output, truncated, original_bytes) = truncate_utf8(original, limit, "output");
        assert!(truncated);
        assert_eq!(original_bytes, expected_bytes);
        assert!(output.len() <= limit);
        assert!(output.starts_with("HEAD"), "{output}");
        assert!(output.ends_with("TAIL"));
        assert!(output.contains("[tool output truncated:"));
        assert!(output.contains(&format!("of {expected_bytes} bytes")));
    }
}

#[test]
fn truncate_utf8_preserves_character_boundaries() {
    let original = format!("🙂HEAD{}TAIL🙂", "й".repeat(80));
    for limit in [0, 1, 32, 96, 97] {
        let (output, truncated, original_bytes) = truncate_utf8(original.clone(), limit, "error");
        assert!(truncated);
        assert_eq!(original_bytes, original.len());
        assert!(output.len() <= limit);
        if limit >= 96 {
            assert!(output.starts_with("🙂HEAD"));
            assert!(output.ends_with("TAIL🙂"));
            assert!(output.contains("[tool error truncated:"));
        }
    }
}

#[test]
fn interactive_ask_keeps_local_tool_visible_but_hides_provider_hosted_tool() {
    let local = ToolSpec::new(
        "shell",
        "Run a command",
        json!({ "type": "object" }),
        ToolSafety::RunsCommands,
    );
    let hosted = ToolSpec::new(
        "web_search",
        "Search the web",
        json!({ "type": "object" }),
        ToolSafety::Network,
    )
    .with_surface(ToolSurface::provider_hosted(HostedToolConfig::WebSearch {
        config: WebSearchHostedToolConfig::default(),
    }));
    let ask = || PolicyDecision::Ask {
        reason: "approval required".to_owned(),
    };
    assert!(visibility_decision_allows(&local, ask(), true));
    assert!(!visibility_decision_allows(&hosted, ask(), true));
    assert!(visibility_decision_allows(
        &hosted,
        PolicyDecision::Allow,
        false
    ));
}
