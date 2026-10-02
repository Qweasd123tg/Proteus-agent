use leptos::prelude::*;
use web_sys::HtmlTextAreaElement;

pub(crate) fn insert_textarea_newline(
    textarea: HtmlTextAreaElement,
    set_draft: WriteSignal<String>,
) {
    let value = textarea.value();
    let start = textarea
        .selection_start()
        .ok()
        .flatten()
        .unwrap_or(value.encode_utf16().count() as u32);
    let end = textarea.selection_end().ok().flatten().unwrap_or(start);
    let start_index = utf16_offset_to_byte_index(&value, start);
    let end_index = utf16_offset_to_byte_index(&value, end);
    let mut next = String::with_capacity(value.len() + 1);
    next.push_str(&value[..start_index]);
    next.push('\n');
    next.push_str(&value[end_index..]);
    let next_cursor = start + 1;

    textarea.set_value(&next);
    let _ = textarea.set_selection_start(Some(next_cursor));
    let _ = textarea.set_selection_end(Some(next_cursor));
    set_draft.set(next);
}

fn utf16_offset_to_byte_index(text: &str, offset: u32) -> usize {
    let mut units = 0;
    for (index, ch) in text.char_indices() {
        if units >= offset {
            return index;
        }
        units += ch.len_utf16() as u32;
    }
    text.len()
}
