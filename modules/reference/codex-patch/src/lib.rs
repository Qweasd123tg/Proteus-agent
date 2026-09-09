//! Codex patch algorithm behind the ordinary patch/v1 component export.

use std::path::Path;

use proteus_contracts::{
    domain::{Patch, PatchResult},
    process_module::{ModuleRegistry, PatchModule, ProcessModuleError},
};

mod files;
mod parser;
mod paths;
mod seek_sequence;
mod streaming_parser;
mod update;

#[derive(Debug, PartialEq)]
struct ApplyPatchArgs {
    hunks: Vec<parser::Hunk>,
    environment_id: Option<String>,
}

struct CodexPatchModule;

impl PatchModule for CodexPatchModule {
    fn apply_json(&self, patch_json: String, cwd: String) -> Result<String, ProcessModuleError> {
        let patch: Patch = serde_json::from_str(&patch_json)
            .map_err(|error| ProcessModuleError::new(format!("invalid Patch JSON: {error}")))?;
        let result =
            apply_patch(&patch.content, Path::new(&cwd)).map_err(ProcessModuleError::new)?;
        serde_json::to_string(&result).map_err(|error| {
            ProcessModuleError::new(format!("failed to serialize PatchResult: {error}"))
        })
    }
}

fn apply_patch(input: &str, workspace: &Path) -> Result<PatchResult, String> {
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
    files::verify(&args.hunks, &workspace)
        .map_err(|error| format!("apply_patch verification failed: {error}"))?;
    files::apply(&args.hunks, &workspace).map(|summary| PatchResult::new(true, summary))
}

pub fn register_modules(registry: &mut dyn ModuleRegistry) -> Result<(), ProcessModuleError> {
    registry.register_patch("codex".to_owned(), Box::new(CodexPatchModule))
}

#[cfg(test)]
mod tests;
