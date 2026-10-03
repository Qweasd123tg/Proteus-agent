//! Search over the whole chat, not only the rows the virtual list rendered.
use leptos::{html, prelude::*};

use super::icons::{ArrowDownIcon, ArrowUpIcon, CloseIcon};
use crate::types::{Message, MessageRole};

/// Text messages containing `query`, oldest first; case-insensitive.
pub(crate) fn search_matches(messages: &[Message], query: &str) -> Vec<u64> {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return Vec::new();
    }
    messages
        .iter()
        .filter(|message| message.tool.is_none() && message.subagent.is_none())
        .filter(|message| {
            matches!(
                message.role,
                MessageRole::User | MessageRole::Assistant | MessageRole::System
            )
        })
        .filter(|message| message.text.to_lowercase().contains(&needle))
        .map(|message| message.id)
        .collect()
}

/// The match after a step; searching starts from the newest message.
pub(crate) fn step(current: Option<usize>, count: usize, older: bool) -> Option<usize> {
    if count == 0 {
        return None;
    }
    Some(match current {
        None => count - 1,
        Some(index) if older => (index + count - 1) % count,
        Some(index) => (index + 1) % count,
    })
}

#[component]
pub(crate) fn ChatSearch(
    messages: crate::transcript::Transcript,
    results_ref: NodeRef<html::Section>,
    visible: Signal<bool>,
    on_jump: Callback<u64>,
) -> impl IntoView {
    let open = RwSignal::new(false);
    let query = RwSignal::new(String::new());
    let matches = RwSignal::new(Vec::<u64>::new());
    let current = RwSignal::new(None::<usize>);
    let input = NodeRef::<html::Input>::new();

    // Streaming updates do not re-run the search; a keystroke or a step does.
    let refresh = move || {
        let found = messages.with_untracked(|items| search_matches(items, &query.get_untracked()));
        let selected = current
            .get_untracked()
            .and_then(|index| matches.with_untracked(|ids| ids.get(index).copied()));
        current.set(selected.and_then(|id| found.iter().position(|item| *item == id)));
        matches.set(found);
    };
    let go = move |older: bool| {
        refresh();
        let next = step(
            current.get_untracked(),
            matches.with_untracked(Vec::len),
            older,
        );
        current.set(next);
        if let Some(id) =
            next.and_then(|index| matches.with_untracked(|ids| ids.get(index).copied()))
        {
            on_jump.run(id);
        }
    };
    let close = move || {
        open.set(false);
        query.set(String::new());
        matches.set(Vec::new());
        current.set(None);
    };

    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen::prelude::*;
        #[wasm_bindgen(raw_module = "/ui/chat-search.js")]
        extern "C" {
            #[wasm_bindgen(js_name = paintChatSearch)]
            fn paint(results: &web_sys::Element, query: &str, current: &str);
        }
        Effect::new(move |_| {
            let text = if open.get() {
                query.get().trim().to_owned()
            } else {
                String::new()
            };
            let selected = current
                .get()
                .and_then(|index| matches.with(|ids| ids.get(index).copied()))
                .map(|id| id.to_string())
                .unwrap_or_default();
            if let Some(results) = results_ref.get() {
                paint(results.as_ref(), &text, &selected);
            }
        });
        let keys = window_event_listener(leptos::ev::keydown, move |event| {
            let modifier = event.ctrl_key() || event.meta_key();
            if modifier
                && !event.alt_key()
                && !event.shift_key()
                && event.code() == "KeyF"
                && visible.get_untracked()
            {
                event.prevent_default();
                open.set(true);
                request_animation_frame(move || {
                    if let Some(input) = input.get_untracked() {
                        let _ = input.focus();
                        input.select();
                    }
                });
            }
        });
        on_cleanup(move || keys.remove());
    }
    #[cfg(not(target_arch = "wasm32"))]
    let _ = (results_ref, visible);

    let counter = move || {
        let count = matches.with(Vec::len);
        match (count, current.get()) {
            (0, _) if !query.with(|text| text.trim().is_empty()) => "Нет совпадений".to_owned(),
            (0, _) => String::new(),
            (count, Some(index)) => format!("{} из {count}", index + 1),
            (count, None) => format!("{count}"),
        }
    };
    view! {
        <Show when=move || open.get()>
            <div class="chat-search" role="search" data-chat-search="">
                <input type="search" node_ref=input placeholder="Поиск по чату" aria-label="Поиск по чату"
                    prop:value=move || query.get()
                    on:input:target=move |event| {
                        query.set(event.target().value());
                        current.set(None);
                        go(true);
                    }
                    on:keydown=move |event| match event.key().as_str() {
                        "Enter" => { event.prevent_default(); go(!event.shift_key()); }
                        // Escape here closes search instead of stopping the answer.
                        "Escape" => { event.prevent_default(); close(); }
                        _ => {}
                    } />
                <span class="chat-search-count" aria-live="polite">{counter}</span>
                <button type="button" class="icon-button" title="Предыдущее совпадение · Enter" aria-label="Предыдущее совпадение"
                    disabled=move || matches.with(Vec::is_empty) on:click=move |_| go(true)><ArrowUpIcon/></button>
                <button type="button" class="icon-button" title="Следующее совпадение · Shift+Enter" aria-label="Следующее совпадение"
                    disabled=move || matches.with(Vec::is_empty) on:click=move |_| go(false)><ArrowDownIcon/></button>
                <button type="button" class="icon-button" title="Закрыть · Esc" aria-label="Закрыть поиск"
                    on:click=move |_| close()><CloseIcon/></button>
            </div>
        </Show>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(id: u64, role: MessageRole, text: &str) -> Message {
        Message {
            images: Vec::new(),
            id,
            message_id: None,
            version: 0,
            phase: None,
            text_offset: 0,
            role,
            text: text.to_owned(),
            tool: None,
            subagent: None,
            streaming: false,
        }
    }

    #[test]
    fn search_finds_text_messages_and_steps_from_the_newest() {
        let items = vec![
            message(1, MessageRole::User, "Почини Сборку"),
            message(2, MessageRole::Assistant, "сборка исправлена"),
            message(3, MessageRole::Reasoning, "сборка в размышлениях"),
            message(4, MessageRole::Assistant, "готово"),
        ];
        assert_eq!(search_matches(&items, " СБОРК "), [1, 2]);
        assert!(search_matches(&items, "  ").is_empty());
        assert_eq!(step(None, 2, true), Some(1));
        assert_eq!(step(Some(1), 2, true), Some(0));
        assert_eq!(step(Some(0), 2, true), Some(1), "wraps to the newest");
        assert_eq!(step(Some(1), 2, false), Some(0), "wraps to the oldest");
        assert_eq!(step(None, 0, true), None);
    }
}
