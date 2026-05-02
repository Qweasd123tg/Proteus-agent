//! Ripgrep SearchBackend plugin.
//!
//! Registers search backend id `"rg"` through the stable plugin ABI.

#![allow(non_local_definitions)]
#![allow(non_camel_case_types)]
#![allow(improper_ctypes_definitions)]

use std::process::Command;

use agent_contracts::{
    abi_stable::{
        export_root_module,
        prefix_type::PrefixTypeTrait,
        sabi_trait::TD_Opaque,
        std_types::{RResult, RStr, RString},
    },
    contracts::SearchQuery,
    domain::ContextChunk,
    plugin::{
        PluginRegisterError, PluginRegistryMut, PluginRoot, PluginRoot_Ref, PluginSearchBackend,
        PluginSearchBackend_TO, PluginSearchError, SearchBackendObject,
    },
};
use serde_json::json;

struct RgSearchPlugin;

impl PluginSearchBackend for RgSearchPlugin {
    fn search_json(&self, query_json: RString) -> RResult<RString, PluginSearchError> {
        let query: SearchQuery = match serde_json::from_str(query_json.as_str()) {
            Ok(query) => query,
            Err(error) => {
                return RResult::RErr(PluginSearchError::new(format!(
                    "invalid SearchQuery JSON: {error}"
                )));
            }
        };

        match run_rg(query) {
            Ok(chunks) => match serde_json::to_string(&chunks) {
                Ok(json) => RResult::ROk(RString::from(json)),
                Err(error) => RResult::RErr(PluginSearchError::new(format!(
                    "failed to serialize search chunks: {error}"
                ))),
            },
            Err(error) => RResult::RErr(PluginSearchError::new(error)),
        }
    }
}

fn run_rg(query: SearchQuery) -> Result<Vec<ContextChunk>, String> {
    if query.text.trim().is_empty() || query.max_results == 0 {
        return Ok(Vec::new());
    }

    let output = match Command::new("rg")
        .arg("--line-number")
        .arg("--no-heading")
        .arg("--color=never")
        .arg("--max-count")
        .arg(query.max_results.to_string())
        .arg("--")
        .arg(&query.text)
        .current_dir(&query.cwd)
        .output()
    {
        Ok(output) => output,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("failed to run ripgrep: {error}")),
    };

    match output.status.code() {
        Some(0) | Some(1) => {}
        _ => return Ok(Vec::new()),
    }

    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(parse_rg_line)
        .filter(|chunk| {
            chunk
                .path
                .as_ref()
                .and_then(|path| path.to_str())
                .is_some_and(|path| query.matches_path(path))
        })
        .take(query.max_results)
        .collect())
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

extern "C" fn register_modules(
    registry: &mut PluginRegistryMut<'_>,
) -> RResult<(), PluginRegisterError> {
    let backend: SearchBackendObject =
        PluginSearchBackend_TO::from_value(RgSearchPlugin, TD_Opaque);
    registry.register_search_backend(RString::from("rg"), backend)
}

#[export_root_module]
pub fn get_plugin_root() -> PluginRoot_Ref {
    PluginRoot {
        name: RStr::from_str("rg-search"),
        description: RStr::from_str("Workspace SearchBackend backed by ripgrep"),
        register_modules,
    }
    .leak_into_prefix()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_rg_line_extracts_path_line_and_content() {
        let chunk = parse_rg_line("src/main.rs:42:let value = 1;").unwrap();

        assert_eq!(chunk.source, "rg");
        assert_eq!(chunk.path.unwrap().display().to_string(), "src/main.rs");
        assert_eq!(chunk.content, "let value = 1;");
        assert_eq!(chunk.metadata["line"], 42);
    }
}
