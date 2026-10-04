use super::*;

async fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    use tokio::io::AsyncWriteExt;
    let temporary = path.with_file_name(format!(
        ".config-builder-{}.tmp",
        crate::domain::new_call_id()
    ));
    let result = async {
        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .await?;
        if let Ok(metadata) = tokio::fs::metadata(path).await {
            file.set_permissions(metadata.permissions()).await?;
        }
        file.write_all(bytes).await?;
        file.sync_all().await?;
        drop(file);
        tokio::fs::rename(&temporary, path).await?;
        Ok(())
    }
    .await;
    if result.is_err() {
        let _ = tokio::fs::remove_file(&temporary).await;
    }
    result
}

pub(in crate::app_server) fn config_builder_target_path(
    config_path: Option<&Path>,
) -> Option<PathBuf> {
    let path = config_path?;
    if path.is_dir() {
        Some(path.join(crate::core::CONFIG_BUILDER_OVERLAY))
    } else {
        Some(std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf()))
    }
}

#[derive(serde::Serialize)]
struct ModuleConfigToml<'a> {
    module_config: &'a BTreeMap<String, BTreeMap<String, Value>>,
}

#[derive(serde::Serialize)]
struct ProvidersToml<'a> {
    providers: &'a BTreeMap<String, ProviderProfileConfig>,
}

pub(super) fn validate_module_config_toml(
    module_config: &BTreeMap<String, BTreeMap<String, Value>>,
) -> Result<()> {
    module_config_toml_document(module_config).map(|_| ())
}

fn module_config_toml_document(
    module_config: &BTreeMap<String, BTreeMap<String, Value>>,
) -> Result<toml_edit::DocumentMut> {
    let text = toml::to_string_pretty(&ModuleConfigToml { module_config })
        .context("module_config contains values that cannot be represented as TOML")?;
    text.parse::<toml_edit::DocumentMut>()
        .context("serialized module_config TOML could not be parsed")
}

pub(super) async fn persist_config_builder(path: &Path, config: &AppConfig) -> Result<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let mut doc = if !tokio::fs::try_exists(path).await?
        && path.file_name().and_then(|name| name.to_str())
            != Some(crate::core::CONFIG_BUILDER_OVERLAY)
    {
        toml::to_string_pretty(config)
            .context("cannot serialize initial profile")?
            .parse::<toml_edit::DocumentMut>()?
    } else {
        read_toml_document_or_empty(path).await?
    };

    if let Some(provider) = &config.active_provider {
        doc["active_provider"] = toml_edit::value(provider.clone());
    } else {
        doc.remove("active_provider");
    }
    if doc.get("providers").is_none() {
        let text = toml::to_string_pretty(&ProvidersToml {
            providers: &config.providers,
        })
        .context("providers cannot be represented as TOML")?;
        let providers_doc = text
            .parse::<toml_edit::DocumentMut>()
            .context("serialized providers TOML could not be parsed")?;
        doc["providers"] = providers_doc["providers"].clone();
    }

    if doc
        .get("permissions")
        .is_none_or(|item| !item.is_table_like())
    {
        doc["permissions"] = toml_edit::table();
    }
    doc["permissions"]["mode"] = toml_edit::value(permission_mode_str(config.permissions.mode));

    doc["modules"] = toml_edit::table();
    for (kind, id) in config.modules.iter() {
        doc["modules"][kind.as_str()] = toml_edit::value(id.to_owned());
    }

    doc["modules"]["hooks"] = toml_edit::value(
        config
            .modules
            .hooks
            .iter()
            .cloned()
            .collect::<toml_edit::Array>(),
    );

    if doc
        .get("agent_control")
        .is_none_or(|item| !item.is_table_like())
    {
        doc["agent_control"] = toml_edit::table();
    }
    doc["agent_control"]["surface"] = toml_edit::value(config.agent_control.surface.as_str());

    let module_config_doc = module_config_toml_document(&config.module_config)?;
    if let Some(item) = module_config_doc.as_table().get("module_config") {
        doc["module_config"] = item.clone();
    } else {
        doc["module_config"] = toml_edit::table();
    }

    if doc.get("tools").is_none_or(|item| !item.is_table_like()) {
        doc["tools"] = toml_edit::table();
    }
    doc["tools"]["enabled"] = toml_edit::value(
        config
            .tools
            .enabled
            .iter()
            .cloned()
            .collect::<toml_edit::Array>(),
    );

    atomic_write(path, doc.to_string().as_bytes()).await?;
    Ok(())
}

pub(super) async fn read_toml_document_or_empty(path: &Path) -> Result<toml_edit::DocumentMut> {
    let existing = match tokio::fs::read_to_string(path).await {
        Ok(existing) => existing,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => {
            return Err(error).with_context(|| format!("failed to read config {}", path.display()));
        }
    };
    existing
        .parse::<toml_edit::DocumentMut>()
        .map_err(|err| anyhow!("failed to parse config TOML at {}: {err}", path.display()))
}
