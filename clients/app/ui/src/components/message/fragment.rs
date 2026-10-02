//! Подписка фрагмента переносит только Arc; неизменившийся блок не будит DOM.
use crate::markdown::Block;
use leptos::prelude::*;
use std::sync::Arc;

type Blocks = Memo<Vec<Arc<Block>>>;

fn selected(blocks: Blocks, index: usize) -> Memo<Option<Arc<Block>>> {
    Memo::new(move |_| blocks.with(|blocks| blocks.get(index).cloned()))
}

pub(super) fn view(blocks: Blocks, index: usize, streaming: Memo<bool>) -> impl IntoView {
    let block = selected(blocks, index);
    #[cfg(target_arch = "wasm32")]
    {
        use leptos::html;
        use wasm_bindgen::prelude::*;
        #[wasm_bindgen(raw_module = "/ui/markdown-fragment.js")]
        extern "C" {
            #[wasm_bindgen(js_name = updateMarkdownFragment)]
            fn update(root: &web_sys::Element, html: &str, streaming: bool);
            #[wasm_bindgen(js_name = disposeMarkdownFragment)]
            fn dispose(root: &web_sys::Element);
        }
        let root = NodeRef::<html::Div>::new();
        Effect::new(move |_| {
            let Some(root) = root.get() else { return };
            let streaming = streaming.get();
            block.with(|block| {
                update(
                    root.as_ref(),
                    block
                        .as_ref()
                        .map(|block| block.html.as_str())
                        .unwrap_or_default(),
                    streaming,
                )
            });
        });
        on_cleanup(move || {
            if let Some(root) = root.get_untracked() {
                dispose(root.as_ref());
            }
        });
        view! { <div class="markdown-fragment" node_ref=root></div> }
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = streaming;
        view! { <div class="markdown-fragment" inner_html=move || block.with(|block| block.as_ref().map(|block| block.html.clone()).unwrap_or_default())></div> }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    };

    #[tokio::test]
    async fn unchanged_blocks_do_not_rerun_fragment_effects() {
        _ = any_spawner::Executor::init_tokio();
        let owner = Owner::new();
        let count = Arc::new(AtomicUsize::new(0));
        let retained = Arc::new(Mutex::new(None));
        let (text, blocks) = owner.with(|| {
            let text = RwSignal::new("First.\n\nLast".to_owned());
            let blocks = Memo::new(move |previous| {
                text.with(|text| crate::markdown::markdown_blocks(text, previous))
            });
            let block = selected(blocks, 0);
            let count = count.clone();
            let retained = retained.clone();
            Effect::new_isomorphic(move |_| {
                *retained.lock().unwrap() = block.get();
                count.fetch_add(1, Ordering::Relaxed);
            });
            (text, blocks)
        });
        tokio::task::yield_now().await;
        let first = retained.lock().unwrap().clone().unwrap();
        text.set("First.\n\nLast grows".to_owned());
        tokio::task::yield_now().await;
        assert_eq!(count.load(Ordering::Relaxed), 1);
        assert!(blocks.with(|blocks| Arc::ptr_eq(&first, &blocks[0])));
        text.set("Changed.\n\nLast grows".to_owned());
        tokio::task::yield_now().await;
        assert_eq!(count.load(Ordering::Relaxed), 2);
    }
}
