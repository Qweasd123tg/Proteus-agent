use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Result, bail};
use proteus_core::core::AppConfig;

const CODING_PROFILE_CONFIG: &str = include_str!("../../../proteus.coding.example.toml");
const CODEX_PROFILE_CONFIG: &str = include_str!("../../../codex.config.toml");
const PROVIDER_PROFILE_CONFIG: &str = include_str!("../../../proteus.provider.example.toml");
const SAFE_PROFILE_CONFIG: &str = include_str!("../../../proteus.example.toml");
const CODEX_DEFAULT_PROMPT: &str = include_str!("../../../prompts/codex-default.md");
/// Относительный путь совпадает с `file` в codex-конфиге: резолвится от
/// каталога config-файла.
const CODEX_PROMPT_FILE: &str = "prompts/codex-default.md";
pub(crate) const INIT_CONFIG_FILE: &str = "config.toml";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InitProfile {
    Coding,
    Codex,
    Full,
    Safe,
}

impl InitProfile {
    fn config_name(self) -> &'static str {
        match self {
            Self::Coding => "coding",
            Self::Codex => "codex",
            Self::Full => "full",
            Self::Safe => "safe",
        }
    }

    fn config_body(self) -> &'static str {
        match self {
            Self::Coding | Self::Full => CODING_PROFILE_CONFIG,
            Self::Codex => CODEX_PROFILE_CONFIG,
            Self::Safe => SAFE_PROFILE_CONFIG,
        }
    }

    fn config_body_for_init(self) -> String {
        match self {
            Self::Coding | Self::Codex | Self::Full => {
                let profile_body = strip_profile_include(self.config_body()).trim_start();
                format!("{}\n\n{}", PROVIDER_PROFILE_CONFIG.trim_end(), profile_body)
            }
            Self::Safe => self.config_body().to_owned(),
        }
    }

    /// Файлы, на которые ссылается config profile; кладутся рядом с ним.
    fn support_files(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Self::Codex => &[(CODEX_PROMPT_FILE, CODEX_DEFAULT_PROMPT)],
            Self::Coding | Self::Full | Self::Safe => &[],
        }
    }
}

pub(crate) fn parse_init_command(task: &[String]) -> Result<Option<InitProfile>> {
    match task {
        [command] if command == "init" => Ok(Some(InitProfile::Coding)),
        [command, profile] if command == "init" => match profile.as_str() {
            "coding" => Ok(Some(InitProfile::Coding)),
            "codex" => Ok(Some(InitProfile::Codex)),
            "full" => Ok(Some(InitProfile::Full)),
            "safe" => Ok(Some(InitProfile::Safe)),
            other => bail!("unknown init profile '{other}', expected coding, codex, full, or safe"),
        },
        [command, ..] if command == "init" => {
            bail!("usage: proteus init [coding|codex|full|safe]")
        }
        _ => Ok(None),
    }
}

pub(crate) fn run_init(profile: InitProfile, explicit_config: Option<&Path>) -> Result<()> {
    let config_path = explicit_config
        .map(init_config_path_from_arg)
        .or_else(AppConfig::default_user_config_path)
        .ok_or_else(|| anyhow::anyhow!("could not resolve default config path"))?;
    let destination = init_destination_path(&config_path, profile);
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&destination, profile.config_body_for_init())?;
    let config_dir = destination.parent().unwrap_or_else(|| Path::new("."));
    for (relative, body) in profile.support_files() {
        let path = config_dir.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, body)?;
    }

    println!(
        "Initialized {} profile: {}",
        profile.config_name(),
        destination.display()
    );
    if let Some(warning) = mixed_config_files_warning(&destination) {
        println!("warning: {warning}");
    }
    println!("Next: proteus doctor");
    Ok(())
}

pub(crate) fn init_config_path_from_arg(path: &Path) -> PathBuf {
    AppConfig::named_config_destination_path(path).unwrap_or_else(|| path.to_path_buf())
}

fn strip_profile_include(config: &str) -> &str {
    if let Some(rest) = config.strip_prefix("include = \"proteus.provider.example.toml\"") {
        rest
    } else {
        config
    }
}

pub(crate) fn init_destination_path(config_path: &Path, _profile: InitProfile) -> PathBuf {
    if is_config_file_path(config_path) {
        config_path.to_path_buf()
    } else {
        config_path.join(INIT_CONFIG_FILE)
    }
}

pub(crate) fn is_config_file_path(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|extension| extension.to_str()),
        Some("toml" | "json")
    )
}

pub(crate) fn single_config_file_for_warning(config_path: Option<&Path>) -> Option<PathBuf> {
    let path = config_path?;
    if is_config_file_path(path) {
        return (path.file_name().and_then(|name| name.to_str()) == Some(INIT_CONFIG_FILE))
            .then(|| path.to_path_buf());
    }
    Some(path.join(INIT_CONFIG_FILE)).filter(|path| path.exists())
}

pub(crate) fn mixed_config_files_warning(config_file: &Path) -> Option<String> {
    if config_file.file_name().and_then(|name| name.to_str()) != Some(INIT_CONFIG_FILE) {
        return None;
    }
    let siblings = sibling_config_files(config_file);
    if siblings.is_empty() {
        return None;
    }
    Some(format!(
        "config dir also contains {}. Proteus loads every .toml/.json file when given the directory; move old files away or pass --config {} to load only this file.",
        siblings
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(", "),
        config_file.display()
    ))
}

fn sibling_config_files(config_file: &Path) -> Vec<PathBuf> {
    let Some(parent) = config_file.parent() else {
        return Vec::new();
    };
    let config_name = config_file.file_name();
    let mut files = fs::read_dir(parent)
        .ok()
        .into_iter()
        .flat_map(|entries| entries.flatten())
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file() && is_config_file_path(path) && path.file_name() != config_name
        })
        .collect::<Vec<_>>();
    files.sort();
    files
}
