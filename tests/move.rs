mod common;

use common::Env;
use serde_json::{Value, json};

/// Text-mode stdout of a successful run.
fn text(env: &Env, args: &[&str]) -> String {
    let out = env.apm().args(args).assert().success();
    String::from_utf8(out.get_output().stdout.clone()).unwrap()
}

fn count(env: &Env, sql: &str) -> i64 {
    env.conn().query_row(sql, [], |r| r.get(0)).unwrap()
}

/// One events row: (kind, field, old, new).
type Event = (String, Option<String>, Option<String>, Option<String>);

/// The events of a task, oldest first, found by its current KEY-N.
fn events(env: &Env, key: &str) -> Vec<Event> {
    let id = env.json(&["show", key])["id"].as_str().unwrap().to_string();
    let (project, seq) = id.split_once('-').unwrap();
    let conn = env.conn();
    let mut stmt = conn
        .prepare(
            "SELECT e.kind, e.field, e.old, e.new FROM events e
             JOIN items i ON i.id = e.item_id JOIN projects p ON p.id = i.project_id
             WHERE p.key = ?1 AND i.seq = ?2 ORDER BY e.id",
        )
        .unwrap();
    stmt.query_map(rusqlite::params![project, seq], |r| {
        Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
    })
    .unwrap()
    .collect::<rusqlite::Result<_>>()
    .unwrap()
}

fn ev(kind: &str, field: Option<&str>, old: Option<&str>, new: Option<&str>) -> Event {
    let s = |v: Option<&str>| v.map(str::to_string);
    (kind.to_string(), s(field), s(old), s(new))
}

/// Two projects:
/// - APM: APM-1 "parent", then APM-2 "root" under it (milestone v1, status doing, points=2,
///   size=m), APM-3 "child" under APM-2 (status review), APM-4 "grandchild" under APM-3
///   (milestone v1), and APM-5 "other".
/// - OPS: OPS-1 "existing"; milestone v1; status `Doing` (doing, renamed), no `review`;
///   `points` is a number as in APM, `size` is text (another type).
fn seeded() -> Env {
    let env = Env::new();
    for (key, name) in [("APM", "agent-pm"), ("OPS", "ops")] {
        env.json(&["project", "add", key, name]);
        env.json(&["milestone", "add", "v1", "-p", key]);
        env.json(&["attr", "add", "points", "-p", key, "--type", "number"]);
    }
    env.json(&[
        "attr",
        "add",
        "size",
        "-p",
        "APM",
        "--type",
        "select",
        "--options",
        "s,m,l",
    ]);
    env.json(&["attr", "add", "size", "-p", "OPS", "--type", "text"]);
    env.json(&[
        "status",
        "add",
        "review",
        "-p",
        "APM",
        "--category",
        "active",
    ]);
    env.json(&["status", "edit", "doing", "-p", "OPS", "--rename", "Doing"]);
    env.json(&["add", "existing", "-p", "OPS"]);
    env.json(&["add", "parent", "-p", "APM"]);
    env.json(&[
        "add", "root", "--parent", "APM-1", "-m", "v1", "--status", "doing", "--set", "points=2",
        "--set", "size=m",
    ]);
    env.json(&["add", "child", "--parent", "APM-2", "--status", "review"]);
    env.json(&["add", "grandchild", "--parent", "APM-3", "-m", "v1"]);
    env.json(&["add", "other", "-p", "APM"]);
    env.json(&["comment", "APM-2", "keep me"]);
    env
}

#[test]
fn move_renumbers_the_task_and_its_subtasks() {
    let env = seeded();
    let v = env.json(&["move", "apm-2", "ops"]);
    assert_eq!(v["id"], "OPS-2");
    assert_eq!(v["project"], "OPS");
    assert_eq!(v["title"], "root");
    // Parent stays behind: the moved task is top-level now.
    assert_eq!(v["parent"], Value::Null);
    assert_eq!(v["milestone"], "v1");
    assert_eq!(v["status"], "Doing");
    assert_eq!(v["attributes"], json!({"points": 2}));
    assert_eq!(
        v["renumbered"],
        json!({"APM-2": "OPS-2", "APM-3": "OPS-3", "APM-4": "OPS-4"})
    );
    assert_eq!(v["dropped"], json!({"OPS-2": {"size": "m"}}));

    let child = env.json(&["show", "OPS-3"]);
    assert_eq!(child["parent"], "OPS-2");
    // `review` doesn't exist in OPS: the target's default status.
    assert_eq!(child["status"], "todo");
    assert_eq!(child["subtasks"][0]["id"], "OPS-4");
    assert_eq!(env.json(&["show", "OPS-4"])["milestone"], "v1");
    assert_eq!(
        env.json(&["show", "OPS-2"])["comments"][0]["body"],
        "keep me"
    );
    // APM keeps the rest; numbers are not reused.
    let left: Vec<Value> = env
        .json(&["list", "-p", "APM", "--all"])
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].clone())
        .collect();
    assert_eq!(left, ["APM-1", "APM-5"]);
    assert_eq!(env.json(&["show", "APM-1"])["subtasks"], json!([]));
    assert_eq!(env.json(&["add", "new", "-p", "APM"])["id"], "APM-6");
    assert_eq!(env.json(&["add", "new", "-p", "OPS"])["id"], "OPS-5");
}

#[test]
fn renumbered_keeps_the_move_order() {
    let env = seeded();
    let out = env
        .apm()
        .args(["--json", "move", "APM-2", "OPS"])
        .assert()
        .success();
    let raw = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    let at = |k: &str| raw.find(&format!("\"{k}\": \"OPS")).unwrap();
    assert!(
        at("APM-2") < at("APM-3") && at("APM-3") < at("APM-4"),
        "{raw}"
    );
}

#[test]
fn old_ids_keep_resolving() {
    let env = seeded();
    env.json(&["move", "APM-2", "OPS"]);
    assert_eq!(env.json(&["show", "APM-2"])["id"], "OPS-2");
    assert_eq!(
        env.json(&["edit", "apm-3", "--title", "kid"])["id"],
        "OPS-3"
    );
    assert_eq!(env.json(&["comment", "APM-4", "hi"])["item"], "OPS-4");
    assert_eq!(env.json(&["done", "APM-4"])["status"], "done");
    assert_eq!(env.json(&["list", "--parent", "APM-2"])[0]["id"], "OPS-3");
    // A new subtask under an old id lands in the task's current project.
    assert_eq!(env.json(&["add", "x", "--parent", "APM-2"])["id"], "OPS-5");
    assert_eq!(env.json(&["rm", "APM-4"])["id"], "OPS-4");
    assert_eq!(env.json_err(&["show", "APM-4"]), "no such item: APM-4");
    assert_eq!(env.json_err(&["show", "APM-9"]), "no such item: APM-9");
}

#[test]
fn moving_twice_and_back_chains_aliases() {
    let env = seeded();
    env.json(&["project", "add", "WEB", "web"]);
    env.json(&["move", "APM-5", "OPS"]);
    // Move it again through its first old id.
    let v = env.json(&["move", "APM-5", "WEB"]);
    assert_eq!(v["id"], "WEB-1");
    assert_eq!(v["renumbered"], json!({"OPS-2": "WEB-1"}));
    for old in ["APM-5", "OPS-2", "WEB-1"] {
        assert_eq!(env.json(&["show", old])["id"], "WEB-1", "{old}");
    }
    // Back home: a new number, never the old one.
    assert_eq!(env.json(&["move", "WEB-1", "APM"])["id"], "APM-6");
    for old in ["APM-5", "OPS-2", "WEB-1", "APM-6"] {
        assert_eq!(env.json(&["show", old])["id"], "APM-6", "{old}");
    }
    assert_eq!(
        events(&env, "APM-6")
            .into_iter()
            .filter(|e| e.0 == "moved")
            .collect::<Vec<_>>(),
        [
            ev("moved", None, Some("APM-5"), Some("OPS-2")),
            ev("moved", None, Some("OPS-2"), Some("WEB-1")),
            ev("moved", None, Some("WEB-1"), Some("APM-6")),
        ]
    );
}

#[test]
fn move_keeps_tags_links_comments_and_history() {
    let env = seeded();
    // B2 builds the tag and link commands; seed the rows directly.
    env.conn()
        .execute_batch(
            "INSERT INTO tags (item_id, tag) SELECT id, 'ui' FROM items WHERE title = 'root';
             INSERT INTO links (from_id, to_id, kind)
                 SELECT a.id, b.id, 'blocks' FROM items a, items b
                 WHERE a.title = 'root' AND b.title = 'other';",
        )
        .unwrap();
    env.json(&["move", "APM-2", "OPS"]);
    let tags = count(
        &env,
        "SELECT count(*) FROM tags t JOIN items i ON i.id = t.item_id
         WHERE i.title = 'root' AND t.tag = 'ui'",
    );
    assert_eq!(tags, 1);
    assert_eq!(count(&env, "SELECT count(*) FROM links"), 1);
    assert_eq!(
        events(&env, "OPS-2"),
        [
            ev("created", None, None, Some("root")),
            ev("commented", None, None, Some("keep me")),
            ev("moved", None, Some("APM-2"), Some("OPS-2")),
            ev("changed", Some("status"), Some("doing"), Some("Doing")),
            ev("changed", Some("parent"), Some("APM-1"), None),
            ev("attr", Some("size"), Some("m"), None),
        ]
    );
    assert_eq!(
        events(&env, "OPS-3")[1..],
        [
            ev("moved", None, Some("APM-3"), Some("OPS-3")),
            ev("changed", Some("status"), Some("review"), Some("todo")),
        ]
    );
    // Same status name and milestone, parent moved along: only `moved`.
    assert_eq!(
        events(&env, "OPS-4")[1..],
        [ev("moved", None, Some("APM-4"), Some("OPS-4"))]
    );
}

#[test]
fn milestone_without_a_match_is_cleared() {
    let env = seeded();
    env.json(&["project", "add", "WEB", "web"]);
    let v = env.json(&["move", "APM-4", "WEB"]);
    assert_eq!(v["milestone"], Value::Null);
    assert_eq!(v["dropped"], json!({}));
    assert!(events(&env, "WEB-1").contains(&ev("changed", Some("milestone"), Some("v1"), None)));
}

#[test]
fn select_values_need_the_option_in_the_target() {
    let env = Env::new();
    for key in ["APM", "OPS"] {
        env.json(&["project", "add", key, "x"]);
    }
    env.json(&[
        "attr",
        "add",
        "size",
        "-p",
        "APM",
        "--type",
        "select",
        "--options",
        "s,xl",
    ]);
    env.json(&[
        "attr",
        "add",
        "size",
        "-p",
        "OPS",
        "--type",
        "select",
        "--options",
        "s,m",
    ]);
    env.json(&["add", "a", "-p", "APM", "--set", "size=s"]);
    env.json(&["add", "b", "-p", "APM", "--set", "size=xl"]);
    assert_eq!(
        env.json(&["move", "APM-1", "OPS"])["attributes"],
        json!({"size": "s"})
    );
    assert_eq!(
        env.json(&["move", "APM-2", "OPS"])["dropped"],
        json!({"OPS-2": {"size": "xl"}})
    );
    assert_eq!(count(&env, "SELECT count(*) FROM item_values"), 1);
}

#[test]
fn move_text_lists_every_dropped_value() {
    let env = Env::new();
    for key in ["APM", "OPS"] {
        env.json(&["project", "add", key, "x"]);
    }
    env.json(&["attr", "add", "size", "-p", "APM", "--type", "text"]);
    env.json(&["attr", "add", "pts", "-p", "APM", "--type", "number"]);
    env.json(&["add", "a", "-p", "APM", "--set", "size=m"]);
    env.json(&["add", "b", "--parent", "APM-1", "--set", "pts=3"]);
    assert_eq!(
        text(&env, &["move", "APM-1", "OPS"]),
        "moved APM-1 -> OPS-1 (1 subtask(s); dropped OPS-1 size=m, OPS-2 pts=3)\n"
    );
}

#[test]
fn move_text_output() {
    let env = seeded();
    assert_eq!(
        text(&env, &["move", "APM-2", "OPS"]),
        "moved APM-2 -> OPS-2 (2 subtask(s); dropped OPS-2 size=m)\n"
    );
    assert_eq!(
        text(&env, &["move", "APM-5", "OPS"]),
        "moved APM-5 -> OPS-5\n"
    );
}

#[test]
fn move_errors() {
    let env = seeded();
    assert_eq!(
        env.json_err(&["move", "APM-2", "apm"]),
        "APM-2 is already in project APM"
    );
    assert_eq!(
        env.json_err(&["move", "APM-2", "NOPE"]),
        "no such project: NOPE"
    );
    assert_eq!(
        env.json_err(&["move", "APM-9", "OPS"]),
        "no such item: APM-9"
    );
    assert!(
        env.json_err(&["move", "nope", "OPS"])
            .contains("invalid item reference")
    );
    // A status that doesn't map needs a todo status in the target; without one nothing moves.
    env.json(&["status", "rm", "todo", "-p", "OPS", "--to", "Doing"]);
    assert_eq!(
        env.json_err(&["move", "APM-2", "OPS"]),
        "project OPS has no todo-category status"
    );
    assert_eq!(env.json(&["show", "APM-2"])["id"], "APM-2");
    assert_eq!(env.json(&["show", "APM-3"])["id"], "APM-3");
    assert_eq!(count(&env, "SELECT count(*) FROM item_aliases"), 0);
    // The failed move used no numbers.
    assert_eq!(
        env.json(&["add", "n", "-p", "OPS", "--status", "doing"])["id"],
        "OPS-2"
    );
    env.json(&["status", "add", "todo", "-p", "OPS", "--category", "todo"]);
    // A move to where the task already is, through an old id, is still an error.
    env.json(&["move", "APM-5", "OPS"]);
    assert_eq!(
        env.json_err(&["move", "APM-5", "OPS"]),
        "OPS-3 is already in project OPS"
    );
}
