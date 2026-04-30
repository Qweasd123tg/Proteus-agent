//! SQLite FTS5 реализация `MemoryStore`.
//!
//! Хранит `MemoryItem`'ы в обычной таблице и дублирует `content` + `kind`
//! в FTS5 virtual table через триггер. `recall` выполняет `MATCH` поиск
//! по FTS5 и возвращает topN по rank'у.
//!
//! Concurrency: одна `rusqlite::Connection` защищена `std::sync::Mutex`.
//! SQLite-операции блокирующие, поэтому `remember`/`recall` оборачиваются
//! в `tokio::task::spawn_blocking`. Один long-lived connection + очередь
//! блокировок адекватно для single-process agent; масштабирование при
//! большом количестве turn'ов — задача будущей итерации.

use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use rusqlite::{Connection, OpenFlags, params};
use serde_json::Value;

use crate::{
    contracts::MemoryStore,
    domain::{MemoryItem, MemoryQuery},
};

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS memory_items (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    kind       TEXT    NOT NULL,
    content    TEXT    NOT NULL,
    metadata   TEXT    NOT NULL DEFAULT '{}',
    created_at INTEGER NOT NULL
);
CREATE VIRTUAL TABLE IF NOT EXISTS memory_fts
    USING fts5(content, kind, content='memory_items', content_rowid='id');
CREATE TRIGGER IF NOT EXISTS memory_items_ai AFTER INSERT ON memory_items BEGIN
    INSERT INTO memory_fts(rowid, content, kind)
    VALUES (new.id, new.content, new.kind);
END;
CREATE TRIGGER IF NOT EXISTS memory_items_ad AFTER DELETE ON memory_items BEGIN
    INSERT INTO memory_fts(memory_fts, rowid, content, kind)
    VALUES ('delete', old.id, old.content, old.kind);
END;
";

pub struct SqliteMemory {
    conn: Arc<Mutex<Connection>>,
}

impl SqliteMemory {
    /// Открывает/создаёт файл SQLite и применяет schema. Путь должен
    /// указывать на существующую папку (родительская не создаётся
    /// автоматически).
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let conn = Connection::open_with_flags(
            path.as_ref(),
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_URI
                | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .with_context(|| format!("failed to open sqlite at {}", path.as_ref().display()))?;
        conn.execute_batch(SCHEMA)
            .with_context(|| "failed to apply sqlite memory schema")?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }
}

#[async_trait]
impl MemoryStore for SqliteMemory {
    async fn remember(&self, item: MemoryItem) -> Result<()> {
        let conn = self.conn.clone();
        let created_at = chrono::Utc::now().timestamp_millis();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let c = conn
                .lock()
                .map_err(|_| anyhow!("sqlite memory mutex poisoned"))?;
            c.execute(
                "INSERT INTO memory_items (kind, content, metadata, created_at) VALUES (?1, ?2, ?3, ?4)",
                params![
                    item.kind,
                    item.content,
                    item.metadata.to_string(),
                    created_at
                ],
            )
            .with_context(|| "failed to insert memory item")?;
            Ok(())
        })
        .await
        .map_err(|join_err| anyhow!("sqlite memory join error: {join_err}"))?
    }

    async fn recall(&self, query: MemoryQuery) -> Result<Vec<MemoryItem>> {
        let conn = self.conn.clone();
        let limit = query.limit.max(1) as i64;
        let text = query.text;
        tokio::task::spawn_blocking(move || -> Result<Vec<MemoryItem>> {
            let c = conn
                .lock()
                .map_err(|_| anyhow!("sqlite memory mutex poisoned"))?;
            let match_expr = fts_match_expression(&text);
            // Пустой match_expr (пустой query после очистки) — отдаём
            // недавние items в обратном порядке вставки, без MATCH.
            let items = if match_expr.is_empty() {
                let mut stmt = c
                    .prepare(
                        "SELECT kind, content, metadata FROM memory_items \
                         ORDER BY id DESC LIMIT ?1",
                    )
                    .with_context(|| "failed to prepare recall fallback")?;
                let rows = stmt
                    .query_map([limit], row_to_memory_item)
                    .with_context(|| "failed to query memory recall fallback")?;
                rows.collect::<std::result::Result<Vec<_>, _>>()
                    .with_context(|| "failed to read memory recall rows")?
            } else {
                let mut stmt = c
                    .prepare(
                        "SELECT memory_items.kind, memory_items.content, memory_items.metadata \
                         FROM memory_items \
                         JOIN memory_fts ON memory_items.id = memory_fts.rowid \
                         WHERE memory_fts MATCH ?1 \
                         ORDER BY rank LIMIT ?2",
                    )
                    .with_context(|| "failed to prepare recall fts query")?;
                let rows = stmt
                    .query_map(params![match_expr, limit], row_to_memory_item)
                    .with_context(|| "failed to query memory recall fts")?;
                rows.collect::<std::result::Result<Vec<_>, _>>()
                    .with_context(|| "failed to read memory recall fts rows")?
            };
            Ok(items)
        })
        .await
        .map_err(|join_err| anyhow!("sqlite memory join error: {join_err}"))?
    }
}

fn row_to_memory_item(row: &rusqlite::Row<'_>) -> rusqlite::Result<MemoryItem> {
    let kind: String = row.get(0)?;
    let content: String = row.get(1)?;
    let metadata_json: String = row.get(2)?;
    let metadata: Value = serde_json::from_str(&metadata_json).unwrap_or(Value::Null);
    Ok(MemoryItem::new(kind, content, metadata))
}

/// Готовит строку для FTS5 `MATCH`. FTS5 parser не любит одиночные
/// кавычки и control-символы, плюс пустой запрос — ошибка. Простой
/// sanitization: оставляем буквы/цифры/пробелы, склеиваем tokens через
/// `AND`, чтобы "react router" искался как и то, и то.
fn fts_match_expression(text: &str) -> String {
    let tokens: Vec<String> = text
        .split_whitespace()
        .map(|token| {
            token
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
                .collect::<String>()
        })
        .filter(|token| !token.is_empty())
        .map(|token| format!("\"{token}\"*"))
        .collect();
    tokens.join(" AND ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::tempdir;

    fn fresh_store() -> (tempfile::TempDir, SqliteMemory) {
        let dir = tempdir().unwrap();
        let path = dir.path().join("memory.sqlite");
        let store = SqliteMemory::open(&path).unwrap();
        (dir, store)
    }

    #[tokio::test]
    async fn remember_then_recall_returns_item_by_fts_match() {
        let (_dir, store) = fresh_store();
        store
            .remember(MemoryItem::new(
                "preference",
                "user prefers tabs over spaces",
                json!({ "source": "manual" }),
            ))
            .await
            .unwrap();
        store
            .remember(MemoryItem::new(
                "fact",
                "repo uses pnpm",
                Value::Null,
            ))
            .await
            .unwrap();

        let hits = store
            .recall(MemoryQuery::new("tabs", 10))
            .await
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].kind, "preference");
        assert!(hits[0].content.contains("tabs"));
        assert_eq!(hits[0].metadata["source"], "manual");
    }

    #[tokio::test]
    async fn recall_respects_limit() {
        let (_dir, store) = fresh_store();
        for i in 0..5 {
            store
                .remember(MemoryItem::new(
                    "fact",
                    format!("sample content {i}"),
                    Value::Null,
                ))
                .await
                .unwrap();
        }
        let hits = store
            .recall(MemoryQuery::new("sample", 3))
            .await
            .unwrap();
        assert_eq!(hits.len(), 3);
    }

    #[tokio::test]
    async fn empty_query_returns_recent_items_in_reverse_order() {
        let (_dir, store) = fresh_store();
        for i in 0..3 {
            store
                .remember(MemoryItem::new("fact", format!("item {i}"), Value::Null))
                .await
                .unwrap();
        }
        let hits = store.recall(MemoryQuery::new("", 2)).await.unwrap();
        assert_eq!(hits.len(), 2);
        // Последний записанный приходит первым.
        assert_eq!(hits[0].content, "item 2");
        assert_eq!(hits[1].content, "item 1");
    }

    #[tokio::test]
    async fn recall_with_no_matches_is_empty() {
        let (_dir, store) = fresh_store();
        store
            .remember(MemoryItem::new("fact", "unrelated", Value::Null))
            .await
            .unwrap();
        let hits = store
            .recall(MemoryQuery::new("tabs", 10))
            .await
            .unwrap();
        assert!(hits.is_empty());
    }

    #[test]
    fn fts_match_expression_sanitizes_tokens() {
        assert_eq!(fts_match_expression(""), "");
        assert_eq!(fts_match_expression("!!!"), "");
        assert_eq!(fts_match_expression("react router"), "\"react\"* AND \"router\"*");
        assert_eq!(fts_match_expression("snake_case"), "\"snake_case\"*");
        assert_eq!(fts_match_expression("tabs  spaces"), "\"tabs\"* AND \"spaces\"*");
    }
}

/// Helper для `module_catalog`: резолвит путь к базе на основе cwd.
/// По умолчанию — `{cwd}/.agent/memory.sqlite`. Создаёт родительские
/// директории если их нет.
pub fn default_sqlite_memory_path(cwd: &Path) -> Result<PathBuf> {
    let dir = cwd.join(".agent");
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("failed to create {}", dir.display()))?;
    Ok(dir.join("memory.sqlite"))
}
