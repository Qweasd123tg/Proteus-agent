//! Model-visible terminal specifications and process tool entrypoints.
use super::{EXEC_SPEC_TIMEOUT_MS, WRITE_SPEC_TIMEOUT_MS, exec_command_impl, write_stdin_impl};
use proteus_contracts::process_module::{ProcessModuleError, ToolModule, ToolModuleHostMut};
use serde_json::json;

pub(crate) struct ExecCommandTool;

impl ToolModule for ExecCommandTool {
    fn spec_json(&self) -> String {
        let spec = json!({
            "name": "exec_command",
            "description": "Runs a shell command (sh -lc) with closed stdin and stdout/stderr pipes by default. Set `tty: true` for a PTY with interactive stdin. Waits up to `yield_time_ms` for output; if the process is still running, returns a Session ID for follow-up interaction via `write_stdin`. Non-escalated commands require bwrap and run with no network access, a private PID namespace, and a read-only filesystem outside the workspace; execution fails closed when bwrap is unavailable or disabled. The sandbox network is isolated per session: a localhost server started in a sandboxed session is unreachable from other tool calls and from the user's machine; start servers that must stay reachable with `with_escalated_permissions: true`. Set `with_escalated_permissions: true` with a short `justification` to request an unsandboxed run (requires user approval). Non-escalated workdirs must stay inside the workspace. Live sessions belong to the current runtime session/thread/workspace, expire after 30 minutes idle, and are killed when the invocation is cancelled. At most 16 live sessions: the least recently used one is killed to make room, so close finished sessions via write_stdin (Ctrl-C/Ctrl-D). Safety: RunsCommands.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "cmd": { "type": "string", "description": "Shell command to execute." },
                    "tty": { "type": "boolean", "description": "Allocate a PTY and keep stdin open for interactive input; defaults to false (pipes with closed stdin)." },
                    "workdir": {
                        "type": "string",
                        "description": "Working directory for the command; relative paths resolve against the workspace root. Defaults to the workspace root."
                    },
                    "yield_time_ms": {
                        "type": "integer",
                        "description": "How long to wait (in milliseconds) for output before yielding; 250-30000, default 10000."
                    },
                    "max_output_tokens": {
                        "type": "integer",
                        "description": "Approximate cap on returned output tokens; excess is truncated in the middle."
                    },
                    "with_escalated_permissions": {
                        "type": "boolean",
                        "description": "Request an unsandboxed run (network / writes outside workspace). Requires user approval."
                    },
                    "justification": {
                        "type": "string",
                        "description": "One sentence explaining why escalated permissions are needed."
                    }
                },
                "required": ["cmd"]
            },
            "surface": { "kind": "function", "strict": false, "output_schema": null },
            "safety": "RunsCommands",
            "supports_parallel_tool_calls": true,
            "timeout_ms": EXEC_SPEC_TIMEOUT_MS,
            "metadata": {
                "category": "terminal",
                "tags": ["terminal", "interactive", "session", "repl"],
                "aliases": ["interactive shell", "repl", "long-running command"]
            }
        });
        spec.to_string()
    }

    fn invoke_json(
        &self,
        call_json: String,
        context_json: String,
        host: &mut ToolModuleHostMut<'_>,
    ) -> Result<String, ProcessModuleError> {
        match exec_command_impl(call_json.as_str(), context_json.as_str(), host) {
            Ok(result_json) => Ok(result_json),
            Err(error) => Err(ProcessModuleError::new(format!("{error:#}"))),
        }
    }
}

pub(crate) struct WriteStdinTool;

impl ToolModule for WriteStdinTool {
    fn spec_json(&self) -> String {
        let spec = json!({
            "name": "write_stdin",
            "description": "Writes characters to a running exec_command session owned by the current runtime session/thread/workspace and returns output produced within `yield_time_ms`. Send \"\\u0003\" (Ctrl-C) to interrupt or \"\\u0004\" (Ctrl-D) to close PTY stdin; empty `chars` polls for more output. Without tty=true, only empty polls and Ctrl-C are accepted; other input fails because stdin is closed. Cancelling the invocation kills and removes the session. Safety: RunsCommands.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "session_id": {
                        "type": "integer",
                        "description": "Session ID returned by exec_command while the process was still running."
                    },
                    "chars": {
                        "type": "string",
                        "description": "Bytes to write to stdin (may be empty to poll for output)."
                    },
                    "yield_time_ms": {
                        "type": "integer",
                        "description": "How long to wait (in milliseconds) for output before yielding; non-empty writes: 250-30000, default 250; empty polls: 5000-300000, default 5000."
                    },
                    "max_output_tokens": {
                        "type": "integer",
                        "description": "Approximate cap on returned output tokens; excess is truncated in the middle."
                    }
                },
                "required": ["session_id"]
            },
            "surface": { "kind": "function", "strict": false, "output_schema": null },
            "safety": "RunsCommands",
            "supports_parallel_tool_calls": true,
            "timeout_ms": WRITE_SPEC_TIMEOUT_MS,
            "metadata": {
                "category": "terminal",
                "tags": ["terminal", "interactive", "session", "stdin"],
                "aliases": ["send input", "interrupt process", "poll output"]
            }
        });
        spec.to_string()
    }

    fn invoke_json(
        &self,
        call_json: String,
        context_json: String,
        host: &mut ToolModuleHostMut<'_>,
    ) -> Result<String, ProcessModuleError> {
        match write_stdin_impl(call_json.as_str(), context_json.as_str(), host) {
            Ok(result_json) => Ok(result_json),
            Err(error) => Err(ProcessModuleError::new(format!("{error:#}"))),
        }
    }
}
