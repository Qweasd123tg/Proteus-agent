//! One-shot shell output and completion rendering.
use crate::execution::ShellOutput;
use serde_json::{Value, json};

pub(super) fn render_output(
    call_id: String,
    output: ShellOutput,
    timed_out: bool,
    timeout_ms: u64,
    mut metadata: Value,
) -> String {
    let stdout = output.stdout.to_text();
    let stderr = output.stderr.to_text();
    let status = output.status.code();
    let success = output.status.success() && !timed_out;
    let mut rendered = stdout;
    if !stderr.is_empty() {
        if !rendered.is_empty() {
            rendered.push('\n');
        }
        rendered.push_str(&stderr);
    }
    let error_msg = if timed_out {
        Some(format!("process timed out after {timeout_ms}ms"))
    } else if !success {
        Some(match status {
            Some(code) => format!("process exited with code {code}"),
            None => "process terminated by signal".to_owned(),
        })
    } else {
        None
    };
    metadata["exit_code"] = json!(status);
    metadata["stdout_bytes"] = json!(output.stdout.original_len);
    metadata["stderr_bytes"] = json!(output.stderr.original_len);
    metadata["stdout_truncated"] = json!(output.stdout.truncated());
    metadata["stderr_truncated"] = json!(output.stderr.truncated());
    metadata["timed_out"] = json!(timed_out);
    metadata["timeout_ms"] = json!(timeout_ms);
    json!({
        "call_id": call_id, "ok": success, "output": rendered,
        "content": [], "error": error_msg, "metadata": metadata,
    })
    .to_string()
}
