use super::*;
use crate::domain::{HostedToolConfig, ToolSafety, ToolSurface, WebSearchHostedToolConfig};

#[test]
fn extract_apply_patch_body_supports_heredoc_quotes_and_bare() {
    let patch = "*** Begin Patch\n*** Add File: hi.txt\n+hi\n*** End Patch";
    let heredoc = format!("apply_patch <<'EOF'\n{patch}\nEOF");
    assert_eq!(extract_apply_patch_body(&heredoc).as_deref(), Some(patch));
    let heredoc_plain = format!("apply_patch <<EOF\n{patch}\nEOF\n");
    assert_eq!(
        extract_apply_patch_body(&heredoc_plain).as_deref(),
        Some(patch)
    );
    let quoted = format!("apply_patch '{patch}'");
    assert_eq!(extract_apply_patch_body(&quoted).as_deref(), Some(patch));
    let bare = format!("apply_patch {patch}");
    assert_eq!(extract_apply_patch_body(&bare).as_deref(), Some(patch));
}

#[test]
fn extract_apply_patch_body_rejects_non_patch_commands() {
    assert_eq!(extract_apply_patch_body("cargo test"), None);
    assert_eq!(extract_apply_patch_body("apply_patch --help"), None);
    assert_eq!(
        extract_apply_patch_body("apply_patch <<'EOF'\nnot a patch\nEOF"),
        None
    );
    assert_eq!(
        extract_apply_patch_body("echo apply_patch <<'EOF'\n*** Begin Patch\nEOF"),
        None
    );
}

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
