use std::path::Path;

use anyhow::{Result, anyhow, bail};
use serde::Serialize;
use serde_json::{Value, json};

const MAX_DIAGNOSTICS: usize = 100;
const MAX_MESSAGE_BYTES: usize = 2 * 1024;
const MAX_OUTPUT_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, Serialize)]
pub(crate) struct DiagnosticPosition {
    line: u64,
    character: u64,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct DiagnosticRange {
    start: DiagnosticPosition,
    end: DiagnosticPosition,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct RustDiagnostic {
    severity: String,
    code: Option<String>,
    source: Option<String>,
    message: String,
    range: DiagnosticRange,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct DiagnosticReport {
    pub(crate) document_version: i64,
    published_version: Option<i64>,
    diagnostic_count: usize,
    returned_diagnostics: usize,
    truncated: bool,
    diagnostics: Vec<RustDiagnostic>,
}

impl DiagnosticReport {
    pub(crate) fn render(&self, path: &Path) -> String {
        if self.diagnostic_count == 0 {
            return format!("No Rust diagnostics for {}.", path.display());
        }

        let mut output = format!(
            "Rust diagnostics for {} ({}):\n",
            path.display(),
            self.diagnostic_count
        );
        let mut rendered = 0usize;
        for diagnostic in &self.diagnostics {
            let code = diagnostic
                .code
                .as_deref()
                .map(|code| format!("[{code}]"))
                .unwrap_or_default();
            let source = diagnostic
                .source
                .as_deref()
                .map(|source| format!(" ({source})"))
                .unwrap_or_default();
            let line = format!(
                "{}{} {}:{}:{}-{}:{}{}: {}\n",
                diagnostic.severity,
                code,
                path.display(),
                diagnostic.range.start.line + 1,
                diagnostic.range.start.character + 1,
                diagnostic.range.end.line + 1,
                diagnostic.range.end.character + 1,
                source,
                diagnostic.message
            );
            if output.len().saturating_add(line.len()) > MAX_OUTPUT_BYTES {
                break;
            }
            output.push_str(&line);
            rendered += 1;
        }
        let omitted = self.diagnostic_count.saturating_sub(rendered);
        if omitted > 0 {
            output.push_str(&format!(
                "... {omitted} diagnostic(s) omitted by output bounds.\n"
            ));
        }
        output.trim_end().to_owned()
    }

    pub(crate) fn metadata(&self, path: &Path, uri: &str) -> Value {
        json!({
            "tool": "lsp_diagnostics",
            "language": "rust",
            "server": "rust-analyzer",
            "path": path,
            "uri": uri,
            "document_version": self.document_version,
            "published_version": self.published_version,
            "diagnostic_count": self.diagnostic_count,
            "returned_diagnostics": self.returned_diagnostics,
            "truncated": self.truncated,
            "diagnostics": self.diagnostics,
        })
    }
}

pub(crate) fn parse_publish_diagnostics(
    message: &Value,
    expected_uri: &str,
    document_version: i64,
) -> Result<Option<DiagnosticReport>> {
    if message.get("method").and_then(Value::as_str) != Some("textDocument/publishDiagnostics") {
        return Ok(None);
    }
    let params = message
        .get("params")
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow!("publishDiagnostics notification is missing object params"))?;
    let uri = params
        .get("uri")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("publishDiagnostics notification is missing uri"))?;
    if uri != expected_uri {
        return Ok(None);
    }
    let published_version = match params.get("version") {
        Some(Value::Null) | None => None,
        Some(value) => Some(
            value
                .as_i64()
                .ok_or_else(|| anyhow!("publishDiagnostics version is not an integer"))?,
        ),
    };
    if published_version.is_some_and(|version| version < document_version) {
        return Ok(None);
    }
    let values = params
        .get("diagnostics")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("publishDiagnostics notification is missing diagnostics"))?;
    let diagnostic_count = values.len();
    let diagnostics = values
        .iter()
        .take(MAX_DIAGNOSTICS)
        .map(parse_diagnostic)
        .collect::<Result<Vec<_>>>()?;
    let returned_diagnostics = diagnostics.len();

    Ok(Some(DiagnosticReport {
        document_version,
        published_version,
        diagnostic_count,
        returned_diagnostics,
        truncated: returned_diagnostics < diagnostic_count,
        diagnostics,
    }))
}

fn parse_diagnostic(value: &Value) -> Result<RustDiagnostic> {
    let object = value
        .as_object()
        .ok_or_else(|| anyhow!("diagnostic is not an object"))?;
    let message = object
        .get("message")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("diagnostic is missing message"))?;
    let range = object
        .get("range")
        .ok_or_else(|| anyhow!("diagnostic is missing range"))?;
    let code = object.get("code").and_then(render_code);
    let source = object
        .get("source")
        .and_then(Value::as_str)
        .map(str::to_owned);

    Ok(RustDiagnostic {
        severity: severity_name(object.get("severity")),
        code,
        source,
        message: truncate_utf8(&normalize_message(message), MAX_MESSAGE_BYTES),
        range: parse_range(range)?,
    })
}

fn parse_range(value: &Value) -> Result<DiagnosticRange> {
    let object = value
        .as_object()
        .ok_or_else(|| anyhow!("diagnostic range is not an object"))?;
    Ok(DiagnosticRange {
        start: parse_position(
            object
                .get("start")
                .ok_or_else(|| anyhow!("diagnostic range is missing start"))?,
        )?,
        end: parse_position(
            object
                .get("end")
                .ok_or_else(|| anyhow!("diagnostic range is missing end"))?,
        )?,
    })
}

fn parse_position(value: &Value) -> Result<DiagnosticPosition> {
    let object = value
        .as_object()
        .ok_or_else(|| anyhow!("diagnostic position is not an object"))?;
    let line = object
        .get("line")
        .and_then(Value::as_u64)
        .ok_or_else(|| anyhow!("diagnostic position is missing line"))?;
    let character = object
        .get("character")
        .and_then(Value::as_u64)
        .ok_or_else(|| anyhow!("diagnostic position is missing character"))?;
    if line > u32::MAX as u64 || character > u32::MAX as u64 {
        bail!("diagnostic position exceeds LSP u32 bounds");
    }
    Ok(DiagnosticPosition { line, character })
}

fn severity_name(value: Option<&Value>) -> String {
    match value.and_then(Value::as_u64) {
        Some(1) => "error",
        Some(2) => "warning",
        Some(3) => "information",
        Some(4) => "hint",
        _ => "diagnostic",
    }
    .to_owned()
}

fn render_code(value: &Value) -> Option<String> {
    match value {
        Value::String(code) => Some(code.clone()),
        Value::Number(code) => Some(code.to_string()),
        _ => None,
    }
}

fn normalize_message(message: &str) -> String {
    message.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn truncate_utf8(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_owned();
    }
    let mut end = max_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...", &value[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_renders_matching_diagnostics() {
        let notification = json!({
            "jsonrpc": "2.0",
            "method": "textDocument/publishDiagnostics",
            "params": {
                "uri": "file:///workspace/src/lib.rs",
                "version": 2,
                "diagnostics": [{
                    "range": {
                        "start": { "line": 1, "character": 2 },
                        "end": { "line": 1, "character": 5 }
                    },
                    "severity": 1,
                    "code": "E0308",
                    "source": "rustc",
                    "message": "mismatched\n types"
                }]
            }
        });

        let report = parse_publish_diagnostics(&notification, "file:///workspace/src/lib.rs", 2)
            .expect("valid notification")
            .expect("matching uri");

        assert_eq!(report.document_version, 2);
        assert!(
            report
                .render(Path::new("src/lib.rs"))
                .contains("error[E0308] src/lib.rs:2:3-2:6 (rustc): mismatched types")
        );
    }

    #[test]
    fn ignores_other_uris_and_stale_versions() {
        let other = json!({
            "method": "textDocument/publishDiagnostics",
            "params": { "uri": "file:///other.rs", "diagnostics": [] }
        });
        assert!(
            parse_publish_diagnostics(&other, "file:///wanted.rs", 1)
                .expect("notification")
                .is_none()
        );

        let stale = json!({
            "method": "textDocument/publishDiagnostics",
            "params": { "uri": "file:///wanted.rs", "version": 1, "diagnostics": [] }
        });
        assert!(
            parse_publish_diagnostics(&stale, "file:///wanted.rs", 2)
                .expect("notification")
                .is_none()
        );
    }
}
