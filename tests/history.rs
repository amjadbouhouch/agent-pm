//! Task history: every mutation records rows in `events`. There is no `apm history` command
//! yet, so these tests read the table directly.
mod common;

use common::Env;

/// One events row: (actor, kind, field, old, new).
type Event = (
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
);

fn ev(actor: &str, kind: &str, field: Option<&str>, old: Option<&str>, new: Option<&str>) -> Event {
    let s = |v: Option<&str>| v.map(str::to_string);
    (
        actor.to_string(),
        kind.to_string(),
        s(field),
        s(old),
        s(new),
    )
}

/// The events of task `KEY-seq`, oldest first.
fn events(env: &Env, key: &str, seq: i64) -> Vec<Event> {
    let conn = env.conn();
    let mut stmt = conn
        .prepare(
            "SELECT e.actor, e.kind, e.field, e.old, e.new FROM events e
             JOIN items i ON i.id = e.item_id JOIN projects p ON p.id = i.project_id
             WHERE p.key = ?1 AND i.seq = ?2 ORDER BY e.id",
        )
        .unwrap();
    stmt.query_map(rusqlite::params![key, seq], |r| {
        Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
    })
    .unwrap()
    .collect::<rusqlite::Result<_>>()
    .unwrap()
}

fn env_apm() -> Env {
    let env = Env::new();
    env.json(&["project", "add", "APM", "Agent PM"]);
    env.json(&["milestone", "add", "v1", "-p", "APM"]);
    env.json(&["attr", "add", "points", "-p", "APM", "--type", "number"]);
    env.json(&["attr", "add", "note", "-p", "APM", "--type", "text"]);
    env
}

#[test]
fn every_mutation_records_events() {
    let env = env_apm();
    env.json(&["add", "parent", "-p", "APM"]);
    env.json(&[
        "--as", "alice", "add", "task", "-p", "APM", "--set", "points=2",
    ]);
    env.json(&[
        "--as",
        "bob",
        "edit",
        "APM-2",
        "--title",
        "renamed",
        "--status",
        "doing",
        "-a",
        "carol",
        "-m",
        "v1",
        "--parent",
        "APM-1",
        "--due",
        "2026-12-01",
        "--set",
        "points=3",
        "--set",
        "note=hi",
    ]);
    env.json(&["edit", "APM-2", "--unset", "points", "--unset", "due"]);
    env.json(&["done", "APM-2"]);
    env.json(&["--as", "dave", "comment", "APM-2", "looks good"]);
    let tester = "tester";
    assert_eq!(
        events(&env, "APM", 2),
        [
            ev("alice", "created", None, None, Some("task")),
            ev(
                "bob",
                "changed",
                Some("title"),
                Some("task"),
                Some("renamed")
            ),
            ev(
                "bob",
                "changed",
                Some("status"),
                Some("todo"),
                Some("doing")
            ),
            ev("bob", "changed", Some("assignee"), None, Some("carol")),
            ev("bob", "changed", Some("due"), None, Some("2026-12-01")),
            ev("bob", "changed", Some("milestone"), None, Some("v1")),
            ev("bob", "changed", Some("parent"), None, Some("APM-1")),
            // Attributes in name order.
            ev("bob", "attr", Some("note"), None, Some("hi")),
            ev("bob", "attr", Some("points"), Some("2"), Some("3")),
            ev(tester, "changed", Some("due"), Some("2026-12-01"), None),
            ev(tester, "attr", Some("points"), Some("3"), None),
            ev(
                tester,
                "changed",
                Some("status"),
                Some("doing"),
                Some("done")
            ),
            ev("dave", "commented", None, None, Some("looks good")),
        ]
    );
    // The event time is a UTC timestamp.
    let at: String = env
        .conn()
        .query_row("SELECT at FROM events LIMIT 1", [], |r| r.get(0))
        .unwrap();
    assert!(at.ends_with('Z'), "{at}");
}

#[test]
fn unchanged_values_record_nothing() {
    let env = env_apm();
    env.json(&["add", "t", "-p", "APM", "--status", "doing"]);
    env.json(&["edit", "APM-1", "--title", "t", "--status", "DOING"]);
    env.json(&["start", "APM-1"]);
    assert_eq!(
        events(&env, "APM", 1),
        [ev("tester", "created", None, None, Some("t"))]
    );
}

#[test]
fn failed_mutations_record_nothing() {
    let env = env_apm();
    env.json(&["add", "t", "-p", "APM"]);
    env.json_err(&["edit", "APM-1", "--title", "x", "--set", "points=abc"]);
    env.json_err(&["add", "u", "-p", "APM", "--set", "points=abc"]);
    let n: i64 = env
        .conn()
        .query_row("SELECT count(*) FROM events", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 1);
}

#[test]
fn status_rm_records_moved_tasks() {
    let env = env_apm();
    env.json(&["add", "t", "-p", "APM", "--status", "blocked"]);
    env.json(&[
        "--as", "erin", "status", "rm", "blocked", "-p", "APM", "--to", "doing",
    ]);
    assert_eq!(
        events(&env, "APM", 1)[1],
        ev(
            "erin",
            "changed",
            Some("status"),
            Some("blocked"),
            Some("doing")
        )
    );
}

#[test]
fn milestone_rm_records_cleared_tasks() {
    let env = env_apm();
    env.json(&["add", "a", "-p", "APM", "-m", "v1"]);
    env.json(&["add", "b", "-p", "APM"]);
    env.json(&["--as", "frank", "milestone", "rm", "v1", "-p", "APM"]);
    assert_eq!(
        events(&env, "APM", 1)[1],
        ev("frank", "changed", Some("milestone"), Some("v1"), None)
    );
    assert_eq!(events(&env, "APM", 2).len(), 1);
    assert_eq!(
        env.json(&["show", "APM-1"])["milestone"],
        serde_json::Value::Null
    );
}

#[test]
fn attr_events_hold_stored_values() {
    let env = env_apm();
    env.json(&["attr", "add", "flag", "-p", "APM", "--type", "checkbox"]);
    env.json(&["add", "t", "-p", "APM"]);
    env.json(&["edit", "APM-1", "--set", "flag=yes", "--set", "points=2.50"]);
    assert_eq!(
        events(&env, "APM", 1)[1..],
        [
            ev("tester", "attr", Some("flag"), None, Some("true")),
            ev("tester", "attr", Some("points"), None, Some("2.5")),
        ]
    );
}

#[test]
fn events_go_with_their_task() {
    let env = env_apm();
    env.json(&["add", "a", "-p", "APM"]);
    env.json(&["add", "b", "--parent", "APM-1"]);
    env.json(&["comment", "APM-2", "x"]);
    env.json(&["rm", "APM-1"]);
    let n: i64 = env
        .conn()
        .query_row("SELECT count(*) FROM events", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 0);
}
