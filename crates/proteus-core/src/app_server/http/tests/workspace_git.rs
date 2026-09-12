use super::*;
use std::{fs, path::Path, process::Command};

fn git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .current_dir(root)
        .args(["--literal-pathspecs"])
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn init(root: &Path) {
    git(root, &["init", "-q"]);
    git(root, &["config", "user.name", "Workspace fixture"]);
    git(root, &["config", "user.email", "fixture@example.invalid"]);
    git(root, &["config", "commit.gpgsign", "false"]);
}

fn changes(root: &Path) -> Value {
    serde_json::to_value(super::super::workspace::git::changes(root).unwrap()).unwrap()
}

fn diff(root: &Path, path: &str) -> Value {
    serde_json::to_value(super::super::workspace::git::diff(root, path.to_owned()).unwrap())
        .unwrap()
}

#[test]
fn workspace_git_combines_staged_and_unstaged_changes_against_head() {
    let root = tempfile::tempdir().unwrap();
    init(root.path());
    for name in [
        "edited.txt",
        "deleted.txt",
        "removed-index.txt",
        "old.txt",
        ":(glob)*.txt",
    ] {
        fs::write(root.path().join(name), "base\n").unwrap();
    }
    git(root.path(), &["add", "."]);
    git(root.path(), &["commit", "-qm", "base"]);
    fs::write(root.path().join("edited.txt"), "staged\n").unwrap();
    git(root.path(), &["add", "edited.txt"]);
    fs::write(root.path().join("edited.txt"), "working\n").unwrap();
    fs::remove_file(root.path().join("deleted.txt")).unwrap();
    git(root.path(), &["rm", "-q", "removed-index.txt"]);
    git(root.path(), &["mv", "old.txt", "renamed.txt"]);
    fs::write(root.path().join(":(glob)*.txt"), "literal\n").unwrap();
    fs::write(root.path().join("new name\nline.txt"), "new\n").unwrap();
    let body = changes(root.path());
    assert_eq!(body["repository"], true);
    assert_eq!(body["truncated"], false);
    for (path, status) in [
        ("edited.txt", "modified"),
        ("deleted.txt", "deleted"),
        ("removed-index.txt", "deleted"),
        ("renamed.txt", "renamed"),
        ("new name\nline.txt", "untracked"),
    ] {
        assert!(
            body["entries"]
                .as_array()
                .unwrap()
                .iter()
                .any(|e| e["path"] == path && e["status"] == status),
            "{body}"
        );
    }
    let patch = diff(root.path(), "edited.txt")["patch"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(
        patch.contains("-base\n") && patch.contains("+working\n") && !patch.contains("+staged\n")
    );
    for path in ["deleted.txt", "removed-index.txt"] {
        assert!(
            diff(root.path(), path)["patch"]
                .as_str()
                .unwrap()
                .contains("-base\n")
        );
    }
    assert!(
        diff(root.path(), ":(glob)*.txt")["patch"]
            .as_str()
            .unwrap()
            .contains("+literal\n")
    );
    assert!(
        diff(root.path(), "new name\nline.txt")["patch"]
            .as_str()
            .unwrap()
            .contains("+new\n")
    );
}

#[test]
fn workspace_git_handles_unborn_nonrepo_binary_and_output_limits() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("new.txt"), "new\n").unwrap();
    assert_eq!(changes(root.path())["repository"], false);
    assert_eq!(diff(root.path(), "new.txt")["kind"], "unavailable");
    init(root.path());
    git(root.path(), &["add", "new.txt"]);
    assert_eq!(changes(root.path())["entries"][0]["status"], "added");
    assert!(
        diff(root.path(), "new.txt")["patch"]
            .as_str()
            .unwrap()
            .contains("+new\n")
    );
    fs::write(root.path().join("binary"), [0, 255]).unwrap();
    assert_eq!(diff(root.path(), "binary")["kind"], "binary");
    fs::write(root.path().join("large"), vec![b'x'; 512 * 1024 + 1]).unwrap();
    assert_eq!(diff(root.path(), "large")["kind"], "too_large");
    // Each source is below the limit, but the deleted+added patch exceeds it.
    fs::write(root.path().join("bounded"), "a\n".repeat(100_000)).unwrap();
    git(root.path(), &["add", "bounded"]);
    git(root.path(), &["commit", "-qm", "base"]);
    fs::write(root.path().join("bounded"), "b\n".repeat(100_000)).unwrap();
    assert_eq!(diff(root.path(), "bounded")["kind"], "too_large");
    assert_eq!(diff(root.path(), "missing")["kind"], "unavailable");
    for i in 0..1001 {
        fs::write(root.path().join(format!("untracked-{i}")), "").unwrap();
    }
    let status = changes(root.path());
    assert_eq!(status["truncated"], true);
    assert_eq!(status["entries"].as_array().unwrap().len(), 1000);
}

#[test]
fn workspace_git_subdirectory_scope_and_deleted_path_validation() {
    let root = tempfile::tempdir().unwrap();
    init(root.path());
    fs::create_dir(root.path().join("sub")).unwrap();
    fs::write(root.path().join("outside"), "before\n").unwrap();
    fs::write(root.path().join("sub/inside"), "before\n").unwrap();
    git(root.path(), &["add", "."]);
    git(root.path(), &["commit", "-qm", "base"]);
    fs::write(root.path().join("outside"), "outside changed\n").unwrap();
    fs::remove_file(root.path().join("sub/inside")).unwrap();
    let sub = root.path().join("sub");
    assert_eq!(
        changes(&sub)["entries"],
        json!([{"path":"inside", "status":"deleted"}])
    );
    let patch = diff(&sub, "inside")["patch"].as_str().unwrap().to_owned();
    assert!(patch.contains("-before\n") && !patch.contains("outside"));
    for path in ["../outside", "/etc/passwd", ".", ""] {
        assert!(super::super::workspace::git::diff(&sub, path.to_owned()).is_err());
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(root.path(), sub.join("escape")).unwrap();
        for path in ["escape/outside", "escape/deleted"] {
            assert!(super::super::workspace::git::diff(&sub, path.to_owned()).is_err());
        }
    }
}

#[tokio::test]
async fn workspace_git_http_requires_auth_and_explicit_live_session() {
    let (state, server, root) = test_state().await;
    init(root.path());
    fs::write(root.path().join("file.txt"), "new\n").unwrap();
    for endpoint in ["/workspace/changes", "/workspace/diff"] {
        let unauthorized = route_request(
            state.clone(),
            Request::builder().uri(endpoint).body(empty_body()).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);
        let missing_session = route_request(state.clone(), authed_get_request(endpoint))
            .await
            .unwrap();
        assert_eq!(missing_session.status(), StatusCode::BAD_REQUEST);
        let uri = format!(
            "{}{}",
            session_uri(endpoint, &server),
            if endpoint.ends_with("diff") {
                "&path=file.txt"
            } else {
                ""
            }
        );
        let response = route_request(state.clone(), authed_get_request(&uri))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body: Value = serde_json::from_slice(&response_bytes(response).await).unwrap();
        if endpoint.ends_with("diff") {
            assert_eq!(body["kind"], "text");
        } else {
            assert_eq!(body["repository"], true);
        }
    }
    server.shutdown().await;
}
