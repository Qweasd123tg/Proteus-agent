use std::process::Command;

#[test]
fn malformed_commands_fail_before_config_loading_or_inference() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("broken.toml");
    std::fs::write(&config, "invalid = [").unwrap();
    for args in [
        vec!["server", "stdio", "--new-session"],
        vec!["server"],
        vec!["server", "web"],
        vec!["server", "http", "--new-session"],
        vec!["server", "a2a", "--host", "0.0.0.0"],
        vec!["server", "a2a", "--port", "1", "--port", "2"],
        vec!["modules"],
        vec!["modules", "list", "extra"],
        vec!["tools", "ls"],
        vec!["doctor", "extra"],
        vec!["inspect", "plan", "--unknown"],
        vec!["replay", "unknown"],
        vec!["eval", "report"],
        vec!["init", "coding", "extra"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_proteus"))
            .arg("--config")
            .arg(&config)
            .args(&args)
            .current_dir(dir.path())
            .output()
            .unwrap();
        assert!(!output.status.success(), "{args:?}");
        assert!(output.stdout.is_empty(), "{args:?}: unexpected stdout");
        let error = String::from_utf8(output.stderr).unwrap();
        assert!(error.contains("usage: proteus"), "{args:?}: {error}");
        assert!(!error.contains("broken.toml"), "config was loaded: {error}");
    }
    assert!(!dir.path().join("sessions").exists());
}
