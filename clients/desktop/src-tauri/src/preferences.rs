use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[cfg(test)]
#[path = "preferences_tests.rs"]
mod tests;
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preferences {
    pub workspace: String,
    pub config: String,
}

pub fn load(path: &Path) -> Result<Option<Preferences>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(
            serde_json::from_slice(&bytes).context("Не удалось прочитать настройки приложения")?,
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

pub fn save(path: &Path, preferences: &Preferences) -> Result<()> {
    fs::create_dir_all(path.parent().context("Missing preferences directory")?)?;
    let temporary = path.with_extension("json.tmp");
    fs::write(&temporary, serde_json::to_vec_pretty(preferences)?)?;
    fs::rename(temporary, path)?;
    Ok(())
}

pub fn config_dir() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("PROTEUS_CONFIG_PATH") {
        return PathBuf::from(path)
            .parent()
            .map(Path::to_path_buf)
            .context("PROTEUS_CONFIG_PATH has no parent");
    }
    if let Some(home) = std::env::var_os("PROTEUS_CONFIG_HOME") {
        return Ok(PathBuf::from(home).join("configs"));
    }
    Ok(
        PathBuf::from(std::env::var_os("HOME").context("HOME is not set")?)
            .join(".config/Proteus-agent/configs"),
    )
}

/// Same ownership rule as install.sh: named configs belong to the user;
/// packaged fragments and prompts are managed assets of the current release.
pub fn install_configs(source: &Path, destination: &Path) -> Result<()> {
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let target = destination.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_managed(&entry.path(), &target)?;
        } else if !target.exists() {
            fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

fn copy_managed(source: &Path, destination: &Path) -> Result<()> {
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let target = destination.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_managed(&entry.path(), &target)?;
        } else {
            let content = fs::read(entry.path())?;
            if fs::read(&target).ok().as_ref() != Some(&content) {
                fs::write(target, content)?;
            }
        }
    }
    Ok(())
}

pub fn profiles(directory: &Path) -> Result<Vec<String>> {
    let mut profiles = Vec::new();
    for entry in fs::read_dir(directory)? {
        let name = entry?.file_name().to_string_lossy().into_owned();
        if let Some(name) = name.strip_suffix(".config.toml") {
            profiles.push(name.to_owned());
        }
    }
    profiles.sort();
    Ok(profiles)
}
