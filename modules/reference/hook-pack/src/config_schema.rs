use proteus_contracts::domain::{
    ConfigField as Field, ConfigValueSchema as Kind, ModuleConfigSchema,
};
pub fn config_schema(id: &str) -> ModuleConfigSchema {
    ModuleConfigSchema {
        fields: match id {
            "hook.instructions" => vec![
                Field::new(
                    "text",
                    "Инструкции",
                    "Текст, добавляемый к инструкциям модели.",
                    Kind::String {
                        multiline: true,
                        secret: false,
                    },
                )
                .required(),
                Field::new(
                    "placement",
                    "Расположение",
                    "Порядок относительно остальных инструкций.",
                    Kind::choices(&[("prepend", "В начале"), ("append", "В конце")]),
                )
                .required(),
            ],
            "hook.output_budget" => vec![
                Field::new(
                    "max_bytes",
                    "Лимит результата",
                    "Максимальный объём результата инструмента.",
                    Kind::integer(1),
                )
                .unit("байт")
                .required(),
                Field::new(
                    "head_bytes",
                    "Начало результата",
                    "Объём сохраняемого начала; не больше общего лимита.",
                    Kind::integer(0),
                )
                .unit("байт")
                .required(),
            ],
            _ => vec![],
        },
    }
}
