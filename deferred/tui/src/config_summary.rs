use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ConfigSummary {
    pub config_path: String,
    pub config_files: Vec<String>,
    pub cwd: String,
    pub profile: String,
    pub model: String,
    pub permission_mode: String,
    pub modules: Vec<ConfigModule>,
    pub enabled_tools: Vec<String>,
    pub registered_tools: Vec<ConfigTool>,
    pub plugins: Vec<ConfigPlugin>,
    pub fallback_text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ConfigModule {
    pub slot: String,
    pub id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ConfigTool {
    pub name: String,
    pub source: String,
    pub safety: String,
    pub description: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ConfigPlugin {
    pub name: String,
    pub version: String,
    pub status: String,
    pub description: String,
}

impl ConfigSummary {
    pub(crate) fn from_output(output: &Value) -> Option<Self> {
        let fallback_text = string_field(output, "display_text").unwrap_or_default();
        let has_structured = output.get("modules").is_some()
            || output.get("registered_tools").is_some()
            || output.get("plugins").is_some();
        if !has_structured && fallback_text.is_empty() {
            return None;
        }

        let model = output
            .get("model")
            .and_then(|model| string_field(model, "label"))
            .unwrap_or_else(|| "unknown".to_owned());

        Some(Self {
            config_path: string_field(output, "config_path")
                .unwrap_or_else(|| "(default discovery / none)".to_owned()),
            config_files: string_array_field(output, "config_files"),
            cwd: string_field(output, "cwd").unwrap_or_default(),
            profile: string_field(output, "profile").unwrap_or_default(),
            model,
            permission_mode: string_field(output, "permission_mode").unwrap_or_default(),
            modules: output
                .get("modules")
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .map(|item| ConfigModule {
                            slot: string_field(item, "slot").unwrap_or_default(),
                            id: string_field(item, "id").unwrap_or_default(),
                        })
                        .collect()
                })
                .unwrap_or_default(),
            enabled_tools: string_array_field(output, "tools_enabled"),
            registered_tools: output
                .get("registered_tools")
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .map(|item| ConfigTool {
                            name: string_field(item, "name").unwrap_or_default(),
                            source: string_field(item, "source").unwrap_or_default(),
                            safety: string_field(item, "safety").unwrap_or_default(),
                            description: string_field(item, "description").unwrap_or_default(),
                        })
                        .collect()
                })
                .unwrap_or_default(),
            plugins: output
                .get("plugins")
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .map(|item| ConfigPlugin {
                            name: string_field(item, "name").unwrap_or_default(),
                            version: string_field(item, "version").unwrap_or_default(),
                            status: string_field(item, "status").unwrap_or_default(),
                            description: string_field(item, "description").unwrap_or_default(),
                        })
                        .collect()
                })
                .unwrap_or_default(),
            fallback_text,
        })
    }
}

fn string_field(value: &Value, field: &str) -> Option<String> {
    value.get(field).and_then(Value::as_str).map(str::to_owned)
}

fn string_array_field(value: &Value, field: &str) -> Vec<String> {
    value
        .get(field)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn parses_structured_config_summary() {
        let summary = ConfigSummary::from_output(&json!({
            "display_text": "fallback",
            "config_path": "/tmp/configs",
            "config_files": ["/tmp/configs/config.toml"],
            "cwd": "/repo",
            "profile": "coding-local",
            "model": { "label": "anthropic/deepseek-v4-pro" },
            "permission_mode": "Normal",
            "modules": [{ "slot": "workflow", "id": "coding.plan_execute_review" }],
            "tools_enabled": ["read_file", "write_file"],
            "registered_tools": [{
                "name": "write_file",
                "source": "dynamic:plugin:dylib",
                "safety": "WritesFiles",
                "description": "Create files"
            }],
            "plugins": [{
                "name": "file-tools",
                "version": "0.1.0",
                "status": "loaded",
                "description": "Basic file tools"
            }]
        }))
        .expect("summary");

        assert_eq!(summary.profile, "coding-local");
        assert_eq!(summary.modules[0].slot, "workflow");
        assert_eq!(summary.registered_tools[0].name, "write_file");
        assert_eq!(summary.plugins[0].name, "file-tools");
    }
}
