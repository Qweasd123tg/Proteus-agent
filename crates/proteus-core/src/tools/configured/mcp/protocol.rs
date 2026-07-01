use std::{
    io::{BufRead, BufReader as StdBufReader, Write},
    sync::mpsc::{self, Receiver, RecvTimeoutError},
    time::{Duration, Instant},
};

use anyhow::{Result, anyhow, bail};
use serde_json::Value;
#[cfg(test)]
use tokio::io::{AsyncBufRead, AsyncBufReadExt};

use crate::core::process_output::DEFAULT_PROCESS_OUTPUT_LIMIT_BYTES;

const MCP_STDIO_RESPONSE_LIMIT_BYTES: usize = DEFAULT_PROCESS_OUTPUT_LIMIT_BYTES;

pub(super) fn spawn_sync_json_line_reader<R>(reader: R) -> Receiver<Result<Value>>
where
    R: std::io::Read + Send + 'static,
{
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut reader = StdBufReader::new(reader);
        loop {
            let value = sync_read_json_line(&mut reader);
            let done = value.is_err();
            if tx.send(value).is_err() || done {
                break;
            }
        }
    });
    rx
}

pub(super) fn recv_sync_jsonrpc_success(
    rx: &Receiver<Result<Value>>,
    expected_id: i64,
    timeout: Duration,
    child: &mut std::process::Child,
) -> Result<Value> {
    let started = Instant::now();
    loop {
        let elapsed = started.elapsed();
        if elapsed >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            bail!(
                "MCP server did not send response id {} within {}ms",
                expected_id,
                timeout.as_millis()
            );
        }

        let remaining = timeout - elapsed;
        let response = match rx.recv_timeout(remaining) {
            Ok(value) => value?,
            Err(RecvTimeoutError::Timeout) => {
                let _ = child.kill();
                let _ = child.wait();
                bail!(
                    "MCP server did not send response id {} within {}ms",
                    expected_id,
                    timeout.as_millis()
                );
            }
            Err(RecvTimeoutError::Disconnected) => bail!("MCP server stdout reader stopped"),
        };

        let Some(id) = response.get("id") else {
            continue;
        };
        let Some(id) = id.as_i64() else {
            bail!("MCP response id is not numeric: {id}");
        };
        if id != expected_id {
            bail!("MCP response id {id} did not match expected id {expected_id}");
        }
        return ensure_jsonrpc_success(&response, expected_id).cloned();
    }
}

pub(super) fn sync_write_json_line<W>(writer: &mut W, message: Value) -> Result<()>
where
    W: Write,
{
    writer.write_all(message.to_string().as_bytes())?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    Ok(())
}

fn sync_read_json_line<R>(reader: &mut R) -> Result<Value>
where
    R: BufRead,
{
    let mut line = Vec::with_capacity(MCP_STDIO_RESPONSE_LIMIT_BYTES.min(8192));
    loop {
        let buffer = reader.fill_buf()?;
        if buffer.is_empty() {
            if line.is_empty() {
                bail!("MCP server closed stdout before sending a response");
            }
            break;
        }

        let bytes_to_take = buffer
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(buffer.len(), |position| position + 1);
        if line.len().saturating_add(bytes_to_take) > MCP_STDIO_RESPONSE_LIMIT_BYTES {
            bail!("MCP response exceeded {MCP_STDIO_RESPONSE_LIMIT_BYTES} bytes before newline");
        }

        line.extend_from_slice(&buffer[..bytes_to_take]);
        reader.consume(bytes_to_take);

        if line.last() == Some(&b'\n') {
            break;
        }
    }
    if line.last() == Some(&b'\n') {
        line.pop();
    }
    if line.last() == Some(&b'\r') {
        line.pop();
    }
    let line = std::str::from_utf8(&line)?;
    serde_json::from_str(line).map_err(Into::into)
}

#[cfg(test)]
async fn read_json_line<R>(stdout: &mut R) -> Result<Value>
where
    R: AsyncBufRead + Unpin,
{
    let mut line = Vec::with_capacity(MCP_STDIO_RESPONSE_LIMIT_BYTES.min(8192));

    loop {
        let buffer = stdout.fill_buf().await?;
        if buffer.is_empty() {
            if line.is_empty() {
                bail!("MCP server closed stdout before sending a response");
            }
            break;
        }

        let bytes_to_take = buffer
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(buffer.len(), |position| position + 1);
        if line.len().saturating_add(bytes_to_take) > MCP_STDIO_RESPONSE_LIMIT_BYTES {
            bail!("MCP response exceeded {MCP_STDIO_RESPONSE_LIMIT_BYTES} bytes before newline");
        }

        line.extend_from_slice(&buffer[..bytes_to_take]);
        stdout.consume(bytes_to_take);

        if line.last() == Some(&b'\n') {
            break;
        }
    }

    if line.last() == Some(&b'\n') {
        line.pop();
    }
    if line.last() == Some(&b'\r') {
        line.pop();
    }

    let line = std::str::from_utf8(&line)?;
    serde_json::from_str(line).map_err(Into::into)
}

fn ensure_jsonrpc_success(response: &Value, expected_id: i64) -> Result<&Value> {
    let id = response
        .get("id")
        .and_then(Value::as_i64)
        .ok_or_else(|| anyhow!("MCP response missing numeric id"))?;
    if id != expected_id {
        bail!("MCP response id {id} did not match expected id {expected_id}");
    }
    if let Some(error) = response.get("error") {
        bail!("MCP error response: {error}");
    }
    response
        .get("result")
        .ok_or_else(|| anyhow!("MCP response missing result"))
}

pub(super) fn render_mcp_content(content: Option<&Value>) -> String {
    let Some(Value::Array(items)) = content else {
        return String::new();
    };
    items
        .iter()
        .map(|item| match item.get("type").and_then(Value::as_str) {
            Some("text") => item
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            _ => item.to_string(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mcp_json_line_rejects_oversized_response_without_newline() {
        let response = vec![b' '; MCP_STDIO_RESPONSE_LIMIT_BYTES + 1];
        let mut stdout = tokio::io::BufReader::new(&response[..]);

        let error = read_json_line(&mut stdout)
            .await
            .expect_err("oversized MCP response should fail");

        assert!(
            error
                .to_string()
                .contains("MCP response exceeded 20000 bytes before newline")
        );
    }

    #[test]
    fn sync_mcp_json_line_rejects_oversized_response_without_newline() {
        let response = vec![b' '; MCP_STDIO_RESPONSE_LIMIT_BYTES + 1];
        let mut stdout = StdBufReader::new(&response[..]);

        let error =
            sync_read_json_line(&mut stdout).expect_err("oversized MCP response should fail");

        assert!(
            error
                .to_string()
                .contains("MCP response exceeded 20000 bytes before newline")
        );
    }
}
