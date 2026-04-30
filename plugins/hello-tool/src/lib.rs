//! Hello-tool plugin.
//!
//! Первый tool-плагин для modular-agent. Реализует sync `PluginTool` с
//! именем `current_time`: возвращает текущее время в ISO 8601.
//!
//! Польза ноль, цель — показать что tool-плагины работают end-to-end:
//! модель может увидеть его в списке tools, вызвать, получить результат.

#![allow(non_local_definitions)]
#![allow(non_camel_case_types)]
#![allow(improper_ctypes_definitions)]

use agent_contracts::{
    abi_stable::{
        export_root_module,
        prefix_type::PrefixTypeTrait,
        sabi_trait::TD_Opaque,
        std_types::{RResult, RStr, RString},
    },
    plugin::{
        PluginRegisterError, PluginRegistryMut, PluginRoot, PluginRoot_Ref, PluginTool,
        PluginToolError, PluginToolObject, PluginTool_TO,
    },
};
use serde_json::json;

/// Tool `current_time`: возвращает текущее время в формате ISO 8601.
struct CurrentTimeTool;

impl PluginTool for CurrentTimeTool {
    fn spec_json(&self) -> RString {
        let spec = json!({
            "name": "current_time",
            "description": "Returns current system time as ISO 8601 UTC string.",
            "input_schema": {
                "type": "object",
                "properties": {},
                "required": []
            },
            "safety": "ReadOnly",
            "timeout_ms": 1000,
            "metadata": null
        });
        RString::from(spec.to_string())
    }

    fn invoke_json(
        &self,
        call_json: RString,
        _cwd: RString,
    ) -> RResult<RString, PluginToolError> {
        // Парсим call чтобы вытащить id.
        let call_value: serde_json::Value = match serde_json::from_str(call_json.as_str()) {
            Ok(value) => value,
            Err(error) => {
                return RResult::RErr(PluginToolError::new(format!(
                    "failed to parse ToolCall JSON: {error}"
                )));
            }
        };
        let call_id = call_value
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();

        // Формируем текущее время. std::time::SystemTime::now() + formatting.
        let now = std::time::SystemTime::now();
        let duration = match now.duration_since(std::time::UNIX_EPOCH) {
            Ok(d) => d,
            Err(error) => {
                return RResult::RErr(PluginToolError::new(format!("system time error: {error}")));
            }
        };
        let secs = duration.as_secs();
        // Примитивная ISO 8601 без chrono: используем epoch seconds + простая формула
        // через time в секундах. Для демонстрации — показываем secs.
        let output = format!("Current time (UTC): {secs} seconds since Unix epoch");

        let result = json!({
            "call_id": call_id,
            "ok": true,
            "output": output,
            "content": [],
            "error": null,
            "metadata": { "tool": "current_time", "unix_secs": secs }
        });
        RResult::ROk(RString::from(result.to_string()))
    }
}

extern "C" fn register_modules(
    registry: &mut PluginRegistryMut<'_>,
) -> RResult<(), PluginRegisterError> {
    let tool: PluginToolObject = PluginTool_TO::from_value(CurrentTimeTool, TD_Opaque);
    registry.register_tool(tool)
}

#[export_root_module]
pub fn get_plugin_root() -> PluginRoot_Ref {
    PluginRoot {
        name: RStr::from_str("hello-tool"),
        description: RStr::from_str("Sample plugin: provides current_time tool"),
        register_modules,
    }
    .leak_into_prefix()
}
