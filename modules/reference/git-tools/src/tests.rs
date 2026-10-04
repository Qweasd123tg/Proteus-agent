use super::*;

const _: () = assert!(TIMEOUT_MS >= 60_000);

fn invoke(tool_name: &str, cwd: &Path, args: Value) -> Value {
    let command = if tool_name == "git_status" {
        GitCommand::Status
    } else {
        GitCommand::Diff
    };
    let call = ToolCallDto {
        id: "call_test".to_owned(),
        name: tool_name.to_owned(),
        args,
    };
    let result = invoke_impl(&call, cwd, command).expect("tool result json");
    serde_json::from_str(&result).expect("result json")
}

fn git_available() -> bool {
    Command::new("git")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("run git");
    assert!(status.success(), "git {args:?} failed");
}

#[test]
fn git_status_reports_modified_file() {
    if !git_available() {
        return;
    }
    let dir = tempfile::tempdir().expect("workspace");
    git(dir.path(), &["init"]);
    std::fs::write(dir.path().join("notes.txt"), "one\n").expect("write file");

    let result = invoke("git_status", dir.path(), json!({}));

    assert_eq!(result["ok"], true);
    assert!(result["output"].as_str().unwrap().contains("notes.txt"));
}

#[test]
fn git_diff_supports_path_filter() {
    if !git_available() {
        return;
    }
    let dir = tempfile::tempdir().expect("workspace");
    git(dir.path(), &["init"]);
    git(dir.path(), &["config", "user.email", "test@example.com"]);
    git(dir.path(), &["config", "user.name", "Test"]);
    std::fs::write(dir.path().join("a.txt"), "one\n").expect("write a");
    std::fs::write(dir.path().join("b.txt"), "one\n").expect("write b");
    git(dir.path(), &["add", "."]);
    git(dir.path(), &["commit", "-m", "initial"]);
    std::fs::write(dir.path().join("a.txt"), "one\ntwo\n").expect("modify a");
    std::fs::write(dir.path().join("b.txt"), "one\nthree\n").expect("modify b");

    let result = invoke("git_diff", dir.path(), json!({ "path": "a.txt" }));

    assert_eq!(result["ok"], true);
    let output = result["output"].as_str().unwrap();
    assert!(output.contains("a.txt"), "{output}");
    assert!(!output.contains("b.txt"), "{output}");

    let zero = invoke(
        "git_diff",
        dir.path(),
        json!({"path":"a.txt", "context_lines":0}),
    );
    assert_eq!(zero["ok"], true);
    let zero_output = zero["output"].as_str().unwrap();
    assert!(zero_output.contains("+two"), "{zero_output}");
    assert!(
        !zero_output.lines().any(|line| line == " one"),
        "{zero_output}"
    );
    assert!(
        output.lines().any(|line| line == " one"),
        "default context lost: {output}"
    );
    let call = ToolCallDto {
        id: "zero-bytes".into(),
        name: "git_diff".into(),
        args: json!({"max_bytes":0}),
    };
    assert!(
        invoke_impl(&call, dir.path(), GitCommand::Diff)
            .unwrap_err()
            .contains("greater than zero")
    );
}

#[test]
fn pathspec_rejects_parent_escape() {
    let error = validate_relative_pathspec("../outside.txt").expect_err("reject parent");
    assert!(error.contains("parent traversal"), "{error}");
}
