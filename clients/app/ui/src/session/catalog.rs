use crate::{api::get_json, types::SessionSummary};
use leptos::{prelude::*, task::spawn_local};

/// A HTTP catalog response is valid only until the next request or live mutation.
#[derive(Clone, Copy)]
pub(crate) struct SessionCatalog {
    revision: RwSignal<u64>,
    loading: RwSignal<bool>,
}

impl SessionCatalog {
    pub(crate) fn new() -> Self {
        Self {
            revision: RwSignal::new(0),
            loading: RwSignal::new(false),
        }
    }

    pub(crate) fn invalidate(self) -> u64 {
        self.loading.set(false);
        self.revision
            .update(|value| *value = value.checked_add(1).expect("catalog revision overflow"));
        self.revision.get_untracked()
    }

    /// A live mutation drops the response in flight; it may carry other
    /// changes (a new chat's title), so the caller requests the list again.
    pub(crate) fn supersede(self) -> bool {
        let dropped = self.loading.get_untracked();
        self.invalidate();
        dropped
    }

    pub(crate) fn current(self, revision: u64) -> bool {
        self.revision.try_get_untracked() == Some(revision)
    }

    pub(crate) fn load(
        self,
        set_sessions: WriteSignal<Vec<SessionSummary>>,
        set_status: WriteSignal<String>,
    ) {
        let revision = self.invalidate();
        self.loading.set(true);
        set_status.set("загружаю сессии".to_owned());
        spawn_local(async move {
            let response = get_json::<Vec<SessionSummary>>("/sessions").await;
            if !self.current(revision) {
                return;
            }
            self.loading.set(false);
            match response {
                Ok(items) => {
                    let count = items.len();
                    set_sessions.set(items);
                    set_status.set(if count == 0 {
                        "прошлых сессий нет".to_owned()
                    } else {
                        format!("{count} сессий")
                    });
                }
                Err(error) => set_status.set(format!("сессии недоступны: {error}")),
            }
        });
    }
}
