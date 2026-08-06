//! OpenCode-shaped approval policy.
//!
//! Порт permission engine из opencode (`permission/index.ts`,
//! `core/util/wildcard.ts`): правила — тройки `(permission, pattern, action)`,
//! действует последнее совпавшее правило (last match wins), дефолт при
//! отсутствии совпадений — `ask`. Permission — это группа, а не имя tool-а:
//! `edit` покрывает edit/write/apply_patch tools, `bash` матчит pattern по
//! тексту команды, `read` — по пути файла. Маппинг tool → группа и ключи
//! аргументов для извлечения pattern задаются config-ом.

use std::collections::BTreeMap;

use proteus_contracts::{
    abi_stable::std_types::{RResult, RString},
    domain::{PolicyDecision, ToolCall},
    plugin::{PluginApprovalPolicy, PluginPolicyError},
};
use serde::Deserialize;
use serde_json::Value;

use crate::{PolicyContextDto, PolicyVisibilityContextDto, decision, policy_error};

const DEFAULT_ACTION: RuleAction = RuleAction::Ask;
const COMMAND_SEPARATORS: [&str; 4] = ["&&", "||", ";", "|"];

#[derive(Default)]
pub struct OpencodePolicyPlugin;

impl PluginApprovalPolicy for OpencodePolicyPlugin {
    fn evaluate_json(
        &self,
        call_json: RString,
        ctx_json: RString,
    ) -> RResult<RString, PluginPolicyError> {
        let call: ToolCall = match serde_json::from_str(call_json.as_str()) {
            Ok(call) => call,
            Err(error) => return policy_error(format!("invalid ToolCall JSON: {error}")),
        };
        let ctx: PolicyContextDto = match serde_json::from_str(ctx_json.as_str()) {
            Ok(ctx) => ctx,
            Err(error) => return policy_error(format!("invalid PolicyContext JSON: {error}")),
        };
        let config = match OpencodePolicyConfig::from_value(&ctx.config) {
            Ok(config) => config,
            Err(error) => return policy_error(error),
        };
        decision(evaluate_opencode_call(&config, &call, &ctx.cwd))
    }

    fn evaluate_visibility_json(&self, ctx_json: RString) -> RResult<RString, PluginPolicyError> {
        let ctx: PolicyVisibilityContextDto = match serde_json::from_str(ctx_json.as_str()) {
            Ok(ctx) => ctx,
            Err(error) => {
                return policy_error(format!("invalid PolicyVisibilityContext JSON: {error}"));
            }
        };
        let config = match OpencodePolicyConfig::from_value(&ctx.config) {
            Ok(config) => config,
            Err(error) => return policy_error(error),
        };
        decision(evaluate_opencode_visibility(&config, &ctx.tool_spec.name))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum RuleAction {
    Allow,
    Ask,
    Deny,
}

#[derive(Debug, Deserialize)]
struct OpencodeRule {
    permission: String,
    #[serde(default = "default_pattern")]
    pattern: String,
    action: RuleAction,
}

#[derive(Debug, Default, Deserialize)]
struct GroupConfig {
    #[serde(default)]
    tools: Vec<String>,
    /// Ключи `ToolCall.args`, из которых извлекается pattern
    /// (строка или массив строк). Пусто — pattern всегда `*`.
    #[serde(default)]
    pattern_args: Vec<String>,
    /// bash-семантика: составная команда разбивается на подкоманды по
    /// `&& || ; |`, каждая матчится отдельно.
    #[serde(default)]
    split_commands: bool,
}

#[derive(Debug, Default, Deserialize)]
struct OpencodePolicyConfig {
    /// Порядок значим: действует последнее совпавшее правило.
    #[serde(default)]
    rules: Vec<OpencodeRule>,
    #[serde(default)]
    groups: BTreeMap<String, GroupConfig>,
}

impl OpencodePolicyConfig {
    fn from_value(value: &Value) -> Result<Self, String> {
        if value.is_null() {
            return Ok(Self::default());
        }
        serde_json::from_value(value.clone())
            .map_err(|error| format!("invalid opencode_policy config: {error}"))
    }

    fn group_for_tool(&self, tool_name: &str) -> Option<(&str, &GroupConfig)> {
        self.groups
            .iter()
            .find(|(_, group)| group.tools.iter().any(|tool| tool == tool_name))
            .map(|(name, group)| (name.as_str(), group))
    }

    fn last_match(&self, permission: &str, pattern: &str) -> Option<&OpencodeRule> {
        self.rules.iter().rev().find(|rule| {
            wildcard_match(permission, &rule.permission) && wildcard_match(pattern, &rule.pattern)
        })
    }
}

fn default_pattern() -> String {
    "*".to_owned()
}

fn evaluate_opencode_call(
    config: &OpencodePolicyConfig,
    call: &ToolCall,
    cwd: &str,
) -> PolicyDecision {
    let (permission, patterns) = match config.group_for_tool(&call.name) {
        Some((group_name, group)) => (group_name.to_owned(), call_patterns(group, &call.args, cwd)),
        None => (call.name.clone(), vec!["*".to_owned()]),
    };

    let mut needs_ask = false;
    for pattern in &patterns {
        let (action, matched) = match config.last_match(&permission, pattern) {
            Some(rule) => (rule.action, rule.pattern.as_str()),
            None => (DEFAULT_ACTION, "*"),
        };
        match action {
            RuleAction::Deny => {
                return PolicyDecision::Deny {
                    reason: format!(
                        "opencode policy denies '{permission}' for '{pattern}' (rule '{matched}')"
                    ),
                };
            }
            RuleAction::Allow => {}
            RuleAction::Ask => needs_ask = true,
        }
    }

    if needs_ask {
        return PolicyDecision::Ask {
            reason: format!(
                "opencode policy requires approval for '{permission}': {}",
                patterns.join(", ")
            ),
        };
    }
    PolicyDecision::Allow
}

/// Порт `Permission.disabled`: tool скрывается только если последнее правило,
/// совпавшее по permission (pattern не учитывается), — это `deny` с pattern
/// `*`. Точечные deny по pattern не убирают tool из surface.
fn evaluate_opencode_visibility(config: &OpencodePolicyConfig, tool_name: &str) -> PolicyDecision {
    let permission = config
        .group_for_tool(tool_name)
        .map(|(group_name, _)| group_name.to_owned())
        .unwrap_or_else(|| tool_name.to_owned());
    let hidden = config
        .rules
        .iter()
        .rev()
        .find(|rule| wildcard_match(&permission, &rule.permission))
        .is_some_and(|rule| rule.pattern == "*" && rule.action == RuleAction::Deny);
    if hidden {
        return PolicyDecision::Deny {
            reason: format!("opencode policy hides '{permission}' (deny on '*')"),
        };
    }
    PolicyDecision::Allow
}

fn call_patterns(group: &GroupConfig, args: &Value, cwd: &str) -> Vec<String> {
    let mut raw = Vec::new();
    for key in &group.pattern_args {
        match args.get(key) {
            Some(Value::String(value)) => raw.push(value.clone()),
            Some(Value::Array(values)) => {
                raw.extend(values.iter().filter_map(Value::as_str).map(str::to_owned));
            }
            _ => {}
        }
    }
    if raw.is_empty() {
        return vec!["*".to_owned()];
    }

    let mut patterns = Vec::new();
    for value in raw {
        if group.split_commands {
            patterns.extend(split_command(&value));
        } else {
            patterns.push(relativize(&value, cwd));
        }
    }
    if patterns.is_empty() {
        patterns.push("*".to_owned());
    }
    patterns
}

/// Наивная замена tree-sitter разбора из upstream: составная команда
/// разбивается по разделителям вне контекста кавычек не отслеживается —
/// каждая часть матчится правилами отдельно.
fn split_command(command: &str) -> Vec<String> {
    let mut parts = vec![command.to_owned()];
    for separator in COMMAND_SEPARATORS {
        parts = parts
            .into_iter()
            .flat_map(|part| part.split(separator).map(str::to_owned).collect::<Vec<_>>())
            .collect();
    }
    parts
        .into_iter()
        .map(|part| part.trim().to_owned())
        .filter(|part| !part.is_empty())
        .collect()
}

fn relativize(value: &str, cwd: &str) -> String {
    let normalized = value.replace('\\', "/");
    let cwd = cwd.trim_end_matches('/');
    if cwd.is_empty() {
        return normalized;
    }
    match normalized.strip_prefix(cwd) {
        Some(rest) => rest.trim_start_matches('/').to_owned(),
        None => normalized,
    }
}

/// Порт upstream `Wildcard.match`: `*` — любая подстрока, `?` — один символ,
/// матч по всей строке. Спец-случай: pattern с хвостом `" *"` матчит и голый
/// префикс без аргументов (`"git push *"` матчит `"git push"`).
fn wildcard_match(input: &str, pattern: &str) -> bool {
    let input = input.replace('\\', "/");
    let pattern = pattern.replace('\\', "/");
    if let Some(prefix) = pattern.strip_suffix(" *")
        && glob_match(prefix, &input)
    {
        return true;
    }
    glob_match(&pattern, &input)
}

fn glob_match(pattern: &str, text: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let text: Vec<char> = text.chars().collect();
    let mut p = 0;
    let mut t = 0;
    let mut star: Option<usize> = None;
    let mut mark = 0;
    while t < text.len() {
        if p < pattern.len() && (pattern[p] == '?' || pattern[p] == text[t]) {
            p += 1;
            t += 1;
        } else if p < pattern.len() && pattern[p] == '*' {
            star = Some(p);
            mark = t;
            p += 1;
        } else if let Some(star_index) = star {
            p = star_index + 1;
            mark += 1;
            t = mark;
        } else {
            return false;
        }
    }
    while p < pattern.len() && pattern[p] == '*' {
        p += 1;
    }
    p == pattern.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use proteus_contracts::domain::new_call_id;
    use serde_json::json;

    fn config(value: Value) -> OpencodePolicyConfig {
        OpencodePolicyConfig::from_value(&value).expect("config")
    }

    fn groups() -> Value {
        json!({
            "edit": { "tools": ["edit_file", "write_file"], "pattern_args": ["path"] },
            "bash": { "tools": ["shell", "exec_command"], "pattern_args": ["command", "cmd"], "split_commands": true },
            "read": { "tools": ["read_file"], "pattern_args": ["path"] },
        })
    }

    fn evaluate(config_value: Value, tool: &str, args: Value) -> PolicyDecision {
        let call = ToolCall::new(new_call_id(), tool.to_owned(), args);
        evaluate_opencode_call(&config(config_value), &call, "/ws")
    }

    #[test]
    fn wildcard_matches_star_question_and_optional_args_suffix() {
        assert!(wildcard_match("git push origin", "git push*"));
        assert!(wildcard_match("git push", "git push *"));
        assert!(wildcard_match("git push origin main", "git push *"));
        assert!(wildcard_match(".env.local", "*.env.*"));
        assert!(wildcard_match("a.rs", "?.rs"));
        assert!(!wildcard_match("git pull", "git push*"));
        assert!(!wildcard_match("prefix git push", "git push*"));
    }

    #[test]
    fn default_without_rules_is_ask() {
        assert!(matches!(
            evaluate(
                json!({ "groups": groups() }),
                "read_file",
                json!({ "path": "a.rs" })
            ),
            PolicyDecision::Ask { .. }
        ));
    }

    #[test]
    fn last_matching_rule_wins() {
        let config_value = json!({
            "groups": groups(),
            "rules": [
                { "permission": "*", "action": "allow" },
                { "permission": "bash", "pattern": "git push*", "action": "ask" },
                { "permission": "bash", "pattern": "git push --dry-run*", "action": "allow" },
            ],
        });
        assert!(matches!(
            evaluate(
                config_value.clone(),
                "shell",
                json!({ "command": "git push origin" })
            ),
            PolicyDecision::Ask { .. }
        ));
        assert_eq!(
            evaluate(
                config_value.clone(),
                "shell",
                json!({ "command": "git push --dry-run origin" })
            ),
            PolicyDecision::Allow
        );
        assert_eq!(
            evaluate(config_value, "shell", json!({ "command": "cargo test" })),
            PolicyDecision::Allow
        );
    }

    #[test]
    fn compound_commands_check_every_subcommand() {
        let config_value = json!({
            "groups": groups(),
            "rules": [
                { "permission": "*", "action": "allow" },
                { "permission": "bash", "pattern": "git push*", "action": "deny" },
            ],
        });
        assert!(matches!(
            evaluate(
                config_value,
                "shell",
                json!({ "command": "cargo test && git push origin" })
            ),
            PolicyDecision::Deny { reason } if reason.contains("git push")
        ));
    }

    #[test]
    fn edit_group_covers_write_tools_and_relativizes_paths() {
        let config_value = json!({
            "groups": groups(),
            "rules": [
                { "permission": "*", "action": "allow" },
                { "permission": "edit", "pattern": "src/*", "action": "ask" },
            ],
        });
        assert!(matches!(
            evaluate(
                config_value.clone(),
                "write_file",
                json!({ "path": "/ws/src/main.rs", "content": "x" })
            ),
            PolicyDecision::Ask { .. }
        ));
        assert_eq!(
            evaluate(config_value, "edit_file", json!({ "path": "README.md" })),
            PolicyDecision::Allow
        );
    }

    #[test]
    fn env_file_reads_ask_like_upstream_defaults() {
        let config_value = json!({
            "groups": groups(),
            "rules": [
                { "permission": "*", "action": "allow" },
                { "permission": "read", "pattern": "*.env", "action": "ask" },
                { "permission": "read", "pattern": "*.env.*", "action": "ask" },
                { "permission": "read", "pattern": "*.env.example", "action": "allow" },
            ],
        });
        assert!(matches!(
            evaluate(config_value.clone(), "read_file", json!({ "path": ".env" })),
            PolicyDecision::Ask { .. }
        ));
        assert!(matches!(
            evaluate(
                config_value.clone(),
                "read_file",
                json!({ "path": ".env.local" })
            ),
            PolicyDecision::Ask { .. }
        ));
        assert_eq!(
            evaluate(
                config_value.clone(),
                "read_file",
                json!({ "path": ".env.example" })
            ),
            PolicyDecision::Allow
        );
        assert_eq!(
            evaluate(config_value, "read_file", json!({ "path": "src/lib.rs" })),
            PolicyDecision::Allow
        );
    }

    #[test]
    fn ungrouped_tool_uses_its_own_name_as_permission() {
        let config_value = json!({
            "groups": groups(),
            "rules": [
                { "permission": "*", "action": "allow" },
                { "permission": "update_plan", "action": "deny" },
            ],
        });
        assert!(matches!(
            evaluate(config_value, "update_plan", json!({})),
            PolicyDecision::Deny { .. }
        ));
    }

    #[test]
    fn visibility_hides_only_full_deny() {
        let config_value = json!({
            "groups": groups(),
            "rules": [
                { "permission": "*", "action": "allow" },
                { "permission": "edit", "action": "deny" },
                { "permission": "bash", "pattern": "git push*", "action": "deny" },
            ],
        });
        let config = config(config_value);
        assert!(matches!(
            evaluate_opencode_visibility(&config, "write_file"),
            PolicyDecision::Deny { .. }
        ));
        // deny только на паттерн команды — сам tool остаётся видимым.
        assert_eq!(
            evaluate_opencode_visibility(&config, "shell"),
            PolicyDecision::Allow
        );
        assert_eq!(
            evaluate_opencode_visibility(&config, "read_file"),
            PolicyDecision::Allow
        );
    }

    #[test]
    fn missing_pattern_arg_falls_back_to_star() {
        let config_value = json!({
            "groups": groups(),
            "rules": [
                { "permission": "bash", "action": "ask" },
            ],
        });
        assert!(matches!(
            evaluate(config_value, "shell", json!({})),
            PolicyDecision::Ask { .. }
        ));
    }

    #[test]
    fn rejects_invalid_config_shape() {
        let error = OpencodePolicyConfig::from_value(&json!({ "rules": "nope" }))
            .expect_err("invalid config");
        assert!(error.contains("invalid opencode_policy config"));
    }
}
