use super::icons::{EditIcon, QueueIcon, TrashIcon};
use crate::{actions::AppActions, types::QueuedPromptInfo};
use leptos::{html, prelude::*, task::spawn_local};

#[component]
pub(super) fn QueuedPrompts(
    items: ReadSignal<Vec<QueuedPromptInfo>>,
    actions: AppActions,
) -> impl IntoView {
    let editing = RwSignal::new(None::<proteus_contracts::domain::MessageId>);
    let draft = RwSignal::new(String::new());
    let busy = RwSignal::new(false);
    let error = RwSignal::new(None::<String>);
    let editor = NodeRef::<html::Textarea>::new();
    let pending = Memo::new(move |_| {
        editing.with(|id| {
            id.as_ref().is_some_and(|id| {
                items.with(|items| items.iter().any(|item| &item.message_id == id))
            })
        })
    });
    Effect::new(move |_| {
        if editing.get().is_some()
            && let Some(editor) = editor.get()
        {
            let _ = editor.focus();
        }
    });
    let save = move || {
        if busy.get_untracked() || !pending.get_untracked() {
            return;
        }
        let Some(id) = editing.get_untracked() else {
            return;
        };
        let text = draft.get_untracked().trim().to_owned();
        if text.is_empty() {
            return;
        }
        busy.set(true);
        error.set(None);
        spawn_local(async move {
            match actions.edit_queued_prompt(id, text).await {
                Ok(()) => {
                    editing.try_set(None);
                }
                Err(message) => {
                    error.try_set(Some(message));
                }
            }
            busy.try_set(false);
        });
    };
    view! {
        <Show when=move || !items.with(Vec::is_empty) || editing.get().is_some() || error.get().is_some()>
            <section class="composer-queue" aria-label="Сообщения в очереди">
                <div class="composer-queue-list">
                    <For each=move || items.with(|items| items.iter().map(|item| item.message_id.clone()).collect::<Vec<_>>()) key=|id| id.clone() children=move |id| {
                        let current_id = id.clone();
                        let text = Memo::new(move |_| items.with(|items| items.iter().find(|item| item.message_id == current_id).map(|item| item.text.clone()).unwrap_or_default()));
                        let edit_id = id.clone();
                        let delete_id = id.clone();
                        view! {
                            <div class="queued-prompt-row" data-queued-id=id.to_string()>
                                <QueueIcon />
                                <span class="queued-prompt-text" title=move || text.get()>{move || text.get()}</span>
                                <button type="button" class="queue-icon-button" aria-label="Редактировать сообщение" title="Редактировать сообщение"
                                    disabled=move || busy.get() || editing.get().is_some()
                                    on:click=move |_| { draft.set(text.get_untracked()); editing.set(Some(edit_id.clone())); error.set(None); }><EditIcon /></button>
                                <button type="button" class="queue-icon-button queue-delete" aria-label="Удалить из очереди" title="Удалить из очереди"
                                    disabled=move || busy.get() || editing.get().is_some()
                                    on:click=move |_| {
                                        let id = delete_id.clone(); busy.set(true); error.set(None);
                                        spawn_local(async move {
                                            if let Err(message) = actions.delete_queued_prompt(id).await { error.try_set(Some(message)); }
                                            busy.try_set(false);
                                        });
                                    }><TrashIcon /></button>
                            </div>
                        }
                    } />
                </div>
                <Show when=move || editing.get().is_some()>
                    <div class="queued-prompt-editor">
                        <textarea node_ref=editor aria-label="Текст сообщения в очереди" prop:value=move || draft.get()
                            on:input:target=move |event| draft.set(event.target().value())
                            on:keydown=move |event: web_sys::KeyboardEvent| {
                                if event.key() == "Escape" {
                                    event.prevent_default(); event.stop_propagation();
                                    if !busy.get_untracked() { editing.set(None); error.set(None); }
                                } else if event.key() == "Enter" && (event.ctrl_key() || event.meta_key()) {
                                    event.prevent_default(); event.stop_propagation(); save();
                                }
                            } />
                        <Show when=move || !pending.get()><p class="queue-notice">"Сообщение уже передано агенту или удалено. Текст правки оставлен здесь для копирования."</p></Show>
                        <div class="queue-editor-actions">
                            <button type="button" class="secondary" disabled=move || busy.get() on:click=move |_| { editing.set(None); error.set(None); }>"Отмена"</button>
                            <button type="button" class="primary" disabled=move || busy.get() || !pending.get() || draft.with(|text| text.trim().is_empty()) on:click=move |_| save()>"Сохранить"</button>
                        </div>
                    </div>
                </Show>
                <Show when=move || error.get().is_some()>
                    <div class="queue-error" role="alert"><span>{move || error.get()}</span><button type="button" class="queue-icon-button" aria-label="Закрыть ошибку" on:click=move |_| error.set(None)>"×"</button></div>
                </Show>
            </section>
        </Show>
    }
}
