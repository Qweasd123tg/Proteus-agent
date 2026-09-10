//! Клиентская история: один экземпляр данных, отдельные подписки на порядок и записи.
//! Bulk reducers сохраняют version существующих Message; streaming обновляет одну запись.
use crate::types::{Message, MessagePhase, MessageRole};
use leptos::prelude::*;
use std::collections::HashMap;

#[derive(Clone, Copy)]
pub(crate) struct Transcript {
    data: StoredValue<TranscriptData>,
    changed: RwSignal<()>,
    order: RwSignal<Vec<u64>>,
}

#[derive(Clone, Copy)]
pub(crate) struct TranscriptWriter(Transcript);

struct Entry {
    position: usize,
    changed: ArcRwSignal<()>,
    stamp: Stamp,
}

#[derive(PartialEq, Eq)]
struct Stamp(u64, bool, usize, usize, MessageRole, Option<MessagePhase>);
impl Stamp {
    fn of(message: &Message) -> Self {
        Self(
            message.version,
            message.streaming,
            message.text.len(),
            message.text_offset,
            message.role,
            message.phase,
        )
    }
}

#[derive(Default)]
struct TranscriptData {
    items: Vec<Message>,
    entries: HashMap<u64, Entry>,
    #[cfg(test)]
    message_reads: std::sync::atomic::AtomicUsize,
}

pub(crate) fn transcript(items: Vec<Message>) -> (Transcript, TranscriptWriter) {
    let read = Transcript {
        data: StoredValue::new(TranscriptData::default()),
        changed: RwSignal::new(()),
        order: RwSignal::new(Vec::new()),
    };
    let write = TranscriptWriter(read);
    write.set(items);
    (read, write)
}

impl Transcript {
    pub(crate) fn ids(self) -> Vec<u64> {
        self.order.get()
    }
    pub(crate) fn len(self) -> usize {
        self.order.with(Vec::len)
    }
    pub(crate) fn with<T>(self, f: impl FnOnce(&Vec<Message>) -> T) -> T {
        self.changed.track();
        self.with_untracked(f)
    }
    pub(crate) fn with_untracked<T>(self, f: impl FnOnce(&Vec<Message>) -> T) -> T {
        self.data.with_value(|data| f(&data.items))
    }
    #[cfg(test)]
    pub(crate) fn get_untracked(self) -> Vec<Message> {
        self.with_untracked(Clone::clone)
    }

    /// Подписка живёт вместе с карточкой. Поиск O(1); чужие изменения её не будят.
    pub(crate) fn message(self, id: u64) -> Memo<Option<Message>> {
        let changed = self.data.with_value(|data| {
            data.entries
                .get(&id)
                .expect("message must be present in transcript order")
                .changed
                .clone()
        });
        Memo::new(move |_| {
            changed.track();
            self.data.with_value(|data| {
                #[cfg(test)]
                data.message_reads
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                data.entries
                    .get(&id)
                    .map(|entry| data.items[entry.position].clone())
            })
        })
    }
}

impl TranscriptWriter {
    /// Authoritative history/reset: одинаковые локальные id могут содержать другие данные.
    pub(crate) fn set(self, items: Vec<Message>) {
        self.reconcile(true, |current| *current = items);
    }
    /// Редкие структурные изменения и bulk reducers обходят историю один раз.
    pub(crate) fn update(self, f: impl FnOnce(&mut Vec<Message>)) {
        self.reconcile(false, f);
    }
    /// Горячий путь: после поиска обновляется и уведомляется только найденная запись.
    pub(crate) fn update_matching(
        self,
        predicate: impl Fn(&Message) -> bool,
        update: impl FnOnce(&mut Message),
    ) -> bool {
        let mut changed = None;
        self.0.data.update_value(|data| {
            let Some(position) = data.items.iter().position(predicate) else {
                return;
            };
            let message = &mut data.items[position];
            let id = message.id;
            update(message);
            assert_eq!(
                message.id, id,
                "point update cannot change message identity"
            );
            let entry = data.entries.get_mut(&id).expect("indexed message");
            entry.stamp = Stamp::of(message);
            changed = Some(entry.changed.clone());
        });
        if let Some(changed) = changed {
            changed.notify();
            self.0.changed.notify();
            true
        } else {
            false
        }
    }

    /// Terminal/phase updates do not change membership or rebuild the index.
    pub(crate) fn update_where(
        self,
        predicate: impl Fn(&Message) -> bool,
        mut update: impl FnMut(&mut Message),
    ) {
        let mut notifications = Vec::new();
        self.0.data.update_value(|data| {
            for message in data.items.iter_mut().filter(|message| predicate(message)) {
                let id = message.id;
                update(message);
                assert_eq!(
                    message.id, id,
                    "point update cannot change message identity"
                );
                let entry = data.entries.get_mut(&id).expect("indexed message");
                entry.stamp = Stamp::of(message);
                notifications.push(entry.changed.clone());
            }
        });
        if !notifications.is_empty() {
            for notification in notifications {
                notification.notify();
            }
            self.0.changed.notify();
        }
    }

    fn reconcile(self, replace: bool, f: impl FnOnce(&mut Vec<Message>)) {
        let mut notifications = Vec::new();
        let mut ids = Vec::new();
        self.0.data.update_value(|data| {
            f(&mut data.items);
            let mut previous = std::mem::take(&mut data.entries);
            for (position, message) in data.items.iter().enumerate() {
                ids.push(message.id);
                let stamp = Stamp::of(message);
                let mut entry = previous.remove(&message.id).unwrap_or_else(|| Entry {
                    position,
                    changed: ArcRwSignal::new(()),
                    stamp: Stamp::of(message),
                });
                if replace || entry.stamp != stamp {
                    notifications.push(entry.changed.clone());
                }
                entry.position = position;
                entry.stamp = stamp;
                assert!(
                    data.entries.insert(message.id, entry).is_none(),
                    "duplicate local message id"
                );
            }
            notifications.extend(previous.into_values().map(|entry| entry.changed));
        });
        let reordered = self.0.order.with_untracked(|order| *order != ids);
        if reordered {
            self.0.order.set(ids);
        }
        let changed = reordered || !notifications.is_empty();
        // Notifications occur after the data lock is released, so every reader sees one snapshot.
        for notification in notifications {
            notification.notify();
        }
        if changed {
            self.0.changed.notify();
        }
    }
}

#[cfg(test)]
mod tests;
