use crate::actions::AppActions;
use leptos::{html, prelude::*};
use proteus_contracts::domain::{ImageAttachment, MAX_IMAGE_BYTES, MAX_INPUT_IMAGES};
use wasm_bindgen::JsCast;

#[component]
pub(super) fn ImageAttachments(actions: AppActions) -> impl IntoView {
    let input = NodeRef::<html::Input>::new();
    let (error, set_error) = signal(None::<String>);
    let loading = actions.attachments_loading;
    view! {
        <div class="composer-attachments">
            <input type="file" node_ref=input accept="image/png,image/jpeg,image/webp,image/gif" multiple hidden
                on:change=move |event| {
                    let target = event.target().unwrap().unchecked_into::<web_sys::HtmlInputElement>();
                    let files = target.files().map(|files| (0..files.length()).filter_map(|i| files.get(i)).collect::<Vec<_>>()).unwrap_or_default();
                    target.set_value("");
                    if files.is_empty() { return; }
                    set_error.set(None);
                    loading.set(true);
                    let generation = actions.transcript_generation.get_untracked();
                    leptos::task::spawn_local(async move {
                        let result = async {
                            let mut images = actions.attachments.get_untracked();
                            if images.len() + files.len() > MAX_INPUT_IMAGES { return Err("Можно прикрепить до 4 изображений.".to_owned()); }
                            let mut total = images.iter().map(|image| image.decode().map(|b| b.len()).unwrap_or(0)).sum::<usize>();
                            for file in files {
                                total += file.size() as usize;
                                if total > MAX_IMAGE_BYTES { return Err("Общий размер изображений — до 5 МБ.".to_owned()); }
                                let buffer = wasm_bindgen_futures::JsFuture::from(file.array_buffer()).await.map_err(|_| "Не удалось прочитать изображение.".to_owned())?;
                                let bytes = js_sys::Uint8Array::new(&buffer).to_vec();
                                images.push(ImageAttachment::from_bytes(file.name(), &bytes).map_err(|_| "Поддерживаются PNG, JPEG, WebP и GIF.".to_owned())?);
                            }
                            Ok(images)
                        }.await;
                        if generation == actions.transcript_generation.get_untracked() {
                            match result { Ok(images) => actions.attachments.set(images), Err(message) => set_error.set(Some(message)) }
                        }
                        loading.set(false);
                    });
                } />
            <button type="button" class="icon-button attach-image" title="Прикрепить изображения · до 4 файлов, суммарно до 5 МБ" aria-label="Прикрепить изображения" disabled=move || loading.get()
                on:click=move |_| { if let Some(input) = input.get() { input.click(); } }><super::super::icons::PlusIcon/></button>
            <div class="attachment-previews">
                <For each={move || actions.attachments.get().into_iter().enumerate().collect::<Vec<_>>()} key=preview_key children={move |(index, image)| {
                    let url = format!("data:{};base64,{}", image.mime_type, image.data);
                    let alt = image.name.clone();
                    view! { <div class="attachment-preview"><img src=url alt=alt/><span>{image.name}</span><button type="button" aria-label="Убрать изображение" on:click=move |_| actions.attachments.update(|items| { if index < items.len() { items.remove(index); } })>"×"</button></div> }
                }}/>
            </div>
            {move || error.get().map(|message| view! { <p class="attachment-error" role="alert">{message}</p> })}
        </div>
    }
}

fn preview_key((index, image): &(usize, ImageAttachment)) -> (usize, u64) {
    use std::hash::{Hash, Hasher};
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    image.data.hash(&mut hash);
    (*index, hash.finish())
}
