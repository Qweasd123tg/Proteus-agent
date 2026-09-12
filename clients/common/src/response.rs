//! Decode a command envelope once, using the same DTO as the server.
use proteus_contracts::app_protocol::StdioOutput;
use serde::de::DeserializeOwned;

pub fn command_output<T: DeserializeOwned>(response: StdioOutput) -> Result<T, String> {
    match response {
        StdioOutput::Response {
            ok: true, output, ..
        } => {
            let output = output.ok_or("command succeeded without output")?;
            serde_json::from_value(output)
                .map_err(|error| format!("invalid command output: {error}"))
        }
        StdioOutput::Response { error, .. } => {
            Err(error.unwrap_or_else(|| "command failed without an error".into()))
        }
        StdioOutput::Event { .. } => Err("unexpected event instead of command response".into()),
    }
}
