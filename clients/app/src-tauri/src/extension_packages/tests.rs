use super::store::PackageStore;
use serde_json::{Value, json};
use std::{
    io::{Cursor, Write},
    path::PathBuf,
};

fn manifest() -> Value {
    json!({"apiVersion":3,"id":"archive-notes","name":"Archive notes","description":"",
        "icon":{"src":"assets/icon.svg"},"preview":{"src":"assets/preview.svg","alt":"Notes"},
        "views":[{"surfaces":["compact","workspace"],"entry":"panel.js","requires":[],"layout":"scroll","isolation":"shadow"}]})
}

fn zip(manifest: Value, extra: &[(&str, &[u8])]) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for (name, content) in [
        ("extension.json", serde_json::to_vec(&manifest).unwrap()),
        ("panel.js", b"export {mount} from './lib/view.mjs'".to_vec()),
        ("lib/view.mjs", b"export function mount() {}".to_vec()),
        (
            "assets/icon.svg",
            b"<svg xmlns='http://www.w3.org/2000/svg'/>".to_vec(),
        ),
        (
            "assets/preview.svg",
            b"<svg xmlns='http://www.w3.org/2000/svg'/>".to_vec(),
        ),
        ("assets/style.css", b"p {color: red}".to_vec()),
    ]
    .into_iter()
    .chain(extra.iter().map(|(name, bytes)| (*name, bytes.to_vec())))
    {
        writer.start_file(name, options).unwrap();
        writer.write_all(&content).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

#[test]
fn install_cold_read_assets_remove_and_reinstall_use_distinct_module_urls() {
    let directory = tempfile::tempdir().unwrap();
    let store = PackageStore::new(directory.path().to_owned());
    let bytes = zip(manifest(), &[]);
    let installed = store.install(&bytes, &[]).unwrap();
    assert_eq!(installed.id, "archive-notes");
    let cold = PackageStore::new(directory.path().to_owned());
    for (path, mime) in [
        ("panel.js", "text/javascript"),
        ("lib/view.mjs", "text/javascript"),
        ("assets/icon.svg", "image/svg+xml"),
        ("assets/style.css", "text/css"),
    ] {
        let (data, actual) = cold.read(&installed.key, path).unwrap();
        assert!(!data.is_empty());
        assert_eq!(actual, mime);
    }
    assert!(cold.read(&installed.key, "../outside").is_err());
    assert!(cold.read(&installed.key, "%2e%2e/outside").is_err());
    assert!(cold.read("../outside", "panel.js").is_err());
    cold.remove(&installed.key).unwrap();
    cold.remove(&installed.key).unwrap();
    assert!(cold.read(&installed.key, "panel.js").is_err());
    assert_ne!(installed.url, cold.install(&bytes, &[]).unwrap().url);
}

#[test]
fn malformed_or_incomplete_archives_and_reserved_ids_never_publish_files() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("packages");
    let store = PackageStore::new(root.clone());
    let mut cases = Vec::new();
    for change in [
        json!({"apiVersion":2}),
        json!({"unknown":true}),
        json!({"icon":"analysis"}),
        json!({"icon":null}),
        json!({"icon":{"src":"assets/missing.svg"}}),
        json!({"preview":{"src":"https://example.test/preview.svg","alt":"Preview"}}),
    ] {
        let mut value = manifest();
        value
            .as_object_mut()
            .unwrap()
            .extend(change.as_object().unwrap().clone());
        cases.push(zip(value, &[]));
    }
    for entry in ["../panel.js", "/panel.js", "absent.js"] {
        let mut value = manifest();
        value["views"][0]["entry"] = json!(entry);
        cases.push(zip(value, &[]));
    }
    cases.push(zip(manifest(), &[("../outside", b"outside")]));
    cases.push(zip(manifest(), &[("/outside", b"outside")]));
    cases.push(zip(
        manifest(),
        &[("./panel.js", b"duplicate normalized path")],
    ));
    cases.push(b"not a ZIP".to_vec());
    for bytes in cases {
        assert!(store.install(&bytes, &[]).is_err());
        assert!(!root.exists());
    }
    assert!(
        store
            .install(&zip(manifest(), &[]), &["archive-notes".into()])
            .is_err()
    );
    assert!(!root.exists());
}

#[test]
fn file_directory_collision_rolls_back_the_staging_directory() {
    let directory = tempfile::tempdir().unwrap();
    let store = PackageStore::new(directory.path().to_owned());
    let bytes = zip(
        manifest(),
        &[("collision", b"file"), ("collision/child", b"child")],
    );
    assert!(store.install(&bytes, &[]).is_err());
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    assert!(super::manifest::relative_path(".").is_err());
    assert_eq!(
        super::manifest::relative_path("./assets/icon.svg").unwrap(),
        PathBuf::from("assets/icon.svg")
    );
}
