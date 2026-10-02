use crate::types::ConfigBuilderModule;
use leptos::prelude::*;

#[component]
pub(super) fn HooksEditor(
    modules: Vec<ConfigBuilderModule>,
    draft_hooks: ReadSignal<Vec<String>>,
    set_hooks: WriteSignal<Vec<String>>,
) -> impl IntoView {
    view! {
        <section class="cfg-panel">
            <h3>"Hooks — порядок выполнения"</h3>
            <p>"Укажите id по одному на строку. Пустой список отключает hooks."</p>
            <textarea aria-label="Hooks в порядке выполнения"
                prop:value=move || draft_hooks.get().join("\n")
                on:input:target=move |ev| set_hooks.set(ev.target().value().lines().filter(|id| !id.trim().is_empty()).map(str::to_owned).collect())/>
            <p>"Доступны: "{modules.iter().map(|module| module.id.clone()).collect::<Vec<_>>().join(", ")}</p>
        </section>
    }
}
