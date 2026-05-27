use std::process::ExitStatus;

use anyhow::{Context, Result};
use serde_json::{Value, json};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::Child,
};

pub(crate) const DEFAULT_PROCESS_OUTPUT_LIMIT_BYTES: usize = 20_000;

#[derive(Debug, Clone)]
pub(crate) struct BoundedProcessOutput {
    pub status: ExitStatus,
    pub stdout: BoundedStreamOutput,
    pub stderr: BoundedStreamOutput,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct BoundedStreamOutput {
    pub text: String,
    pub original_bytes: usize,
    pub truncated: bool,
}

pub(crate) async fn wait_with_bounded_output(
    mut child: Child,
    max_stdout_bytes: usize,
    max_stderr_bytes: usize,
) -> Result<BoundedProcessOutput> {
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    let stdout_task = tokio::spawn(async move {
        match stdout {
            Some(stdout) => read_bounded(stdout, max_stdout_bytes).await,
            None => Ok(BoundedStreamOutput::default()),
        }
    });
    let stderr_task = tokio::spawn(async move {
        match stderr {
            Some(stderr) => read_bounded(stderr, max_stderr_bytes).await,
            None => Ok(BoundedStreamOutput::default()),
        }
    });

    let status = child.wait().await?;
    let stdout = stdout_task.await.context("stdout reader task failed")??;
    let stderr = stderr_task.await.context("stderr reader task failed")??;

    Ok(BoundedProcessOutput {
        status,
        stdout,
        stderr,
    })
}

pub(crate) fn annotate_bounded_output(
    metadata: Value,
    output: &BoundedProcessOutput,
    max_stdout_bytes: usize,
    max_stderr_bytes: usize,
) -> Value {
    let mut metadata = match metadata {
        Value::Object(object) => object,
        _ => serde_json::Map::new(),
    };

    if output.stdout.truncated {
        metadata.insert("stdout_truncated".to_owned(), json!(true));
        metadata.insert(
            "stdout_original_bytes".to_owned(),
            json!(output.stdout.original_bytes),
        );
        metadata.insert("stdout_max_bytes".to_owned(), json!(max_stdout_bytes));
    }

    if output.stderr.truncated {
        metadata.insert("stderr_truncated".to_owned(), json!(true));
        metadata.insert(
            "stderr_original_bytes".to_owned(),
            json!(output.stderr.original_bytes),
        );
        metadata.insert("stderr_max_bytes".to_owned(), json!(max_stderr_bytes));
    }

    Value::Object(metadata)
}

async fn read_bounded<R>(mut reader: R, max_bytes: usize) -> Result<BoundedStreamOutput>
where
    R: AsyncRead + Unpin,
{
    let mut stored = Vec::with_capacity(max_bytes.min(8192));
    let mut original_bytes = 0usize;
    let mut buffer = [0u8; 8192];

    loop {
        let bytes = reader.read(&mut buffer).await?;
        if bytes == 0 {
            break;
        }

        original_bytes = original_bytes.saturating_add(bytes);
        if stored.len() < max_bytes {
            let remaining = max_bytes - stored.len();
            let take = bytes.min(remaining);
            stored.extend_from_slice(&buffer[..take]);
        }
    }

    Ok(BoundedStreamOutput {
        text: String::from_utf8_lossy(&stored).into_owned(),
        original_bytes,
        truncated: original_bytes > stored.len(),
    })
}

#[cfg(test)]
mod tests {
    use std::process::Stdio;

    use tokio::process::Command;

    use super::*;

    #[tokio::test]
    async fn bounded_reader_discards_output_after_limit_without_losing_status() {
        let child = Command::new("sh")
            .arg("-c")
            .arg("i=0; while [ \"$i\" -lt 5000 ]; do printf 0123456789; i=$((i+1)); done")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .expect("spawn child");

        let output = wait_with_bounded_output(child, 1024, 1024)
            .await
            .expect("bounded output");

        assert!(output.status.success());
        assert_eq!(output.stdout.text.len(), 1024);
        assert_eq!(output.stdout.original_bytes, 50_000);
        assert!(output.stdout.truncated);
    }
}
