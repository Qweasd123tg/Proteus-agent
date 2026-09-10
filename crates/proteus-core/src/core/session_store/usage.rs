use super::*;
use crate::{core::session_journal::UsageProjection, domain::SessionUsageSnapshot};

impl SessionStore {
    /// Живой writer отдаёт готовую проекцию; cold read валидирует журнал без
    /// захвата write ownership, исправления хвоста или изменения файлов.
    pub async fn usage_snapshot(&self) -> Result<SessionUsageSnapshot> {
        let writer = self.writer.lock().await;
        if let Some(snapshot) = writer.usage_snapshot(self.session_id) {
            return Ok(snapshot);
        }
        let store = self.clone();
        let snapshot = tokio::task::spawn_blocking(move || {
            let projection = store.load_projection()?;
            let mut usage = UsageProjection::default();
            for record in &projection.records {
                usage.apply(record);
            }
            Ok(usage.snapshot(store.session_id))
        })
        .await?;
        drop(writer);
        snapshot
    }
}

#[cfg(test)]
#[path = "usage_tests.rs"]
mod tests;
