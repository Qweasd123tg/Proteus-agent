//! Bounded, read-only acceleration for the UI catalog. Audit/replay never use it.
use std::{
    collections::VecDeque,
    ffi::OsString,
    fs::{self, Metadata},
    io::ErrorKind,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};

use anyhow::Result;
use proteus_contracts::app_protocol::AppSessionSummary;

const MAX_SESSIONS: usize = 512;
const MAX_FILE_STAMPS: usize = 8192;
const MAX_BLOBS: usize = 256;

#[derive(PartialEq, Eq)]
struct FileStamp {
    device: u64,
    inode: u64,
    bytes: u64,
    modified: (i64, i64),
    changed: (i64, i64),
    mode: u32,
}
impl From<Metadata> for FileStamp {
    fn from(m: Metadata) -> Self {
        Self {
            device: m.dev(),
            inode: m.ino(),
            bytes: m.len(),
            modified: (m.mtime(), m.mtime_nsec()),
            changed: (m.ctime(), m.ctime_nsec()),
            mode: m.mode(),
        }
    }
}

#[derive(PartialEq, Eq)]
struct Fingerprint {
    directory: FileStamp,
    metadata: FileStamp,
    journal: Option<FileStamp>,
    blobs_directory: Option<FileStamp>,
    blobs: Vec<(OsString, FileStamp)>,
}
impl Fingerprint {
    fn read(dir: &Path) -> Option<Self> {
        let directory = fs::metadata(dir).ok()?.into();
        let metadata = fs::metadata(dir.join("session.json")).ok()?.into();
        let journal = optional_stamp(&super::journal_path(dir))?;
        let blob_dir = dir.join("blobs");
        let blobs_directory = optional_stamp(&blob_dir)?;
        let mut blobs = Vec::new();
        if blobs_directory.is_some() {
            for entry in fs::read_dir(blob_dir).ok()? {
                let entry = entry.ok()?;
                if blobs.len() == MAX_BLOBS || !entry.file_type().ok()?.is_file() {
                    return None;
                }
                blobs.push((entry.file_name(), entry.metadata().ok()?.into()));
            }
            blobs.sort_by(|a, b| a.0.cmp(&b.0));
        }
        Some(Self {
            directory,
            metadata,
            journal,
            blobs_directory,
            blobs,
        })
    }
}
fn optional_stamp(path: &Path) -> Option<Option<FileStamp>> {
    match fs::metadata(path) {
        Ok(m) => Some(Some(m.into())),
        Err(e) if e.kind() == ErrorKind::NotFound => Some(None),
        Err(_) => None,
    }
}

struct CachedSummary {
    fingerprint: Fingerprint,
    summary: AppSessionSummary,
}
#[derive(Default)]
struct SummaryCache(VecDeque<CachedSummary>);
impl SummaryCache {
    fn take(&mut self, path: &Path) -> Option<CachedSummary> {
        let index = self
            .0
            .iter()
            .position(|entry| entry.summary.session_dir == path)?;
        self.0.remove(index)
    }
    fn get(&mut self, path: &Path, fingerprint: &Fingerprint) -> Option<AppSessionSummary> {
        let entry = self.take(path)?;
        if entry.fingerprint != *fingerprint {
            return None;
        }
        let summary = entry.summary.clone();
        self.0.push_back(entry);
        Some(summary)
    }
    fn insert(&mut self, fingerprint: Fingerprint, summary: AppSessionSummary) {
        self.take(&summary.session_dir);
        let mut files = self
            .0
            .iter()
            .map(|entry| entry.fingerprint.blobs.len() + 4)
            .sum::<usize>();
        let incoming = fingerprint.blobs.len() + 4;
        while self.0.len() >= MAX_SESSIONS || files + incoming > MAX_FILE_STAMPS {
            if let Some(old) = self.0.pop_front() {
                files -= old.fingerprint.blobs.len() + 4;
            }
        }
        self.0.push_back(CachedSummary {
            fingerprint,
            summary,
        });
    }
}

pub(super) fn summary(dir: PathBuf) -> Result<AppSessionSummary> {
    static CACHE: OnceLock<Mutex<SummaryCache>> = OnceLock::new();
    load(
        CACHE.get_or_init(Default::default),
        dir,
        super::session_summary_from_dir,
    )
}

fn load(
    cache: &Mutex<SummaryCache>,
    dir: PathBuf,
    read: impl FnOnce(PathBuf) -> Result<AppSessionSummary>,
) -> Result<AppSessionSummary> {
    // A removed or moved workspace must not remain visible through a cache hit.
    super::workspace_path_from_session_dir(&dir)?;
    let before = Fingerprint::read(&dir);
    if let Some(fingerprint) = &before {
        if let Some(summary) = cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&dir, fingerprint)
        {
            return Ok(summary);
        }
    } else {
        cache.lock().unwrap_or_else(|e| e.into_inner()).take(&dir);
    }
    // Full validation stays outside the global cache lock. Never cache errors or
    // a snapshot whose files changed while it was being read.
    let summary = read(dir.clone())?;
    if let Some(before) = before {
        if Fingerprint::read(&dir).as_ref() == Some(&before) {
            cache
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(before, summary.clone());
        }
    }
    Ok(summary)
}

#[cfg(test)]
mod tests;
