use super::*;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex as StdMutex, OnceLock, Weak},
};
use tokio::sync::Mutex;

pub(in crate::app_server) fn path_lock(path: &Path) -> Result<Arc<Mutex<()>>> {
    static LOCKS: OnceLock<StdMutex<HashMap<PathBuf, Weak<Mutex<()>>>>> = OnceLock::new();
    let absolute = std::path::absolute(path)?;
    let key = absolute
        .ancestors()
        .find_map(|ancestor| {
            std::fs::canonicalize(ancestor).ok().map(|canonical| {
                canonical.join(absolute.strip_prefix(ancestor).expect("path ancestor"))
            })
        })
        .ok_or_else(|| anyhow!("cannot resolve config path {}", path.display()))?;
    let mut locks = LOCKS
        .get_or_init(StdMutex::default)
        .lock()
        .map_err(|_| anyhow!("config path lock registry poisoned"))?;
    locks.retain(|_, lock| lock.strong_count() > 0);
    if let Some(lock) = locks.get(&key).and_then(Weak::upgrade) {
        return Ok(lock);
    }
    let lock = Arc::new(Mutex::new(()));
    locks.insert(key, Arc::downgrade(&lock));
    Ok(lock)
}
