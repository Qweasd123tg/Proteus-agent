use anyhow::{Result, anyhow};
use async_trait::async_trait;

use crate::contracts::{
    RuntimeContext, SubagentRequest, SubagentResult, SubagentRoleSpec, SubagentRunner,
};

/// Slot `subagent` выключен: ролей нет, `run` возвращает ошибку.
///
/// Пустой `roles()` означает, что workflow не должен генерировать task-тул,
/// поэтому `run` в нормальном сценарии никогда не вызывается.
#[derive(Debug, Default)]
pub struct NoSubagent;

#[async_trait]
impl SubagentRunner for NoSubagent {
    fn roles(&self) -> Vec<SubagentRoleSpec> {
        Vec::new()
    }

    async fn run(&self, _request: SubagentRequest, _ctx: RuntimeContext) -> Result<SubagentResult> {
        Err(anyhow!("subagent slot is disabled (module 'none')"))
    }
}
