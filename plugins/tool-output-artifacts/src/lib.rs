//! Draft artifact-backed tool result processor.
//!
//! This crate is deliberately not loadable by the current dylib plugin loader:
//! there is no `ToolResultProcessor` / `ToolOutputStore` ABI slot yet. It keeps
//! the artifact strategy as a compiled draft so the behavior can be promoted to
//! a real plugin once the slot contract exists.

use std::path::{Component, Path, PathBuf};

use agent_contracts::domain::ToolResult;
use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use serde_json::json;

const DEFAULT_MAX_PREVIEW_BYTES: usize = 20_000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolOutputArtifactConfig {
    #[serde(default = "default_max_preview_bytes")]
    pub max_preview_bytes: usize,
    #[serde(default = "default_artifact_root")]
    pub artifact_root: PathBuf,
}

impl Default for ToolOutputArtifactConfig {
    fn default() -> Self {
        Self {
            max_preview_bytes: default_max_preview_bytes(),
            artifact_root: default_artifact_root(),
        }
    }
}

fn default_max_preview_bytes() -> usize {
    DEFAULT_MAX_PREVIEW_BYTES
}

fn default_artifact_root() -> PathBuf {
    PathBuf::from(".agent").join("tool-outputs")
}

#[derive(Debug, Clone)]
pub struct ArtifactOutputStore {
    config: ToolOutputArtifactConfig,
}

impl ArtifactOutputStore {
    pub fn new(config: ToolOutputArtifactConfig) -> Self {
        Self { config }
    }

    pub async fn process_result(
        &self,
        workspace: &Path,
        tool_name: &str,
        mut result: ToolResult,
    ) -> Result<ToolResult> {
        let (output, output_truncated, output_original_bytes) =
            truncate_utf8(&result.output, self.config.max_preview_bytes);
        let output_artifact = if output_truncated {
            Some(
                self.write_artifact(
                    workspace,
                    tool_name,
                    &result.call_id,
                    "output",
                    &result.output,
                )
                .await?,
            )
        } else {
            None
        };
        result.output = output;

        let (error, error_truncated, error_original_bytes, error_artifact) = match result
            .error
            .take()
        {
            Some(error) => {
                let (preview, truncated, original_bytes) =
                    truncate_utf8(&error, self.config.max_preview_bytes);
                let artifact = if truncated {
                    Some(
                        self.write_artifact(workspace, tool_name, &result.call_id, "error", &error)
                            .await?,
                    )
                } else {
                    None
                };
                (Some(preview), truncated, original_bytes, artifact)
            }
            None => (None, false, 0, None),
        };
        result.error = error;

        if output_truncated || error_truncated {
            let mut metadata = result.metadata;
            if output_truncated {
                metadata = metadata_with(metadata, "output_truncated", json!(true));
                metadata = metadata_with(
                    metadata,
                    "output_original_bytes",
                    json!(output_original_bytes),
                );
                if let Some(artifact) = output_artifact {
                    result.output.push_str(&format!(
                        "\n\n[output truncated to {} bytes; full output saved to {}]",
                        self.config.max_preview_bytes,
                        artifact.relative_path.display()
                    ));
                    metadata = metadata_with(
                        metadata,
                        "output_artifact_path",
                        json!(artifact.relative_path.to_string_lossy()),
                    );
                    metadata =
                        metadata_with(metadata, "output_artifact_bytes", json!(artifact.bytes));
                }
            }
            if error_truncated {
                metadata = metadata_with(metadata, "error_truncated", json!(true));
                metadata = metadata_with(
                    metadata,
                    "error_original_bytes",
                    json!(error_original_bytes),
                );
                if let Some(artifact) = error_artifact {
                    let note = format!(
                        "\n\n[error truncated to {} bytes; full error saved to {}]",
                        self.config.max_preview_bytes,
                        artifact.relative_path.display()
                    );
                    result.error = Some(match result.error.take() {
                        Some(mut error) => {
                            error.push_str(&note);
                            error
                        }
                        None => note,
                    });
                    metadata = metadata_with(
                        metadata,
                        "error_artifact_path",
                        json!(artifact.relative_path.to_string_lossy()),
                    );
                    metadata =
                        metadata_with(metadata, "error_artifact_bytes", json!(artifact.bytes));
                }
            }
            metadata = metadata_with(
                metadata,
                "max_preview_bytes",
                json!(self.config.max_preview_bytes),
            );
            result.metadata = metadata;
        }

        Ok(result)
    }

    async fn write_artifact(
        &self,
        workspace: &Path,
        tool_name: &str,
        call_id: &str,
        stream: &str,
        content: &str,
    ) -> Result<ToolArtifactRef> {
        let relative_path = self.artifact_relative_path(tool_name, call_id, stream);
        write_workspace_text_artifact(workspace, &relative_path, content).await?;
        Ok(ToolArtifactRef {
            relative_path,
            bytes: content.len(),
        })
    }

    fn artifact_relative_path(&self, tool_name: &str, call_id: &str, stream: &str) -> PathBuf {
        self.config
            .artifact_root
            .join(sanitize_path_segment(tool_name))
            .join(format!(
                "{}-{}.txt",
                sanitize_path_segment(call_id),
                sanitize_path_segment(stream)
            ))
    }
}

#[derive(Debug, Clone)]
struct ToolArtifactRef {
    relative_path: PathBuf,
    bytes: usize,
}

async fn write_workspace_text_artifact(
    workspace: &Path,
    relative_path: &Path,
    content: &str,
) -> Result<()> {
    ensure_relative_artifact_path(relative_path)?;
    let parent = relative_path
        .parent()
        .ok_or_else(|| anyhow!("artifact path has no parent: {}", relative_path.display()))?;
    reject_existing_symlink_components(workspace, parent).await?;

    let path = workspace.join(relative_path);
    tokio::fs::create_dir_all(workspace.join(parent))
        .await
        .with_context(|| format!("failed to create {}", workspace.join(parent).display()))?;
    tokio::fs::write(&path, content)
        .await
        .with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}

fn ensure_relative_artifact_path(path: &Path) -> Result<()> {
    for component in path.components() {
        match component {
            Component::Normal(_) | Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                anyhow::bail!(
                    "artifact path must stay inside workspace: {}",
                    path.display()
                );
            }
        }
    }
    Ok(())
}

async fn reject_existing_symlink_components(workspace: &Path, relative_dir: &Path) -> Result<()> {
    let mut current = workspace.to_path_buf();
    for component in relative_dir.components() {
        match component {
            Component::Normal(part) => {
                current.push(part);
                match tokio::fs::symlink_metadata(&current).await {
                    Ok(metadata) if metadata.file_type().is_symlink() => {
                        anyhow::bail!(
                            "artifact directory must not contain symlink component: {}",
                            current.display()
                        );
                    }
                    Ok(_) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
                    Err(error) => {
                        return Err(error).with_context(|| {
                            format!("failed to inspect artifact path {}", current.display())
                        });
                    }
                }
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                anyhow::bail!(
                    "artifact directory must stay inside workspace: {}",
                    relative_dir.display()
                );
            }
        }
    }
    Ok(())
}

fn truncate_utf8(value: &str, max_bytes: usize) -> (String, bool, usize) {
    let original_bytes = value.len();
    if original_bytes <= max_bytes {
        return (value.to_owned(), false, original_bytes);
    }

    let mut end = max_bytes;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    (value[..end].to_owned(), true, original_bytes)
}

fn sanitize_path_segment(value: &str) -> String {
    let sanitized = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    if sanitized.is_empty() {
        "unknown".to_owned()
    } else {
        sanitized
    }
}

fn metadata_with(
    metadata: serde_json::Value,
    key: &str,
    value: serde_json::Value,
) -> serde_json::Value {
    let mut object = match metadata {
        serde_json::Value::Object(object) => object,
        _ => serde_json::Map::new(),
    };
    object.insert(key.to_owned(), value);
    serde_json::Value::Object(object)
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_contracts::domain::ToolResult;

    #[tokio::test]
    async fn saves_truncated_output_as_workspace_artifact() {
        let dir = tempfile::tempdir().expect("workspace");
        let store = ArtifactOutputStore::new(ToolOutputArtifactConfig {
            max_preview_bytes: 12,
            ..Default::default()
        });
        let result = ToolResult::ok("call/1".into(), "0123456789abcdefghijklmnopqrstuvwxyz");

        let result = store
            .process_result(dir.path(), "shell/tool", result)
            .await
            .expect("processed");

        assert!(result.output.starts_with("0123456789ab"));
        assert!(result.output.contains("full output saved to"));
        assert_eq!(result.metadata["output_truncated"], true);
        assert_eq!(result.metadata["output_original_bytes"], 36);
        assert_eq!(result.metadata["output_artifact_bytes"], 36);
        let artifact_path = result.metadata["output_artifact_path"]
            .as_str()
            .expect("artifact path");
        assert_eq!(
            artifact_path,
            ".agent/tool-outputs/shell_tool/call_1-output.txt"
        );
        let full_output =
            std::fs::read_to_string(dir.path().join(artifact_path)).expect("artifact");
        assert_eq!(full_output, "0123456789abcdefghijklmnopqrstuvwxyz");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn rejects_symlink_artifact_directory() {
        let dir = tempfile::tempdir().expect("workspace");
        let outside = tempfile::tempdir().expect("outside");
        std::os::unix::fs::symlink(outside.path(), dir.path().join(".agent")).expect("symlink");
        let store = ArtifactOutputStore::new(ToolOutputArtifactConfig {
            max_preview_bytes: 12,
            ..Default::default()
        });
        let result = ToolResult::ok("call-1".into(), "0123456789abcdefghijklmnopqrstuvwxyz");

        let error = store
            .process_result(dir.path(), "shell", result)
            .await
            .expect_err("symlink should be rejected");

        assert!(error.to_string().contains("symlink component"));
        assert!(
            std::fs::read_dir(outside.path())
                .expect("outside dir")
                .next()
                .is_none()
        );
    }

    #[test]
    fn truncate_utf8_keeps_valid_boundary() {
        let (text, truncated, original) = truncate_utf8("привет", 3);
        assert_eq!(text, "п");
        assert!(truncated);
        assert_eq!(original, "привет".len());
    }
}
