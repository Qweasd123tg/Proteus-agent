use proteus_contracts::domain::{ConfigField, ConfigValueSchema, ModuleConfigSchema};
pub fn config_schema(id: &str) -> ModuleConfigSchema {
    ModuleConfigSchema {
        fields: if id == "coding.codex_loop" {
            vec![ConfigField::new("stream_max_retries", "Повторы потока", "Число переподключений при обрыве ответа. Значения выше 100 ограничиваются сотней.", ConfigValueSchema::integer(0)).with_default(5)]
        } else {
            vec![]
        },
    }
}
