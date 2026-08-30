use std::{
    env, fs,
    io::{self, BufReader, Write},
    path::Path,
    time::Duration,
};

use anyhow::{Result, anyhow, bail};
use proteus_contracts::{
    contracts::ExecutionAttribution,
    domain::{ToolResult, new_execution_id},
    process_module::{ProcessModuleError, ToolModule, ToolModuleHost, ToolModuleInvocationContext},
};
use proteus_process_host::{ContentLengthFraming, Framing};
use rust_lsp::RustLspDiagnosticsTool;
use serde_json::{Value, json};

const TEST_TIMEOUT: Duration = Duration::from_secs(2);

fn main() {
    let result = if env::args().nth(1).as_deref() == Some("__mock_lsp") {
        run_mock_lsp()
    } else {
        run_tests()
    };
    if let Err(error) = result {
        eprintln!("{error:#}");
        std::process::exit(1);
    }
}

type NamedTest = (&'static str, fn() -> Result<()>);

fn run_tests() -> Result<()> {
    let tests: &[NamedTest] = &[
        (
            "persistent_did_open_and_did_change_diagnostics",
            persistent_did_open_and_did_change_diagnostics,
        ),
        (
            "missing_rust_analyzer_is_a_failed_tool_result",
            missing_rust_analyzer_is_a_failed_tool_result,
        ),
    ];
    let mut failed = 0usize;
    for (name, test) in tests {
        print!("test {name} ... ");
        io::stdout().flush()?;
        match test() {
            Ok(()) => println!("ok"),
            Err(error) => {
                failed += 1;
                println!("FAILED");
                eprintln!("{name}: {error:#}");
            }
        }
    }
    if failed > 0 {
        bail!("{failed} rust-lsp protocol test(s) failed");
    }
    Ok(())
}

fn persistent_did_open_and_did_change_diagnostics() -> Result<()> {
    let workspace = tempfile::tempdir()?;
    fs::create_dir_all(workspace.path().join("src"))?;
    let source = workspace.path().join("src/lib.rs");
    fs::write(&source, "fn value() -> u32 { broken }\n")?;
    let command = env::current_exe()?;
    let tool = RustLspDiagnosticsTool::with_process(
        command.to_string_lossy(),
        ["__mock_lsp"],
        TEST_TIMEOUT,
        TEST_TIMEOUT,
    );

    let first = invoke(&tool, workspace.path(), "src/lib.rs")?;
    if !first.ok {
        bail!("first diagnostics failed: {first:?}");
    }
    if !first.output.contains("error[E0308]") {
        bail!("first diagnostics missing mock error: {}", first.output);
    }
    if first.metadata["document_version"] != 1 {
        bail!("first document version was not 1: {}", first.metadata);
    }

    fs::write(&source, "fn value() -> u32 { 42 }\n")?;
    let second = invoke(&tool, workspace.path(), "src/lib.rs")?;
    if !second.ok {
        bail!("second diagnostics failed: {second:?}");
    }
    if second.output != "No Rust diagnostics for src/lib.rs." {
        bail!("unexpected second diagnostics: {}", second.output);
    }
    if second.metadata["document_version"] != 2 {
        bail!(
            "persistent client did not advance to version 2: {}",
            second.metadata
        );
    }
    Ok(())
}

fn missing_rust_analyzer_is_a_failed_tool_result() -> Result<()> {
    let workspace = tempfile::tempdir()?;
    fs::write(workspace.path().join("lib.rs"), "fn main() {}\n")?;
    let tool = RustLspDiagnosticsTool::with_process(
        "/definitely/missing/proteus-rust-analyzer",
        std::iter::empty::<String>(),
        Duration::from_millis(100),
        Duration::from_millis(100),
    );

    let result = invoke(&tool, workspace.path(), "lib.rs")?;

    if result.ok {
        bail!("missing rust-analyzer unexpectedly succeeded");
    }
    let error = result.error.unwrap_or_default();
    if !error.contains("rust-analyzer is unavailable") {
        bail!("missing binary error is not actionable: {error}");
    }
    Ok(())
}

struct TestToolHost;

impl ToolModuleHost for TestToolHost {
    fn is_cancelled(&self) -> Result<bool, ProcessModuleError> {
        Ok(false)
    }
}

fn invoke(tool: &RustLspDiagnosticsTool, cwd: &Path, path: &str) -> Result<ToolResult> {
    let call = json!({
        "id": format!("call-{path}"),
        "name": "lsp_diagnostics",
        "args": { "path": path }
    });
    let context = ToolModuleInvocationContext {
        cwd: cwd.to_path_buf(),
        attribution: ExecutionAttribution::detached(new_execution_id()),
        config: json!({}),
    };
    let mut host = TestToolHost;
    match tool.invoke_json(
        call.to_string(),
        serde_json::to_string(&context)?,
        &mut host,
    ) {
        Ok(result) => Ok(serde_json::from_str(result.as_str())?),
        Err(error) => Err(anyhow!("module error: {}", error.message)),
    }
}

fn run_mock_lsp() -> Result<()> {
    let framing = ContentLengthFraming::default();
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut reader = BufReader::new(stdin.lock());
    let mut writer = stdout.lock();
    let mut initialized = false;
    let mut opened_uri = None::<String>;
    loop {
        let message = framing.read_frame(&mut reader)?;
        match message.get("method").and_then(Value::as_str) {
            Some("initialize") => {
                let id = message
                    .get("id")
                    .cloned()
                    .ok_or_else(|| anyhow!("initialize request missing id"))?;
                framing.write_frame(
                    &mut writer,
                    &json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {
                            "capabilities": {
                                "textDocumentSync": { "openClose": true, "change": 1 }
                            },
                            "serverInfo": { "name": "mock-rust-analyzer", "version": "1" }
                        }
                    }),
                )?;
            }
            Some("initialized") => initialized = true,
            Some("textDocument/didOpen") => {
                if !initialized {
                    bail!("didOpen arrived before initialized");
                }
                let uri = string_at(&message, "/params/textDocument/uri")?;
                let version = integer_at(&message, "/params/textDocument/version")?;
                let text = string_at(&message, "/params/textDocument/text")?;
                opened_uri = Some(uri.clone());
                publish_after_configuration(
                    &framing,
                    &mut reader,
                    &mut writer,
                    &uri,
                    version,
                    &text,
                )?;
            }
            Some("textDocument/didChange") => {
                let uri = string_at(&message, "/params/textDocument/uri")?;
                if opened_uri.as_deref() != Some(uri.as_str()) {
                    bail!("didChange arrived for a document that was not opened");
                }
                let version = integer_at(&message, "/params/textDocument/version")?;
                let text = string_at(&message, "/params/contentChanges/0/text")?;
                publish_after_configuration(
                    &framing,
                    &mut reader,
                    &mut writer,
                    &uri,
                    version,
                    &text,
                )?;
            }
            Some(method) => bail!("unexpected mock LSP method: {method}"),
            None => bail!("unexpected response received by mock LSP: {message}"),
        }
    }
}

fn publish_after_configuration(
    framing: &ContentLengthFraming,
    reader: &mut impl io::BufRead,
    writer: &mut impl Write,
    uri: &str,
    version: i64,
    text: &str,
) -> Result<()> {
    let request_id = 10_000 + version;
    framing.write_frame(
        writer,
        &json!({
            "jsonrpc": "2.0",
            "id": request_id,
            "method": "workspace/configuration",
            "params": { "items": [{ "section": "rust-analyzer" }] }
        }),
    )?;
    let response = framing.read_frame(reader)?;
    if response.get("id") != Some(&json!(request_id))
        || response.get("result") != Some(&json!([null]))
    {
        bail!("client returned an invalid workspace/configuration response: {response}");
    }
    framing.write_frame(
        writer,
        &json!({
            "jsonrpc": "2.0",
            "method": "textDocument/publishDiagnostics",
            "params": {
                "uri": "file:///unrelated.rs",
                "version": version,
                "diagnostics": []
            }
        }),
    )?;
    let diagnostics = if text.contains("broken") {
        vec![json!({
            "range": {
                "start": { "line": 0, "character": 20 },
                "end": { "line": 0, "character": 26 }
            },
            "severity": 1,
            "code": "E0308",
            "source": "mock-rust-analyzer",
            "message": "mismatched types"
        })]
    } else {
        Vec::new()
    };
    framing.write_frame(
        writer,
        &json!({
            "jsonrpc": "2.0",
            "method": "textDocument/publishDiagnostics",
            "params": {
                "uri": uri,
                "version": version,
                "diagnostics": diagnostics
            }
        }),
    )?;
    Ok(())
}

fn string_at(message: &Value, pointer: &str) -> Result<String> {
    message
        .pointer(pointer)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| anyhow!("missing string at {pointer}: {message}"))
}

fn integer_at(message: &Value, pointer: &str) -> Result<i64> {
    message
        .pointer(pointer)
        .and_then(Value::as_i64)
        .ok_or_else(|| anyhow!("missing integer at {pointer}: {message}"))
}
