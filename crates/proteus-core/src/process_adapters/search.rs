use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use proteus_process_host::{NewlineJsonFraming, ProcessHost, ProcessSession, ProcessSpec};
use serde::Deserialize;
use serde_json::Value;

use crate::{
    contracts::{
        PROCESS_MODULE_INITIALIZE_METHOD, PROCESS_MODULE_PROTOCOL_VERSION,
        PROCESS_SEARCH_CONTRACT_VERSION, PROCESS_SEARCH_METHOD, ProcessModuleInitialize,
        ProcessModuleManifest, ProcessSearchResponse, SearchBackend, SearchQuery,
    },
    core::expand_user_path,
    domain::ContextChunk,
};

const DEFAULT_PROCESS_SEARCH_TIMEOUT_MS: u64 = 30_000;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProcessSearchConfig {
    module_id: String,
    command: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    cwd: Option<PathBuf>,
    #[serde(default)]
    env_allowlist: Vec<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    #[serde(default = "default_timeout_ms")]
    timeout_ms: u64,
}

fn default_timeout_ms() -> u64 {
    DEFAULT_PROCESS_SEARCH_TIMEOUT_MS
}

/// `SearchBackend` implemented by one persistent newline-JSON child process.
pub struct ProcessSearchBackend {
    module_id: String,
    timeout: Duration,
    host: Arc<ProcessHost<NewlineJsonFraming>>,
}

impl ProcessSearchBackend {
    pub fn from_config(config: Value, workspace: &Path) -> Result<Self> {
        if config.is_null() {
            bail!("module_config.search.process is required when modules.search = \"process\"");
        }
        let config: ProcessSearchConfig = serde_json::from_value(config)
            .context("failed to parse module_config.search.process")?;
        validate_config(&config)?;

        let process_cwd = resolve_process_cwd(config.cwd.as_deref(), workspace)?;
        let timeout = Duration::from_millis(config.timeout_ms);
        let expected_module_id = config.module_id.clone();
        let initializer_module_id = expected_module_id.clone();
        let spec = ProcessSpec::new(config.command)
            .args(config.args)
            .env_allowlist(config.env_allowlist)
            .envs(config.env)
            .cwd(process_cwd);
        let host = Arc::new(ProcessHost::with_initializer(
            spec,
            NewlineJsonFraming::default(),
            move |session| initialize_search_process(session, &initializer_module_id, timeout),
        ));

        // Handshake is part of snapshot construction: an incompatible command
        // is a config error now, not a surprise on the first model turn.
        drop(host.ensure_session().with_context(|| {
            format!("process search module {expected_module_id:?} handshake failed")
        })?);

        Ok(Self {
            module_id: expected_module_id,
            timeout,
            host,
        })
    }
}

#[async_trait]
impl SearchBackend for ProcessSearchBackend {
    async fn search(&self, query: SearchQuery) -> Result<Vec<ContextChunk>> {
        let host = Arc::clone(&self.host);
        let module_id = self.module_id.clone();
        let timeout = self.timeout;
        tokio::task::spawn_blocking(move || {
            let params = serde_json::to_value(query)
                .context("process search: failed to serialize SearchQuery")?;
            let value = host
                .request(PROCESS_SEARCH_METHOD, params, timeout)
                .with_context(|| format!("process search module {module_id:?} request failed"))?;
            match serde_json::from_value::<ProcessSearchResponse>(value) {
                Ok(response) => Ok(response.chunks),
                Err(error) => {
                    // A syntactically valid JSON-RPC response with the wrong
                    // slot payload poisons this session. Restart lazily on the
                    // next call instead of continuing an unknown protocol.
                    host.reset();
                    Err(error).with_context(|| {
                        format!("process search module {module_id:?} returned invalid response")
                    })
                }
            }
        })
        .await
        .map_err(|error| anyhow::anyhow!("process search join error: {error}"))?
    }
}

fn initialize_search_process(
    session: &mut ProcessSession<NewlineJsonFraming>,
    expected_module_id: &str,
    timeout: Duration,
) -> Result<()> {
    let params = serde_json::to_value(ProcessModuleInitialize::new(
        "search",
        PROCESS_SEARCH_CONTRACT_VERSION,
    ))?;
    let value = session
        .request(PROCESS_MODULE_INITIALIZE_METHOD, params, timeout)
        .context("initialize request failed")?;
    let manifest: ProcessModuleManifest =
        serde_json::from_value(value).context("initialize returned an invalid manifest")?;
    validate_manifest(&manifest, expected_module_id)
}

fn validate_manifest(manifest: &ProcessModuleManifest, expected_module_id: &str) -> Result<()> {
    if manifest.protocol_version != PROCESS_MODULE_PROTOCOL_VERSION {
        bail!(
            "process module protocol mismatch: expected {:?}, got {:?}",
            PROCESS_MODULE_PROTOCOL_VERSION,
            manifest.protocol_version
        );
    }
    if manifest.slot != "search" {
        bail!(
            "process module slot mismatch: expected \"search\", got {:?}",
            manifest.slot
        );
    }
    if manifest.module_id != expected_module_id {
        bail!(
            "process module id mismatch: expected {:?}, got {:?}",
            expected_module_id,
            manifest.module_id
        );
    }
    if manifest.contract_version != PROCESS_SEARCH_CONTRACT_VERSION {
        bail!(
            "process search contract mismatch: expected {:?}, got {:?}",
            PROCESS_SEARCH_CONTRACT_VERSION,
            manifest.contract_version
        );
    }
    Ok(())
}

fn validate_config(config: &ProcessSearchConfig) -> Result<()> {
    if config.module_id.trim().is_empty() {
        bail!("module_config.search.process.module_id must not be empty");
    }
    if config.command.trim().is_empty() {
        bail!("module_config.search.process.command must not be empty");
    }
    if config.timeout_ms == 0 {
        bail!("module_config.search.process.timeout_ms must be greater than zero");
    }
    Ok(())
}

fn resolve_process_cwd(configured: Option<&Path>, workspace: &Path) -> Result<PathBuf> {
    let path = configured.map_or_else(|| workspace.to_path_buf(), expand_user_path);
    let path = if path.is_relative() {
        workspace.join(path)
    } else {
        path
    };
    std::fs::canonicalize(&path)
        .with_context(|| format!("failed to resolve process search cwd {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest() -> ProcessModuleManifest {
        ProcessModuleManifest {
            protocol_version: "v0".to_owned(),
            slot: "search".to_owned(),
            module_id: "fixture".to_owned(),
            contract_version: "v0".to_owned(),
        }
    }

    #[test]
    fn manifest_requires_exact_protocol_slot_module_and_contract() {
        validate_manifest(&manifest(), "fixture").expect("valid manifest");

        let mut wrong = manifest();
        wrong.slot = "memory".to_owned();
        assert!(validate_manifest(&wrong, "fixture").is_err());

        let mut wrong = manifest();
        wrong.module_id = "other".to_owned();
        assert!(validate_manifest(&wrong, "fixture").is_err());

        let mut wrong = manifest();
        wrong.contract_version = "v1".to_owned();
        assert!(validate_manifest(&wrong, "fixture").is_err());
    }

    #[test]
    fn config_rejects_unknown_fields_and_zero_timeout() {
        let unknown = serde_json::json!({
            "module_id": "fixture",
            "command": "python3",
            "legacy_command": "python",
        });
        serde_json::from_value::<ProcessSearchConfig>(unknown)
            .expect_err("legacy fields must be rejected");

        let zero_timeout = ProcessSearchConfig {
            module_id: "fixture".to_owned(),
            command: "python3".to_owned(),
            args: Vec::new(),
            cwd: None,
            env_allowlist: Vec::new(),
            env: BTreeMap::new(),
            timeout_ms: 0,
        };
        assert!(validate_config(&zero_timeout).is_err());
    }
}
