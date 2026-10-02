use super::SaveFeedback;
use crate::types::*;
use serde_json::Value;
use std::collections::BTreeMap;

pub(super) type ModuleDrafts = BTreeMap<String, BTreeMap<String, String>>;

pub(super) fn builder_active_modules(builder: &ConfigBuilderSnapshot) -> BTreeMap<String, String> {
    builder
        .active_modules
        .iter()
        .map(|module| (module.slot.clone(), module.id.clone()))
        .collect()
}

pub(super) fn builder_config_texts(builder: &ConfigBuilderSnapshot) -> ModuleDrafts {
    let mut result = ModuleDrafts::new();
    for slot in &builder.slots {
        for module in &slot.modules {
            let text = builder
                .module_config
                .get(&slot.id)
                .and_then(|items| items.get(&module.id))
                .map(pretty_json)
                .unwrap_or_else(|| "{\n}".to_owned());
            result
                .entry(slot.id.clone())
                .or_default()
                .insert(module.id.clone(), text);
        }
    }
    for (slot, configs) in &builder.module_config {
        for (module, value) in configs {
            result
                .entry(slot.clone())
                .or_default()
                .entry(module.clone())
                .or_insert_with(|| pretty_json(value));
        }
    }
    result
}

pub(super) fn parse_module_drafts(
    drafts: &ModuleDrafts,
    baseline: &BTreeMap<String, BTreeMap<String, Value>>,
) -> Result<BTreeMap<String, BTreeMap<String, Value>>, String> {
    let mut parsed = BTreeMap::new();
    for (slot, modules) in drafts {
        for (module, text) in modules {
            let value = if text.trim().is_empty() {
                Value::Object(Default::default())
            } else {
                serde_json::from_str::<Value>(text)
                    .map_err(|error| format!("{slot}/{module}: {error}"))?
            };
            let existed = baseline
                .get(slot)
                .is_some_and(|configs| configs.contains_key(module));
            if existed || value.as_object().is_none_or(|object| !object.is_empty()) {
                parsed
                    .entry(slot.clone())
                    .or_insert_with(BTreeMap::new)
                    .insert(module.clone(), value);
            }
        }
    }
    Ok(parsed)
}

pub(super) fn filter_slots(slots: &[ConfigBuilderSlot], query: &str) -> Vec<ConfigBuilderSlot> {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return slots.to_vec();
    }
    slots
        .iter()
        .filter(|slot| {
            let (display_title, display_responsibility) = slot_presentation(slot);
            slot.id.to_lowercase().contains(&needle)
                || slot.title.to_lowercase().contains(&needle)
                || slot.responsibility.to_lowercase().contains(&needle)
                || display_title.to_lowercase().contains(&needle)
                || display_responsibility.to_lowercase().contains(&needle)
        })
        .cloned()
        .collect()
}

fn pretty_json(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".to_owned())
}
pub(super) fn slot_presentation(slot: &ConfigBuilderSlot) -> (String, String) {
    let localized = match slot.id.as_str() {
        "workflow" => Some((
            "Рабочий цикл",
            "Управляет шагами агента от запроса до результата.",
        )),
        "context" => Some(("Контекст", "Собирает сведения, которые получает модель.")),
        "compactor" => Some((
            "Сжатие истории",
            "Сокращает длинную историю без потери рабочего контекста.",
        )),
        "tool_exposure" => Some((
            "Выбор инструментов",
            "Определяет доступный модели набор инструментов.",
        )),
        "policy" => Some((
            "Подтверждения",
            "Решает, когда действие требует разрешения.",
        )),
        "search" => Some(("Поиск", "Ищет нужные сведения в рабочем проекте.")),
        "memory" => Some((
            "Память",
            "Сохраняет и возвращает сведения между обращениями.",
        )),
        "patch" => Some((
            "Применение правок",
            "Вносит подготовленные изменения в файлы.",
        )),
        _ => None,
    };
    localized
        .map(|(title, description)| (title.to_owned(), description.to_owned()))
        .unwrap_or_else(|| (slot.title.clone(), slot.responsibility.clone()))
}
pub(super) fn permission_label(mode: &str) -> String {
    match mode {
        "plan" => "Только чтение · plan".into(),
        "normal" => "Запрашивать разрешение · normal".into(),
        "auto" => "Автоматически · auto".into(),
        _ => mode.to_owned(),
    }
}

pub(super) fn save_state_class(
    saving: bool,
    dirty: bool,
    valid_json: bool,
    field_errors: bool,
    feedback: &SaveFeedback,
) -> &'static str {
    let state = if saving {
        "saving"
    } else if !valid_json || field_errors || matches!(feedback, SaveFeedback::Error(_)) {
        "error"
    } else if dirty {
        "dirty"
    } else {
        "saved"
    };
    match state {
        "saving" => "cfg-save-state saving",
        "error" => "cfg-save-state error",
        "dirty" => "cfg-save-state dirty",
        _ => "cfg-save-state saved",
    }
}

pub(super) fn save_state_text(
    saving: bool,
    dirty: bool,
    valid_json: bool,
    field_errors: bool,
    feedback: &SaveFeedback,
) -> String {
    if saving {
        return "Сохраняю сборку…".into();
    }
    if !valid_json {
        return "Исправьте JSON в параметрах модулей".into();
    }
    if field_errors {
        return "Исправьте отмеченные значения параметров".into();
    }
    if let SaveFeedback::Error(error) = feedback {
        return error.clone();
    }
    if dirty {
        return "Есть несохранённые изменения".into();
    }
    if matches!(feedback, SaveFeedback::Saved) {
        "Сохранено · runtime перезагружен".into()
    } else {
        "Изменений нет".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_drafts_preserve_each_module_and_reject_invalid_json() {
        let drafts = BTreeMap::from([(
            "slot".into(),
            BTreeMap::from([
                ("a".into(), "{\"x\":1}".into()),
                ("b".into(), "{\"x\":2}".into()),
            ]),
        )]);
        let parsed = parse_module_drafts(&drafts, &BTreeMap::new()).unwrap();
        assert_eq!(parsed["slot"]["a"]["x"], 1);
        assert_eq!(parsed["slot"]["b"]["x"], 2);
        let invalid = BTreeMap::from([(
            "slot".into(),
            BTreeMap::from([("a".into(), "{ nope".into())]),
        )]);
        assert!(
            parse_module_drafts(&invalid, &BTreeMap::new())
                .unwrap_err()
                .contains("slot/a")
        );
    }

    #[test]
    fn untouched_empty_module_configs_are_not_added_to_request() {
        let drafts = BTreeMap::from([(
            "slot".into(),
            BTreeMap::from([
                ("new".into(), "{}".into()),
                ("scalar".into(), "null".into()),
            ]),
        )]);
        let parsed = parse_module_drafts(&drafts, &BTreeMap::new()).unwrap();
        assert!(!parsed.get("slot").unwrap().contains_key("new"));
        assert_eq!(parsed["slot"]["scalar"], Value::Null);
    }

    #[test]
    fn slot_filter_matches_title_id_and_description() {
        let slots = vec![ConfigBuilderSlot {
            id: "memory".into(),
            title: "Память".into(),
            responsibility: "История сессии".into(),
            ..Default::default()
        }];
        assert_eq!(filter_slots(&slots, "пам").len(), 1);
        assert_eq!(filter_slots(&slots, "memory").len(), 1);
        assert_eq!(filter_slots(&slots, "история").len(), 1);
        assert!(filter_slots(&slots, "tools").is_empty());
    }
}
