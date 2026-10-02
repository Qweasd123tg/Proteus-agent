use super::*;

#[test]
fn package_refresh_preserves_personal_profiles_and_updates_managed_assets() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("package");
    let destination = temp.path().join("user");
    fs::create_dir_all(source.join("fragments")).unwrap();
    fs::create_dir_all(&destination).unwrap();
    fs::write(source.join("codex.config.toml"), "packaged").unwrap();
    fs::write(source.join("fragments/runtime.toml"), "first").unwrap();
    fs::write(destination.join("codex.config.toml"), "personal").unwrap();
    install_configs(&source, &destination).unwrap();
    fs::write(source.join("fragments/runtime.toml"), "updated").unwrap();
    install_configs(&source, &destination).unwrap();
    assert_eq!(
        fs::read_to_string(destination.join("codex.config.toml")).unwrap(),
        "personal"
    );
    assert_eq!(
        fs::read_to_string(destination.join("fragments/runtime.toml")).unwrap(),
        "updated"
    );
}
