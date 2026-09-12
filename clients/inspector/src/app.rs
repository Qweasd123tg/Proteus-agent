use leptos::prelude::*;
use leptos::task::spawn_local;
use proteus_client_common::desktop;
use web_sys::window;

use crate::{
    api::{
        app_server_origin, chat_link_url, has_session_token, initialize_selected_session,
        load_session_token, query_value,
    },
    architecture::ArchitectureView,
    configs::ConfigsView,
};

#[component]
pub(crate) fn App() -> impl IntoView {
    let path = window().and_then(|window| window.location().pathname().ok());
    let is_architecture = is_architecture_route(
        path.as_deref(),
        query_value("view").as_deref(),
        desktop::is_desktop(),
    );
    let token_error = load_session_token().err();
    let origin = app_server_origin();
    let endpoint = origin
        .trim_start_matches("http://")
        .trim_start_matches("https://")
        .to_owned();
    let endpoint = desktop::connection()
        .ok()
        .flatten()
        .map(|c| c.workspace)
        .unwrap_or(endpoint);
    let access_label = match token_error.as_ref() {
        Some(_) => "Хранилище сессии недоступно",
        None if desktop::is_desktop() => "Подключено",
        None if has_session_token() => "Токен сессии настроен",
        None => "Локальный сервер",
    };
    let selected_session = RwSignal::new(None::<Result<String, String>>);
    let chat_url = RwSignal::new(chat_link_url());
    if let Some(error) = token_error {
        selected_session.set(Some(Err(error)));
    } else {
        spawn_local(async move {
            let result = initialize_selected_session().await;
            if result.is_ok() {
                chat_url.set(chat_link_url());
            }
            selected_session.set(Some(result));
        });
    }

    view! {
        <div class="inspector-shell">
            <a class="skip-link" href="#inspector-content">"Перейти к содержимому"</a>
            <aside class="inspector-sidebar">
                <a class="inspector-brand" href=desktop::inspector_route(false) aria-label="Proteus — сборка агента">
                    <span class="brand-mark" aria-hidden="true"><i></i><i></i><i></i></span>
                    <span><strong>"proteus"</strong><small>"INSPECTOR"</small></span>
                </a>
                <div class="sidebar-section-label">"РАБОЧЕЕ ПРОСТРАНСТВО"</div>
                <nav class="inspector-nav" aria-label="Разделы Inspector">
                    <a class="inspector-nav-item" class:active=!is_architecture
                        aria-current=if !is_architecture { Some("page") } else { None }
                        href=desktop::inspector_route(false)>
                        <crate::icons::Icon name="modules"/>
                        <span>"Сборка агента"</span>
                        <span class="nav-indicator" aria-hidden="true"></span>
                    </a>
                    <a class="inspector-nav-item" class:active=is_architecture
                        aria-current=if is_architecture { Some("page") } else { None }
                        href=desktop::inspector_route(true)>
                        <crate::icons::Icon name="inspector"/>
                        <span>"Архитектура"</span>
                        <span class="nav-indicator" aria-hidden="true"></span>
                    </a>
                </nav>
                <div class="sidebar-note">
                    <span class="sidebar-note-glyph" aria-hidden="true"><crate::icons::Icon name="modules" size=24/></span>
                    <strong>"Агент из ваших модулей"</strong>
                    <p>"Выберите поведение. Настройте инструменты. Посмотрите, как всё связано."</p>
                </div>
                <div class="sidebar-connection">
                    <span class="connection-label"><span class="connection-dot"></span>{access_label}</span>
                    <code title=origin>{endpoint}</code>
                </div>
            </aside>
            <main class="inspector-main">
                <header class="inspector-topbar">
                    <div class="inspector-breadcrumb"><span>"Рабочее пространство"</span><span aria-hidden="true">"/"</span><strong>"Inspector"</strong></div>
                    <a class="inspector-chat-link" href=move || chat_url.get()>"Открыть чат"<crate::icons::Icon name="external-link" size=16/></a>
                </header>
                <div class="inspector-content" id="inspector-content" tabindex="-1">
                    {move || match selected_session.get() {
                        Some(Ok(_)) if is_architecture => view! { <ArchitectureView/> }.into_any(),
                        Some(Ok(_)) => view! { <ConfigsView/> }.into_any(),
                        Some(Err(error)) => view! {
                            <div class="empty-state">
                                <div class="empty-state-title">"Не удалось выбрать сессию"</div>
                                <p>{error}</p>
                            </div>
                        }.into_any(),
                        None => view! {
                            <div class="empty-state">
                                <div class="empty-state-title">"Подключаю сессию…"</div>
                            </div>
                        }.into_any(),
                    }}
                </div>
            </main>
        </div>
    }
}

fn is_architecture_route(path: Option<&str>, view: Option<&str>, is_desktop: bool) -> bool {
    path == Some("/architecture") || (is_desktop && view == Some("architecture"))
}

#[cfg(test)]
mod tests {
    use super::is_architecture_route;

    #[test]
    fn desktop_architecture_view_does_not_depend_on_other_query_parameters() {
        // `view` is parsed by key before this decision, so a sibling
        // `session_dir` query parameter cannot change the selected page.
        assert!(is_architecture_route(
            Some("/inspector.html"),
            Some("architecture"),
            true
        ));
        assert!(!is_architecture_route(Some("/inspector.html"), None, true));
    }
}
