use super::*;
use crate::{
    core::SessionStore,
    domain::{new_session_id, new_thread_id},
    model_standard::{CanonicalMessage, MessageRole},
};
use std::{cell::Cell, time::Instant};

async fn saved(root: &Path, workspace: &Path, text: &str) -> SessionStore {
    let store = SessionStore::new(root, workspace, new_session_id()).unwrap();
    store
        .append_history(
            new_thread_id(),
            None,
            &[CanonicalMessage::text(MessageRole::User, text)],
        )
        .await
        .unwrap();
    store
}
fn counted(
    cache: &Mutex<SummaryCache>,
    dir: &Path,
    reads: &Cell<usize>,
) -> Result<AppSessionSummary> {
    load(cache, dir.to_owned(), |dir| {
        reads.set(reads.get() + 1);
        super::super::session_summary_from_dir(dir)
    })
}

#[tokio::test]
async fn warm_catalog_reuses_summary_but_append_replace_and_delete_are_visible() {
    let root = tempfile::tempdir().unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let store = saved(root.path(), workspace.path(), "first").await;
    let cache = Mutex::new(SummaryCache::default());
    let reads = Cell::new(0);
    let first = counted(&cache, store.session_dir(), &reads).unwrap();
    for _ in 0..5 {
        assert_eq!(counted(&cache, store.session_dir(), &reads).unwrap(), first);
    }
    assert_eq!(reads.get(), 1);
    store
        .append_history(
            new_thread_id(),
            None,
            &[CanonicalMessage::text(MessageRole::Assistant, "reply")],
        )
        .await
        .unwrap();
    assert_eq!(
        counted(&cache, store.session_dir(), &reads)
            .unwrap()
            .message_count,
        2
    );
    store
        .replace_history(
            new_thread_id(),
            None,
            &[CanonicalMessage::text(MessageRole::User, "replacement")],
            None,
        )
        .await
        .unwrap();
    let replaced = counted(&cache, store.session_dir(), &reads).unwrap();
    assert_eq!(replaced.message_count, 1);
    assert_eq!(replaced.preview.as_deref(), Some("replacement"));
    assert_eq!(reads.get(), 3);
    fs::remove_file(store.journal_path()).unwrap();
    assert_eq!(
        counted(&cache, store.session_dir(), &reads)
            .unwrap()
            .message_count,
        0
    );
    assert_eq!(reads.get(), 4);
}

#[tokio::test]
async fn warmed_cache_does_not_hide_corruption_even_with_preserved_mtime_and_size() {
    let workspace = tempfile::tempdir().unwrap();
    for target in ["metadata", "journal", "blob"] {
        let root = tempfile::tempdir().unwrap();
        let text = if target == "blob" {
            "large".repeat(60_000)
        } else {
            "small".into()
        };
        let store = saved(root.path(), workspace.path(), &text).await;
        let cache = Mutex::new(SummaryCache::default());
        let reads = Cell::new(0);
        counted(&cache, store.session_dir(), &reads).unwrap();
        let path = match target {
            "metadata" => store.session_dir().join("session.json"),
            "journal" => store.journal_path(),
            _ => fs::read_dir(store.session_dir().join("blobs"))
                .unwrap()
                .next()
                .unwrap()
                .unwrap()
                .path(),
        };
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        let mut bytes = fs::read(&path).unwrap();
        bytes[0] = b'!';
        fs::write(&path, bytes).unwrap();
        fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(modified))
            .unwrap();
        assert!(
            counted(&cache, store.session_dir(), &reads).is_err(),
            "{target} corruption was cached"
        );
        assert_eq!(reads.get(), 2);
        assert!(super::super::list_session_summaries_for_audit(root.path(), None).is_err());
    }
}

#[tokio::test]
async fn concurrent_change_is_not_cached_and_cache_is_bounded() {
    let root = tempfile::tempdir().unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let store = saved(root.path(), workspace.path(), "first").await;
    let cache = Mutex::new(SummaryCache::default());
    load(&cache, store.session_dir().to_owned(), |dir| {
        let result = super::super::session_summary_from_dir(dir)?;
        fs::write(store.journal_path(), b"{broken}\n")?;
        Ok(result)
    })
    .unwrap();
    assert!(cache.lock().unwrap().0.is_empty());
    assert!(counted(&cache, store.session_dir(), &Cell::new(0)).is_err());
    let valid = saved(root.path(), workspace.path(), "valid").await;
    let summary = super::super::session_summary_from_dir(valid.session_dir().to_owned()).unwrap();
    let mut cache = SummaryCache::default();
    for index in 0..MAX_SESSIONS + 10 {
        let mut item = summary.clone();
        item.session_dir = PathBuf::from(index.to_string());
        cache.insert(Fingerprint::read(valid.session_dir()).unwrap(), item);
    }
    assert_eq!(cache.0.len(), MAX_SESSIONS);
    assert!(cache.take(Path::new("0")).is_none());
}

#[tokio::test]
async fn removed_workspace_is_not_served_from_cache() {
    let root = tempfile::tempdir().unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let store = saved(root.path(), workspace.path(), "first").await;
    let cache = Mutex::new(SummaryCache::default());
    let reads = Cell::new(0);
    counted(&cache, store.session_dir(), &reads).unwrap();
    workspace.close().unwrap();
    assert!(counted(&cache, store.session_dir(), &reads).is_err());
}

#[tokio::test]
#[ignore = "explicit storage measurement, no timing assertion"]
async fn catalog_benchmark() {
    let root = tempfile::tempdir().unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let mut stores = Vec::new();
    let text = "catalog measurement ".repeat(100);
    for _ in 0..200 {
        let store = SessionStore::new(root.path(), workspace.path(), new_session_id()).unwrap();
        let messages = (0..40)
            .map(|_| CanonicalMessage::text(MessageRole::User, &text))
            .collect::<Vec<_>>();
        store
            .append_history(new_thread_id(), None, &messages)
            .await
            .unwrap();
        stores.push(store);
    }
    let cache = Mutex::new(SummaryCache::default());
    let reads = Cell::new(0);
    let baseline = Instant::now();
    let expected = stores
        .iter()
        .map(|s| super::super::session_summary_from_dir(s.session_dir().to_owned()).unwrap())
        .collect::<Vec<_>>();
    let baseline = baseline.elapsed();
    for store in &stores {
        counted(&cache, store.session_dir(), &reads).unwrap();
    }
    reads.set(0);
    let warm = Instant::now();
    let actual = stores
        .iter()
        .map(|s| counted(&cache, s.session_dir(), &reads).unwrap())
        .collect::<Vec<_>>();
    let warm = warm.elapsed();
    assert_eq!(actual, expected);
    assert_eq!(reads.get(), 0);
    stores[0]
        .append_history(
            new_thread_id(),
            None,
            &[CanonicalMessage::text(MessageRole::Assistant, "new")],
        )
        .await
        .unwrap();
    let changed = Instant::now();
    for store in &stores {
        counted(&cache, store.session_dir(), &reads).unwrap();
    }
    assert_eq!(reads.get(), 1);
    println!(
        "CATALOG_BENCH sessions=200 messages=8000 full_read_ms={:.2} warm_ms={:.2} one_changed_ms={:.2} changed_full_reads={}",
        baseline.as_secs_f64() * 1000.,
        warm.as_secs_f64() * 1000.,
        changed.elapsed().as_secs_f64() * 1000.,
        reads.get()
    );
}
