//! The SQLite file: where it lives, its schema, and the lookups every command group shares.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rusqlite::{
    Connection, ErrorCode, OptionalExtension, Transaction, TransactionBehavior, params,
};

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
    // The timeout first: switching to WAL and migrating both wait for other writers.
    conn.busy_timeout(Duration::from_secs(5))?;
    enable_wal(&conn)?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    migrate(&mut conn)?;
    add_functions(&conn)?;
    Ok(conn)
}

/// Switch to WAL (a no-op once the file is WAL; the mode persists). On a new or
/// rollback-journal file the switch needs an exclusive lock, and SQLite fails at once with
/// "database is locked" instead of waiting (busy_timeout does not cover it) when another
/// process opening the same file holds a lock. Retry that for up to about 2 seconds.
fn enable_wal(conn: &Connection) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut pause = Duration::from_millis(5);
    loop {
        match conn.pragma_update(None, "journal_mode", "WAL") {
            Err(rusqlite::Error::SqliteFailure(e, _))
                if matches!(e.code, ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked)
                    && Instant::now() < deadline =>
            {
                std::thread::sleep(pause);
                pause = (pause * 2).min(Duration::from_millis(100));
            }
            res => return Ok(res?),
        }
    }
}

/// SQL functions apm's queries use: `fold(text)`, Unicode lowercase (SQLite's `lower` and
/// `LIKE` fold ASCII only). NULL stays NULL.
fn add_functions(conn: &Connection) -> Result<()> {
    use rusqlite::functions::FunctionFlags;
    conn.create_scalar_function(
        "fold",
        1,
        FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC,
        |ctx| Ok(ctx.get::<Option<String>>(0)?.map(|s| s.to_lowercase())),
    )?;
    Ok(())
}

/// Schema migrations, applied in order. `PRAGMA user_version` = number applied.
/// Never edit a released entry; append a new one.
const MIGRATIONS: &[&str] = &[
    r#"
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
"#,
    // v2: per-project statuses (items.status text -> status_id), the `datetime` attribute type,
    // history, links, tags, aliases.
    // Rebuilds `items` and `attributes` (SQLite cannot alter a CHECK): `migrate` turns foreign keys off around
    // it and runs `foreign_key_check` before commit, per SQLite's 12-step ALTER procedure.
    r#"
CREATE TABLE statuses (
    id         INTEGER PRIMARY KEY,
    project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    name       TEXT NOT NULL,
    category   TEXT NOT NULL CHECK (category IN ('todo', 'active', 'done', 'cancelled')),
    position   INTEGER NOT NULL,
    UNIQUE (project_id, name COLLATE NOCASE)
);
INSERT INTO statuses (project_id, name, category, position)
    SELECT p.id, d.column1, d.column2, d.column3
    FROM projects p,
         (VALUES ('todo', 'todo', 1), ('doing', 'active', 2), ('blocked', 'active', 3),
                 ('done', 'done', 4), ('cancelled', 'cancelled', 5)) d;

CREATE TABLE new_items (
    id           INTEGER PRIMARY KEY,
    project_id   INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    seq          INTEGER NOT NULL,
    parent_id    INTEGER REFERENCES new_items(id) ON DELETE CASCADE,
    milestone_id INTEGER REFERENCES milestones(id) ON DELETE SET NULL,
    title        TEXT NOT NULL,
    body         TEXT NOT NULL DEFAULT '',
    status_id    INTEGER NOT NULL REFERENCES statuses(id),
    priority     TEXT NOT NULL DEFAULT 'none'
                 CHECK (priority IN ('none', 'low', 'medium', 'high', 'urgent')),
    assignee     TEXT,
    due          TEXT,
    created_by   TEXT NOT NULL,
    created_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    updated_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    UNIQUE (project_id, seq)
);
INSERT INTO new_items (id, project_id, seq, parent_id, milestone_id, title, body, status_id,
                       priority, assignee, due, created_by, created_at, updated_at)
    SELECT i.id, i.project_id, i.seq, i.parent_id, i.milestone_id, i.title, i.body,
           (SELECT s.id FROM statuses s WHERE s.project_id = i.project_id AND s.name = i.status),
           i.priority, i.assignee, i.due, i.created_by, i.created_at, i.updated_at
    FROM items i;
DROP TABLE items;
ALTER TABLE new_items RENAME TO items;
CREATE INDEX items_parent ON items(parent_id);
CREATE INDEX items_milestone ON items(milestone_id);
CREATE INDEX items_status ON items(status_id);

-- Same rebuild for `attributes`, to allow the `datetime` type.
CREATE TABLE new_attributes (
    id         INTEGER PRIMARY KEY,
    project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    name       TEXT NOT NULL,
    type       TEXT NOT NULL
               CHECK (type IN ('text', 'number', 'select', 'date', 'datetime', 'checkbox')),
    options    TEXT NOT NULL DEFAULT '[]',
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    UNIQUE (project_id, name)
);
INSERT INTO new_attributes (id, project_id, name, type, options, created_at)
    SELECT id, project_id, name, type, options, created_at FROM attributes;
DROP TABLE attributes;
ALTER TABLE new_attributes RENAME TO attributes;

CREATE TABLE events (
    id      INTEGER PRIMARY KEY,
    item_id INTEGER NOT NULL REFERENCES items(id) ON DELETE CASCADE,
    actor   TEXT NOT NULL,
    at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    kind    TEXT NOT NULL,
    field   TEXT,
    old     TEXT,
    new     TEXT
);
CREATE INDEX events_item ON events(item_id);

CREATE TABLE links (
    from_id INTEGER NOT NULL REFERENCES items(id) ON DELETE CASCADE,
    to_id   INTEGER NOT NULL REFERENCES items(id) ON DELETE CASCADE,
    kind    TEXT NOT NULL CHECK (kind IN ('blocks', 'relates', 'duplicates')),
    PRIMARY KEY (from_id, to_id, kind),
    CHECK (from_id != to_id)
);
CREATE INDEX links_to ON links(to_id);

CREATE TABLE tags (
    item_id INTEGER NOT NULL REFERENCES items(id) ON DELETE CASCADE,
    tag     TEXT NOT NULL CHECK (tag = lower(tag)),
    PRIMARY KEY (item_id, tag)
);
CREATE INDEX tags_tag ON tags(tag);

CREATE TABLE project_aliases (
    key        TEXT PRIMARY KEY,
    project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE
);
CREATE INDEX project_aliases_project ON project_aliases(project_id);

CREATE TABLE item_aliases (
    project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    seq        INTEGER NOT NULL,
    item_id    INTEGER NOT NULL REFERENCES items(id) ON DELETE CASCADE,
    PRIMARY KEY (project_id, seq)
);
CREATE INDEX item_aliases_item ON item_aliases(item_id);
"#,
];

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
    if current == MIGRATIONS.len() {
        return Ok(());
    }
    // Table rebuilds drop and recreate parents of foreign keys: with enforcement on, the DROP
    // would cascade-delete their children. The pragma is a no-op inside a transaction, so it
    // is switched here and every migration checks the keys itself before committing.
    conn.pragma_update(None, "foreign_keys", "OFF")?;
    let res = apply(conn);
    conn.pragma_update(None, "foreign_keys", "ON")?;
    res
}

/// Apply pending migrations one per transaction. Each takes the write lock first and re-reads
/// the version inside it, so when several agents open an old database at once, one migrates
/// and the others find the work done.
fn apply(conn: &mut Connection) -> Result<()> {
    loop {
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let i = schema_version(&tx)? as usize;
        let Some(sql) = MIGRATIONS.get(i) else {
            return Ok(());
        };
        tx.execute_batch(sql)?;
        let broken: bool = tx.query_row(
            "SELECT count(*) > 0 FROM pragma_foreign_key_check",
            [],
            |r| r.get(0),
        )?;
        if broken {
            return Err(err(format!(
                "migration to schema v{} broke foreign keys",
                i + 1
            )));
        }
        tx.pragma_update(None, "user_version", (i + 1) as i64)?;
        tx.commit()?;
    }
}

/// A transaction that takes the write lock up front, so a read-then-write cannot fail with
/// SQLITE_BUSY_SNAPSHOT when another agent commits in between (busy_timeout does not retry that).
pub fn write_tx(conn: &Connection) -> Result<Transaction<'_>> {
    Ok(Transaction::new_unchecked(
        conn,
        TransactionBehavior::Immediate,
    )?)
}

/// What a history event records; stored as `events.kind`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventKind {
    /// The task was created; new = title.
    Created,
    /// A built-in field changed; field = its name, old/new as shown (status name, milestone
    /// name, parent KEY-N), None = empty.
    Changed,
    /// A custom attribute changed; field = attribute name, old/new = stored values,
    /// None = unset.
    Attr,
    /// A comment was added; new = its text.
    Commented,
    /// The task moved to another project; old/new = its KEY-N before and after.
    Moved,
    /// A link was added; field = the link as seen from this task (`blocks`, `blocked_by`,
    /// `relates`, `duplicates`, `duplicated_by`), new = the other task's KEY-N.
    Linked,
    /// A link was removed; field as for `Linked`, old = the other task's KEY-N.
    Unlinked,
    /// A tag was added; new = the tag.
    Tagged,
    /// A tag was removed; old = the tag.
    Untagged,
}

impl EventKind {
    pub fn as_str(self) -> &'static str {
        match self {
            EventKind::Created => "created",
            EventKind::Changed => "changed",
            EventKind::Attr => "attr",
            EventKind::Commented => "commented",
            EventKind::Moved => "moved",
            EventKind::Linked => "linked",
            EventKind::Unlinked => "unlinked",
            EventKind::Tagged => "tagged",
            EventKind::Untagged => "untagged",
        }
    }
}

/// Record one history event on a task. Call it inside the mutation's transaction.
pub fn record_event(
    conn: &Connection,
    item_id: i64,
    actor: &str,
    kind: EventKind,
    field: Option<&str>,
    old: Option<&str>,
    new: Option<&str>,
) -> Result<()> {
    conn.execute(
        "INSERT INTO events (item_id, actor, kind, field, old, new)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![item_id, actor, kind.as_str(), field, old, new],
    )?;
    Ok(())
}

/// Project id for a key, case-insensitive (`apm` finds `APM`). A key the project had before a
/// rename resolves through `project_aliases`. A live key and an alias never share a name:
/// creating or renaming to a key deletes the alias of that name.
pub fn project_id(conn: &Connection, key: &str) -> Result<i64> {
    let key = key.to_uppercase();
    conn.query_row(
        "SELECT id FROM projects WHERE key = ?1
         UNION ALL SELECT project_id FROM project_aliases WHERE key = ?1
         LIMIT 1",
        params![key],
        |r| r.get(0),
    )
    .optional()?
    .ok_or_else(|| err(format!("no such project: {key}")))
}

/// Make `key` a live key: drop the alias of that name, if any (a live key always wins).
/// Call it in the transaction that creates or re-keys `project_id` (None for a new project) to
/// `key`. Returns the current key of the other project the alias belonged to: its old ids
/// under `key` stop resolving, and the caller should say so.
pub fn claim_project_key(
    conn: &Connection,
    key: &str,
    project_id: Option<i64>,
) -> Result<Option<String>> {
    let owner = conn
        .query_row(
            "DELETE FROM project_aliases WHERE key = ?1 RETURNING project_id",
            params![key],
            |r| r.get::<_, i64>(0),
        )
        .optional()?
        .filter(|&owner| Some(owner) != project_id);
    let Some(owner) = owner else {
        return Ok(None);
    };
    Ok(Some(conn.query_row(
        "SELECT key FROM projects WHERE id = ?1",
        params![owner],
        |r| r.get(0),
    )?))
}

/// Take the next task number of a project. Call it inside the write transaction that uses it.
pub fn next_seq(conn: &Connection, project_id: i64) -> Result<i64> {
    Ok(conn.query_row(
        "UPDATE projects SET next_seq = next_seq + 1 WHERE id = ?1 RETURNING next_seq - 1",
        params![project_id],
        |r| r.get(0),
    )?)
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

/// Resolve `KEY-N` to an item. The key may be an alias, and `N` a number the task had before
/// it moved (`item_aliases`); either way the result is the task where it is now. Numbers are
/// never reused, so a live number and an alias never collide.
pub fn item(conn: &Connection, reference: &str) -> Result<ItemRef> {
    let (key, seq) = parse_item_key(reference)?;
    let project_id = project_id(conn, &key)?;
    conn.query_row(
        "SELECT id, project_id FROM items WHERE project_id = ?1 AND seq = ?2
         UNION ALL
         SELECT i.id, i.project_id FROM item_aliases a JOIN items i ON i.id = a.item_id
         WHERE a.project_id = ?1 AND a.seq = ?2
         LIMIT 1",
        params![project_id, seq],
        |r| {
            Ok(ItemRef {
                id: r.get(0)?,
                project_id: r.get(1)?,
            })
        },
    )
    .optional()?
    .ok_or_else(|| err(format!("no such item: {}", item_key(&key, seq))))
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
