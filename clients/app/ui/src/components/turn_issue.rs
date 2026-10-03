use leptos::prelude::*;

use super::icons::CloseIcon;
use crate::types::TurnIssue;

/// How a turn ended when it did not end with a complete answer.
#[component]
pub(crate) fn TurnIssueView(
    issue: TurnIssue,
    #[prop(optional)] on_dismiss: Option<Callback<()>>,
) -> impl IntoView {
    let detail = issue.detail().map(str::to_owned);
    view! {
        <article class=issue.class() role="status" data-turn-issue="">
            <div class="turn-issue-head">
                <strong>{issue.title()}</strong>
                {on_dismiss.map(|dismiss| view! {
                    <button type="button" class="icon-button" title="Скрыть" aria-label="Скрыть"
                        on:click=move |_| dismiss.run(())><CloseIcon/></button>
                })}
            </div>
            {detail.map(|message| view! { <pre class="turn-issue-detail">{message}</pre> })}
            {issue.hint().map(|hint| view! { <p class="turn-issue-hint">{hint}</p> })}
        </article>
    }
}

/// The live-only part: the last answer was cut or filtered.
#[component]
pub(crate) fn LiveTurnIssue(issue: RwSignal<Option<TurnIssue>>) -> impl IntoView {
    move || {
        issue.get().map(|current| {
            view! { <TurnIssueView issue=current on_dismiss=Callback::new(move |()| issue.set(None))/> }
        })
    }
}
