use proteus_contracts::app_protocol::AppSessionSummary;
use std::path::{Path, PathBuf};

/// Browser state is reusable only while the server catalog still lists it.
/// The bootstrap suggestion is authoritative, including a fresh live session.
pub fn select_startup_session(
    requested: Option<String>,
    bootstrap: Option<PathBuf>,
    catalog: &[AppSessionSummary],
) -> Option<String> {
    requested
        .filter(|path| {
            catalog
                .iter()
                .any(|item| item.session_dir == Path::new(path))
        })
        .or_else(|| bootstrap.map(|path| path.to_string_lossy().into_owned()))
}
