use super::AppServerHandle;
use crate::domain::SessionUsageSnapshot;
use anyhow::Result;

impl AppServerHandle {
    pub async fn usage_snapshot(&self) -> Result<Option<SessionUsageSnapshot>> {
        self.runtime.usage_snapshot().await
    }
}
