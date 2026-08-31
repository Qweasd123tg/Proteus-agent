//! SQLite FTS5 memory store как reference process module.
//!
//! `proteus-core` не зависит от `rusqlite`; backend исполняется во внешнем
//! worker process.
//!
//! Регистрируется под id `"sqlite"`.
//!
//! Путь к базе задаётся `module_config.memory.sqlite.path`; без него worker
//! использует `.proteus/memory.sqlite` относительно своего `cwd`.

use std::{path::PathBuf, sync::Mutex};

use anyhow::{Context, Result, anyhow};
use proteus_contracts::process_module::{
    MemoryModule, MemoryModuleHost, MemoryModuleObject, ModuleRegistry, ProcessModuleError,
};
use rusqlite::{Connection, OpenFlags, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Копия `MemoryItem` / `MemoryQuery` из `proteus-contracts::domain`.
/// Process contract передаёт эти значения как JSON, поэтому worker-side
/// реализация разбирает локальную wire-форму вручную.
#[derive(Serialize, Deserialize)]
struct ItemWire {
    kind: String,
    content: String,
    #[serde(default)]
    metadata: Value,
}

#[derive(Serialize, Deserialize)]
struct QueryWire {
    text: String,
    limit: usize,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SqliteMemoryConfig {
    #[serde(default = "default_memory_db_path")]
    path: PathBuf,
}

fn default_memory_db_path() -> PathBuf {
    PathBuf::from(".proteus/memory.sqlite")
}

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

struct SqliteMemoryStore {
    conn: Mutex<Connection>,
}

impl SqliteMemoryStore {
    fn open(path: PathBuf) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        let conn = Connection::open_with_flags(
            &path,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_URI
                | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .with_context(|| format!("failed to open {}", path.display()))?;
        conn.execute_batch(SCHEMA)
            .with_context(|| "failed to apply schema")?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }
}

impl MemoryModule for SqliteMemoryStore {
    fn remember_json(
        &self,
        item_json: String,
        _context_json: String,
        _host: &mut dyn MemoryModuleHost,
    ) -> Result<(), ProcessModuleError> {
        let payload = item_json;
        match remember_impl(&self.conn, &payload) {
            Ok(()) => Ok(()),
            Err(error) => Err(ProcessModuleError::new(format!("{error:#}"))),
        }
    }

    fn recall_json(
        &self,
        query_json: String,
        _context_json: String,
        _host: &mut dyn MemoryModuleHost,
    ) -> Result<String, ProcessModuleError> {
        let payload = query_json;
        match recall_impl(&self.conn, &payload) {
            Ok(body) => Ok(String::from(body)),
            Err(error) => Err(ProcessModuleError::new(format!("{error:#}"))),
        }
    }
}

fn remember_impl(conn: &Mutex<Connection>, payload: &str) -> Result<()> {
    let item: ItemWire =
        serde_json::from_str(payload).with_context(|| "failed to deserialize MemoryItem JSON")?;
    let created_at = chrono::Utc::now().timestamp_millis();
    let c = conn.lock().map_err(|_| anyhow!("sqlite mutex poisoned"))?;
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
}

fn recall_impl(conn: &Mutex<Connection>, payload: &str) -> Result<String> {
    let query: QueryWire =
        serde_json::from_str(payload).with_context(|| "failed to deserialize MemoryQuery JSON")?;
    let limit = query.limit.max(1) as i64;
    let c = conn.lock().map_err(|_| anyhow!("sqlite mutex poisoned"))?;

    let match_expr = fts_match_expression(&query.text);
    let items: Vec<ItemWire> = if match_expr.is_empty() {
        let mut stmt = c.prepare(
            "SELECT kind, content, metadata FROM memory_items ORDER BY id DESC LIMIT ?1",
        )?;
        stmt.query_map([limit], row_to_item)?
            .collect::<std::result::Result<Vec<_>, _>>()?
    } else {
        let mut stmt = c.prepare(
            "SELECT memory_items.kind, memory_items.content, memory_items.metadata \
             FROM memory_items \
             JOIN memory_fts ON memory_items.id = memory_fts.rowid \
             WHERE memory_fts MATCH ?1 \
             ORDER BY rank LIMIT ?2",
        )?;
        stmt.query_map(params![match_expr, limit], row_to_item)?
            .collect::<std::result::Result<Vec<_>, _>>()?
    };

    Ok(serde_json::to_string(&items)?)
}

fn row_to_item(row: &rusqlite::Row<'_>) -> rusqlite::Result<ItemWire> {
    let kind: String = row.get(0)?;
    let content: String = row.get(1)?;
    let metadata_json: String = row.get(2)?;
    let metadata: Value = serde_json::from_str(&metadata_json).unwrap_or(Value::Null);
    Ok(ItemWire {
        kind,
        content,
        metadata,
    })
}

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

pub fn register_modules(registry: &mut dyn ModuleRegistry) -> Result<(), ProcessModuleError> {
    let config: SqliteMemoryConfig = match serde_json::from_value(registry.module_config().clone())
    {
        Ok(config) => config,
        Err(error) => {
            return Err(ProcessModuleError::new(format!(
                "invalid sqlite memory config: {error}"
            )));
        }
    };
    let store = match SqliteMemoryStore::open(config.path) {
        Ok(store) => store,
        Err(error) => {
            return Err(ProcessModuleError::new(format!(
                "sqlite-memory init failed: {error:#}"
            )));
        }
    };
    let obj: MemoryModuleObject = Box::new(store);
    if let Err(err) = registry.register_memory(String::from("sqlite"), obj) {
        return Err(err);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_conn() -> Mutex<Connection> {
        let conn = Connection::open_in_memory().expect("in-memory sqlite");
        conn.execute_batch(SCHEMA).expect("schema");
        Mutex::new(conn)
    }

    #[test]
    fn remember_then_recall_by_fts_match() {
        let conn = fresh_conn();
        remember_impl(
            &conn,
            r#"{"kind":"preference","content":"prefer dark mode","metadata":{"source":"test"}}"#,
        )
        .expect("remember preference");
        remember_impl(
            &conn,
            r#"{"kind":"fact","content":"React Router v6 is in use","metadata":null}"#,
        )
        .expect("remember fact");

        let payload = recall_impl(&conn, r#"{"text":"dark","limit":5}"#).expect("recall");
        let items: Vec<ItemWire> = serde_json::from_str(&payload).expect("items");

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].kind, "preference");
        assert_eq!(items[0].metadata["source"], "test");
    }

    #[test]
    fn empty_query_returns_recent_items_first() {
        let conn = fresh_conn();
        remember_impl(
            &conn,
            r#"{"kind":"fact","content":"first","metadata":null}"#,
        )
        .expect("first");
        remember_impl(
            &conn,
            r#"{"kind":"fact","content":"second","metadata":null}"#,
        )
        .expect("second");

        let payload = recall_impl(&conn, r#"{"text":"","limit":2}"#).expect("recall");
        let items: Vec<ItemWire> = serde_json::from_str(&payload).expect("items");

        assert_eq!(items.len(), 2);
        assert_eq!(items[0].content, "second");
        assert_eq!(items[1].content, "first");
    }

    #[test]
    fn fts_match_expression_sanitizes_tokens() {
        assert_eq!(
            fts_match_expression("React Router!!! v6"),
            "\"React\"* AND \"Router\"* AND \"v6\"*"
        );
    }
}
