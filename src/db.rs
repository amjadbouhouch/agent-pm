//! The SQLite file: where it lives, its schema, and the lookups every command group shares.

use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension, params};

use crate::error::{AppError, Result, err};

/// SQL for the current UTC time, in the format the schema's timestamp defaults use.
/// For `updated_at = {NOW_SQL}` in updates.
pub const NOW_SQL: &str = "strftime('%Y-%m-%dT%H:%M:%SZ', 'now')";

/// `--db` > `$APM_DB` > `~/.agent-pm/apm.db`.
pub fn path(flag: Option<&Path>) -> Result<PathBuf> {
    if let Some(p) = flag {
        return Ok(p.to_path_buf());
    }
    if let Some(p) = std::env::var_os("APM_DB").filter(|p| !p.is_empty()) {
        return Ok(PathBuf::from(p));
    }
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .filter(|p| !p.is_empty())
        .ok_or_else(|| err("cannot find a home directory; set APM_DB or pass --db"))?;
    Ok(PathBuf::from(home).join(".agent-pm").join("apm.db"))
}

/// Open (creating if needed) and migrate the database.
pub fn open(path: &Path) -> Result<Connection> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    let mut conn = Connection::open(path)?;
    // WAL + busy timeout: several agents may write at once.
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    migrate(&mut conn)?;
    Ok(conn)
}

/// Schema migrations, applied in order. `PRAGMA user_version` = number applied.
/// Never edit a released entry; append a new one.
const MIGRATIONS: &[&str] = &[r#"
CREATE TABLE projects (
    id          INTEGER PRIMARY KEY,
    key         TEXT NOT NULL UNIQUE,
    name        TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    next_seq    INTEGER NOT NULL DEFAULT 1,
    created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    updated_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

CREATE TABLE milestones (
    id          INTEGER PRIMARY KEY,
    project_id  INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    name        TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    due         TEXT,
    status      TEXT NOT NULL DEFAULT 'open' CHECK (status IN ('open', 'closed')),
    created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    updated_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    UNIQUE (project_id, name)
);

CREATE TABLE items (
    id           INTEGER PRIMARY KEY,
    project_id   INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    seq          INTEGER NOT NULL,
    parent_id    INTEGER REFERENCES items(id) ON DELETE CASCADE,
    milestone_id INTEGER REFERENCES milestones(id) ON DELETE SET NULL,
    title        TEXT NOT NULL,
    body         TEXT NOT NULL DEFAULT '',
    status       TEXT NOT NULL DEFAULT 'todo'
                 CHECK (status IN ('todo', 'doing', 'blocked', 'done', 'cancelled')),
    priority     TEXT NOT NULL DEFAULT 'none'
                 CHECK (priority IN ('none', 'low', 'medium', 'high', 'urgent')),
    assignee     TEXT,
    due          TEXT,
    created_by   TEXT NOT NULL,
    created_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    updated_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    UNIQUE (project_id, seq)
);
CREATE INDEX items_parent ON items(parent_id);
CREATE INDEX items_milestone ON items(milestone_id);

CREATE TABLE attributes (
    id         INTEGER PRIMARY KEY,
    project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    name       TEXT NOT NULL,
    type       TEXT NOT NULL CHECK (type IN ('text', 'number', 'select', 'date', 'checkbox')),
    options    TEXT NOT NULL DEFAULT '[]',
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    UNIQUE (project_id, name)
);

CREATE TABLE item_values (
    item_id      INTEGER NOT NULL REFERENCES items(id) ON DELETE CASCADE,
    attribute_id INTEGER NOT NULL REFERENCES attributes(id) ON DELETE CASCADE,
    value        TEXT NOT NULL,
    PRIMARY KEY (item_id, attribute_id)
);

CREATE TABLE comments (
    id         INTEGER PRIMARY KEY,
    item_id    INTEGER NOT NULL REFERENCES items(id) ON DELETE CASCADE,
    author     TEXT NOT NULL,
    body       TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);
CREATE INDEX comments_item ON comments(item_id);
"#];

pub fn schema_version(conn: &Connection) -> Result<i64> {
    Ok(conn.pragma_query_value(None, "user_version", |r| r.get(0))?)
}

fn migrate(conn: &mut Connection) -> Result<()> {
    let current = schema_version(conn)? as usize;
    if current > MIGRATIONS.len() {
        return Err(err(format!(
            "database schema v{current} is newer than this apm (v{}); upgrade apm",
            MIGRATIONS.len()
        )));
    }
    for (i, sql) in MIGRATIONS.iter().enumerate().skip(current) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", (i + 1) as i64)?;
        tx.commit()?;
    }
    Ok(())
}

/// Project id for a key, case-insensitive (`apm` finds `APM`).
pub fn project_id(conn: &Connection, key: &str) -> Result<i64> {
    conn.query_row(
        "SELECT id FROM projects WHERE key = ?1",
        params![key.to_uppercase()],
        |r| r.get(0),
    )
    .optional()?
    .ok_or_else(|| err(format!("no such project: {}", key.to_uppercase())))
}

/// Map a write's UNIQUE-constraint failure to `exists()` (e.g. `project already exists: X`).
/// The constraint, not a pre-check, decides: several agents may write at once.
pub fn unique<T>(res: rusqlite::Result<T>, exists: impl FnOnce() -> AppError) -> Result<T> {
    match res {
        Err(rusqlite::Error::SqliteFailure(e, _))
            if e.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE =>
        {
            Err(exists())
        }
        res => Ok(res?),
    }
}

/// A task's `KEY-N` reference, e.g. `APM-12`.
pub fn item_key(key: &str, seq: i64) -> String {
    format!("{key}-{seq}")
}

/// An item found by its `KEY-N` reference.
#[derive(Debug, Clone, Copy)]
pub struct ItemRef {
    pub id: i64,
    pub project_id: i64,
}

/// Validate a project key (1-10 ASCII letters/digits, starting with a letter); returns it uppercased.
pub fn check_project_key(key: &str) -> Result<String> {
    let ok = (1..=10).contains(&key.len())
        && key.starts_with(|c: char| c.is_ascii_alphabetic())
        && key.chars().all(|c| c.is_ascii_alphanumeric());
    if !ok {
        return Err(err(format!(
            "invalid project key `{key}`: 1-10 letters/digits, starting with a letter"
        )));
    }
    Ok(key.to_uppercase())
}

/// Split `APM-12` into (`APM`, 12).
pub fn parse_item_key(s: &str) -> Result<(String, i64)> {
    let bad = || {
        err(format!(
            "invalid item reference `{s}`, expected KEY-N like APM-12"
        ))
    };
    let (key, n) = s.split_once('-').ok_or_else(bad)?;
    let key = check_project_key(key).map_err(|_| bad())?;
    if n.is_empty() || !n.bytes().all(|b| b.is_ascii_digit()) {
        return Err(bad());
    }
    let n: i64 = n.parse().map_err(|_| bad())?;
    if n < 1 {
        return Err(bad());
    }
    Ok((key, n))
}

/// Resolve `KEY-N` to an item.
pub fn item(conn: &Connection, reference: &str) -> Result<ItemRef> {
    let (key, seq) = parse_item_key(reference)?;
    let project_id = project_id(conn, &key)?;
    let id = conn
        .query_row(
            "SELECT id FROM items WHERE project_id = ?1 AND seq = ?2",
            params![project_id, seq],
            |r| r.get(0),
        )
        .optional()?
        .ok_or_else(|| err(format!("no such item: {key}-{seq}")))?;
    Ok(ItemRef { id, project_id })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn item_keys() {
        assert_eq!(parse_item_key("apm-12").unwrap(), ("APM".to_string(), 12));
        assert_eq!(item_key("APM", 12), "APM-12");
        assert!(parse_item_key("MY-APP-3").is_err());
        assert!(parse_item_key("APM-+3").is_err());
        assert!(parse_item_key("APM").is_err());
        assert!(parse_item_key("APM-0").is_err());
        assert!(parse_item_key("-3").is_err());
    }

    #[test]
    fn project_keys() {
        assert_eq!(check_project_key("apm2").unwrap(), "APM2");
        assert!(check_project_key("").is_err());
        assert!(check_project_key("2APM").is_err());
        assert!(check_project_key("A-B").is_err());
        assert!(check_project_key("ABCDEFGHIJK").is_err());
    }
}
