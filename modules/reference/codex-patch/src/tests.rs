use std::fs;

use super::apply_patch;

fn patch(body: &str) -> String {
    format!("*** Begin Patch\n{body}\n*** End Patch")
}

#[test]
fn context_and_eof_select_the_last_matching_block_and_keep_a_newline() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("f"),
        "fn first() {\nold\nfn last() {\nold\n",
    )
    .unwrap();
    let result = apply_patch(
        &patch("*** Update File: f\n@@ fn last() {\n-old\n+new\n*** End of File"),
        dir.path(),
    )
    .unwrap();
    assert_eq!(
        fs::read_to_string(dir.path().join("f")).unwrap(),
        "fn first() {\nold\nfn last() {\nnew\n"
    );
    assert_eq!(
        result.summary,
        "Success. Updated the following files:\nM f\n"
    );

    let error = apply_patch(
        &patch("*** Update File: f\n@@\n-old\n+bad\n*** End of File"),
        dir.path(),
    )
    .unwrap_err();
    assert!(error.contains("Failed to find expected lines"), "{error}");
    assert!(
        fs::read_to_string(dir.path().join("f"))
            .unwrap()
            .ends_with("new\n")
    );
}

#[test]
fn matching_uses_exact_before_whitespace_and_unicode_passes() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("f"), "  same  \nsame\n  “hello”—world\n").unwrap();
    apply_patch(
        &patch("*** Update File: f\n@@\n-same\n+exact\n@@\n-\"hello\"-world\n+normalized"),
        dir.path(),
    )
    .unwrap();
    assert_eq!(
        fs::read_to_string(dir.path().join("f")).unwrap(),
        "  same  \nexact\nnormalized\n"
    );
}

#[test]
fn bare_empty_context_and_missing_final_newline_follow_pinned_default() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("f"), "one\n\ntwo").unwrap();
    apply_patch(
        &patch("*** Update File: f\n one\n\n-two\n+three\n "),
        dir.path(),
    )
    .unwrap();
    assert_eq!(
        fs::read_to_string(dir.path().join("f")).unwrap(),
        "one\n\nthree\n"
    );
}

#[test]
fn pure_insertions_and_multiple_chunks_use_original_indices() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("f"), "a\nb\nc\n").unwrap();
    apply_patch(
        &patch("*** Update File: f\n@@\n+tail\n@@\n-b\n+B\n+extra\n@@\n-c\n+C"),
        dir.path(),
    )
    .unwrap();
    assert_eq!(
        fs::read_to_string(dir.path().join("f")).unwrap(),
        "a\nB\nextra\nC\ntail\n"
    );
}

#[test]
fn add_and_move_overwrite_destinations_and_summary_groups_operation_kinds() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("source"), "old\n").unwrap();
    fs::write(dir.path().join("dest"), [0xff]).unwrap();
    fs::write(dir.path().join("added"), [0xff]).unwrap();
    fs::write(dir.path().join("delete"), "gone").unwrap();
    let result = apply_patch(&patch("*** Delete File: delete\n*** Update File: source\n*** Move to: dest\n@@\n-old\n+new\n*** Add File: added\n+replacement\n*** Add File: nested/empty"), dir.path()).unwrap();
    assert_eq!(
        result.summary,
        "Success. Updated the following files:\nA added\nA nested/empty\nM dest\nD delete\n"
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("dest")).unwrap(),
        "new\n"
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("added")).unwrap(),
        "replacement\n"
    );
    assert_eq!(fs::read(dir.path().join("nested/empty")).unwrap(), b"");
    assert!(!dir.path().join("source").exists());
    assert!(!dir.path().join("delete").exists());
}

#[test]
fn verification_rejects_bad_later_update_duplicate_targets_and_move_only() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("f"), "original\n").unwrap();
    for body in [
        "*** Add File: added\n+ok\n*** Update File: f\n@@\n-missing\n+bad",
        "*** Add File: added\n+ok\n*** Add File: ./added\n+again",
        "*** Add File: added\n+ok\n*** Update File: f\n*** Move to: dest",
    ] {
        let error = apply_patch(&patch(body), dir.path()).unwrap_err();
        assert!(
            error.starts_with("apply_patch verification failed:"),
            "{error}"
        );
        assert!(!dir.path().join("added").exists());
        assert_eq!(
            fs::read_to_string(dir.path().join("f")).unwrap(),
            "original\n"
        );
    }
}

#[test]
fn application_failure_keeps_completed_writes() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("directory")).unwrap();
    let error = apply_patch(
        &patch("*** Add File: written\n+kept\n*** Add File: directory\n+cannot write"),
        dir.path(),
    )
    .unwrap_err();
    assert_eq!(
        error,
        format!(
            "Failed to write file {}",
            dir.path().join("directory").display()
        )
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("written")).unwrap(),
        "kept\n"
    );
}

#[test]
fn application_rereads_after_move_instead_of_using_stale_verified_contents() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("a"), "A\n").unwrap();
    fs::write(dir.path().join("b"), "B\n").unwrap();
    let error = apply_patch(&patch("*** Update File: a\n*** Move to: b\n@@\n-A\n+moved\n*** Update File: b\n@@\n-B\n+stale"), dir.path()).unwrap_err();
    assert!(
        error.starts_with("Failed to find expected lines"),
        "{error}"
    );
    assert!(!dir.path().join("a").exists());
    assert_eq!(fs::read_to_string(dir.path().join("b")).unwrap(), "moved\n");
}

#[test]
fn current_lenient_wrapper_empty_patch_and_environment_boundary_are_explicit() {
    let dir = tempfile::tempdir().unwrap();
    apply_patch(
        &format!("<<'EOF'\n{}\nEOF", patch("*** Add File: f\n+ok")),
        dir.path(),
    )
    .unwrap();
    assert_eq!(
        apply_patch("*** Begin Patch\n*** End Patch", dir.path()).unwrap_err(),
        "No files were modified."
    );
    assert!(
        apply_patch(
            &patch("*** Environment ID: remote\n*** Add File: g\n+bad"),
            dir.path()
        )
        .unwrap_err()
        .contains("environment selection is unavailable")
    );
    assert!(!dir.path().join("g").exists());
}

#[test]
fn workspace_path_policy_still_applies_before_any_writes() {
    let dir = tempfile::tempdir().unwrap();
    for path in [
        "../outside".to_owned(),
        dir.path().join("absolute").display().to_string(),
    ] {
        assert!(
            apply_patch(
                &patch(&format!(
                    "*** Add File: first\n+ok\n*** Add File: {path}\n+bad"
                )),
                dir.path()
            )
            .is_err()
        );
        assert!(!dir.path().join("first").exists());
    }
    #[cfg(unix)]
    {
        fs::create_dir(dir.path().join("real")).unwrap();
        std::os::unix::fs::symlink("real", dir.path().join("link")).unwrap();
        assert!(
            apply_patch(&patch("*** Add File: link/f\n+bad"), dir.path())
                .unwrap_err()
                .contains("symlink")
        );
        assert!(!dir.path().join("real/f").exists());
    }
}
