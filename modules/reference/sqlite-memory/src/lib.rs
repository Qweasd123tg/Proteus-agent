//! SQLite FTS5 memory store как reference process module.
//!
//! `proteus-core` не зависит от `rusqlite`; backend исполняется во внешнем
//! worker process.
//!
//! Экспорты `tool/sqlite_memory` и `context_provider/sqlite_memory`.
//!
//! Путь к базе задаётся config активного export; без него worker
//! использует `.proteus/memory.sqlite` относительно своего `cwd`.

use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

use anyhow::{Context, Result, anyhow};
use proteus_contracts::{
    domain::{MemoryItem, MemoryQuery},
    process_module::{ModuleRegistry, ProcessModuleError},
};
use rusqlite::{Connection, OpenFlags, params};
use serde::Deserialize;
use serde_json::Value;

mod provider;
mod tools;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SqliteMemoryConfig {
    #[serde(default = "default_memory_db_path")]
    path: PathBuf,
}

fn default_memory_db_path() -> PathBuf {
    PathBuf::from(".proteus/memory.sqlite")
}

pub fn config_schema() -> proteus_contracts::domain::ModuleConfigSchema {
    use proteus_contracts::domain::{ConfigField, ConfigValueSchema, ModuleConfigSchema};
    ModuleConfigSchema {
        fields: vec![
            ConfigField::new(
                "path",
                "База памяти",
                "Путь к SQLite относительно рабочей папки модуля.",
                ConfigValueSchema::text(),
            )
            .with_default(default_memory_db_path().to_string_lossy().into_owned()),
        ],
    }
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

impl SqliteMemoryStore {
    fn remember(&self, item: &MemoryItem) -> Result<(), ProcessModuleError> {
        let payload = serde_json::to_string(item)
            .map_err(|error| ProcessModuleError::new(error.to_string()))?;
        remember_impl(&self.conn, &payload)
            .map_err(|error| ProcessModuleError::new(format!("{error:#}")))
    }
    fn recall(&self, query: &MemoryQuery) -> Result<Vec<MemoryItem>, ProcessModuleError> {
        let payload = serde_json::to_string(query)
            .map_err(|error| ProcessModuleError::new(error.to_string()))?;
        let body = recall_impl(&self.conn, &payload)
            .map_err(|error| ProcessModuleError::new(format!("{error:#}")))?;
        serde_json::from_str(&body).map_err(|error| ProcessModuleError::new(error.to_string()))
    }
}

fn remember_impl(conn: &Mutex<Connection>, payload: &str) -> Result<()> {
    let item: MemoryItem =
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
    let query: MemoryQuery =
        serde_json::from_str(payload).with_context(|| "failed to deserialize MemoryQuery JSON")?;
    if query.limit == 0 {
        return Ok("[]".to_owned());
    }
    let limit = i64::try_from(query.limit).unwrap_or(i64::MAX);
    let c = conn.lock().map_err(|_| anyhow!("sqlite mutex poisoned"))?;

    let match_expr = fts_match_expression(&query.text);
    let items: Vec<MemoryItem> = if match_expr.is_empty() {
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

fn row_to_item(row: &rusqlite::Row<'_>) -> rusqlite::Result<MemoryItem> {
    let kind: String = row.get(0)?;
    let content: String = row.get(1)?;
    let metadata_json: String = row.get(2)?;
    let metadata: Value = serde_json::from_str(&metadata_json).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(2, rusqlite::types::Type::Text, Box::new(error))
    })?;
    Ok(MemoryItem::new(kind, content, metadata))
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
    let store = Arc::new(store);
    registry.register_tool(Box::new(tools::MemoryTool {
        store: store.clone(),
        remember: true,
    }))?;
    registry.register_tool(Box::new(tools::MemoryTool {
        store: store.clone(),
        remember: false,
    }))?;
    registry.register_context_provider(
        "sqlite_memory".into(),
        Box::new(provider::MemoryProvider { store }),
    )
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
        let items: Vec<MemoryItem> = serde_json::from_str(&payload).expect("items");

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].kind, "preference");
        assert_eq!(items[0].metadata["source"], "test");

        conn.lock()
            .unwrap()
            .execute("UPDATE memory_items SET metadata = 'broken-json'", [])
            .unwrap();
        for text in ["", "dark"] {
            let error = recall_impl(
                &conn,
                &serde_json::json!({"text":text,"limit":5}).to_string(),
            )
            .unwrap_err();
            assert!(error.to_string().contains("Conversion error"), "{error:#}");
        }
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
        let items: Vec<MemoryItem> = serde_json::from_str(&payload).expect("items");

        assert_eq!(items.len(), 2);
        assert_eq!(items[0].content, "second");
        assert_eq!(items[1].content, "first");
    }

    #[test]
    fn recall_limit_zero_returns_no_items_for_recent_and_fts() {
        let conn = fresh_conn();
        remember_impl(
            &conn,
            r#"{"kind":"fact","content":"remembered","metadata":null}"#,
        )
        .unwrap();
        for text in ["", "remembered"] {
            let payload = recall_impl(
                &conn,
                &serde_json::json!({ "text": text, "limit": 0 }).to_string(),
            )
            .unwrap();
            let items: Vec<MemoryItem> = serde_json::from_str(&payload).unwrap();
            assert!(items.is_empty());
        }
    }

    #[test]
    fn fts_match_expression_sanitizes_tokens() {
        assert_eq!(
            fts_match_expression("React Router!!! v6"),
            "\"React\"* AND \"Router\"* AND \"v6\"*"
        );
    }
}
