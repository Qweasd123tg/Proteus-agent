use proteus_contracts::domain::{ConfigField, ConfigValueSchema, ModuleConfigSchema};
pub fn config_schema(id: &str) -> ModuleConfigSchema {
    let mut fields = Vec::new();
    if id == "opencode_policy" {
        return ModuleConfigSchema {
            fields: vec![
                ConfigField::new(
                    "rules",
                    "Правила",
                    "Правила проверяются по порядку; последнее совпадение определяет действие.",
                    ConfigValueSchema::Array {
                        items: Box::new(ConfigValueSchema::Object {
                            fields: vec![
                                ConfigField::new(
                                    "permission",
                                    "Группа",
                                    "Имя группы разрешений.",
                                    ConfigValueSchema::text(),
                                )
                                .required(),
                                ConfigField::new(
                                    "pattern",
                                    "Шаблон",
                                    "Шаблон команды или пути.",
                                    ConfigValueSchema::text(),
                                )
                                .with_default("*"),
                                ConfigField::new(
                                    "action",
                                    "Действие",
                                    "Что делать при совпадении.",
                                    ConfigValueSchema::choices(&[
                                        ("allow", "Разрешить"),
                                        ("ask", "Спросить"),
                                        ("deny", "Запретить"),
                                    ]),
                                )
                                .required(),
                            ],
                        }),
                    },
                )
                .with_default(serde_json::json!([])),
                ConfigField::new(
                    "groups",
                    "Группы инструментов",
                    "Именованные группы: tools, pattern_args и split_commands.",
                    ConfigValueSchema::Json {},
                )
                .with_default(serde_json::json!({}))
                .advanced(),
            ],
        };
    }
    if id != "allow_all" {
        for (key, title, description) in [
            (
                "allow",
                "Разрешать",
                "Инструменты, выполняемые без подтверждения.",
            ),
            (
                "ask_before",
                "Спрашивать",
                "Инструменты, требующие подтверждения.",
            ),
            (
                "deny",
                "Запрещать",
                "Инструменты, выполнение которых запрещено.",
            ),
            (
                "allow_sandboxed",
                "Разрешать ограниченные вызовы",
                "Инструменты со своей изоляцией; расширение прав требует подтверждения.",
            ),
        ] {
            if id == "ask_write" && matches!(key, "deny" | "allow_sandboxed") {
                continue;
            }
            fields.push(
                ConfigField::new(key, title, description, ConfigValueSchema::strings())
                    .with_default(serde_json::json!([])),
            );
        }
    }
    ModuleConfigSchema { fields }
}
