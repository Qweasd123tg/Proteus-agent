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
    /// Максимум одновременно запущенных (`spawn`) детей runner-а
    /// (поверх per-role `max_processes`).
    #[serde(default = "default_max_parallel")]
    pub max_parallel: usize,
}

impl Default for ProcessSubagentConfig {
    fn default() -> Self {
        Self {
            roles: Vec::new(),
            binary: None,
            max_depth: default_max_depth(),
            cancel_grace_ms: default_cancel_grace_ms(),
            max_parallel: default_max_parallel(),
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
    /// Роль можно запускать конкурентно рядом с другими субагентами
    /// (декларация оператора: config ребёнка должен быть read-only
    /// профилем).
    #[serde(default)]
    pub parallel_safe: bool,
    /// Максимум одновременных процессов роли. По умолчанию 4 для
    /// `parallel_safe`-ролей и 1 для остальных.
    #[serde(default)]
    pub max_processes: Option<usize>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    #[serde(default)]
    pub max_summary_bytes: Option<usize>,
}

impl ProcessRoleConfig {
    /// Размер пула процессов роли (минимум 1).
    pub(super) fn effective_max_processes(&self) -> usize {
        match self.max_processes {
            Some(max_processes) => max_processes.max(1),
            None if self.parallel_safe => default_parallel_max_processes(),
            None => 1,
        }
    }
}

fn default_max_depth() -> u64 {
    1
}

fn default_cancel_grace_ms() -> u64 {
    5_000
}

fn default_max_parallel() -> usize {
    8
}

fn default_parallel_max_processes() -> usize {
    4
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
            .with_parallel_safe(role.parallel_safe)
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
            parallel_safe: false,
            max_processes: None,
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
            parallel_safe: false,
            max_processes: None,
            timeout_ms: None,
            max_summary_bytes: None,
        };
        let error = build_process_role_specs(&[role.clone(), role]).unwrap_err();
        assert!(error.to_string().contains("duplicate subagent role"));
    }

    #[test]
    fn parallel_safe_marks_spec_and_widens_default_pool() {
        let config: ProcessSubagentConfig = serde_json::from_value(json!({
            "roles": [
                { "name": "explore", "description": "d", "config": "c", "parallel_safe": true },
                { "name": "writer", "description": "d", "config": "c" },
                {
                    "name": "capped",
                    "description": "d",
                    "config": "c",
                    "parallel_safe": true,
                    "max_processes": 2
                }
            ]
        }))
        .unwrap();

        assert_eq!(config.max_parallel, 8, "default runner-level cap");
        let by_name = |name: &str| {
            config
                .roles
                .iter()
                .find(|role| role.name == name)
                .expect("role")
        };
        assert_eq!(by_name("explore").effective_max_processes(), 4);
        assert_eq!(by_name("writer").effective_max_processes(), 1);
        assert_eq!(by_name("capped").effective_max_processes(), 2);

        let specs = build_process_role_specs(&config.roles).unwrap();
        assert!(specs[0].parallel_safe);
        assert!(!specs[1].parallel_safe);
    }
}
