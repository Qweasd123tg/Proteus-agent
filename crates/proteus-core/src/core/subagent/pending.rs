//! Реестр запущенных (`spawn`) детей subagent-runner-а: `JoinHandle` +
//! cancel-токен на spawn_id. Общий для builtin-реализаций (`sequential`,
//! `process`).
//!
//! Жизненный цикл записи: `reserve` (жёсткий cap по `max_parallel`) →
//! `attach` (JoinHandle после `tokio::spawn`) → `remove` в `wait`. Ребёнок,
//! которого так и не дождались (workflow упал между spawn и wait),
//! доработает detached-таской; его завершённая запись вытесняется при
//! следующем `reserve`, когда реестр упирается в cap.

use std::collections::HashMap;

use anyhow::{Result, anyhow, bail};
use tokio::task::JoinHandle;

use crate::contracts::{CancellationToken, SubagentResult};

#[derive(Default)]
pub(super) struct PendingChildren {
    entries: HashMap<String, PendingChild>,
    next_seq: u64,
}

pub(super) struct PendingChild {
    seq: u64,
    cancel: CancellationToken,
    join: Option<JoinHandle<Result<SubagentResult>>>,
}

impl PendingChildren {
    /// Резервирует слот под ребёнка. Если реестр упёрся в `max_parallel`,
    /// сначала вытесняются самые старые завершённые-но-не-затребованные
    /// записи; активные дети не вытесняются никогда.
    pub(super) fn reserve(
        &mut self,
        spawn_id: &str,
        cancel: CancellationToken,
        max_parallel: usize,
    ) -> Result<()> {
        if self.entries.contains_key(spawn_id) {
            bail!("duplicate subagent spawn_id: {spawn_id}");
        }
        while self.entries.len() >= max_parallel {
            let finished_oldest = self
                .entries
                .iter()
                .filter(|(_, entry)| entry.join.as_ref().is_some_and(|join| join.is_finished()))
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
                join: None,
            },
        );
        Ok(())
    }

    /// Прикрепляет JoinHandle к зарезервированному слоту.
    pub(super) fn attach(&mut self, spawn_id: &str, join: JoinHandle<Result<SubagentResult>>) {
        if let Some(entry) = self.entries.get_mut(spawn_id) {
            entry.join = Some(join);
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

    /// Забирает ребёнка для `wait`. Каждый spawn_id выдаётся один раз.
    pub(super) fn take(&mut self, spawn_id: &str) -> Result<JoinHandle<Result<SubagentResult>>> {
        let entry = self.entries.remove(spawn_id).ok_or_else(|| {
            anyhow!("unknown subagent spawn_id (never spawned, already waited, or evicted)")
        })?;
        match entry.join {
            Some(join) => Ok(join),
            None => {
                // Резервация без таски: вернём запись на место, чтобы spawn
                // мог корректно завершить attach.
                self.entries.insert(spawn_id.to_owned(), entry);
                bail!("subagent spawn is still being attached; retry wait");
            }
        }
    }

    /// Отменяет запущенного ребёнка по spawn_id (запись остаётся до `wait`).
    pub(super) fn cancel(&mut self, spawn_id: &str) -> Result<()> {
        let entry = self.entries.get(spawn_id).ok_or_else(|| {
            anyhow!("unknown subagent spawn_id (never spawned, already waited, or evicted)")
        })?;
        entry.cancel.cancel();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::SubagentStatus;

    fn dummy_join() -> JoinHandle<Result<SubagentResult>> {
        tokio::spawn(async { Ok(SubagentResult::new("", SubagentStatus::Completed, 0)) })
    }

    #[tokio::test]
    async fn reserve_attach_take_round_trip() {
        let mut pending = PendingChildren::default();
        pending
            .reserve("a", CancellationToken::new(), 4)
            .expect("reserve");
        pending.attach("a", dummy_join());
        let join = pending.take("a").expect("take");
        let result = join.await.expect("join").expect("result");
        assert_eq!(result.status, SubagentStatus::Completed);
        assert!(pending.take("a").is_err(), "handle выдаётся один раз");
    }

    #[tokio::test]
    async fn reserve_respects_max_parallel_for_active_children() {
        let mut pending = PendingChildren::default();
        pending
            .reserve("a", CancellationToken::new(), 1)
            .expect("reserve");
        // Слот занят активной (не завершённой) резервацией: cap жёсткий.
        let error = pending
            .reserve("b", CancellationToken::new(), 1)
            .expect_err("cap");
        assert!(error.to_string().contains("max_parallel"));
    }

    #[tokio::test]
    async fn reserve_evicts_finished_unclaimed_entries() {
        let mut pending = PendingChildren::default();
        pending
            .reserve("a", CancellationToken::new(), 1)
            .expect("reserve");
        let join = dummy_join();
        // Дожидаемся завершения таски, не забирая запись из реестра.
        while !join.is_finished() {
            tokio::task::yield_now().await;
        }
        pending.attach("a", join);
        pending
            .reserve("b", CancellationToken::new(), 1)
            .expect("evicts finished entry");
        assert!(pending.take("a").is_err(), "запись вытеснена");
    }

    #[tokio::test]
    async fn cancel_cancels_only_target_token() {
        let mut pending = PendingChildren::default();
        let first = CancellationToken::new();
        let second = CancellationToken::new();
        pending.reserve("a", first.clone(), 4).expect("reserve a");
        pending.reserve("b", second.clone(), 4).expect("reserve b");
        pending.cancel("a").expect("cancel");
        assert!(first.is_cancelled());
        assert!(!second.is_cancelled());
        assert!(pending.cancel("missing").is_err());
    }
}
