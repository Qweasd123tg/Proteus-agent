use proteus_contracts::domain::{ConfigField, ConfigValueSchema, ModuleConfigSchema};
pub fn config_schema() -> ModuleConfigSchema {
    ModuleConfigSchema {
        fields: vec![
            ConfigField::new(
                "trigger_tokens",
                "Порог сжатия",
                "Без явного значения порог определяется моделью и текущим запросом.",
                ConfigValueSchema::Integer {
                    minimum: Some(1),
                    maximum: Some(u32::MAX as i64),
                },
            )
            .unit("токенов"),
            ConfigField::new(
                "stream_max_retries",
                "Повторы потока",
                "Число переподключений при обрыве ответа. Значения выше 100 ограничиваются сотней.",
                ConfigValueSchema::integer(0),
            )
            .with_default(5)
            .advanced(),
        ],
    }
}
