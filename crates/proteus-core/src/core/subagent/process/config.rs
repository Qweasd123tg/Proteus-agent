//! Конфиг process-runner-а: `module_config.subagent.process`.
//!
//! Роль = профиль: каждый ребёнок — отдельный процесс `proteus server stdio`
//! со своим named config (mini-сборка модулей под роль). Родительская
//! сторона не задаёт ребёнку системный prompt и tools — это делает config
//! роли; опциональный `prompt` роли префиксуется к тексту задачи.

use std::path::PathBuf;

use anyhow::{Result, bail};
use serde::Deserialize;
use serde_json::json;

use crate::contracts::{SubagentLimits, SubagentRoleSpec};

/// Формат `module_config.subagent.process`.
#[derive(Debug, Clone, Deserialize)]
pub(super) struct ProcessSubagentConfig {
    #[serde(default)]
    pub roles: Vec<ProcessRoleConfig>,
    /// Бинарь для запуска детей. По умолчанию — текущий исполняемый файл
    /// (`std::env::current_exe`).
    #[serde(default)]
    pub binary: Option<PathBuf>,
    #[serde(default = "default_max_depth")]
    pub max_depth: u64,
    /// Сколько ждать штатного завершения turn-а ребёнка после Cancel,
    /// прежде чем убить процесс.
    #[serde(default = "default_cancel_grace_ms")]
    pub cancel_grace_ms: u64,
}

impl Default for ProcessSubagentConfig {
    fn default() -> Self {
        Self {
            roles: Vec::new(),
            binary: None,
            max_depth: default_max_depth(),
            cancel_grace_ms: default_cancel_grace_ms(),
        }
    }
}

/// Роль process-runner-а.
#[derive(Debug, Clone, Deserialize)]
pub(super) struct ProcessRoleConfig {
    pub name: String,
    pub description: String,
    /// Named config (или путь к config-файлу) ребёнка — передаётся в
    /// `--config`. Безопасность роли структурная: policy/tools/model
    /// задаются этим конфигом, а не промптом.
    pub config: String,
    /// Опциональный префикс к тексту задачи (не системный prompt ребёнка —
    /// system-слой владеет config роли).
    #[serde(default)]
    pub prompt: Option<String>,
    /// Дополнительные CLI-аргументы ребёнка (например, `--permission-mode`).
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    #[serde(default)]
    pub max_summary_bytes: Option<usize>,
}

fn default_max_depth() -> u64 {
    1
}

fn default_cancel_grace_ms() -> u64 {
    5_000
}

pub(super) fn build_process_role_specs(
    roles: &[ProcessRoleConfig],
) -> Result<Vec<SubagentRoleSpec>> {
    let mut specs: Vec<SubagentRoleSpec> = Vec::with_capacity(roles.len());
    for role in roles {
        if role.name.trim().is_empty() {
            bail!("subagent role name must not be empty");
        }
        if role.config.trim().is_empty() {
            bail!("subagent role {} must set a child config", role.name);
        }
        if specs.iter().any(|existing| existing.name == role.name) {
            bail!("duplicate subagent role: {}", role.name);
        }
        let mut limits = SubagentLimits::default();
        limits.timeout_ms = role.timeout_ms;
        limits.max_summary_bytes = role.max_summary_bytes;
        specs.push(
            SubagentRoleSpec::new(
                role.name.clone(),
                role.description.clone(),
                role.prompt.clone().unwrap_or_default(),
            )
            .with_limits(limits)
            .with_config(json!({ "config": role.config })),
        );
    }
    Ok(specs)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn parses_roles_with_child_configs() {
        let config: ProcessSubagentConfig = serde_json::from_value(json!({
            "roles": [
                {
                    "name": "explore",
                    "description": "Read-only explorer",
                    "config": "sub-explorer",
                    "timeout_ms": 60000,
                    "max_summary_bytes": 2048
                }
            ],
            "max_depth": 2,
            "cancel_grace_ms": 1000
        }))
        .unwrap();

        assert_eq!(config.max_depth, 2);
        assert_eq!(config.cancel_grace_ms, 1000);
        let specs = build_process_role_specs(&config.roles).unwrap();
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].name, "explore");
        assert_eq!(specs[0].limits.timeout_ms, Some(60000));
        assert_eq!(specs[0].limits.max_summary_bytes, Some(2048));
        assert_eq!(specs[0].config["config"], "sub-explorer");
    }

    #[test]
    fn role_without_child_config_is_rejected() {
        let error = build_process_role_specs(&[ProcessRoleConfig {
            name: "explore".to_owned(),
            description: "Explorer".to_owned(),
            config: "  ".to_owned(),
            prompt: None,
            args: Vec::new(),
            timeout_ms: None,
            max_summary_bytes: None,
        }])
        .unwrap_err();
        assert!(error.to_string().contains("must set a child config"));
    }

    #[test]
    fn duplicate_process_roles_are_rejected() {
        let role = ProcessRoleConfig {
            name: "explore".to_owned(),
            description: "Explorer".to_owned(),
            config: "sub-explorer".to_owned(),
            prompt: None,
            args: Vec::new(),
            timeout_ms: None,
            max_summary_bytes: None,
        };
        let error = build_process_role_specs(&[role.clone(), role]).unwrap_err();
        assert!(error.to_string().contains("duplicate subagent role"));
    }
}
