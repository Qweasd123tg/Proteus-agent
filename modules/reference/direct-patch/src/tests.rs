use std::fs;

use super::*;

fn workspace() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("sample.txt"), "hello modular agent\n").unwrap();
    dir
}

#[test]
fn replaces_exact_text_once() {
    let dir = workspace();
    let result = apply_patch(
        "*** Begin Patch\n*** Update File: sample.txt\n@@\n-hello modular agent\n+patched modular agent\n*** End Patch",
        dir.path(),
    )
    .unwrap();

    assert!(result.ok);
    assert!(result.summary.contains("updated sample.txt"));
    assert_eq!(
        fs::read_to_string(dir.path().join("sample.txt")).unwrap(),
        "patched modular agent\n"
    );
}

#[test]
fn adds_new_file_from_internal_format() {
    let dir = workspace();
    let result = apply_patch(
        "*** Begin Patch\n*** Add File: nested/new.txt\n+hello\n+patch\n*** End Patch",
        dir.path(),
    )
    .unwrap();

    assert!(result.ok);
    assert!(result.summary.contains("added nested/new.txt"));
    assert_eq!(
        fs::read_to_string(dir.path().join("nested").join("new.txt")).unwrap(),
        "hello\npatch\n"
    );
}

#[test]
fn later_preflight_error_leaves_earlier_files_unchanged() {
    let dir = workspace();
    let error = apply_patch(
        "*** Begin Patch\n*** Update File: sample.txt\n@@\n-hello modular agent\n+changed before failure\n*** Update File: missing.txt\n@@\n-old\n+new\n*** End Patch",
        dir.path(),
    )
    .unwrap_err();

    assert!(error.contains("missing.txt"), "{error}");
    assert_eq!(
        fs::read_to_string(dir.path().join("sample.txt")).unwrap(),
        "hello modular agent\n"
    );
    assert!(!dir.path().join("missing.txt").exists());
}

#[test]
fn rejects_positional_hunk_header_without_modifying_repeated_text() {
    let dir = workspace();
    let path = dir.path().join("repeated.txt");
    fs::write(&path, "old\nseparator\nold\n").unwrap();

    let error = apply_patch(
        "*** Begin Patch\n*** Update File: repeated.txt\n@@ -3,1 +3,1 @@\n-old\n+changed\n*** End Patch",
        dir.path(),
    )
    .unwrap_err();

    assert!(error.contains("non-bare"), "{error}");
    assert_eq!(fs::read_to_string(path).unwrap(), "old\nseparator\nold\n");
}

#[test]
fn preflight_failure_does_not_create_directories_for_an_earlier_add() {
    let dir = workspace();
    let error = apply_patch(
        "*** Begin Patch\n*** Add File: new/nested/file.txt\n+new\n*** Delete File: missing.txt\n*** End Patch",
        dir.path(),
    )
    .unwrap_err();

    assert!(error.contains("missing.txt"), "{error}");
    assert!(!dir.path().join("new").exists());
}

#[test]
fn sequential_operations_use_the_planned_result_of_the_previous_operation() {
    let dir = workspace();
    let result = apply_patch(
        "*** Begin Patch\n*** Update File: sample.txt\n@@\n-hello modular agent\n+first update\n*** Update File: sample.txt\n*** Move to: ./moved.txt\n@@\n-first update\n+second update\n*** Update File: moved.txt\n@@\n-second update\n+final content\n*** End Patch",
        dir.path(),
    )
    .unwrap();

    assert!(result.ok);
    assert!(!dir.path().join("sample.txt").exists());
    assert_eq!(
        fs::read_to_string(dir.path().join("moved.txt")).unwrap(),
        "final content\n"
    );
}

#[cfg(unix)]
#[test]
fn update_preserves_file_permissions() {
    use std::os::unix::fs::PermissionsExt;

    let dir = workspace();
    let path = dir.path().join("sample.txt");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o744)).unwrap();

    apply_patch(
        "*** Begin Patch\n*** Update File: sample.txt\n@@\n-hello modular agent\n+still executable\n*** End Patch",
        dir.path(),
    )
    .unwrap();

    assert_eq!(
        fs::metadata(path).unwrap().permissions().mode() & 0o777,
        0o744
    );
}

#[test]
fn rejects_parent_traversal() {
    let dir = workspace();
    let error = apply_patch(
        "*** Begin Patch\n*** Add File: ../outside.txt\n+outside\n*** End Patch",
        dir.path(),
    )
    .unwrap_err();

    assert!(error.contains("escapes workspace"));
}

#[cfg(unix)]
#[test]
fn add_file_rejects_symlink_parent_without_creating_outside_dirs() {
    let dir = workspace();
    let outside = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(outside.path(), dir.path().join("link")).unwrap();

    let error = apply_patch(
        "*** Begin Patch\n*** Add File: link/new/file.txt\n+outside\n*** End Patch",
        dir.path(),
    )
    .unwrap_err();

    assert!(error.contains("symlink"), "{error}");
    assert!(!outside.path().join("new").exists());
}

#[cfg(unix)]
#[test]
fn add_file_rejects_dangling_final_symlink_without_creating_outside_file() {
    let dir = workspace();
    let outside = tempfile::tempdir().unwrap();
    let outside_file = outside.path().join("created.txt");
    std::os::unix::fs::symlink(&outside_file, dir.path().join("link.txt")).unwrap();

    let error = apply_patch(
        "*** Begin Patch\n*** Add File: link.txt\n+outside\n*** End Patch",
        dir.path(),
    )
    .unwrap_err();

    assert!(error.contains("symlink"), "{error}");
    assert!(!outside_file.exists());
    assert!(
        fs::symlink_metadata(dir.path().join("link.txt"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

#[cfg(unix)]
#[test]
fn update_rejects_internal_final_symlink_and_preserves_target() {
    let dir = workspace();
    let target = dir.path().join("target.txt");
    fs::write(&target, "original\n").unwrap();
    std::os::unix::fs::symlink("target.txt", dir.path().join("link.txt")).unwrap();

    let error = apply_patch(
        "*** Begin Patch\n*** Update File: link.txt\n@@\n-original\n+changed\n*** End Patch",
        dir.path(),
    )
    .unwrap_err();

    assert!(error.contains("symlink path"), "{error}");
    assert_eq!(fs::read_to_string(target).unwrap(), "original\n");
}

#[cfg(unix)]
#[test]
fn delete_rejects_internal_final_symlink_and_preserves_target() {
    let dir = workspace();
    let target = dir.path().join("target.txt");
    fs::write(&target, "original\n").unwrap();
    let link = dir.path().join("link.txt");
    std::os::unix::fs::symlink("target.txt", &link).unwrap();

    let error = apply_patch(
        "*** Begin Patch\n*** Delete File: link.txt\n*** End Patch",
        dir.path(),
    )
    .unwrap_err();

    assert!(error.contains("symlink path"), "{error}");
    assert_eq!(fs::read_to_string(target).unwrap(), "original\n");
    assert!(fs::symlink_metadata(link).is_ok());
}

#[cfg(unix)]
#[test]
fn move_rejects_internal_final_source_symlink_and_preserves_target() {
    let dir = workspace();
    let target = dir.path().join("target.txt");
    fs::write(&target, "original\n").unwrap();
    std::os::unix::fs::symlink("target.txt", dir.path().join("link.txt")).unwrap();

    let error = apply_patch(
        "*** Begin Patch\n*** Update File: link.txt\n*** Move to: moved.txt\n*** End Patch",
        dir.path(),
    )
    .unwrap_err();

    assert!(error.contains("symlink path"), "{error}");
    assert_eq!(fs::read_to_string(target).unwrap(), "original\n");
    assert!(!dir.path().join("moved.txt").exists());
}

#[cfg(unix)]
#[test]
fn move_rejects_final_destination_symlink_and_preserves_both_files() {
    let dir = workspace();
    let destination_target = dir.path().join("destination-target.txt");
    fs::write(&destination_target, "destination\n").unwrap();
    std::os::unix::fs::symlink(
        "destination-target.txt",
        dir.path().join("destination-link.txt"),
    )
    .unwrap();

    let error = apply_patch(
        "*** Begin Patch\n*** Update File: sample.txt\n*** Move to: destination-link.txt\n*** End Patch",
        dir.path(),
    )
    .unwrap_err();

    assert!(error.contains("symlink"), "{error}");
    assert_eq!(
        fs::read_to_string(dir.path().join("sample.txt")).unwrap(),
        "hello modular agent\n"
    );
    assert_eq!(
        fs::read_to_string(destination_target).unwrap(),
        "destination\n"
    );
}
