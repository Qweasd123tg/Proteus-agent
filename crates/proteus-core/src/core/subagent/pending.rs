//! Реестр запущенных (`spawn`) детей subagent-runner-а: `JoinHandle` +
//! cancel-токен на spawn_id. Общий для builtin-реализаций (`sequential`,
//! `process`).
//!
//! Жизненный цикл записи: `reserve` (жёсткий cap по `max_parallel`) →
//! `attach` (detached monitor забирает JoinHandle) → кешированный terminal
//! outcome. Waiter никогда не владеет JoinHandle, поэтому его отмена не теряет
//! результат; завершённая запись вытесняется при следующем `reserve`, когда
//! реестр упирается в cap.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use anyhow::{Result, anyhow, bail};
use tokio::{sync::Notify, task::JoinHandle};

use crate::contracts::{CancellationToken, SubagentResult};

use super::mailbox::ChildMailbox;

#[derive(Default)]
pub(super) struct PendingChildren {
    entries: HashMap<String, PendingChild>,
    next_seq: u64,
}

pub(super) struct PendingChild {
    seq: u64,
    cancel: CancellationToken,
    mailbox: Arc<ChildMailbox>,
    outcome: Arc<PendingOutcome>,
    evictable_when_ready: bool,
}

#[derive(Default)]
pub(super) struct PendingOutcome {
    result: Mutex<Option<std::result::Result<SubagentResult, String>>>,
    notify: Notify,
}

impl PendingChildren {
    /// Резервирует слот под ребёнка. Если реестр упёрся в `max_parallel`,
    /// сначала вытесняются самые старые завершённые-но-не-затребованные
    /// записи; активные дети не вытесняются никогда.
    pub(super) fn reserve(
        &mut self,
        spawn_id: &str,
        cancel: CancellationToken,
        mailbox: Arc<ChildMailbox>,
        max_parallel: usize,
        evictable_when_ready: bool,
    ) -> Result<()> {
        if self.entries.contains_key(spawn_id) {
            bail!("duplicate subagent spawn_id: {spawn_id}");
        }
        while self.entries.len() >= max_parallel {
            let finished_oldest = self
                .entries
                .iter()
                .filter(|(_, entry)| entry.evictable_when_ready && entry.outcome.is_ready())
                .min_by_key(|(_, entry)| entry.seq)
                .map(|(id, _)| id.clone());
            match finished_oldest {
                Some(id) => {
                    self.entries.remove(&id);
                }
                None => bail!(
                    "too many concurrent subagent children (max_parallel {max_parallel}); \
                     wait for spawned tasks before starting new ones"
                ),
            }
        }
        let seq = self.next_seq;
        self.next_seq = self.next_seq.wrapping_add(1);
        self.entries.insert(
            spawn_id.to_owned(),
            PendingChild {
                seq,
                cancel,
                mailbox,
                outcome: Arc::new(PendingOutcome::default()),
                evictable_when_ready,
            },
        );
        Ok(())
    }

    /// Прикрепляет JoinHandle к зарезервированному слоту.
    pub(super) fn attach(&mut self, spawn_id: &str, join: JoinHandle<Result<SubagentResult>>) {
        if let Some(entry) = self.entries.get_mut(spawn_id) {
            let outcome = entry.outcome.clone();
            tokio::spawn(async move {
                let result = match join.await {
                    Ok(Ok(result)) => Ok(result),
                    Ok(Err(error)) => Err(format!("{error:#}")),
                    Err(error) => Err(format!("subagent child task failed: {error}")),
                };
                outcome.complete(result);
            });
        } else {
            // Слот сняли между reserve и attach (не должен случаться:
            // оба вызова идут подряд в spawn) — не оставляем таску без
            // индивидуальной отмены незаметно.
            join.abort();
        }
    }

    /// Снимает резервацию (ошибка между reserve и attach).
    pub(super) fn release(&mut self, spawn_id: &str) {
        self.entries.remove(spawn_id);
    }

    /// Возвращает разделяемый terminal outcome для cancellation-safe wait.
    pub(super) fn outcome(&self, spawn_id: &str) -> Result<Arc<PendingOutcome>> {
        let entry = self.entries.get(spawn_id).ok_or_else(|| {
            anyhow!("unknown subagent spawn_id (never spawned, already waited, or evicted)")
        })?;
        Ok(entry.outcome.clone())
    }

    /// Помечает terminal result забранным. Вызывается только после await:
    /// отменённый waiter не потребляет handle, а два конкурентных waiter-а
    /// всё равно не могут оба успешно вернуть один результат.
    pub(super) fn consume(&mut self, spawn_id: &str) -> Result<()> {
        let entry = self.entries.get(spawn_id).ok_or_else(|| {
            anyhow!("unknown subagent spawn_id (never spawned, already waited, or evicted)")
        })?;
        if !entry.outcome.is_ready() {
            bail!("subagent result is not ready to consume");
        }
        self.entries.remove(spawn_id);
        Ok(())
    }

    /// Отменяет запущенного ребёнка по spawn_id (запись остаётся до `wait`).
    pub(super) fn cancel(&mut self, spawn_id: &str) -> Result<()> {
        let entry = self.entries.get(spawn_id).ok_or_else(|| {
            anyhow!("unknown subagent spawn_id (never spawned, already waited, or evicted)")
        })?;
        entry.cancel.cancel();
        Ok(())
    }

    pub(super) fn send(&self, spawn_id: &str, message: &str) -> Result<usize> {
        let entry = self.entries.get(spawn_id).ok_or_else(|| {
            anyhow!("unknown subagent spawn_id (never spawned, already waited, or evicted)")
        })?;
        entry.mailbox.enqueue(message)
    }
}

impl PendingOutcome {
    fn is_ready(&self) -> bool {
        self.result
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_some()
    }

    fn complete(&self, result: std::result::Result<SubagentResult, String>) {
        *self
            .result
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(result);
        self.notify.notify_waiters();
    }

    pub(super) async fn wait(&self) -> Result<SubagentResult> {
        loop {
            let notified = self.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if let Some(result) = self
                .result
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone()
            {
                return result.map_err(anyhow::Error::msg);
            }
            notified.as_mut().await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::SubagentStatus;

    fn dummy_join() -> JoinHandle<Result<SubagentResult>> {
        tokio::spawn(async { Ok(SubagentResult::new("", SubagentStatus::Completed, 0)) })
    }

    fn mailbox() -> Arc<ChildMailbox> {
        Arc::new(ChildMailbox::default())
    }

    #[tokio::test]
    async fn reserve_attach_take_round_trip() {
        let mut pending = PendingChildren::default();
        pending
            .reserve("a", CancellationToken::new(), mailbox(), 4, true)
            .expect("reserve");
        pending.attach("a", dummy_join());
        let outcome = pending.outcome("a").expect("outcome");
        let result = outcome.wait().await.expect("result");
        assert_eq!(result.status, SubagentStatus::Completed);
        let repeated = outcome.wait().await.expect("repeatable result");
        assert_eq!(repeated.status, SubagentStatus::Completed);
    }

    #[tokio::test]
    async fn reserve_respects_max_parallel_for_active_children() {
        let mut pending = PendingChildren::default();
        pending
            .reserve("a", CancellationToken::new(), mailbox(), 1, true)
            .expect("reserve");
        // Слот занят активной (не завершённой) резервацией: cap жёсткий.
        let error = pending
            .reserve("b", CancellationToken::new(), mailbox(), 1, true)
            .expect_err("cap");
        assert!(error.to_string().contains("max_parallel"));
    }

    #[tokio::test]
    async fn reserve_evicts_finished_unclaimed_entries() {
        let mut pending = PendingChildren::default();
        pending
            .reserve("a", CancellationToken::new(), mailbox(), 1, true)
            .expect("reserve");
        pending.attach("a", dummy_join());
        let outcome = pending.outcome("a").expect("outcome");
        while !outcome.is_ready() {
            tokio::task::yield_now().await;
        }
        pending
            .reserve("b", CancellationToken::new(), mailbox(), 1, true)
            .expect("evicts finished entry");
        assert!(pending.outcome("a").is_err(), "запись вытеснена");
    }

    #[tokio::test]
    async fn reserve_does_not_evict_control_owned_completion_before_monitor_waits() {
        let mut pending = PendingChildren::default();
        pending
            .reserve("a", CancellationToken::new(), mailbox(), 1, false)
            .expect("reserve retained child");
        pending.attach("a", dummy_join());
        let outcome = pending.outcome("a").expect("outcome");
        while !outcome.is_ready() {
            tokio::task::yield_now().await;
        }

        let error = pending
            .reserve("b", CancellationToken::new(), mailbox(), 1, true)
            .expect_err("control-owned result must remain addressable");
        assert!(error.to_string().contains("max_parallel"));
        assert!(pending.outcome("a").is_ok());
    }

    #[tokio::test]
    async fn cancel_cancels_only_target_token() {
        let mut pending = PendingChildren::default();
        let first = CancellationToken::new();
        let second = CancellationToken::new();
        pending
            .reserve("a", first.clone(), mailbox(), 4, true)
            .expect("reserve a");
        pending
            .reserve("b", second.clone(), mailbox(), 4, true)
            .expect("reserve b");
        pending.cancel("a").expect("cancel");
        assert!(first.is_cancelled());
        assert!(!second.is_cancelled());
        assert!(pending.cancel("missing").is_err());
    }

    #[tokio::test]
    async fn cancel_remains_available_while_wait_is_active() {
        let mut pending = PendingChildren::default();
        let token = CancellationToken::new();
        pending
            .reserve("a", token.clone(), mailbox(), 1, true)
            .expect("reserve");
        pending.attach("a", dummy_join());
        let outcome = pending.outcome("a").expect("outcome");
        pending.cancel("a").expect("cancel during wait");
        assert!(token.is_cancelled());
        let _ = outcome.wait().await;
    }
}
