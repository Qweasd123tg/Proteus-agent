use proteus_contracts::domain::{ConfigField, ConfigValueSchema, ModuleConfigSchema};
pub fn config_schema() -> ModuleConfigSchema {
    let defaults = crate::config::CodexDynamicConfig::default();
    ModuleConfigSchema {
        fields: vec![
            ConfigField::new(
                "max_hot_tools",
                "Активные инструменты",
                "Размер набора инструментов, доступного модели на текущем шаге.",
                ConfigValueSchema::integer(1),
            )
            .with_default(defaults.max_hot_tools as u64),
            ConfigField::new(
                "always_include",
                "Всегда доступные",
                "Имена инструментов, включаемых независимо от поиска.",
                ConfigValueSchema::strings(),
            )
            .with_default(serde_json::json!(defaults.always_include)),
        ],
    }
}
