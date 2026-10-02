//! Клиентская история: один экземпляр данных, отдельные подписки на порядок и записи.
//! Bulk reducers сохраняют version существующих Message; streaming обновляет одну запись.
use crate::types::{Message, MessagePhase, MessageRole, ToolActivityStatus};
use leptos::prelude::*;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy)]
pub(crate) struct Transcript {
    data: StoredValue<TranscriptData>,
    changed: RwSignal<()>,
    order: RwSignal<Vec<u64>>,
    user_changed: RwSignal<()>,
    tools_changed: RwSignal<()>,
    groups_changed: RwSignal<()>,
}

#[derive(Clone, Copy)]
pub(crate) struct TranscriptWriter(Transcript);

struct Entry {
    position: usize,
    changed: ArcRwSignal<()>,
    status_changed: ArcRwSignal<()>,
    stamp: Stamp,
    scopes: Scopes,
    group_class: GroupClass,
    tool_status: Option<ToolActivityStatus>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct GroupClass {
    role: MessageRole,
    root_tool: bool,
}
impl GroupClass {
    fn of(message: &Message) -> Self {
        Self {
            role: message.role,
            root_tool: message.tool.is_some() && message.subagent.is_none(),
        }
    }
}

fn root_tool_status(message: &Message) -> Option<ToolActivityStatus> {
    message
        .tool
        .as_ref()
        .filter(|_| message.subagent.is_none())
        .map(|tool| tool.status)
}

fn is_streaming_reasoning(message: &Message) -> bool {
    message.role == MessageRole::Reasoning && message.streaming
}

// Independent projections must not rescan the transcript for assistant text deltas.
#[derive(Clone, Copy, Default)]
struct Scopes {
    user: bool,
    tools: bool,
}
impl Scopes {
    fn of(message: &Message) -> Self {
        Self {
            user: message.role == MessageRole::User,
            tools: message.tool.is_some() && message.subagent.is_none(),
        }
    }
    fn include(&mut self, other: Self) {
        self.user |= other.user;
        self.tools |= other.tools;
    }
    fn notify(self, transcript: Transcript) {
        if self.user {
            transcript.user_changed.notify();
        }
        if self.tools {
            transcript.tools_changed.notify();
        }
    }
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
    streaming_reasoning: HashSet<u64>,
    #[cfg(test)]
    message_reads: std::sync::atomic::AtomicUsize,
    #[cfg(test)]
    reasoning_examined: std::sync::atomic::AtomicUsize,
}

pub(crate) fn transcript(items: Vec<Message>) -> (Transcript, TranscriptWriter) {
    let read = Transcript {
        data: StoredValue::new(TranscriptData::default()),
        changed: RwSignal::new(()),
        order: RwSignal::new(Vec::new()),
        user_changed: RwSignal::new(()),
        tools_changed: RwSignal::new(()),
        groups_changed: RwSignal::new(()),
    };
    let write = TranscriptWriter(read);
    write.set(items);
    (read, write)
}

impl Transcript {
    #[cfg(test)]
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
    pub(crate) fn with_user_messages<T>(self, f: impl FnOnce(&Vec<Message>) -> T) -> T {
        self.user_changed.track();
        self.with_untracked(f)
    }
    pub(crate) fn with_tool_messages<T>(self, f: impl FnOnce(&Vec<Message>) -> T) -> T {
        self.tools_changed.track();
        self.with_untracked(f)
    }
    pub(crate) fn with_group_structure<T>(self, f: impl FnOnce(&Vec<Message>) -> T) -> T {
        self.groups_changed.track();
        self.with_untracked(f)
    }
    /// The mounted chain tracks only its own status signals, without cloning
    /// result previews or subscribing to other chains.
    pub(crate) fn with_tool_statuses<T>(
        self,
        ids: &[u64],
        f: impl FnOnce(&[Option<ToolActivityStatus>]) -> T,
    ) -> T {
        self.data.with_value(|data| {
            let statuses = ids
                .iter()
                .map(|id| {
                    data.entries.get(id).and_then(|entry| {
                        entry.status_changed.track();
                        entry.tool_status
                    })
                })
                .collect::<Vec<_>>();
            f(&statuses)
        })
    }
    pub(crate) fn with_untracked<T>(self, f: impl FnOnce(&Vec<Message>) -> T) -> T {
        self.data.with_value(|data| f(&data.items))
    }
    #[cfg(test)]
    pub(crate) fn get_untracked(self) -> Vec<Message> {
        self.with_untracked(Clone::clone)
    }

    /// Чтение по индексу без клонирования текста. Присутствующая запись будит
    /// читателя только своими изменениями; структурная подписка нужна лишь
    /// отсутствующему id, чтобы увидеть его повторное появление.
    pub(crate) fn with_message<T>(self, id: u64, f: impl FnOnce(Option<&Message>) -> T) -> T {
        self.data.with_value(|data| match data.entries.get(&id) {
            Some(entry) => {
                entry.changed.track();
                f(Some(&data.items[entry.position]))
            }
            None => {
                self.order.track();
                f(None)
            }
        })
    }

    /// Узкая проекция читает запись по ссылке: шапка и выбор вида сообщения
    /// не клонируют растущую историю вложенных вызовов субагента.
    pub(crate) fn select<T>(
        self,
        id: u64,
        select: impl Fn(Option<&Message>) -> T + Send + Sync + 'static,
    ) -> Memo<T>
    where
        T: PartialEq + Send + Sync + 'static,
    {
        let changed = self.data.with_value(|data| {
            data.entries
                .get(&id)
                .expect("indexed message")
                .changed
                .clone()
        });
        Memo::new(move |_| {
            changed.track();
            self.data.with_value(|data| {
                select(
                    data.entries
                        .get(&id)
                        .map(|entry| &data.items[entry.position]),
                )
            })
        })
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
    /// Горячий путь: ищем с конца (активный ответ обычно последний), уведомляем одну запись.
    pub(crate) fn update_matching(
        self,
        predicate: impl Fn(&Message) -> bool,
        update: impl FnOnce(&mut Message),
    ) -> bool {
        let mut changed = None;
        let mut status_changed = None;
        let mut groups_changed = false;
        let mut scopes = Scopes::default();
        self.0.data.update_value(|data| {
            let Some(position) = data.items.iter().rposition(predicate) else {
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
            scopes.include(entry.scopes);
            entry.scopes = Scopes::of(message);
            scopes.include(entry.scopes);
            let group_class = GroupClass::of(message);
            groups_changed = entry.group_class != group_class;
            entry.group_class = group_class;
            let tool_status = root_tool_status(message);
            if entry.tool_status != tool_status {
                status_changed = Some(entry.status_changed.clone());
                entry.tool_status = tool_status;
            }
            if is_streaming_reasoning(message) {
                data.streaming_reasoning.insert(id);
            } else {
                data.streaming_reasoning.remove(&id);
            }
            entry.stamp = Stamp::of(message);
            changed = Some(entry.changed.clone());
        });
        if let Some(changed) = changed {
            changed.notify();
            if let Some(status_changed) = status_changed {
                status_changed.notify();
            }
            self.0.changed.notify();
            scopes.notify(self.0);
            if groups_changed {
                self.0.groups_changed.notify();
            }
            true
        } else {
            false
        }
    }

    /// The common flush path is O(1) when no reasoning is active.
    pub(crate) fn finish_streaming_reasoning(self) {
        let mut notifications = Vec::new();
        let mut scopes = Scopes::default();
        self.0.data.update_value(|data| {
            for id in std::mem::take(&mut data.streaming_reasoning) {
                #[cfg(test)]
                data.reasoning_examined
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let Some(entry) = data.entries.get_mut(&id) else {
                    continue;
                };
                let message = &mut data.items[entry.position];
                if !is_streaming_reasoning(message) {
                    continue;
                }
                message.streaming = false;
                message.version += 1;
                entry.stamp = Stamp::of(message);
                scopes.include(entry.scopes);
                notifications.push(entry.changed.clone());
            }
        });
        if !notifications.is_empty() {
            for notification in notifications {
                notification.notify();
            }
            self.0.changed.notify();
            scopes.notify(self.0);
        }
    }

    fn reconcile(self, replace: bool, f: impl FnOnce(&mut Vec<Message>)) {
        let mut notifications = Vec::new();
        let mut status_notifications = Vec::new();
        let mut groups_changed = false;
        let mut scopes = Scopes::default();
        let mut ids = Vec::new();
        self.0.data.update_value(|data| {
            f(&mut data.items);
            let mut previous = std::mem::take(&mut data.entries);
            data.streaming_reasoning.clear();
            for (position, message) in data.items.iter().enumerate() {
                ids.push(message.id);
                let stamp = Stamp::of(message);
                let group_class = GroupClass::of(message);
                let tool_status = root_tool_status(message);
                let mut entry = previous.remove(&message.id).unwrap_or_else(|| Entry {
                    position,
                    changed: ArcRwSignal::new(()),
                    status_changed: ArcRwSignal::new(()),
                    stamp: Stamp::of(message),
                    scopes: Scopes::of(message),
                    group_class,
                    tool_status,
                });
                if replace
                    || entry.stamp != stamp
                    || entry.group_class != group_class
                    || entry.tool_status != tool_status
                {
                    scopes.include(entry.scopes);
                    scopes.include(Scopes::of(message));
                    notifications.push(entry.changed.clone());
                }
                groups_changed |= entry.group_class != group_class;
                entry.group_class = group_class;
                if entry.tool_status != tool_status {
                    status_notifications.push(entry.status_changed.clone());
                    entry.tool_status = tool_status;
                }
                if is_streaming_reasoning(message) {
                    data.streaming_reasoning.insert(message.id);
                }
                entry.scopes = Scopes::of(message);
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
            // Insertions/removals/reorders can change either projection.
            scopes = Scopes {
                user: true,
                tools: true,
            };
            self.0.order.set(ids);
        }
        groups_changed |= reordered;
        let changed = reordered || !notifications.is_empty();
        // Notifications occur after the data lock is released, so every reader sees one snapshot.
        for notification in notifications {
            notification.notify();
        }
        for notification in status_notifications {
            notification.notify();
        }
        if changed {
            self.0.changed.notify();
            scopes.notify(self.0);
        }
        if groups_changed {
            self.0.groups_changed.notify();
        }
    }
}

#[cfg(test)]
mod tests;
