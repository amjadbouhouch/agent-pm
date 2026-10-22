mod common;

use common::Env;
use serde_json::Value;

/// Text-mode stdout of a successful run.
fn text(env: &Env, args: &[&str]) -> String {
    let out = env.apm().args(args).assert().success();
    String::from_utf8(out.get_output().stdout.clone()).unwrap()
}

/// The `id`s of a JSON array of items.
fn ids(v: &Value) -> Vec<String> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].as_str().unwrap().to_string())
        .collect()
}

/// An env with project APM.
fn env_apm() -> Env {
    let env = Env::new();
    env.json(&["project", "add", "APM", "Agent PM"]);
    env
}

/// Create milestone `name` in `project`.
fn seed_milestone(env: &Env, project: &str, name: &str) {
    env.json(&["milestone", "add", name, "-p", project]);
}

// ---- add

#[test]
fn add_returns_the_item_with_defaults() {
    let env = env_apm();
    let i = env.json(&["add", "First task", "-p", "apm"]);
    assert_eq!(i["id"], "APM-1");
    assert_eq!(i["project"], "APM");
    assert_eq!(i["title"], "First task");
    assert_eq!(i["body"], "");
    assert_eq!(i["status"], "todo");
    assert_eq!(i["status_category"], "todo");
    assert_eq!(i["priority"], "none");
    assert_eq!(i["assignee"], Value::Null);
    assert_eq!(i["due"], Value::Null);
    assert_eq!(i["milestone"], Value::Null);
    assert_eq!(i["parent"], Value::Null);
    assert_eq!(i["created_by"], "tester");
    assert!(i["created_at"].as_str().unwrap().ends_with('Z'));
    assert_eq!(i["attributes"], serde_json::json!({}));
    assert!(i.get("row_id").is_none());
}

#[test]
fn add_sets_every_field() {
    let env = env_apm();
    seed_milestone(&env, "APM", "v1");
    let i = env.json(&[
        "--as",
        "bot",
        "add",
        "T",
        "-p",
        "APM",
        "-m",
        "v1",
        "-a",
        "alice",
        "--priority",
        "high",
        "--status",
        "doing",
        "--due",
        "2026-12-31",
        "-b",
        "details",
    ]);
    assert_eq!(i["milestone"], "v1");
    assert_eq!(i["assignee"], "alice");
    assert_eq!(i["priority"], "high");
    assert_eq!(i["status"], "doing");
    assert_eq!(i["due"], "2026-12-31");
    assert_eq!(i["body"], "details");
    assert_eq!(i["created_by"], "bot");
}

#[test]
fn add_numbers_per_project() {
    let env = env_apm();
    env.json(&["project", "add", "OPS", "Ops"]);
    assert_eq!(env.json(&["add", "a", "-p", "APM"])["id"], "APM-1");
    assert_eq!(env.json(&["add", "b", "-p", "APM"])["id"], "APM-2");
    assert_eq!(env.json(&["add", "c", "-p", "OPS"])["id"], "OPS-1");
    assert_eq!(text(&env, &["add", "d", "-p", "APM"]), "created APM-3\n");
}

#[test]
fn add_subtask_uses_the_parents_project() {
    let env = env_apm();
    env.json(&["add", "parent", "-p", "APM"]);
    let s = env.json(&["add", "child", "--parent", "apm-1"]);
    assert_eq!(s["id"], "APM-2");
    assert_eq!(s["parent"], "APM-1");
    let s = env.json(&["add", "grandchild", "--parent", "APM-2", "-p", "APM"]);
    assert_eq!(s["parent"], "APM-2");
}

#[test]
fn add_subtask_does_not_inherit_the_milestone() {
    let env = env_apm();
    seed_milestone(&env, "APM", "v1");
    env.json(&["add", "parent", "-p", "APM", "-m", "v1"]);
    assert_eq!(
        env.json(&["add", "child", "--parent", "APM-1"])["milestone"],
        Value::Null
    );
}

#[test]
fn add_errors() {
    let env = env_apm();
    env.json(&["project", "add", "OPS", "Ops"]);
    env.json(&["add", "a", "-p", "OPS"]);
    assert_eq!(
        env.json_err(&["add", "x", "-p", "NOPE"]),
        "no such project: NOPE"
    );
    assert!(env.json_err(&["add", "x"]).contains("project is required"));
    assert_eq!(
        env.json_err(&["add", "x", "-p", "APM", "--due", "2026-02-30"]),
        "invalid date `2026-02-30`, expected YYYY-MM-DD"
    );
    assert_eq!(
        env.json_err(&["add", "x", "-p", "APM", "-m", "v9"]),
        "no such milestone: v9"
    );
    assert_eq!(
        env.json_err(&["add", "", "-p", "APM"]),
        "title cannot be empty"
    );
    assert_eq!(
        env.json_err(&["add", "x", "--parent", "APM-7"]),
        "no such item: APM-7"
    );
    assert!(
        env.json_err(&["add", "x", "--parent", "APM"])
            .contains("invalid item reference")
    );
    assert_eq!(
        env.json_err(&["add", "x", "-p", "apm", "--parent", "ops-1"]),
        "parent OPS-1 is not in project APM"
    );
    // A failed add consumes no number.
    assert_eq!(env.json(&["add", "ok", "-p", "APM"])["id"], "APM-1");
}

#[test]
fn add_bad_enum_is_a_usage_error() {
    let env = env_apm();
    env.apm()
        .args(["--json", "add", "x", "-p", "APM", "--priority", "huge"])
        .assert()
        .code(2);
}

#[test]
fn add_with_set_rolls_back_when_attrs_fail() {
    let env = env_apm();
    // `attr::set_values` rejects the pair (unknown attribute / not built yet): nothing is written.
    env.json_err(&["add", "x", "-p", "APM", "--set", "size=L"]);
    assert_eq!(env.json(&["list", "--all"]), serde_json::json!([]));
    assert_eq!(env.json(&["add", "ok", "-p", "APM"])["id"], "APM-1");
}

// ---- list

#[test]
fn list_hides_closed_unless_all_or_status() {
    let env = env_apm();
    env.json(&["add", "open", "-p", "APM"]);
    env.json(&["add", "finished", "-p", "APM", "--status", "done"]);
    env.json(&["add", "dropped", "-p", "APM", "--status", "cancelled"]);
    env.json(&["add", "busy", "-p", "APM", "--status", "doing"]);
    assert_eq!(ids(&env.json(&["list"])), ["APM-1", "APM-4"]);
    assert_eq!(
        ids(&env.json(&["list", "--all"])),
        ["APM-1", "APM-2", "APM-3", "APM-4"]
    );
    assert_eq!(ids(&env.json(&["list", "--status", "done"])), ["APM-2"]);
    assert_eq!(
        ids(&env.json(&["list", "--status", "done,doing"])),
        ["APM-2", "APM-4"]
    );
}

#[test]
fn list_orders_by_project_then_number() {
    let env = Env::new();
    env.json(&["project", "add", "ZED", "z"]);
    env.json(&["project", "add", "APM", "a"]);
    env.json(&["add", "z1", "-p", "ZED"]);
    for t in ["a1", "a2", "a3", "a4", "a5", "a6", "a7", "a8", "a9", "a10"] {
        env.json(&["add", t, "-p", "APM"]);
    }
    let all = ids(&env.json(&["list"]));
    assert_eq!(all.first().unwrap(), "APM-1");
    assert_eq!(all[9], "APM-10");
    assert_eq!(all.last().unwrap(), "ZED-1");
    assert_eq!(ids(&env.json(&["list", "-p", "zed"])), ["ZED-1"]);
}

#[test]
fn list_filters() {
    let env = env_apm();
    seed_milestone(&env, "APM", "v1");
    env.json(&["add", "a", "-p", "APM", "-a", "alice", "-m", "v1"]);
    env.json(&["add", "b", "-p", "APM", "-a", "bob"]);
    env.json(&["add", "c", "--parent", "APM-1", "-a", "bob"]);
    assert_eq!(ids(&env.json(&["list", "-a", "bob"])), ["APM-2", "APM-3"]);
    assert_eq!(ids(&env.json(&["list", "--parent", "APM-1"])), ["APM-3"]);
    assert_eq!(ids(&env.json(&["list", "-m", "v1"])), ["APM-1"]);
    assert_eq!(
        ids(&env.json(&["list", "-p", "APM", "-m", "v1"])),
        ["APM-1"]
    );
    assert_eq!(
        ids(&env.json(&["list", "-a", "bob", "--parent", "APM-1"])),
        ["APM-3"]
    );
    assert_eq!(env.json(&["list", "-a", "nobody"]), serde_json::json!([]));
}

#[test]
fn list_errors() {
    let env = env_apm();
    assert_eq!(
        env.json_err(&["list", "-p", "NOPE"]),
        "no such project: NOPE"
    );
    assert_eq!(
        env.json_err(&["list", "--parent", "APM-9"]),
        "no such item: APM-9"
    );
    assert_eq!(
        env.json_err(&["list", "-p", "APM", "-m", "v9"]),
        "no such milestone: v9"
    );
    assert_eq!(
        env.json_err(&["list", "--status", "maybe"]),
        "no such status: maybe"
    );
    assert_eq!(
        env.json_err(&["list", "-p", "APM", "--status", "doing,maybe"]),
        "no such status: maybe"
    );
    assert_eq!(env.json_err(&["list", "-m", "v9"]), "no such milestone: v9");
    // --where goes through attr::filter, which rejects unknown attributes.
    env.json_err(&["list", "--where", "size=L"]);
}

#[test]
fn list_text_table() {
    let env = env_apm();
    assert_eq!(text(&env, &["list"]), "");
    env.json(&[
        "add",
        "parent",
        "-p",
        "APM",
        "-a",
        "alice",
        "--due",
        "2026-11-01",
    ]);
    env.json(&["add", "child", "--parent", "APM-1"]);
    let out = text(&env, &["list"]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 3);
    let headers: Vec<&str> = lines[0].split_whitespace().collect();
    assert_eq!(headers, ["ID", "STATUS", "PRI", "ASSIGNEE", "DUE", "TITLE"]);
    assert!(lines[1].starts_with("APM-1  todo"));
    assert!(lines[1].contains("alice"));
    assert!(lines[1].contains("2026-11-01"));
    assert!(lines[1].ends_with("parent"));
    assert!(lines[2].ends_with("APM-1 > child"));
}

// ---- show

#[test]
fn show_includes_subtasks_and_comments() {
    let env = env_apm();
    env.json(&["add", "parent", "-p", "APM", "-b", "the body"]);
    env.json(&["add", "child", "--parent", "APM-1", "--status", "done"]);
    env.json(&["add", "other", "-p", "APM"]);
    env.json(&["--as", "alice", "comment", "APM-1", "looks good"]);
    let v = env.json(&["show", "apm-1"]);
    assert_eq!(v["id"], "APM-1");
    assert_eq!(v["body"], "the body");
    assert_eq!(v["attributes"], serde_json::json!({}));
    assert_eq!(ids(&v["subtasks"]), ["APM-2"]);
    assert_eq!(v["subtasks"][0]["status"], "done");
    assert_eq!(v["subtasks"][0]["parent"], "APM-1");
    assert_eq!(v["comments"][0]["author"], "alice");
    assert_eq!(v["comments"][0]["body"], "looks good");

    let out = text(&env, &["show", "APM-1"]);
    assert!(out.starts_with("APM-1  parent\n"));
    assert!(out.contains("status: todo\n"));
    assert!(out.contains("created: "));
    assert!(out.contains("\nthe body\n"));
    assert!(out.contains("  APM-2  done  child\n"));
    assert!(out.contains("alice: looks good\n"));
}

#[test]
fn show_empty_lists_are_arrays() {
    let env = env_apm();
    env.json(&["add", "a", "-p", "APM"]);
    let v = env.json(&["show", "APM-1"]);
    assert_eq!(v["subtasks"], serde_json::json!([]));
    assert_eq!(v["comments"], serde_json::json!([]));
}

#[test]
fn show_errors() {
    let env = env_apm();
    assert_eq!(env.json_err(&["show", "APM-1"]), "no such item: APM-1");
    assert_eq!(env.json_err(&["show", "NOPE-1"]), "no such project: NOPE");
    assert!(
        env.json_err(&["show", "APM1"])
            .contains("invalid item reference")
    );
    env.apm()
        .args(["show", "APM-1"])
        .assert()
        .code(1)
        .stderr("error: no such item: APM-1\n");
}

// ---- edit

#[test]
fn edit_changes_fields_and_bumps_updated_at() {
    let env = env_apm();
    seed_milestone(&env, "APM", "v1");
    let before = env.json(&["add", "a", "-p", "APM"]);
    std::thread::sleep(std::time::Duration::from_millis(1100));
    let i = env.json(&[
        "edit",
        "apm-1",
        "--title",
        "A",
        "-b",
        "body",
        "--status",
        "blocked",
        "--priority",
        "urgent",
        "-a",
        "bob",
        "--due",
        "2027-01-02",
        "-m",
        "v1",
    ]);
    assert_eq!(i["title"], "A");
    assert_eq!(i["body"], "body");
    assert_eq!(i["status"], "blocked");
    assert_eq!(i["priority"], "urgent");
    assert_eq!(i["assignee"], "bob");
    assert_eq!(i["due"], "2027-01-02");
    assert_eq!(i["milestone"], "v1");
    assert_eq!(i["created_at"], before["created_at"]);
    assert_ne!(i["updated_at"], before["updated_at"]);
    assert_eq!(
        text(&env, &["edit", "APM-1", "--title", "B"]),
        "updated APM-1\n"
    );
}

#[test]
fn edit_unset_clears_fields() {
    let env = env_apm();
    seed_milestone(&env, "APM", "v1");
    env.json(&["add", "p", "-p", "APM"]);
    env.json(&[
        "add",
        "c",
        "--parent",
        "APM-1",
        "-a",
        "bob",
        "--due",
        "2026-11-01",
        "-m",
        "v1",
        "-b",
        "x",
    ]);
    let i = env.json(&[
        "edit",
        "APM-2",
        "--unset",
        "assignee",
        "--unset",
        "due",
        "--unset",
        "milestone",
        "--unset",
        "parent",
        "--unset",
        "body",
    ]);
    assert_eq!(i["assignee"], Value::Null);
    assert_eq!(i["due"], Value::Null);
    assert_eq!(i["milestone"], Value::Null);
    assert_eq!(i["parent"], Value::Null);
    assert_eq!(i["body"], "");
}

#[test]
fn edit_moves_under_a_parent() {
    let env = env_apm();
    env.json(&["add", "a", "-p", "APM"]);
    env.json(&["add", "b", "-p", "APM"]);
    assert_eq!(
        env.json(&["edit", "APM-2", "--parent", "APM-1"])["parent"],
        "APM-1"
    );
    assert_eq!(ids(&env.json(&["show", "APM-1"])["subtasks"]), ["APM-2"]);
}

#[test]
fn edit_rejects_parent_cycles() {
    let env = env_apm();
    env.json(&["add", "a", "-p", "APM"]);
    env.json(&["add", "b", "--parent", "APM-1"]);
    env.json(&["add", "c", "--parent", "APM-2"]);
    assert_eq!(
        env.json_err(&["edit", "APM-1", "--parent", "APM-1"]),
        "parent cycle: APM-1 is APM-1 or one of its subtasks"
    );
    assert_eq!(
        env.json_err(&["edit", "apm-1", "--parent", "apm-3"]),
        "parent cycle: APM-3 is APM-1 or one of its subtasks"
    );
    // Unchanged.
    assert_eq!(env.json(&["show", "APM-1"])["parent"], Value::Null);
}

#[test]
fn edit_rejects_cross_project_parent() {
    let env = env_apm();
    env.json(&["project", "add", "OPS", "Ops"]);
    env.json(&["add", "a", "-p", "APM"]);
    env.json(&["add", "b", "-p", "OPS"]);
    assert_eq!(
        env.json_err(&["edit", "APM-1", "--parent", "OPS-1"]),
        "parent OPS-1 is in another project than APM-1"
    );
}

#[test]
fn edit_errors() {
    let env = env_apm();
    assert_eq!(
        env.json_err(&["edit", "APM-1", "--title", "x"]),
        "no such item: APM-1"
    );
    env.json(&["add", "a", "-p", "APM"]);
    assert!(
        env.json_err(&["edit", "APM-1"])
            .contains("nothing to change")
    );
    assert_eq!(
        env.json_err(&["edit", "APM-1", "--unset", "colour"]),
        "unknown field: colour"
    );
    assert!(
        env.json_err(&["edit", "APM-1", "--unset", "title"])
            .contains("cannot unset title")
    );
    assert_eq!(
        env.json_err(&["edit", "APM-1", "-a", "bob", "--unset", "assignee"]),
        "cannot both set and unset assignee"
    );
    assert!(
        env.json_err(&["edit", "APM-1", "--due", "soon"])
            .contains("invalid date")
    );
    assert_eq!(
        env.json_err(&["edit", "APM-1", "-m", "v9"]),
        "no such milestone: v9"
    );
    assert_eq!(
        env.json_err(&["edit", "APM-1", "--title", " "]),
        "title cannot be empty"
    );
    assert_eq!(
        env.json_err(&["edit", "APM-1", "--parent", "APM-5"]),
        "no such item: APM-5"
    );
    assert_eq!(
        env.json_err(&["edit", "APM-1", "--set", "size=L", "--unset", "size"]),
        "cannot both set and unset size"
    );
    // A failed edit changes nothing.
    env.json_err(&["edit", "APM-1", "--title", "new", "--unset", "colour"]);
    assert_eq!(env.json(&["show", "APM-1"])["title"], "a");
}

// ---- start / done / block

#[test]
fn status_shortcuts() {
    let env = env_apm();
    env.json(&["add", "a", "-p", "APM"]);
    assert_eq!(env.json(&["start", "apm-1"])["status"], "doing");
    assert_eq!(env.json(&["block", "APM-1"])["status"], "blocked");
    assert_eq!(env.json(&["done", "APM-1"])["status"], "done");
    assert_eq!(text(&env, &["start", "APM-1"]), "updated APM-1 (doing)\n");
    assert_eq!(env.json_err(&["done", "APM-9"]), "no such item: APM-9");
    assert_eq!(env.json_err(&["start", "X-1"]), "no such project: X");
}

// ---- rm

#[test]
fn rm_deletes_subtasks_and_never_reuses_numbers() {
    let env = env_apm();
    env.json(&["add", "a", "-p", "APM"]);
    env.json(&["add", "b", "--parent", "APM-1"]);
    env.json(&["add", "c", "--parent", "APM-2"]);
    env.json(&["add", "keep", "-p", "APM"]);
    let i = env.json(&["rm", "APM-1"]);
    assert_eq!(i["id"], "APM-1");
    assert_eq!(ids(&env.json(&["list", "--all"])), ["APM-4"]);
    assert_eq!(env.json_err(&["show", "APM-3"]), "no such item: APM-3");
    assert_eq!(env.json(&["add", "next", "-p", "APM"])["id"], "APM-5");
    assert_eq!(text(&env, &["rm", "APM-5"]), "deleted APM-5\n");
}

#[test]
fn rm_text_counts_subtasks() {
    let env = env_apm();
    env.json(&["add", "a", "-p", "APM"]);
    env.json(&["add", "b", "--parent", "APM-1"]);
    assert_eq!(
        text(&env, &["rm", "APM-1"]),
        "deleted APM-1 and 1 subtask(s)\n"
    );
}

#[test]
fn rm_errors() {
    let env = env_apm();
    assert_eq!(env.json_err(&["rm", "APM-1"]), "no such item: APM-1");
    assert!(
        env.json_err(&["rm", "1"])
            .contains("invalid item reference")
    );
}

// ---- concurrency

#[test]
fn concurrent_adds_get_distinct_numbers() {
    let env = env_apm();
    let workers: Vec<_> = (0..6)
        .map(|w| {
            let db = env.db_path();
            std::thread::spawn(move || {
                for n in 0..5 {
                    let title = format!("w{w}-{n}");
                    let out = std::process::Command::new(env!("CARGO_BIN_EXE_apm"))
                        .env("APM_DB", &db)
                        .env("APM_AGENT", "tester")
                        .args(["--json", "add", &title, "-p", "APM"])
                        .output()
                        .unwrap();
                    let stderr = String::from_utf8_lossy(&out.stderr);
                    assert!(out.status.success(), "{stderr}");
                }
            })
        })
        .collect();
    for w in workers {
        w.join().unwrap();
    }
    let mut all = ids(&env.json(&["list"]));
    all.sort();
    all.dedup();
    assert_eq!(all.len(), 30);
}
