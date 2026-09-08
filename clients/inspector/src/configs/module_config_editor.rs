#[cfg(test)]
use std::collections::BTreeMap;

use leptos::prelude::*;
use serde_json::{Map, Value};

use super::{DraftErrors, builder::ModuleDrafts};

#[component]
pub(crate) fn ModuleConfigEditor(
    slot_id: String,
    module_id: Signal<String>,
    module_description: Signal<String>,
    module_capabilities: Signal<Vec<String>>,
    draft_config_texts: ReadSignal<ModuleDrafts>,
    draft_errors: ReadSignal<DraftErrors>,
    set_draft_config_texts: WriteSignal<ModuleDrafts>,
    set_draft_errors: WriteSignal<DraftErrors>,
) -> impl IntoView {
    let raw_mode = RwSignal::new(false);
    let text_slot = slot_id.clone();
    let text =
        Memo::new(move |_| current_text(&draft_config_texts.get(), &text_slot, &module_id.get()));
    let parsed = Memo::new(move |_| parse_object(&text.get()));
    let syntax_error = Memo::new(move |_| json_error(&text.get()));
    let error_slot = slot_id.clone();
    Effect::new(move |_| {
        let error_key = error_key(&error_slot, &module_id.get(), "@raw");
        match syntax_error.get() {
            None => clear_error(set_draft_errors, &error_key),
            Some(error) => set_error(set_draft_errors, error_key, error),
        }
    });

    let form_slot = StoredValue::new(slot_id.clone());
    let add_form_slot = StoredValue::new(slot_id.clone());
    let raw_slot = slot_id.clone();
    let current_error_slot = slot_id.clone();
    let has_current_errors = Memo::new(move |_| {
        has_module_errors(&draft_errors.get(), &current_error_slot, &module_id.get())
    });
    let toggle_mode = move |ev: web_sys::MouseEvent| {
        ev.prevent_default();
        ev.stop_propagation();
        if !has_current_errors.get_untracked() {
            raw_mode.update(|raw| *raw = !*raw);
        }
    };
    view! {
        <details class="config-builder-field config-editor">
            <summary class="config-editor-head">
                <span>"Параметры" {move || parsed.get().ok().filter(|v| !v.is_empty()).map(|v| format!(" · {}", v.len())).unwrap_or_default()}</span>
                <button
                    type="button"
                    class="config-editor-toggle"
                    disabled=move || has_current_errors.get()
                    title=move || if has_current_errors.get() { "Сначала исправьте отмеченное значение" } else { "Переключить режим параметров" }
                    on:click=toggle_mode
                >{move || if raw_mode.get() { "Форма" } else { "JSON" }}</button>
            </summary>
            <div class="config-builder-modules">
                <p class="config-builder-module-note">{move || module_description.get()}</p>
                <div class="config-chip-row">
                    <For each=move || module_capabilities.get() key=|capability| capability.clone() children=move |capability| {
                        view! { <span class="config-chip">{capability}</span> }
                    }/>
                </div>
            </div>
            <Show
                when=move || !raw_mode.get()
                fallback=move || {
                    let slot = raw_slot.clone();
                    view! {
                        <textarea spellcheck="false" aria-label="Параметры модуля в JSON"
                            prop:value=move || text.get()
                            on:input:target=move |ev| {
                                set_text(set_draft_config_texts, &slot, &module_id.get_untracked(), ev.target().value());
                            }></textarea>
                        {move || syntax_error.get().map(|error| view! { <span class="config-editor-error">{error}</span> })}
                    }
                }
            >
                <div class="config-editor-rows">
                    <For
                        each=move || {
                            let active_module = module_id.get();
                            parsed.get().unwrap_or_default().into_iter()
                                .map(|(key, value)| (active_module.clone(), key, value))
                                .collect::<Vec<_>>()
                        }
                        key=|(module, key, _)| format!("{module}:{key}")
                        children=move |(module, key, value)| view! {
                            <EditorRow slot_id=form_slot.get_value() module_id=module key_name=key value set_draft_config_texts set_draft_errors/>
                        }
                    />
                    <For
                        each=move || vec![module_id.get()]
                        key=|module| module.clone()
                        children=move |module| view! {
                            <AddKeyRow slot_id=add_form_slot.get_value() module_id=module set_draft_config_texts/>
                        }
                    />
                    <Show when=move || parsed.get().is_err()>
                        <span class="config-editor-note">"Для этого значения используйте режим JSON"</span>
                    </Show>
                </div>
            </Show>
        </details>
    }
}

#[component]
fn EditorRow(
    slot_id: String,
    module_id: String,
    key_name: String,
    value: Value,
    set_draft_config_texts: WriteSignal<ModuleDrafts>,
    set_draft_errors: WriteSignal<DraftErrors>,
) -> impl IntoView {
    let field_error = RwSignal::new(None::<String>);
    let field_error_key = error_key(&slot_id, &module_id, &key_name);
    let remove_slot = slot_id.clone();
    let remove_module = module_id.clone();
    let remove_key = key_name.clone();
    let remove = move |_| {
        let key = remove_key.clone();
        update_object(
            set_draft_config_texts,
            &remove_slot,
            &remove_module,
            move |map| {
                map.remove(&key);
            },
        );
        clear_error(
            set_draft_errors,
            &error_key(&remove_slot, &remove_module, &remove_key),
        );
    };

    let input = match value {
        Value::Bool(current) => {
            let slot = slot_id.clone();
            let module = module_id.clone();
            let key = key_name.clone();
            view! { <select prop:value=current.to_string() on:change:target=move |ev| {
                let next = Value::Bool(ev.target().value() == "true"); let key = key.clone();
                update_object(set_draft_config_texts, &slot, &module, move |map| { map.insert(key, next); });
            }><option value="true">"true"</option><option value="false">"false"</option></select> }.into_any()
        }
        Value::Number(current) => {
            let raw = RwSignal::new(current.to_string());
            let slot = slot_id.clone();
            let module = module_id.clone();
            let key = key_name.clone();
            let field_key = field_error_key.clone();
            view! { <input type="text" inputmode="decimal" prop:value=move || raw.get() on:input:target=move |ev| {
                let next_raw = ev.target().value(); raw.set(next_raw.clone());
                if let Some(next) = parse_number(&next_raw) {
                    field_error.set(None); clear_error(set_draft_errors, &field_key); let key = key.clone();
                    update_object(set_draft_config_texts, &slot, &module, move |map| { map.insert(key, next); });
                } else {
                    let message = "Введите корректное число".to_owned(); field_error.set(Some(message.clone())); set_error(set_draft_errors, field_key.clone(), message);
                }
            }/> }.into_any()
        }
        Value::String(current) => {
            let slot = slot_id.clone();
            let module = module_id.clone();
            let key = key_name.clone();
            view! { <input type="text" prop:value=current on:input:target=move |ev| {
                let next = Value::String(ev.target().value()); let key = key.clone();
                update_object(set_draft_config_texts, &slot, &module, move |map| { map.insert(key, next); });
            }/> }.into_any()
        }
        other => {
            let raw =
                RwSignal::new(serde_json::to_string(&other).unwrap_or_else(|_| "null".to_owned()));
            let slot = slot_id.clone();
            let module = module_id.clone();
            let key = key_name.clone();
            let field_key = field_error_key.clone();
            view! { <input type="text" class="config-editor-json" prop:value=move || raw.get() on:input:target=move |ev| {
                let next_raw = ev.target().value(); raw.set(next_raw.clone());
                match serde_json::from_str::<Value>(&next_raw) {
                    Ok(next) => { field_error.set(None); clear_error(set_draft_errors, &field_key); let key = key.clone(); update_object(set_draft_config_texts, &slot, &module, move |map| { map.insert(key, next); }); }
                    Err(_) => { let message = "Введите корректное JSON-значение".to_owned(); field_error.set(Some(message.clone())); set_error(set_draft_errors, field_key.clone(), message); }
                }
            }/> }.into_any()
        }
    };

    view! { <div class="config-editor-row" class:error=move || field_error.get().is_some()>
        <code class="config-editor-key">{key_name}</code>{input}
        <button type="button" class="config-editor-remove" title="Удалить параметр" aria-label="Удалить параметр" on:click=remove>"×"</button>
        {move || field_error.get().map(|error| view! { <span class="config-editor-error">{error}</span> })}
    </div> }
}

#[component]
fn AddKeyRow(
    slot_id: String,
    module_id: String,
    set_draft_config_texts: WriteSignal<ModuleDrafts>,
) -> impl IntoView {
    let key_text = RwSignal::new(String::new());
    let value_text = RwSignal::new(String::new());
    let add = move |_| {
        let key = key_text.get_untracked().trim().to_owned();
        if key.is_empty() {
            return;
        }
        let value = parse_lenient(&value_text.get_untracked());
        update_object(set_draft_config_texts, &slot_id, &module_id, move |map| {
            map.insert(key, value);
        });
        key_text.set(String::new());
        value_text.set(String::new());
    };
    view! { <div class="config-editor-row config-editor-add">
        <input type="text" placeholder="Ключ" prop:value=move || key_text.get() on:input:target=move |ev| key_text.set(ev.target().value())/>
        <input type="text" placeholder="Значение (строка или JSON)" prop:value=move || value_text.get() on:input:target=move |ev| value_text.set(ev.target().value())/>
        <button type="button" class="config-editor-remove" title="Добавить параметр" aria-label="Добавить параметр" on:click=add>"+"</button>
    </div> }
}

fn current_text(drafts: &ModuleDrafts, slot: &str, module: &str) -> String {
    drafts
        .get(slot)
        .and_then(|items| items.get(module))
        .cloned()
        .unwrap_or_else(|| "{\n}".to_owned())
}
fn set_text(setter: WriteSignal<ModuleDrafts>, slot: &str, module: &str, text: String) {
    setter.update(|items| {
        items
            .entry(slot.to_owned())
            .or_default()
            .insert(module.to_owned(), text);
    });
}
fn parse_object(text: &str) -> Result<Vec<(String, Value)>, String> {
    if text.trim().is_empty() {
        return Ok(Vec::new());
    }
    match serde_json::from_str::<Value>(text.trim()) {
        Ok(Value::Object(map)) => Ok(map.into_iter().collect()),
        Ok(_) => Err("Параметры должны быть JSON-объектом".to_owned()),
        Err(error) => Err(error.to_string()),
    }
}
fn json_error(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    serde_json::from_str::<Value>(trimmed)
        .err()
        .map(|error| error.to_string())
}
fn update_object(
    setter: WriteSignal<ModuleDrafts>,
    slot: &str,
    module: &str,
    mutate: impl FnOnce(&mut Map<String, Value>),
) {
    setter.update(|items| {
        let current = items
            .get(slot)
            .and_then(|modules| modules.get(module))
            .map(String::as_str)
            .unwrap_or("{}");
        let mut map =
            serde_json::from_str::<Map<String, Value>>(current.trim()).unwrap_or_default();
        mutate(&mut map);
        let pretty =
            serde_json::to_string_pretty(&Value::Object(map)).unwrap_or_else(|_| "{\n}".to_owned());
        items
            .entry(slot.to_owned())
            .or_default()
            .insert(module.to_owned(), pretty);
    });
}
fn error_key(slot: &str, module: &str, field: &str) -> String {
    format!("{slot}\u{1f}{module}\u{1f}{field}")
}
pub(super) fn has_module_errors(errors: &DraftErrors, slot: &str, module: &str) -> bool {
    let prefix = format!("{slot}\u{1f}{module}\u{1f}");
    errors.keys().any(|key| key.starts_with(&prefix))
}
fn set_error(setter: WriteSignal<DraftErrors>, key: String, message: String) {
    setter.update(|errors| {
        errors.insert(key, message);
    });
}
fn clear_error(setter: WriteSignal<DraftErrors>, key: &str) {
    setter.update(|errors| {
        errors.remove(key);
    });
}
fn parse_number(raw: &str) -> Option<Value> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(int) = trimmed.parse::<i64>() {
        return Some(Value::Number(int.into()));
    }
    trimmed
        .parse::<f64>()
        .ok()
        .and_then(serde_json::Number::from_f64)
        .map(Value::Number)
}
fn parse_lenient(raw: &str) -> Value {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Value::String(String::new());
    }
    serde_json::from_str::<Value>(trimmed).unwrap_or_else(|_| Value::String(trimmed.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validation_rejects_non_object_and_bad_numbers() {
        assert!(parse_object("[]").is_err());
        assert!(parse_object("{ nope").is_err());
        assert!(parse_number("12.5").is_some());
        assert!(parse_number("-").is_none());
    }
    #[test]
    fn nested_lookup_keeps_module_drafts_separate() {
        let drafts = BTreeMap::from([(
            "slot".into(),
            BTreeMap::from([
                ("a".into(), "{\"a\":1}".into()),
                ("b".into(), "{\"b\":2}".into()),
            ]),
        )]);
        assert!(current_text(&drafts, "slot", "a").contains("\"a\""));
        assert!(current_text(&drafts, "slot", "b").contains("\"b\""));
    }
}
