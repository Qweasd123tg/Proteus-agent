use std::collections::BTreeMap;

use leptos::prelude::*;
use serde_json::{Map, Value};

/// Структурный редактор module_config одного slot: пары ключ-значение поверх
/// JSON-текста черновика. Текст остаётся source of truth, поэтому save-поток
/// и raw JSON режим работают без отдельного состояния.
#[component]
pub(crate) fn ModuleConfigEditor(
    slot_id: String,
    draft_config_texts: ReadSignal<BTreeMap<String, String>>,
    set_draft_config_texts: WriteSignal<BTreeMap<String, String>>,
) -> impl IntoView {
    let raw_mode = RwSignal::new(false);
    let slot_for_text = slot_id.clone();
    let text = Memo::new(move |_| {
        draft_config_texts
            .with(|items| items.get(&slot_for_text).cloned())
            .unwrap_or_else(|| "{\n}".to_owned())
    });
    let parsed = Memo::new(move |_| parse_object(&text.get()));

    let slot_for_form = slot_id.clone();
    let slot_for_raw = slot_id.clone();

    view! {
        <div class="config-builder-field config-editor">
            <div class="config-editor-head">
                <span>"module_config"</span>
                <button
                    type="button"
                    class="config-editor-toggle"
                    on:click=move |_| raw_mode.update(|raw| *raw = !*raw)
                >
                    {move || if raw_mode.get() { "форма" } else { "JSON" }}
                </button>
            </div>
            {move || {
                let show_form = !raw_mode.get() && parsed.get().is_ok();
                if show_form {
                    let entries = parsed.get().unwrap_or_default();
                    let slot_id = slot_for_form.clone();
                    view! {
                        <div class="config-editor-rows">
                            <For
                                each=move || entries.clone()
                                key=|(key, value)| format!("{key}:{value}")
                                children=move |(key, value)| {
                                    view! {
                                        <EditorRow
                                            slot_id=slot_id.clone()
                                            key_name=key
                                            value
                                            set_draft_config_texts
                                        />
                                    }
                                }
                            />
                            <AddKeyRow slot_id=slot_for_form.clone() set_draft_config_texts/>
                        </div>
                    }
                    .into_any()
                } else {
                    let slot_id = slot_for_raw.clone();
                    view! {
                        <textarea
                            spellcheck="false"
                            prop:value=move || text.get()
                            on:input:target=move |ev| {
                                let value = ev.target().value();
                                set_draft_config_texts.update(|items| {
                                    items.insert(slot_id.clone(), value);
                                });
                            }
                        ></textarea>
                        {move || {
                            parsed
                                .get()
                                .err()
                                .map(|error| {
                                    view! { <span class="config-editor-error">{error}</span> }
                                })
                        }}
                    }
                    .into_any()
                }
            }}
        </div>
    }
}

#[component]
fn EditorRow(
    slot_id: String,
    key_name: String,
    value: Value,
    set_draft_config_texts: WriteSignal<BTreeMap<String, String>>,
) -> impl IntoView {
    let json_error = RwSignal::new(false);
    let slot_for_remove = slot_id.clone();
    let key_for_remove = key_name.clone();
    let remove = move |_| {
        let key = key_for_remove.clone();
        update_object(set_draft_config_texts, &slot_for_remove, move |map| {
            map.remove(&key);
        });
    };

    let input = {
        let slot_id = slot_id.clone();
        let key = key_name.clone();
        match value {
            Value::Bool(current) => view! {
                <select
                    prop:value=current.to_string()
                    on:change:target=move |ev| {
                        let next = Value::Bool(ev.target().value() == "true");
                        let key = key.clone();
                        update_object(set_draft_config_texts, &slot_id, move |map| {
                            map.insert(key, next);
                        });
                    }
                >
                    <option value="true">"true"</option>
                    <option value="false">"false"</option>
                </select>
            }
            .into_any(),
            Value::Number(current) => view! {
                <input
                    type="number"
                    step="any"
                    prop:value=current.to_string()
                    on:change:target=move |ev| {
                        let Some(next) = parse_number(&ev.target().value()) else {
                            return;
                        };
                        let key = key.clone();
                        update_object(set_draft_config_texts, &slot_id, move |map| {
                            map.insert(key, next);
                        });
                    }
                />
            }
            .into_any(),
            Value::String(current) => view! {
                <input
                    type="text"
                    prop:value=current
                    on:change:target=move |ev| {
                        let next = Value::String(ev.target().value());
                        let key = key.clone();
                        update_object(set_draft_config_texts, &slot_id, move |map| {
                            map.insert(key, next);
                        });
                    }
                />
            }
            .into_any(),
            other => {
                let serialized =
                    serde_json::to_string(&other).unwrap_or_else(|_| "null".to_owned());
                view! {
                    <input
                        type="text"
                        class="config-editor-json"
                        prop:value=serialized
                        on:change:target=move |ev| {
                            match serde_json::from_str::<Value>(&ev.target().value()) {
                                Ok(next) => {
                                    json_error.set(false);
                                    let key = key.clone();
                                    update_object(set_draft_config_texts, &slot_id, move |map| {
                                        map.insert(key, next);
                                    });
                                }
                                Err(_) => json_error.set(true),
                            }
                        }
                    />
                }
                .into_any()
            }
        }
    };

    view! {
        <div class="config-editor-row" class:error=move || json_error.get()>
            <code class="config-editor-key">{key_name}</code>
            {input}
            <button type="button" class="config-editor-remove" title="удалить ключ" on:click=remove>
                "×"
            </button>
        </div>
    }
}

#[component]
fn AddKeyRow(
    slot_id: String,
    set_draft_config_texts: WriteSignal<BTreeMap<String, String>>,
) -> impl IntoView {
    let key_text = RwSignal::new(String::new());
    let value_text = RwSignal::new(String::new());

    let add = move |_| {
        let key = key_text.get_untracked().trim().to_owned();
        if key.is_empty() {
            return;
        }
        let value = parse_lenient(&value_text.get_untracked());
        update_object(set_draft_config_texts, &slot_id, move |map| {
            map.insert(key, value);
        });
        key_text.set(String::new());
        value_text.set(String::new());
    };

    view! {
        <div class="config-editor-row config-editor-add">
            <input
                type="text"
                placeholder="ключ"
                prop:value=move || key_text.get()
                on:input:target=move |ev| key_text.set(ev.target().value())
            />
            <input
                type="text"
                placeholder="значение (строка или JSON)"
                prop:value=move || value_text.get()
                on:input:target=move |ev| value_text.set(ev.target().value())
            />
            <button type="button" class="config-editor-remove" title="добавить ключ" on:click=add>
                "+"
            </button>
        </div>
    }
}

fn parse_object(text: &str) -> Result<Vec<(String, Value)>, String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    match serde_json::from_str::<Value>(trimmed) {
        Ok(Value::Object(map)) => Ok(map.into_iter().collect()),
        Ok(_) => Err("module_config должен быть JSON-объектом".to_owned()),
        Err(error) => Err(error.to_string()),
    }
}

/// Обновляет черновик slot-а как JSON-объект и записывает обратно pretty JSON.
/// Вызывается только из режима формы, где текст уже валидный объект.
fn update_object(
    set_draft_config_texts: WriteSignal<BTreeMap<String, String>>,
    slot_id: &str,
    mutate: impl FnOnce(&mut Map<String, Value>),
) {
    set_draft_config_texts.update(|items| {
        let current = items.get(slot_id).map(String::as_str).unwrap_or("{}");
        let mut map =
            serde_json::from_str::<Map<String, Value>>(current.trim()).unwrap_or_default();
        mutate(&mut map);
        let pretty =
            serde_json::to_string_pretty(&Value::Object(map)).unwrap_or_else(|_| "{\n}".to_owned());
        items.insert(slot_id.to_owned(), pretty);
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
