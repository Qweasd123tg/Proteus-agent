//! Optional presentation metadata emitted by tools, independent of module ids.
use crate::domain::ToolResult;
use agent_client_protocol::schema::v1::*;

pub(super) fn plan(result: &ToolResult) -> Option<SessionUpdate> {
    if !result.ok {
        return None;
    }
    let entries = result
        .metadata
        .get("plan")?
        .as_array()?
        .iter()
        .map(|step| {
            let content = step.get("step")?.as_str()?;
            let status = match step.get("status")?.as_str()? {
                "pending" => PlanEntryStatus::Pending,
                "in_progress" => PlanEntryStatus::InProgress,
                "completed" => PlanEntryStatus::Completed,
                _ => return None,
            };
            Some(PlanEntry::new(content, PlanEntryPriority::Medium, status))
        })
        .collect::<Option<Vec<_>>>()?;
    Some(SessionUpdate::Plan(Plan::new(entries)))
}
