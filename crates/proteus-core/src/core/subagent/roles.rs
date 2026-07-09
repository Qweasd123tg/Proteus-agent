//! Роли субагентов: парсинг `module_config.subagent.sequential` и загрузка
//! markdown-ролей из `roles_dir` (YAML frontmatter + prompt-тело).

use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, anyhow, bail};
use serde::Deserialize;
use serde_json::json;

use crate::contracts::{SubagentIsolation, SubagentLimits, SubagentRoleSpec};

/// Формат `module_config.subagent.sequential`.
#[derive(Debug, Clone, Deserialize)]
pub(super) struct SequentialSubagentConfig {
    #[serde(default)]
    pub roles: Vec<SequentialRoleConfig>,
    #[serde(default)]
    pub roles_dir: Option<PathBuf>,
    #[serde(default = "default_max_depth")]
    pub max_depth: u64,
    #[serde(default = "default_max_resumable")]
    pub max_resumable: usize,
    /// Максимум одновременно запущенных (`spawn`) детей runner-а.
    #[serde(default = "default_max_parallel")]
    pub max_parallel: usize,
}

impl Default for SequentialSubagentConfig {
    fn default() -> Self {
        Self {
            roles: Vec::new(),
            roles_dir: None,
            max_depth: default_max_depth(),
            max_resumable: default_max_resumable(),
            max_parallel: default_max_parallel(),
        }
    }
}

/// Роль в конфиге: лимиты заданы плоскими полями рядом с prompt.
#[derive(Debug, Clone, Deserialize)]
pub(super) struct SequentialRoleConfig {
    pub name: String,
    pub description: String,
    pub prompt: String,
    #[serde(default)]
    pub exposure_phase: Option<String>,
    #[serde(default)]
    pub tools: Option<Vec<String>>,
    /// Роль можно запускать конкурентно рядом с другими субагентами
    /// (декларация оператора; для sequential-runner-а роль должна быть
    /// фактически read-only через tools/policy).
    #[serde(default)]
    pub parallel_safe: bool,
    /// Изоляция рабочей копии: `"worktree"` — каждый fresh запуск роли
    /// получает собственный git worktree (для пишущих ролей).
    #[serde(default)]
    pub isolation: Option<String>,
    #[serde(default)]
    pub max_iterations: Option<u32>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    #[serde(default)]
    pub max_summary_bytes: Option<usize>,
    /// Token-бюджет запуска: потолок суммы input+output model-запросов.
    #[serde(default)]
    pub max_total_tokens: Option<u64>,
}

fn default_max_depth() -> u64 {
    1
}

fn default_max_resumable() -> usize {
    8
}

fn default_max_parallel() -> usize {
    8
}

#[derive(Debug, Clone, Deserialize)]
struct MarkdownRoleFrontmatter {
    description: String,
    #[serde(default)]
    exposure_phase: Option<String>,
    #[serde(default)]
    tools: Option<Vec<String>>,
    #[serde(default)]
    parallel_safe: bool,
    #[serde(default)]
    isolation: Option<String>,
    #[serde(default)]
    max_iterations: Option<u32>,
    #[serde(default)]
    timeout_ms: Option<u64>,
    #[serde(default)]
    max_summary_bytes: Option<usize>,
    #[serde(default)]
    max_total_tokens: Option<u64>,
}

/// Собирает итоговые `SubagentRoleSpec` из inline-ролей и `roles_dir`.
pub(super) fn build_role_specs(
    parsed: SequentialSubagentConfig,
    cwd: &Path,
) -> Result<(Vec<SubagentRoleSpec>, u64, usize, usize)> {
    let mut role_configs = parsed.roles;
    if let Some(roles_dir) = parsed.roles_dir.as_ref() {
        role_configs.extend(load_markdown_roles(roles_dir, cwd)?);
    }

    let mut roles: Vec<SubagentRoleSpec> = Vec::with_capacity(role_configs.len());
    for role in role_configs {
        if role.name.trim().is_empty() {
            bail!("subagent role name must not be empty");
        }
        if roles.iter().any(|existing| existing.name == role.name) {
            bail!("duplicate subagent role: {}", role.name);
        }
        let mut limits = SubagentLimits::default();
        if let Some(max_iterations) = role.max_iterations {
            limits.max_iterations = max_iterations;
        }
        limits.timeout_ms = role.timeout_ms;
        limits.max_summary_bytes = role.max_summary_bytes;
        limits.max_total_tokens = role.max_total_tokens;
        let isolation = parse_isolation(role.isolation.as_deref())
            .with_context(|| format!("subagent role {}", role.name))?;
        let mut spec = SubagentRoleSpec::new(role.name, role.description, role.prompt)
            .with_limits(limits)
            .with_parallel_safe(role.parallel_safe)
            .with_isolation(isolation);
        if let Some(phase) = role.exposure_phase {
            spec = spec.with_exposure_phase(phase);
        }
        if let Some(tools) = role.tools {
            spec = spec.with_config(json!({ "tools": tools }));
        }
        roles.push(spec);
    }

    Ok((
        roles,
        parsed.max_depth,
        parsed.max_resumable,
        parsed.max_parallel,
    ))
}

/// Парсит строковое значение изоляции из конфига. Неизвестное значение —
/// ошибка конфигурации, а не тихий fallback. Общая для sequential- и
/// process-конфигов ролей.
pub(super) fn parse_isolation(value: Option<&str>) -> Result<SubagentIsolation> {
    match value {
        None => Ok(SubagentIsolation::None),
        Some("worktree") => Ok(SubagentIsolation::Worktree),
        Some(other) => bail!("unknown isolation value: {other} (expected \"worktree\")"),
    }
}

fn load_markdown_roles(roles_dir: &Path, cwd: &Path) -> Result<Vec<SequentialRoleConfig>> {
    let dir = if roles_dir.is_absolute() {
        roles_dir.to_path_buf()
    } else {
        cwd.join(roles_dir)
    };
    let mut markdown_files = Vec::new();
    for entry in fs::read_dir(&dir)
        .with_context(|| format!("failed to read subagent roles_dir {}", dir.display()))?
    {
        let entry = entry.with_context(|| format!("failed to read entry in {}", dir.display()))?;
        let path = entry.path();
        if path.is_file() && path.extension().is_some_and(|extension| extension == "md") {
            markdown_files.push(path);
        }
    }
    markdown_files.sort();

    markdown_files
        .into_iter()
        .map(|path| {
            parse_markdown_role(&path)
                .with_context(|| format!("failed to parse subagent role file {}", path.display()))
        })
        .collect()
}

fn parse_markdown_role(path: &Path) -> Result<SequentialRoleConfig> {
    let name = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|name| !name.trim().is_empty())
        .ok_or_else(|| anyhow!("markdown role file name must be valid UTF-8 with non-empty stem"))?
        .to_owned();
    let content = fs::read_to_string(path)
        .with_context(|| format!("failed to read markdown role file {}", path.display()))?;
    let mut lines = content.lines();
    let Some(first) = lines.next() else {
        bail!("missing YAML frontmatter");
    };
    if first.trim() != "---" {
        bail!("missing opening YAML frontmatter marker");
    }

    let mut yaml = String::new();
    let mut body = Vec::new();
    let mut closed = false;
    for line in lines.by_ref() {
        if line.trim() == "---" {
            closed = true;
            break;
        }
        yaml.push_str(line);
        yaml.push('\n');
    }
    if !closed {
        bail!("missing closing YAML frontmatter marker");
    }
    body.extend(lines);

    let frontmatter: MarkdownRoleFrontmatter =
        serde_yaml::from_str(&yaml).context("invalid YAML frontmatter")?;
    if frontmatter.description.trim().is_empty() {
        bail!("markdown role description must not be empty");
    }

    Ok(SequentialRoleConfig {
        name,
        description: frontmatter.description,
        prompt: body.join("\n").trim().to_owned(),
        exposure_phase: frontmatter.exposure_phase,
        tools: frontmatter.tools,
        parallel_safe: frontmatter.parallel_safe,
        isolation: frontmatter.isolation,
        max_iterations: frontmatter.max_iterations,
        timeout_ms: frontmatter.timeout_ms,
        max_summary_bytes: frontmatter.max_summary_bytes,
        max_total_tokens: frontmatter.max_total_tokens,
    })
}
