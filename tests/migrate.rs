//! Upgrading a database written by apm v1 (schema v1) to the current schema.
mod common;

use common::Env;
use serde_json::{Value, json};

/// Schema v1 exactly as released (the first entry of `MIGRATIONS` in src/db.rs).
const V1: &str = r#"
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
"#;

/// Data as apm v1 would have written it. Row ids are deliberately not 1..n, and APM-2 was
/// deleted (numbers are never reused), so the test can tell ids and numbers survive.
const V1_DATA: &str = r#"
INSERT INTO projects (id, key, name, description, next_seq, created_at, updated_at) VALUES
    (3, 'APM', 'agent-pm', 'the tracker', 7, '2026-01-01T00:00:00Z', '2026-01-02T00:00:00Z'),
    (8, 'WEB', 'web', '', 2, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z');
INSERT INTO milestones (id, project_id, name, due, status) VALUES
    (4, 3, 'v1', '2026-12-01', 'open');
INSERT INTO items (id, project_id, seq, parent_id, milestone_id, title, body, status, priority,
                   assignee, due, created_by, created_at, updated_at) VALUES
    (10, 3, 1, NULL, 4, 'root', 'the body', 'doing', 'high', 'alice', '2026-11-01', 'alice',
     '2026-01-03T00:00:00Z', '2026-01-04T00:00:00Z'),
    (11, 3, 3, 10, NULL, 'child', '', 'blocked', 'none', NULL, NULL, 'bob',
     '2026-01-03T00:00:00Z', '2026-01-03T00:00:00Z'),
    (12, 3, 4, 11, 4, 'grandchild', '', 'done', 'low', NULL, NULL, 'bob',
     '2026-01-03T00:00:00Z', '2026-01-03T00:00:00Z'),
    (13, 3, 5, NULL, 4, 'dropped', '', 'cancelled', 'none', NULL, NULL, 'bob',
     '2026-01-03T00:00:00Z', '2026-01-03T00:00:00Z'),
    (14, 3, 6, NULL, NULL, 'later', '', 'todo', 'urgent', NULL, NULL, 'carol',
     '2026-01-03T00:00:00Z', '2026-01-03T00:00:00Z'),
    (20, 8, 1, NULL, NULL, 'web task', '', 'done', 'none', NULL, NULL, 'dave',
     '2026-01-03T00:00:00Z', '2026-01-03T00:00:00Z');
INSERT INTO attributes (id, project_id, name, type, options) VALUES
    (2, 3, 'points', 'number', '[]'),
    (5, 3, 'size', 'select', '["s","m","l"]');
INSERT INTO item_values (item_id, attribute_id, value) VALUES
    (10, 2, '3'), (10, 5, 'm'), (11, 2, '1');
INSERT INTO comments (id, item_id, author, body, created_at) VALUES
    (7, 10, 'alice', 'first', '2026-01-05T00:00:00Z'),
    (9, 11, 'bob', 'second', '2026-01-06T00:00:00Z');
"#;

/// An env whose database is a v1 database holding `V1_DATA`. Not opened by apm yet.
fn v1_env() -> Env {
    let env = Env::new();
    let conn = rusqlite::Connection::open(env.db_path()).unwrap();
    // apm v1 switched every database to WAL on open, and the mode persists in the file.
    conn.pragma_update(None, "journal_mode", "WAL").unwrap();
    conn.execute_batch(V1).unwrap();
    conn.execute_batch(V1_DATA).unwrap();
    conn.pragma_update(None, "user_version", 1).unwrap();
    env
}

fn count(env: &Env, sql: &str) -> i64 {
    env.conn().query_row(sql, [], |r| r.get(0)).unwrap()
}

#[test]
fn v1_database_upgrades_to_v2() {
    let env = v1_env();
    assert_eq!(env.json(&["db"])["schema_version"], 3);

    // Every task keeps its row id, number, fields and status (now a row of its project).
    let conn = env.conn();
    let mut stmt = conn
        .prepare(
            "SELECT i.id, i.seq, s.name, s.category, s.project_id = i.project_id
             FROM items i JOIN statuses s ON s.id = i.status_id ORDER BY i.id",
        )
        .unwrap();
    let rows: Vec<(i64, i64, String, String, bool)> = stmt
        .query_map([], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    let s = |v: &str| v.to_string();
    assert_eq!(
        rows,
        [
            (10, 1, s("doing"), s("active"), true),
            (11, 3, s("blocked"), s("active"), true),
            (12, 4, s("done"), s("done"), true),
            (13, 5, s("cancelled"), s("cancelled"), true),
            (14, 6, s("todo"), s("todo"), true),
            (20, 1, s("done"), s("done"), true),
        ]
    );
    drop(stmt);
    drop(conn);

    let root = env.json(&["show", "APM-1"]);
    assert_eq!(root["title"], "root");
    assert_eq!(root["body"], "the body");
    assert_eq!(root["status"], "doing");
    assert_eq!(root["status_category"], "active");
    assert_eq!(root["priority"], "high");
    assert_eq!(root["assignee"], "alice");
    assert_eq!(root["due"], "2026-11-01");
    assert_eq!(root["milestone"], "v1");
    assert_eq!(root["created_by"], "alice");
    assert_eq!(root["created_at"], "2026-01-03T00:00:00Z");
    assert_eq!(root["updated_at"], "2026-01-04T00:00:00Z");
    assert_eq!(root["attributes"], json!({"points": 3, "size": "m"}));
    assert_eq!(root["subtasks"][0]["id"], "APM-3");
    assert_eq!(root["comments"][0]["body"], "first");
    assert_eq!(env.json(&["show", "APM-4"])["parent"], "APM-3");

    // Every project got the default statuses; counts and progress read them.
    for key in ["APM", "WEB"] {
        let names: Vec<Value> = env
            .json(&["status", "list", "-p", key])
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["name"].clone())
            .collect();
        assert_eq!(names, ["todo", "doing", "blocked", "done", "cancelled"]);
    }
    assert_eq!(
        env.json(&["project", "show", "APM"])["counts"],
        json!({"todo": 1, "doing": 1, "blocked": 1, "done": 1, "cancelled": 1})
    );
    // v1 holds APM-1 > (APM-3, no milestone) > APM-4, and APM-5: APM-1 counts through its
    // grandchild, so the units are APM-4 (done) and APM-5 (cancelled).
    assert_eq!(
        env.json(&["milestone", "show", "v1", "-p", "APM"])["progress"],
        json!({"total": 1, "done": 1, "cancelled": 1})
    );
    let open: Vec<Value> = env
        .json(&["list"])
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].clone())
        .collect();
    assert_eq!(open, ["APM-1", "APM-3", "APM-6"]);

    // Numbering continues where v1 left off.
    assert_eq!(env.json(&["add", "new", "-p", "APM"])["id"], "APM-7");
    assert_eq!(env.json(&["add", "new", "-p", "WEB"])["id"], "WEB-2");

    // The new tables exist and start empty (the add above recorded its events).
    for table in ["links", "tags", "project_aliases", "item_aliases"] {
        assert_eq!(count(&env, &format!("SELECT count(*) FROM {table}")), 0);
    }
}

#[test]
fn v1_attributes_survive_the_rebuild() {
    let env = v1_env();
    let attrs = env.json(&["attr", "list", "-p", "APM"]);
    assert_eq!(
        attrs,
        json!([
            {"name": "points", "project": "APM", "type": "number", "options": []},
            {"name": "size", "project": "APM", "type": "select", "options": ["s", "m", "l"]},
        ])
    );
    let conn = env.conn();
    let rows: Vec<(i64, i64, String)> = conn
        .prepare(
            "SELECT item_id, attribute_id, value FROM item_values ORDER BY item_id, attribute_id",
        )
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    let s = |v: &str| v.to_string();
    assert_eq!(rows, [(10, 2, s("3")), (10, 5, s("m")), (11, 2, s("1"))]);
    // Values still follow their attribute and their task.
    let fks: i64 = conn
        .query_row(
            "SELECT count(*) FROM pragma_foreign_key_list('item_values')
             WHERE \"table\" IN ('items', 'attributes') AND on_delete = 'CASCADE'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(fks, 2);
    // The schema now accepts the `datetime` type (B1 adds it to the CLI).
    conn.execute(
        "INSERT INTO attributes (project_id, name, type) VALUES (3, 'at', 'datetime')",
        [],
    )
    .unwrap();
    assert!(
        conn.execute(
            "INSERT INTO attributes (project_id, name, type) VALUES (3, 'x', 'time')",
            [],
        )
        .is_err()
    );
    // Tags are stored lowercase.
    assert!(
        conn.execute("INSERT INTO tags (item_id, tag) VALUES (10, 'Ui')", [])
            .is_err()
    );
    conn.execute("INSERT INTO tags (item_id, tag) VALUES (10, 'ui')", [])
        .unwrap();
    drop(conn);
    env.json(&["attr", "rm", "points", "-p", "APM"]);
    assert_eq!(count(&env, "SELECT count(*) FROM item_values"), 1);
}

#[test]
fn concurrent_first_opens_migrate_once() {
    let env = v1_env();
    let workers: Vec<_> = (0..8)
        .map(|_| {
            let db = env.db_path();
            std::thread::spawn(move || {
                let out = std::process::Command::new(env!("CARGO_BIN_EXE_apm"))
                    .env("APM_DB", &db)
                    .env("APM_AGENT", "tester")
                    .args(["--json", "db"])
                    .output()
                    .unwrap();
                let stderr = String::from_utf8_lossy(&out.stderr);
                assert!(out.status.success(), "{stderr}");
                let v: Value = serde_json::from_slice(&out.stdout).unwrap();
                assert_eq!(v["schema_version"], 3);
            })
        })
        .collect();
    for w in workers {
        w.join().unwrap();
    }
    assert_eq!(count(&env, "SELECT count(*) FROM statuses"), 10);
    assert_eq!(count(&env, "SELECT count(*) FROM items"), 6);
}

#[test]
fn v2_schema_keeps_foreign_keys_and_cascades() {
    let env = v1_env();
    env.json(&["db"]);
    let conn = env.conn();
    // The rebuilt table points at itself and its parents; its children point at it.
    let fk = |table: &str| -> Vec<(String, String, String)> {
        let mut stmt = conn
            .prepare(&format!(
                "SELECT \"from\", \"table\", on_delete FROM pragma_foreign_key_list('{table}')
                 ORDER BY \"from\""
            ))
            .unwrap();
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    };
    let t = |a: &str, b: &str, c: &str| (a.to_string(), b.to_string(), c.to_string());
    assert_eq!(
        fk("items"),
        [
            t("milestone_id", "milestones", "SET NULL"),
            t("parent_id", "items", "CASCADE"),
            t("project_id", "projects", "CASCADE"),
            t("status_id", "statuses", "NO ACTION"),
        ]
    );
    assert_eq!(fk("comments"), [t("item_id", "items", "CASCADE")]);
    assert_eq!(fk("events"), [t("item_id", "items", "CASCADE")]);
    let no_old_column: bool = conn
        .query_row(
            "SELECT count(*) = 0 FROM pragma_table_info('items') WHERE name = 'status'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(no_old_column);
    let indexes: i64 = conn
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type = 'index'
             AND name IN ('items_parent', 'items_milestone', 'items_status')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(indexes, 3);
    drop(conn);

    // Deleting a task cascades to its subtasks, values, comments and events.
    env.json(&["comment", "APM-4", "x"]);
    env.json(&["rm", "APM-1"]);
    assert_eq!(
        count(&env, "SELECT count(*) FROM items WHERE project_id = 3"),
        2
    );
    assert_eq!(count(&env, "SELECT count(*) FROM item_values"), 0);
    assert_eq!(count(&env, "SELECT count(*) FROM comments"), 0);
    assert_eq!(count(&env, "SELECT count(*) FROM events"), 0);
    // Deleting a milestone keeps its tasks.
    env.json(&["milestone", "rm", "v1", "-p", "APM"]);
    assert_eq!(env.json(&["show", "APM-5"])["milestone"], Value::Null);
    // Deleting a project takes its statuses and tasks, and leaves the other project alone.
    env.json(&["project", "rm", "APM"]);
    assert_eq!(count(&env, "SELECT count(*) FROM statuses"), 5);
    assert_eq!(count(&env, "SELECT count(*) FROM items"), 1);
    assert_eq!(
        count(&env, "SELECT count(*) FROM pragma_foreign_key_check"),
        0
    );
}

#[test]
fn a_newer_schema_is_refused() {
    let env = Env::new();
    let conn = rusqlite::Connection::open(env.db_path()).unwrap();
    conn.pragma_update(None, "user_version", 99).unwrap();
    drop(conn);
    assert_eq!(
        env.json_err(&["db"]),
        "database schema v99 is newer than this apm (v3); upgrade apm"
    );
}

/// A database as apm v0.2 left it (schema v2): today's schema without the v3 claim columns.
fn v2_env() -> Env {
    let env = Env::new();
    env.json(&["project", "add", "APM", "Agent PM"]);
    env.json(&["add", "first", "-p", "APM", "-a", "alice"]);
    env.json(&["add", "second", "-p", "APM", "--priority", "high"]);
    env.json(&["comment", "APM-1", "hello"]);
    let conn = env.conn();
    conn.execute_batch(
        "DROP INDEX items_claim_expires;
         ALTER TABLE items DROP COLUMN claim_expires;
         ALTER TABLE items DROP COLUMN claimed_at;
         ALTER TABLE items DROP COLUMN claimed_by;",
    )
    .unwrap();
    conn.pragma_update(None, "user_version", 2).unwrap();
    env
}

#[test]
fn v2_database_upgrades_to_v3() {
    let env = v2_env();
    let before = env.json(&["list"]);
    // Reading migrates; every task is as it was, with no claim.
    assert_eq!(env.json(&["db"])["schema_version"], 3);
    let after = env.json(&["list"]);
    assert_eq!(after, before);
    assert_eq!(after[0]["claim"], Value::Null);
    assert_eq!(
        count(
            &env,
            "SELECT count(*) FROM pragma_index_list('items') WHERE name = 'items_claim_expires'"
        ),
        1
    );
    // Claims work on the upgraded database.
    let v = env.json(&["next"]);
    assert_eq!(v["id"], "APM-2");
    assert_eq!(v["claim"]["by"], "tester");
    assert_eq!(env.json(&["show", "APM-1"])["comments"][0]["body"], "hello");
}
