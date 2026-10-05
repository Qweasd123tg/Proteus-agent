use crate::config::{CodexContextConfig, RepoAwareContextConfig, SimpleContextConfig};
use proteus_contracts::domain::{
    ConfigField as Field, ConfigValueSchema as Kind, ModuleConfigSchema,
};
use serde_json::Value;

pub fn config_schema(id: &str) -> ModuleConfigSchema {
    let defaults = match id {
        "simple" => serde_json::to_value(SimpleContextConfig::default()),
        "repo_aware" => serde_json::to_value(RepoAwareContextConfig::default()),
        "codex_context" => serde_json::to_value(CodexContextConfig::default()),
        _ => return ModuleConfigSchema::default(),
    }
    .expect("context defaults");
    let mut fields = vec![Field::new(
        "max_search_results",
        "Результаты поиска",
        "Максимум фрагментов из поиска. 0 отключает поиск.",
        Kind::integer(0),
    )];
    if id != "simple" {
        let providers = if id == "codex_context" {
            vec![
                ("project_instructions", "Инструкции проекта"),
                ("environment", "Окружение"),
                ("git_status", "Состояние Git"),
                ("git_diff", "Изменения Git"),
                ("repo_tree", "Дерево проекта"),
                ("manifest", "Файлы проекта"),
                ("search", "Поиск"),
            ]
        } else {
            vec![
                ("project_instructions", "Инструкции проекта"),
                ("manifest", "Файлы проекта"),
                ("git_status", "Состояние Git"),
                ("repo_tree", "Дерево проекта"),
                ("memory", "Память"),
                ("search", "Поиск"),
            ]
        };
        fields.insert(
            0,
            Field::new(
                "providers",
                "Источники контекста",
                "Фрагменты собираются в указанном порядке.",
                Kind::Array {
                    items: Box::new(Kind::choices(&providers)),
                },
            ),
        );
        fields.push(
            Field::new(
                "max_context_bytes",
                "Объём контекста",
                "Общий бюджет сведений о проекте.",
                Kind::integer(0),
            )
            .unit("байт"),
        );
        fields.push(Field::new(
            "memory_limit",
            "Записи памяти",
            "Максимум записей, запрашиваемых из памяти.",
            Kind::integer(0),
        ));
        for (key, title, description, unit) in [
            (
                "max_bytes_per_file",
                "Объём одного файла",
                "Лимит чтения файлов проекта.",
                Some("байт"),
            ),
            (
                "repo_tree_max_entries",
                "Элементы дерева",
                "Максимум элементов в дереве проекта.",
                None,
            ),
            (
                "repo_tree_max_depth",
                "Глубина дерева",
                "Максимальная глубина обхода каталогов.",
                None,
            ),
            (
                "project_doc_max_bytes",
                "Объём инструкций",
                "Общий бюджет инструкций проекта.",
                Some("байт"),
            ),
            (
                "git_diff_max_bytes",
                "Объём изменений Git",
                "Максимальный объём diff в контексте.",
                Some("байт"),
            ),
        ] {
            if defaults.get(key).is_some() {
                let mut field = Field::new(key, title, description, Kind::integer(0)).advanced();
                if let Some(unit) = unit {
                    field = field.unit(unit);
                }
                fields.push(field);
            }
        }
        for (key, title, description) in [
            (
                "repo_tree_skip_entries",
                "Исключения дерева",
                "Каталоги и файлы, исключённые из обхода.",
            ),
            (
                "project_instruction_files",
                "Файлы инструкций",
                "Имена файлов инструкций по порядку приоритета.",
            ),
            (
                "manifest_files",
                "Файлы проекта",
                "Файлы, содержимое которых добавляется в контекст.",
            ),
        ] {
            fields.push(Field::new(key, title, description, Kind::strings()).advanced());
        }
    }
    for field in &mut fields {
        field.default = defaults
            .get(&field.key)
            .cloned()
            .filter(|v| *v != Value::Null);
    }
    ModuleConfigSchema { fields }
}
