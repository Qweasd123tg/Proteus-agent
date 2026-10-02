use leptos::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const KEY: &str = "proteus.sidebar.sessions";
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Entry {
    pub title: Option<String>,
    pub pinned: bool,
    pub archived: bool,
}
#[derive(Clone, Copy)]
pub(super) struct Preferences {
    pub entries: RwSignal<BTreeMap<String, Entry>>,
    pub archived: RwSignal<bool>,
    pub error: RwSignal<String>,
}
impl Preferences {
    pub fn new() -> Self {
        let loaded = Self::storage()
            .and_then(|s| {
                s.get_item(KEY)
                    .map_err(|_| "Не удалось прочитать настройки чатов".to_owned())
            })
            .and_then(|value| {
                value
                    .map(|v| {
                        serde_json::from_str(&v)
                            .map_err(|_| "Не удалось прочитать настройки чатов".to_owned())
                    })
                    .transpose()
            });
        let (entries, error) = match loaded {
            Ok(data) => (data.unwrap_or_default(), String::new()),
            Err(error) => (BTreeMap::new(), error),
        };
        Self {
            entries: RwSignal::new(entries),
            archived: RwSignal::new(false),
            error: RwSignal::new(error),
        }
    }
    fn storage() -> Result<web_sys::Storage, String> {
        web_sys::window()
            .and_then(|w| w.local_storage().ok().flatten())
            .ok_or_else(|| "Локальное хранилище недоступно".to_owned())
    }
    pub fn entry(self, id: &str) -> Entry {
        self.entries
            .with(|entries| entries.get(id).cloned().unwrap_or_default())
    }
    pub fn update(self, action: String, id: String, value: String) {
        if action == "error" {
            self.error.set(value);
            return;
        }
        if action == "show-archive" {
            self.archived.update(|v| *v = !*v);
            return;
        }
        let mut entries = self.entries.get_untracked();
        let entry = entries.entry(id).or_default();
        match action.as_str() {
            "pin" => entry.pinned = !entry.pinned,
            "archive" => entry.archived = !entry.archived,
            "rename" => entry.title = Some(value),
            _ => return,
        }
        let saved = Self::storage().and_then(|storage| {
            storage
                .set_item(KEY, &serde_json::to_string(&entries).unwrap())
                .map_err(|_| "Не удалось сохранить настройки чатов".to_owned())
        });
        match saved {
            Ok(()) => {
                self.entries.set(entries);
                self.error.set(String::new());
            }
            Err(error) => self.error.set(error),
        }
    }
}
