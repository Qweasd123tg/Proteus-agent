//! Codex patch algorithm behind the ordinary tool/v5 component export.

use serde::Deserialize;
use std::path::Path;

use proteus_contracts::process_module::{ModuleRegistry, ProcessModuleError};

mod files;
mod parser;
mod paths;
mod seek_sequence;
mod streaming_parser;
mod tool;
mod update;

#[derive(Debug, PartialEq)]
struct ApplyPatchArgs {
    hunks: Vec<parser::Hunk>,
    environment_id: Option<String>,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct CodexPatchConfig {
    #[serde(default)]
    reject_self_move: bool,
}

fn apply_patch(input: &str, workspace: &Path, config: &CodexPatchConfig) -> Result<String, String> {
    let args = parser::parse_patch(input)
        .map_err(|error| format!("apply_patch verification failed: {error}"))?;
    if args.environment_id.is_some() {
        return Err("apply_patch environment selection is unavailable for this turn".into());
    }
    let workspace = std::fs::canonicalize(workspace).map_err(|error| {
        format!(
            "failed to canonicalize cwd {}: {error}",
            workspace.display()
        )
    })?;
    files::verify(&args.hunks, &workspace, config.reject_self_move)
        .map_err(|error| format!("apply_patch verification failed: {error}"))?;
    files::apply(&args.hunks, &workspace)
}

pub fn register_modules(registry: &mut dyn ModuleRegistry) -> Result<(), ProcessModuleError> {
    let config = serde_json::from_value(registry.module_config().clone())
        .map_err(|error| ProcessModuleError::new(format!("invalid codex patch config: {error}")))?;
    registry.register_tool(Box::new(tool::ApplyPatchTool { config }))
}

#[cfg(test)]
mod tests;
