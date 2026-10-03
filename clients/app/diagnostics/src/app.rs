use leptos::prelude::*;
use leptos::task::spawn_local;
use proteus_app_common::desktop;
mod navigation;
use navigation::Navigation;

use crate::{
    api::{
        app_server_origin, chat_link_url, has_session_token, initialize_selected_session,
        load_session_token,
    },
    architecture::ArchitectureView,
};

#[component]
pub(crate) fn App() -> impl IntoView {
    let nav = Navigation::new();
    let is_analysis = move || nav.section.get() == "analysis";
    let is_report = move || nav.section.get() == "usage";
    let report_seen = RwSignal::new(false);
    let is_architecture = move || nav.section.get() == "architecture";
    let analysis_seen = RwSignal::new(false);
    let architecture_seen = RwSignal::new(false);
    Effect::new(move |_| match nav.section.get() {
        "analysis" => analysis_seen.set(true),
        "usage" => report_seen.set(true),
        _ => architecture_seen.set(true),
    });
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
    let initializing = RwSignal::new(false);
    if let Some(error) = token_error {
        selected_session.set(Some(Err(error)));
    }
    Effect::new(move |_| {
        if (is_analysis() || is_report())
            || selected_session.get_untracked().is_some()
            || initializing.get_untracked()
        {
            return;
        }
        initializing.set(true);
        spawn_local(async move {
            let result = initialize_selected_session().await;
            if result.is_ok() {
                chat_url.set(chat_link_url());
            }
            selected_session.set(Some(result));
        });
    });

    view! {
        <div class="inspector-shell">
            <a class="skip-link" href="#inspector-content">"Перейти к содержимому"</a>
            <aside class="inspector-sidebar">
                <a class="inspector-brand" href=move || nav.href("usage") on:click=move |ev| nav.click(ev,"usage") aria-label="Proteus — расход и контекст">
                    <span class="brand-mark" aria-hidden="true"><i></i><i></i><i></i></span>
                    <span><strong>"proteus"</strong><small>"INSPECTOR"</small></span>
                </a>
                <div class="sidebar-section-label">"РАБОЧЕЕ ПРОСТРАНСТВО"</div>
                <nav class="inspector-nav" aria-label="Разделы Inspector">
                    <a class="inspector-nav-item" class:active=is_report
                        aria-current=move || if is_report() { Some("page") } else { None }
                        href=move || nav.href("usage") on:click=move |ev| nav.click(ev,"usage")>
                        <crate::icons::Icon name="analysis"/><span>"Расход и контекст"</span>
                    </a>
                    <a class="inspector-nav-item" class:active=is_analysis
                        aria-current=move || if is_analysis() { Some("page") } else { None }
                        href=move || nav.href("analysis") on:click=move |ev| nav.click(ev,"analysis")>
                        <crate::icons::Icon name="inspector"/>
                        <span>"Анализ ходов"</span>
                    </a>
                    <a class="inspector-nav-item" class:active=is_architecture
                        aria-current=move || if is_architecture() { Some("page") } else { None }
                        href=move || nav.href("architecture") on:click=move |ev| nav.click(ev,"architecture")>
                        <crate::icons::Icon name="inspector"/>
                        <span>"Архитектура"</span>
                        <span class="nav-indicator" aria-hidden="true"></span>
                    </a>
                </nav>
                <div class="sidebar-connection">
                    <span class="connection-label"><span class="connection-dot"></span>{access_label}</span>
                    <code title=origin>{endpoint}</code>
                </div>
            </aside>
            <main class="inspector-main">
                <header class="inspector-topbar">
                    <div class="inspector-breadcrumb"><span>"Рабочее пространство"</span><span aria-hidden="true">"/"</span><strong>"Inspector"</strong></div>
                    <a class="inspector-chat-link" href=move || chat_url.get()>"Вернуться в чат"<crate::icons::Icon name="external-link" size=16/></a>
                </header>
                <div class="inspector-content" id="inspector-content" tabindex="-1">
                    <Show when=move || report_seen.get()>
                        <div class="inspector-view" hidden=move || !is_report()>
                            <crate::session_report::SessionReportView visible=Signal::derive(move || is_report())/>
                        </div>
                    </Show>
                    <Show when=move || analysis_seen.get()>
                        <div class="inspector-view" hidden=move || !is_analysis()><crate::analysis::AnalysisView/></div>
                    </Show>
                    <Show when=move || selected_session.get().is_some_and(|value| value.is_ok())>
                        <Show when=move || architecture_seen.get()>
                            <div class="inspector-view" hidden=move || !is_architecture()><ArchitectureView/></div>
                        </Show>
                    </Show>
                    {move || if is_analysis() || is_report() { None } else { match selected_session.get() {
                        Some(Err(error)) => Some(view! { <div class="empty-state"><div class="empty-state-title">"Не удалось выбрать сессию"</div><p>{error}</p></div> }.into_any()),
                        None => Some(view! { <div class="empty-state"><div class="empty-state-title">"Подключаю сессию…"</div></div> }.into_any()),
                        _ => None,
                    }}}

                </div>
            </main>
        </div>
    }
}
