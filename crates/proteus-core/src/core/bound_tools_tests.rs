use super::*;
use crate::domain::{HostedToolConfig, ToolSafety, ToolSurface, WebSearchHostedToolConfig};

#[test]
fn truncate_utf8_adds_visible_notice_within_limit() {
    let original = "a".repeat(120);
    let (output, truncated, original_bytes) = truncate_utf8(original, 80, "output");
    assert!(truncated);
    assert_eq!(original_bytes, 120);
    assert!(output.len() <= 80);
    assert!(output.contains("[tool output truncated:"));
    assert!(output.contains("of 120 bytes"));
}

#[test]
fn truncate_utf8_preserves_character_boundaries() {
    let original = "й".repeat(80);
    let (output, truncated, original_bytes) = truncate_utf8(original, 96, "error");
    assert!(truncated);
    assert_eq!(original_bytes, 160);
    assert!(output.len() <= 96);
    assert!(output.is_char_boundary(output.len()));
    assert!(output.contains("[tool error truncated:"));
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
