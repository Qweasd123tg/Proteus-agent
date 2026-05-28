use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use proteus_contracts::domain::PermissionMode;
use serde::Deserialize;

pub(crate) struct Cli {
    pub(crate) proteus_bin: Option<PathBuf>,
    pub(crate) config_path: Option<PathBuf>,
    pub(crate) cwd: Option<PathBuf>,
    pub(crate) profile: Option<String>,
    pub(crate) permission_mode: Option<PermissionMode>,
}

pub(crate) fn parse_args(args: &[String]) -> Result<Cli> {
    let mut proteus_bin = None;
    let mut config_path = None;
    let mut cwd = None;
    let mut profile = None;
    let mut permission_mode = None;
    let mut iter = args.iter().peekable();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--plan" => {
                set_permission_mode(&mut permission_mode, PermissionMode::Plan)?;
            }
            "--auto" => {
                set_permission_mode(&mut permission_mode, PermissionMode::Auto)?;
            }
            "--permission-mode" => {
                let raw = iter
                    .next()
                    .context("--permission-mode requires value")
                    .map(String::as_str)?;
                let mode = parse_permission_mode(raw)?;
                set_permission_mode(&mut permission_mode, mode)?;
            }
            "--profile" | "-p" => {
                profile = iter
                    .next()
                    .map(ToOwned::to_owned)
                    .context("--profile requires value")
                    .ok();
            }
            "--proteus-bin" => {
                proteus_bin = iter
                    .next()
                    .map(PathBuf::from)
                    .context("--proteus-bin requires value")
                    .ok();
            }
            "--config" => {
                config_path = iter
                    .next()
                    .map(PathBuf::from)
                    .context("--config requires value")
                    .ok();
            }
            "--cwd" => {
                cwd = iter
                    .next()
                    .map(PathBuf::from)
                    .context("--cwd requires value")
                    .ok();
            }
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            other => {
                eprintln!("unknown arg: {other}");
                print_help();
                std::process::exit(2);
            }
        }
    }
    Ok(Cli {
        proteus_bin,
        config_path,
        cwd,
        profile,
        permission_mode,
    })
}

fn print_help() {
    eprintln!(
        "proteus-tui — terminal UI for proteus-core\n\
         \n\
         usage:\n\
           proteus-tui [--profile NAME] [--proteus-bin PATH] [--config PATH] [--cwd PATH] [--plan|--auto|--permission-mode MODE]\n\
         \n\
         options:\n\
           --profile, -p NAME  load ~/.config/Proteus-agent/profiles/NAME.toml\n\
           --proteus-bin PATH  path to the Proteus binary (default: sibling proteus, then PATH)\n\
           --config PATH       path to Proteus config (toml or json)\n\
           --cwd PATH          workspace directory for Proteus (default: current dir)\n\
           --plan              start in plan mode (planning choices + review chooser)\n\
           --auto              start in auto mode (read/write tools, no command/network)\n\
           --permission-mode   start with explicit mode: plan, normal, or auto\n\
           --help, -h          show this help"
    );
}

#[derive(Debug, Default, Deserialize)]
struct TuiProfileConfig {
    proteus_bin: Option<PathBuf>,
    config: Option<PathBuf>,
    cwd: Option<PathBuf>,
    permission_mode: Option<PermissionMode>,
}

pub(crate) fn apply_profile(cli: Cli) -> Result<Cli> {
    let Some(profile) = cli.profile.as_deref() else {
        return Ok(cli);
    };
    let profile_path = profile_path(profile)?;
    apply_profile_file(cli, &profile_path)
}

fn apply_profile_file(cli: Cli, profile_path: &Path) -> Result<Cli> {
    let content = std::fs::read_to_string(profile_path)
        .with_context(|| format!("failed to read TUI profile {}", profile_path.display()))?;
    let profile_config: TuiProfileConfig = toml::from_str(&content)
        .with_context(|| format!("failed to parse TUI profile {}", profile_path.display()))?;
    let profile_dir = profile_path.parent().unwrap_or_else(|| Path::new("."));

    Ok(Cli {
        proteus_bin: cli.proteus_bin.or_else(|| {
            profile_config
                .proteus_bin
                .map(|path| resolve_profile_path(profile_dir, path))
        }),
        config_path: cli.config_path.or_else(|| {
            profile_config
                .config
                .map(|path| resolve_profile_path(profile_dir, path))
        }),
        cwd: cli.cwd.or_else(|| {
            profile_config
                .cwd
                .map(|path| resolve_profile_path(profile_dir, path))
        }),
        permission_mode: cli.permission_mode.or(profile_config.permission_mode),
        profile: cli.profile,
    })
}

fn set_permission_mode(
    permission_mode: &mut Option<PermissionMode>,
    next: PermissionMode,
) -> Result<()> {
    if permission_mode.is_some() {
        anyhow::bail!("use only one of --plan, --auto, or --permission-mode");
    }
    *permission_mode = Some(next);
    Ok(())
}

fn parse_permission_mode(raw: &str) -> Result<PermissionMode> {
    match raw {
        "plan" => Ok(PermissionMode::Plan),
        "normal" => Ok(PermissionMode::Normal),
        "auto" => Ok(PermissionMode::Auto),
        _ => anyhow::bail!("unsupported permission mode '{raw}'; use plan, normal, or auto"),
    }
}

fn profile_path(profile: &str) -> Result<PathBuf> {
    if profile.trim().is_empty() {
        anyhow::bail!("profile name must not be empty");
    }
    let path = PathBuf::from(profile);
    if path.components().count() != 1 || path.is_absolute() {
        anyhow::bail!("profile name must be a simple file stem, got '{profile}'");
    }
    let home = std::env::var_os("HOME").context("HOME is not set")?;
    Ok(PathBuf::from(home)
        .join(".config/Proteus-agent/profiles")
        .join(format!("{profile}.toml")))
}

fn resolve_profile_path(profile_dir: &Path, path: PathBuf) -> PathBuf {
    let path = expand_home_path(&path);
    if path.is_absolute() {
        path
    } else {
        profile_dir.join(path)
    }
}

fn expand_home_path(path: &Path) -> PathBuf {
    let Some(path_str) = path.to_str() else {
        return path.to_path_buf();
    };
    if path_str == "~"
        && let Some(home) = std::env::var_os("HOME")
    {
        return PathBuf::from(home);
    }
    if let Some(rest) = path_str.strip_prefix("~/")
        && let Some(home) = std::env::var_os("HOME")
    {
        return PathBuf::from(home).join(rest);
    }
    path.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_file_fills_missing_launcher_fields() {
        let dir = tempfile::tempdir().expect("profile dir");
        let profile_path = dir.path().join("claude.toml");
        std::fs::write(
            &profile_path,
            r#"
proteus_bin = "bin/proteus"
config = "~/proteus-config/configs"
cwd = "workspace"
permission_mode = "auto"
"#,
        )
        .expect("profile");
        let cli = Cli {
            proteus_bin: None,
            config_path: Some(PathBuf::from("/explicit/config")),
            cwd: None,
            profile: Some("claude".to_owned()),
            permission_mode: None,
        };

        let cli = apply_profile_file(cli, &profile_path).expect("applied profile");

        assert_eq!(cli.proteus_bin, Some(dir.path().join("bin/proteus")));
        assert_eq!(cli.config_path, Some(PathBuf::from("/explicit/config")));
        assert_eq!(cli.cwd, Some(dir.path().join("workspace")));
        assert_eq!(cli.permission_mode, Some(PermissionMode::Auto));
    }

    #[test]
    fn cli_parses_permission_mode_aliases() {
        let cli = parse_args(&["--plan".to_owned()]).expect("plan args");
        assert_eq!(cli.permission_mode, Some(PermissionMode::Plan));

        let cli = parse_args(&["--permission-mode".to_owned(), "auto".to_owned()])
            .expect("permission args");
        assert_eq!(cli.permission_mode, Some(PermissionMode::Auto));

        assert!(parse_args(&["--plan".to_owned(), "--auto".to_owned()]).is_err());
    }
}
