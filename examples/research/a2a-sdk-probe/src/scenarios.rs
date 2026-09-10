use std::time::Duration;

use a2a::{A2AError, StreamResponse, SubscribeToTaskRequest, Task, TaskState};
use anyhow::{Result, anyhow};
use futures::StreamExt;
use serde_json::{Value, json};

use crate::fixture::{Probe, request};

pub async fn run() -> Result<Vec<Value>> {
    let mut checks = Vec::new();
    let probe = Probe::start().await?;

    let first = probe.send(request("first", None, None)).await?;
    checks.push(json!({
        "name": "task_result", "requirement": "basic task delegation",
        "passed": first.status.state == TaskState::Completed && output(&first) == Some("first"),
        "observed": {"state": first.status.state, "output": output(&first)},
    }));

    let followup = probe
        .send(request("next", None, Some(&first.context_id)))
        .await?;
    checks.push(json!({
        "name": "followup_in_same_context", "requirement": "new task within existing context",
        "passed": followup.id != first.id && followup.context_id == first.context_id
            && followup.status.state == TaskState::Completed && output(&followup) == Some("next"),
        "observed": {"same_context": followup.context_id == first.context_id, "new_task": followup.id != first.id},
        "limit": "context identity only; the SDK does not supply Proteus conversation history",
    }));

    let waiting = probe.send(request("ask", None, None)).await?;
    let answered = probe
        .send(request(
            "answer",
            Some(&waiting.id),
            Some(&waiting.context_id),
        ))
        .await?;
    checks.push(json!({
        "name": "input_required_resume", "requirement": "answer an interrupted task",
        "passed": waiting.status.state == TaskState::InputRequired && answered.id == waiting.id
            && answered.context_id == waiting.context_id && answered.status.state == TaskState::Completed
            && output(&answered) == Some("answer"),
        "observed": {"before": waiting.status.state, "after": answered.status.state},
        "limit": "the executor stream ends at InputRequired; live Proteus approvals are not bridged",
    }));

    let running = probe.send(request("hold", None, None)).await?;
    let steering = probe
        .send(request(
            "steering",
            Some(&running.id),
            Some(&running.context_id),
        ))
        .await;
    let delivered = probe.received().iter().any(|text| text == "steering");
    checks.push(json!({
        "name": "message_while_working", "requirement": "existing Proteus send_message behavior",
        "passed": steering.is_ok() && delivered,
        "observed": {"response": response(&steering), "executor_received_message": delivered},
        "classification": "SDK default handler limitation; not a claim that the A2A protocol forbids steering",
    }));

    // Independent task state on the same server; this does not prove process isolation.
    let sibling = probe.send(request("sibling", None, None)).await?;
    let canceled = probe.cancel(&running.id).await?;
    let after_cancel = probe.get(&running.id).await?;
    let sibling_after = probe.get(&sibling.id).await?;
    checks.push(json!({
        "name": "targeted_cancel", "requirement": "cancel task without changing another task",
        "passed": canceled.status.state == TaskState::Canceled
            && after_cancel.status.state == TaskState::Canceled && sibling_after == sibling,
        "observed": {"target": after_cancel.status.state, "sibling": sibling_after.status.state},
        "limit": "cooperative fixture cancellation; no process kill or cancel/delivery race proof",
    }));

    let terminal_send = probe
        .send(request("late", Some(&first.id), Some(&first.context_id)))
        .await;
    let executed_late = probe.received().iter().any(|text| text == "late");
    let explicitly_rejected = terminal_send
        .as_ref()
        .err()
        .is_some_and(|error| error.code == a2a::error_code::UNSUPPORTED_OPERATION);
    checks.push(json!({
        "name": "terminal_task_rejects_message_before_executor", "requirement": "A2A terminal task semantics",
        "passed": explicitly_rejected && !executed_late,
        "observed": {"response": response(&terminal_send), "executor_processed_late_message": executed_late},
    }));

    checks.push(subscription_reconnect(&probe).await?);
    Ok(checks)
}

async fn subscription_reconnect(probe: &Probe) -> Result<Value> {
    let mut initial = probe
        .client
        .send_streaming_message(&request("hold", None, None))
        .await?;
    let task = match tokio::time::timeout(Duration::from_secs(5), initial.next())
        .await?
        .ok_or_else(|| anyhow!("missing initial stream event"))??
    {
        StreamResponse::Task(task) => task,
        other => return Err(anyhow!("fixture initial event is not a Task: {other:?}")),
    };
    drop(initial);
    let running = probe.get(&task.id).await?;
    let mut reconnected = probe
        .client
        .subscribe_to_task(&SubscribeToTaskRequest {
            id: task.id.clone(),
            tenant: None,
        })
        .await?;
    let snapshot = tokio::time::timeout(Duration::from_secs(5), reconnected.next())
        .await?
        .ok_or_else(|| anyhow!("missing subscription snapshot"))??;
    probe.release(&task.id)?;
    let terminal = tokio::time::timeout(Duration::from_secs(5), reconnected.next())
        .await?
        .ok_or_else(|| anyhow!("missing terminal stream event"))??;
    let complete = matches!(&terminal, StreamResponse::Task(t)
        if t.id == task.id && t.status.state == TaskState::Completed);
    let snapshot_matches = matches!(&snapshot, StreamResponse::Task(t) if t == &running);
    let end = tokio::time::timeout(Duration::from_secs(5), reconnected.next()).await?;
    Ok(json!({
        "name": "subscription_reconnect", "requirement": "task survives a dropped SSE subscriber",
        "passed": running.status.state == TaskState::Working && snapshot_matches && complete && end.is_none(),
        "observed": {"snapshot_matches_get": snapshot_matches, "completed": complete, "stream_closed": end.is_none()},
        "limit": "same live SDK server; not restart recovery or durable storage",
    }))
}

fn output(task: &Task) -> Option<&str> {
    task.status
        .message
        .as_ref()
        .and_then(|message| message.text())
}

fn response(result: &Result<Task, A2AError>) -> Value {
    match result {
        Ok(task) => json!({"ok": true, "state": task.status.state, "output": output(task)}),
        Err(error) => json!({"ok": false, "code": error.code, "message": error.message}),
    }
}
