use std::process::Stdio;

use anyhow::Result;
use async_trait::async_trait;
use serde_json::json;
use tokio::process::Command;

use crate::{
    contracts::{SearchBackend, SearchQuery},
    domain::ContextChunk,
    modules::process_output::{DEFAULT_PROCESS_OUTPUT_LIMIT_BYTES, wait_with_bounded_output},
};

#[derive(Debug)]
pub struct RgSearch {
    pub max_results: usize,
}

#[async_trait]
impl SearchBackend for RgSearch {
    async fn search(&self, query: SearchQuery) -> Result<Vec<ContextChunk>> {
        let max_results = query.max_results.min(self.max_results);
        let output = match Command::new("rg")
            .arg("--line-number")
            .arg("--no-heading")
            .arg("--color=never")
            .arg(&query.text)
            .current_dir(&query.cwd)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
        {
            Ok(child) => {
                wait_with_bounded_output(
                    child,
                    DEFAULT_PROCESS_OUTPUT_LIMIT_BYTES,
                    DEFAULT_PROCESS_OUTPUT_LIMIT_BYTES,
                )
                .await?
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error.into()),
        };

        if !output.status.success() && output.status.code() != Some(1) {
            return Ok(Vec::new());
        }

        let chunks = output
            .stdout
            .text
            .lines()
            .take(max_results)
            .filter_map(parse_rg_line)
            .collect();
        Ok(chunks)
    }
}

fn parse_rg_line(line: &str) -> Option<ContextChunk> {
    let mut parts = line.splitn(3, ':');
    let path = parts.next()?;
    let line_number = parts.next()?.parse::<usize>().ok()?;
    let content = parts.next()?.to_owned();
    Some(
        ContextChunk::new("rg", content)
            .with_path(path.into())
            .with_metadata(json!({ "line": line_number })),
    )
}
