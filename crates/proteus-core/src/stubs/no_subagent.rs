use anyhow::{Result, anyhow};
use async_trait::async_trait;

use crate::contracts::{
    RuntimeContext, SubagentRequest, SubagentResult, SubagentRoleSpec, SubagentRunner,
};

/// Structural absence для slot `subagent`: ролей нет, `run` возвращает ошибку.
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

    fn supports_collaboration(&self) -> bool {
        false
    }

    fn supports_collaboration_messages(&self) -> bool {
        false
    }

    async fn run(&self, _request: SubagentRequest, _ctx: RuntimeContext) -> Result<SubagentResult> {
        Err(anyhow!("no subagent module is selected"))
    }
}
