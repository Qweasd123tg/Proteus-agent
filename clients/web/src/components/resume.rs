use leptos::{prelude::*, task::spawn_local};

use crate::api::get_json;
use crate::types::*;
use crate::ui_utils::{short_id, short_path};

#[component]
pub(crate) fn ResumeView<O>(on_open: O) -> impl IntoView
where
    O: Fn(SessionSummary) + Copy + Send + 'static,
{
    let (sessions, set_sessions) = signal(Vec::<SessionSummary>::new());
    let (status, set_status) = signal("загружаю сессии".to_owned());

    load_sessions(set_sessions, set_status);

    let refresh = move |_| load_sessions(set_sessions, set_status);

    view! {
        <section class="resume-page">
            <div class="resume-toolbar">
                <div>
                    <h2>"Прошлые сессии"</h2>
                    <p>{move || status.get()}</p>
                </div>
                <button type="button" class="secondary" on:click=refresh>"Обновить"</button>
            </div>
            {move || {
                let items = sessions.get();
                if items.is_empty() {
                    view! {
                        <div class="empty-state">
                            <div class="empty-state-title">"Сохранённых сессий нет"</div>
                        </div>
                    }.into_any()
                } else {
                    view! {
                        <div class="resume-list">
                            <For
                                each=move || sessions.get()
                                key=|session| session.session_dir.clone()
                                children=move |session| {
                                    let session_for_open = session.clone();
                                    let workspace = session.workspace_path.clone();
                                    let session_id = short_id(&session.session_id).to_owned();
                                    view! {
                                        <article class="resume-item">
                                            <div class="resume-item-main">
                                                <div class="resume-item-header">
                                                    <strong>{short_path(&workspace)}</strong>
                                                    <code>{session_id}</code>
                                                </div>
                                                <p>{resume_session_preview(&session)}</p>
                                                <div class="resume-meta">
                                                    <span>{workspace}</span>
                                                    <span>{format!("{} сообщений", session.message_count)}</span>
                                                </div>
                                            </div>
                                            <button
                                                type="button"
                                                class="btn-primary"
                                                on:click=move |_| on_open(session_for_open.clone())
                                            >
                                                "Открыть"
                                            </button>
                                        </article>
                                    }
                                }
                            />
                        </div>
                    }.into_any()
                }
            }}
        </section>
    }
}

fn resume_session_preview(session: &SessionSummary) -> String {
    if let Some(preview) = session
        .preview
        .as_deref()
        .filter(|text| !text.trim().is_empty())
    {
        preview.to_owned()
    } else if session.message_count == 0 {
        "Новый чат".to_owned()
    } else {
        "Сессия".to_owned()
    }
}

fn load_sessions(set_sessions: WriteSignal<Vec<SessionSummary>>, set_status: WriteSignal<String>) {
    spawn_local(async move {
        match get_json::<Vec<SessionSummary>>("/sessions").await {
            Ok(items) => {
                let count = items.len();
                set_sessions.set(items);
                set_status.set(format!("{count} сессий"));
            }
            Err(error) => set_status.set(format!("не удалось загрузить сессии: {error}")),
        }
    });
}
