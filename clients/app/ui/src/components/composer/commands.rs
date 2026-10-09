use leptos::{html, prelude::*, task::spawn_local};
use proteus_contracts::app_protocol::{StdioOutput, StdioRequest, commands::UserCommand};
use web_sys::KeyboardEvent;

#[derive(Clone, Copy)]
pub(super) struct Suggestions {
    pub matches: Memo<Vec<UserCommand>>,
    pub selected: RwSignal<usize>,
    error: RwSignal<Option<String>>,
    dismissed: RwSignal<bool>,
    draft: ReadSignal<String>,
    set_draft: WriteSignal<String>,
    textarea: NodeRef<html::Textarea>,
}

fn root(text: &str) -> Option<&str> {
    let text = text.trim_start();
    let root = text.strip_prefix('/')?;
    (!root.starts_with('/') && !root.chars().any(char::is_whitespace)).then_some(root)
}

impl Suggestions {
    pub fn new(
        draft: ReadSignal<String>,
        set_draft: WriteSignal<String>,
        session: ReadSignal<Option<String>>,
        textarea: NodeRef<html::Textarea>,
    ) -> Self {
        let catalog = RwSignal::new(Vec::<UserCommand>::new());
        let error = RwSignal::new(None);
        let selected = RwSignal::new(0);
        let dismissed = RwSignal::new(false);
        let open = Memo::new(move |_| root(&draft.get()).is_some());
        let epoch = RwSignal::new(0u64);
        Effect::new(move |_| {
            let active = session.get();
            let open = open.get();
            epoch.update(|value| *value += 1);
            let current = epoch.get_untracked();
            catalog.set(Vec::new());
            error.set(None);
            dismissed.set(false);
            let Some(active) = active.filter(|_| open) else {
                return;
            };
            spawn_local(async move {
                let response = crate::api::post_json(
                    &crate::api::session_path("/request", &active),
                    &StdioRequest::CommandCatalog { id: None },
                )
                .await;
                if epoch.get_untracked() != current {
                    return;
                }
                let result = match response {
                    Ok(StdioOutput::Response {
                        ok: true,
                        output: Some(value),
                        ..
                    }) => {
                        serde_json::from_value::<Vec<UserCommand>>(value).map_err(|e| e.to_string())
                    }
                    Ok(StdioOutput::Response { error, .. }) => {
                        Err(error.unwrap_or_else(|| "Каталог недоступен".into()))
                    }
                    Ok(_) => Err("Сервер не вернул каталог команд".into()),
                    Err(error) => Err(error),
                };
                match result {
                    Ok(items) => catalog.set(items),
                    Err(message) => error.set(Some(message)),
                }
            });
        });
        Effect::new(move |_| {
            draft.track();
            selected.set(0);
        });
        let matches = Memo::new(move |_| {
            if dismissed.get() {
                return Vec::new();
            }
            let text = draft.get();
            let Some(prefix) = root(&text) else {
                return Vec::new();
            };
            catalog
                .get()
                .into_iter()
                .filter(|item| item.name.starts_with(prefix))
                .take(8)
                .collect()
        });
        Self {
            matches,
            selected,
            error,
            dismissed,
            draft,
            set_draft,
            textarea,
        }
    }

    fn choose(self, command: &UserCommand) {
        self.set_draft.set(format!("/{} ", command.name));
        if let Some(textarea) = self.textarea.get_untracked() {
            let _ = textarea.focus();
        }
    }

    pub fn keydown(self, event: &KeyboardEvent) -> bool {
        if event.is_composing()
            || event.ctrl_key()
            || event.meta_key()
            || event.alt_key()
            || event.shift_key()
        {
            return false;
        }
        if event.key() == "Escape"
            && self.error.get_untracked().is_some()
            && !self.dismissed.get_untracked()
        {
            self.dismissed.set(true);
            event.prevent_default();
            return true;
        }
        let items = self.matches.get_untracked();
        if items.is_empty() {
            return false;
        }
        let index = self.selected.get_untracked().min(items.len() - 1);
        match event.key().as_str() {
            "Escape" => self.dismissed.set(true),
            "ArrowDown" => self.selected.set((index + 1) % items.len()),
            "ArrowUp" => self.selected.set((index + items.len() - 1) % items.len()),
            "Tab" => self.choose(&items[index]),
            "Enter" if root(&self.draft.get_untracked()) != Some(items[index].name.as_str()) => {
                self.choose(&items[index])
            }
            _ => return false,
        }
        event.prevent_default();
        true
    }
}

#[component]
pub(super) fn CommandSuggestions(suggestions: Suggestions) -> impl IntoView {
    view! {
        <Show when=move || !suggestions.matches.get().is_empty() || (suggestions.error.get().is_some() && !suggestions.dismissed.get())>
            <div class="slash-commands" id="slash-command-list" role="listbox" aria-label="Команды">
                {move || suggestions.error.get().map(|error| view! { <div class="slash-command-error">{error}</div> })}
                <For each={move || suggestions.matches.get().into_iter().enumerate().collect::<Vec<_>>()}
                    key=|(index, item)| (*index, item.name.clone()) children=move |(index, item)| {
                        let label = format!("/{}", item.name);
                        let arguments = item.arguments.clone(); let description = item.description.clone();
                        view! { <button type="button" role="option" class="slash-command"
                            class:selected=move || suggestions.selected.get() == index
                            aria-selected=move || suggestions.selected.get() == index
                            on:mousedown=|event| event.prevent_default()
                            on:click=move |_| suggestions.choose(&item)>
                            <strong>{label}</strong><span>{arguments}</span><small>{description}</small>
                        </button> }
                    }/>
            </div>
        </Show>
    }
}
