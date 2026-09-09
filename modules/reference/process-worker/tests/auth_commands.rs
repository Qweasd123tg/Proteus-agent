use std::process::Command;

#[test]
fn provider_auth_commands_preserve_protocol_separation_and_hide_tokens() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("chatgpt.json");
    let invoke = |action: &str| {
        Command::new(env!("CARGO_BIN_EXE_proteus-reference-worker"))
            .args(["auth", "openai_codex", action, "--auth-file"])
            .arg(&file)
            .output()
            .unwrap()
    };
    let missing = invoke("status");
    assert!(missing.status.success());
    assert!(String::from_utf8_lossy(&missing.stdout).contains("вход не выполнен"));
    std::fs::write(
        &file,
        serde_json::json!({
            "access_token": "private-fixture-access", "refresh_token": "private-fixture-refresh",
            "account_id": "private-fixture-account", "expires_at": 4102444800u64,
        })
        .to_string(),
    )
    .unwrap();
    let status = invoke("status");
    assert!(status.status.success());
    assert!(!String::from_utf8_lossy(&status.stdout).contains("private-fixture"));
    assert!(!String::from_utf8_lossy(&status.stderr).contains("private-fixture"));
    assert!(file.exists());
    assert!(invoke("logout").status.success());
    assert!(!file.exists());
    assert!(invoke("logout").status.success());
    assert!(!invoke("typo").status.success());
    let unknown = Command::new(env!("CARGO_BIN_EXE_proteus-reference-worker"))
        .args(["auth", "unknown", "status"])
        .output()
        .unwrap();
    assert!(!unknown.status.success());
}
