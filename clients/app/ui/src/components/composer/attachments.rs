use crate::actions::AppActions;
use leptos::{html, prelude::*};
use proteus_contracts::domain::{ImageAttachment, MAX_IMAGE_BYTES, MAX_INPUT_IMAGES};
use wasm_bindgen::JsCast;

const IMAGE_TYPES: [&str; 4] = ["image/png", "image/jpeg", "image/webp", "image/gif"];

/// One entry for picked, pasted and dropped images.
#[derive(Clone, Copy)]
pub(super) struct ImageInput {
    actions: AppActions,
    error: RwSignal<Option<String>>,
    /// Files are being dragged over the window.
    pub(super) dragging: RwSignal<bool>,
}

impl ImageInput {
    pub(super) fn new(actions: AppActions) -> Self {
        Self {
            actions,
            error: RwSignal::new(None),
            dragging: RwSignal::new(false),
        }
    }

    /// Adds all files or none: one bad file keeps the previous attachments.
    pub(super) fn add(self, files: Vec<web_sys::File>) {
        if files.is_empty() {
            return;
        }
        let Self { actions, error, .. } = self;
        if actions.attachments_loading.get_untracked() {
            return;
        }
        if let Some(file) = files.iter().find(|file| !is_image(file)) {
            error.set(Some(format!(
                "{}: поддерживаются PNG, JPEG, WebP и GIF.",
                file.name()
            )));
            return;
        }
        error.set(None);
        let loading = actions.attachments_loading;
        loading.set(true);
        let generation = actions.transcript_generation.get_untracked();
        leptos::task::spawn_local(async move {
            let result = async {
                if actions.attachments.with_untracked(|images| images.len()) + files.len()
                    > MAX_INPUT_IMAGES
                {
                    return Err("Можно прикрепить до 4 изображений.".to_owned());
                }
                let mut total = 0;
                let mut images = Vec::new();
                for file in files {
                    total += file.size() as usize;
                    if total > MAX_IMAGE_BYTES {
                        return Err("Общий размер изображений — до 5 МБ.".to_owned());
                    }
                    let buffer = wasm_bindgen_futures::JsFuture::from(file.array_buffer())
                        .await
                        .map_err(|_| "Не удалось прочитать изображение.".to_owned())?;
                    let bytes = js_sys::Uint8Array::new(&buffer).to_vec();
                    images.push(
                        ImageAttachment::from_bytes(file.name(), &bytes)
                            .map_err(|_| "Поддерживаются PNG, JPEG, WebP и GIF.".to_owned())?,
                    );
                }
                Ok(images)
            }
            .await;
            if generation == actions.transcript_generation.get_untracked() {
                match result {
                    Ok(images) => {
                        // The user may remove an existing attachment while file reads are
                        // pending. Only merge the new files into the current draft.
                        actions.attachments.update(|current| {
                            let total = current
                                .iter()
                                .chain(&images)
                                .map(|image| image.decode().map(|bytes| bytes.len()).unwrap_or(0))
                                .sum::<usize>();
                            if current.len() + images.len() > MAX_INPUT_IMAGES {
                                error.set(Some("Можно прикрепить до 4 изображений.".to_owned()));
                            } else if total > MAX_IMAGE_BYTES {
                                error.set(Some("Общий размер изображений — до 5 МБ.".to_owned()));
                            } else {
                                current.extend(images);
                            }
                        });
                    }
                    Err(message) => error.set(Some(message)),
                }
            }
            loading.set(false);
        });
    }

    /// Pasted images, unless the clipboard also carries text: spreadsheet
    /// cells come as both, and the text is what was meant.
    pub(super) fn paste(self, event: web_sys::ClipboardEvent) {
        let Some(data) = event.clipboard_data() else {
            return;
        };
        if data
            .get_data("text/plain")
            .is_ok_and(|text| !text.trim().is_empty())
        {
            return;
        }
        let images = transfer_files(&data)
            .into_iter()
            .filter(is_image)
            .collect::<Vec<_>>();
        if !images.is_empty() {
            event.prevent_default();
            self.add(images);
        } else if data.types().length() == 0 {
            // An empty paste is how WebKitGTK reports a clipboard image.
            #[cfg(target_arch = "wasm32")]
            leptos::task::spawn_local(async move {
                if let Ok(file) = desktop::read_clipboard_image().await
                    && let Ok(file) = file.dyn_into::<web_sys::File>()
                {
                    self.add(vec![file]);
                }
            });
        }
    }
}

#[cfg(target_arch = "wasm32")]
mod desktop {
    use wasm_bindgen::prelude::*;

    #[wasm_bindgen(raw_module = "/ui/clipboard-image.js")]
    extern "C" {
        #[wasm_bindgen(js_name = readDesktopClipboardImage, catch)]
        pub(super) async fn read_clipboard_image() -> Result<JsValue, JsValue>;
    }
}

fn is_image(file: &web_sys::File) -> bool {
    IMAGE_TYPES.contains(&file.type_().as_str())
}

fn transfer_files(data: &web_sys::DataTransfer) -> Vec<web_sys::File> {
    data.files()
        .map(|list| (0..list.length()).filter_map(|i| list.get(i)).collect())
        .unwrap_or_default()
}

#[cfg(target_arch = "wasm32")]
fn carries_files(event: &web_sys::DragEvent) -> bool {
    event.data_transfer().is_some_and(|data| {
        data.types()
            .iter()
            .any(|kind| kind.as_string().as_deref() == Some("Files"))
    })
}

/// Files dropped anywhere over the visible chat attach to the message; the
/// webview would otherwise try to open them.
#[cfg(target_arch = "wasm32")]
pub(super) fn accept_window_drops(input: ImageInput, composer: NodeRef<html::Form>) {
    // Enter/leave fire for every element crossed; the depth tells when the
    // drag really left the window.
    let depth = StoredValue::new(0_u32);
    let visible = move || {
        composer
            .get_untracked()
            .is_some_and(|form| form.client_height() > 0)
    };
    let enter = window_event_listener(leptos::ev::dragenter, move |event| {
        if carries_files(&event) && visible() {
            depth.update_value(|depth| *depth += 1);
            input.dragging.set(true);
        }
    });
    let over = window_event_listener(leptos::ev::dragover, move |event| {
        if carries_files(&event) {
            event.prevent_default();
            if let Some(data) = event.data_transfer() {
                data.set_drop_effect(if visible() { "copy" } else { "none" });
            }
        }
    });
    let leave = window_event_listener(leptos::ev::dragleave, move |event| {
        if carries_files(&event) {
            depth.update_value(|depth| *depth = depth.saturating_sub(1));
            if depth.get_value() == 0 {
                input.dragging.set(false);
            }
        }
    });
    let drop = window_event_listener(leptos::ev::drop, move |event| {
        if !carries_files(&event) {
            return;
        }
        event.prevent_default();
        depth.set_value(0);
        input.dragging.set(false);
        if visible()
            && let Some(data) = event.data_transfer()
        {
            input.add(transfer_files(&data));
        }
    });
    on_cleanup(move || {
        enter.remove();
        over.remove();
        leave.remove();
        drop.remove();
    });
}

/// The picker sits in the toolbar corner; previews stay above the text.
#[component]
pub(super) fn AttachButton(input: ImageInput) -> impl IntoView {
    let picker = NodeRef::<html::Input>::new();
    let loading = input.actions.attachments_loading;
    view! {
        <span class="composer-attach">
            <input type="file" node_ref=picker accept=IMAGE_TYPES.join(",") multiple hidden
                on:change=move |event| {
                    let target = event.target().unwrap().unchecked_into::<web_sys::HtmlInputElement>();
                    let files = target.files().map(|files| (0..files.length()).filter_map(|i| files.get(i)).collect::<Vec<_>>()).unwrap_or_default();
                    target.set_value("");
                    input.add(files);
                } />
            <button type="button" class="attach-image" title="Прикрепить изображения · можно вставить Ctrl+V или перетащить · до 4 файлов, суммарно до 5 МБ" aria-label="Прикрепить изображения" disabled=move || loading.get()
                on:click=move |_| { if let Some(picker) = picker.get() { picker.click(); } }><super::super::icons::PlusIcon/></button>
        </span>
    }
}

#[component]
pub(super) fn ImageAttachments(input: ImageInput) -> impl IntoView {
    let actions = input.actions;
    view! {
        <div class="composer-attachments">
            <div class="attachment-previews">
                <For each={move || actions.attachments.get().into_iter().enumerate().collect::<Vec<_>>()} key=preview_key children={move |(index, image)| {
                    let url = format!("data:{};base64,{}", image.mime_type, image.data);
                    let alt = image.name.clone();
                    view! { <div class="attachment-preview"><img src=url alt=alt/><span>{image.name}</span><button type="button" aria-label="Убрать изображение" on:click=move |_| actions.attachments.update(|items| { if index < items.len() { items.remove(index); } })>"×"</button></div> }
                }}/>
            </div>
            {move || input.error.get().map(|message| view! { <p class="attachment-error" role="alert">{message}</p> })}
        </div>
    }
}

fn preview_key((index, image): &(usize, ImageAttachment)) -> (usize, u64) {
    use std::hash::{Hash, Hasher};
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    image.data.hash(&mut hash);
    (*index, hash.finish())
}
