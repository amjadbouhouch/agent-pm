mod common;

use common::Env;

/// Create project `key`.
fn project(env: &Env, key: &str) {
    env.json(&["project", "add", key, key]);
}

/// Add the next APM task, assigned to ann, with a status and an optional milestone.
fn task(env: &Env, title: &str, status: &str, milestone: Option<&str>) {
    let mut args = vec!["add", title, "-p", "APM", "-a", "ann", "--status", status];
    if let Some(m) = milestone {
        args.extend(["-m", m]);
    }
    env.json(&args);
}

#[test]
fn add_returns_the_milestone() {
    let env = Env::new();
    project(&env, "APM");
    let v = env.json(&[
        "milestone",
        "add",
        "v1",
        "-p",
        "apm",
        "-d",
        "first",
        "--due",
        "2026-12-01",
    ]);
    assert_eq!(v["name"], "v1");
    assert_eq!(v["project"], "APM");
    assert_eq!(v["description"], "first");
    assert_eq!(v["due"], "2026-12-01");
    assert_eq!(v["status"], "open");
    assert_eq!(
        v["progress"],
        serde_json::json!({"total": 0, "done": 0, "cancelled": 0})
    );
    assert!(v["created_at"].is_string() && v["updated_at"].is_string());

    let v = env.json(&["milestone", "add", "v2", "-p", "APM"]);
    assert!(v["due"].is_null());
    assert_eq!(v["description"], "");
}

#[test]
fn add_text_output() {
    let env = Env::new();
    project(&env, "APM");
    env.apm()
        .args(["milestone", "add", "v1", "-p", "APM"])
        .assert()
        .success()
        .stdout("created milestone v1\n");
}

#[test]
fn add_errors() {
    let env = Env::new();
    project(&env, "APM");
    assert_eq!(
        env.json_err(&["milestone", "add", "v1", "-p", "NOPE"]),
        "no such project: NOPE"
    );
    env.json(&["milestone", "add", "v1", "-p", "APM"]);
    assert_eq!(
        env.json_err(&["milestone", "add", "v1", "-p", "APM"]),
        "milestone already exists: v1"
    );
    assert!(
        env.json_err(&["milestone", "add", "v2", "-p", "APM", "--due", "2026-02-30"])
            .contains("invalid date")
    );
    assert_eq!(
        env.json_err(&["milestone", "add", " ", "-p", "APM"]),
        "milestone name cannot be empty"
    );
    // Missing -p is a usage error.
    env.apm()
        .args(["--json", "milestone", "add", "v3"])
        .assert()
        .code(2);
}

#[test]
fn same_name_in_another_project_is_fine() {
    let env = Env::new();
    project(&env, "APM");
    project(&env, "WEB");
    env.json(&["milestone", "add", "v1", "-p", "APM"]);
    assert_eq!(
        env.json(&["milestone", "add", "v1", "-p", "WEB"])["project"],
        "WEB"
    );
}

#[test]
fn list_shows_open_with_progress() {
    let env = Env::new();
    project(&env, "APM");
    env.json(&["milestone", "add", "v1", "-p", "APM", "--due", "2026-12-01"]);
    env.json(&["milestone", "add", "v2", "-p", "APM"]);
    env.json(&[
        "milestone",
        "add",
        "old",
        "-p",
        "APM",
        "--due",
        "2026-01-01",
    ]);
    env.json(&["milestone", "close", "old", "-p", "APM"]);
    task(&env, "a", "done", Some("v1"));
    task(&env, "b", "todo", Some("v1"));
    task(&env, "c", "cancelled", Some("v1"));
    task(&env, "d", "doing", None);

    let v = env.json(&["milestone", "list", "-p", "APM"]);
    let names: Vec<&str> = v
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["v1", "v2"]);
    assert_eq!(
        v[0]["progress"],
        serde_json::json!({"total": 2, "done": 1, "cancelled": 1})
    );
    assert_eq!(v[1]["progress"]["total"], 0);

    let v = env.json(&["milestone", "list", "-p", "APM", "--all"]);
    assert_eq!(v.as_array().unwrap().len(), 3);
    assert_eq!(v[0]["name"], "old");
    assert_eq!(v[0]["status"], "closed");

    let out = env
        .apm()
        .args(["milestone", "list", "-p", "APM"])
        .assert()
        .success();
    let text = String::from_utf8_lossy(&out.get_output().stdout).to_string();
    assert!(
        text.lines()
            .any(|l| l.starts_with("v1") && l.ends_with("1/2"))
    );
    assert!(!text.contains("old"));
}

#[test]
fn list_empty_and_errors() {
    let env = Env::new();
    project(&env, "APM");
    assert_eq!(
        env.json(&["milestone", "list", "-p", "APM"]),
        serde_json::json!([])
    );
    assert_eq!(
        env.json_err(&["milestone", "list", "-p", "NOPE"]),
        "no such project: NOPE"
    );
}

#[test]
fn show_includes_tasks_in_order() {
    let env = Env::new();
    project(&env, "APM");
    env.json(&["milestone", "add", "v1", "-p", "APM"]);
    task(&env, "first", "done", Some("v1"));
    task(&env, "second", "todo", Some("v1"));
    task(&env, "elsewhere", "todo", None);

    let v = env.json(&["milestone", "show", "v1", "-p", "apm"]);
    assert_eq!(v["name"], "v1");
    assert_eq!(v["progress"]["done"], 1);
    assert_eq!(
        v["items"],
        serde_json::json!([
            {"id": "APM-1", "status": "done", "title": "first", "assignee": "ann"},
            {"id": "APM-2", "status": "todo", "title": "second", "assignee": "ann"},
        ])
    );

    let out = env
        .apm()
        .args(["milestone", "show", "v1", "-p", "APM"])
        .assert()
        .success();
    let text = String::from_utf8_lossy(&out.get_output().stdout).to_string();
    assert!(text.contains("APM-1") && text.contains("second"));
    assert!(!text.contains("elsewhere"));
}

#[test]
fn show_errors() {
    let env = Env::new();
    project(&env, "APM");
    assert_eq!(
        env.json_err(&["milestone", "show", "v9", "-p", "APM"]),
        "no such milestone: v9"
    );
    assert_eq!(
        env.json_err(&["milestone", "show", "v9", "-p", "NOPE"]),
        "no such project: NOPE"
    );
}

#[test]
fn edit_changes_fields() {
    let env = Env::new();
    project(&env, "APM");
    env.json(&["milestone", "add", "v1", "-p", "APM", "--due", "2026-12-01"]);
    let v = env.json(&[
        "milestone",
        "edit",
        "v1",
        "-p",
        "APM",
        "--rename",
        "v1.0",
        "-d",
        "ship it",
    ]);
    assert_eq!(v["name"], "v1.0");
    assert_eq!(v["description"], "ship it");
    assert_eq!(v["due"], "2026-12-01");

    let v = env.json(&[
        "milestone",
        "edit",
        "v1.0",
        "-p",
        "APM",
        "--due",
        "2027-01-15",
    ]);
    assert_eq!(v["due"], "2027-01-15");
    let v = env.json(&["milestone", "edit", "v1.0", "-p", "APM", "--due", ""]);
    assert!(v["due"].is_null());
    assert_eq!(v["description"], "ship it");

    // Renaming to its own name is allowed.
    env.json(&["milestone", "edit", "v1.0", "-p", "APM", "--rename", "v1.0"]);
    env.apm()
        .args(["milestone", "edit", "v1.0", "-p", "APM", "-d", "x"])
        .assert()
        .success()
        .stdout("updated milestone v1.0\n");
}

#[test]
fn edit_errors() {
    let env = Env::new();
    project(&env, "APM");
    env.json(&["milestone", "add", "v1", "-p", "APM"]);
    env.json(&["milestone", "add", "v2", "-p", "APM"]);
    assert_eq!(
        env.json_err(&["milestone", "edit", "v1", "-p", "APM", "--rename", "v2"]),
        "milestone already exists: v2"
    );
    assert_eq!(
        env.json_err(&["milestone", "edit", "v1", "-p", "APM", "--rename", ""]),
        "milestone name cannot be empty"
    );
    assert!(
        env.json_err(&["milestone", "edit", "v1", "-p", "APM", "--due", "soon"])
            .contains("invalid date")
    );
    assert!(
        env.json_err(&["milestone", "edit", "v1", "-p", "APM"])
            .starts_with("nothing to change")
    );
    assert_eq!(
        env.json_err(&["milestone", "edit", "v9", "-p", "APM", "-d", "x"]),
        "no such milestone: v9"
    );
}

#[test]
fn close_and_reopen() {
    let env = Env::new();
    project(&env, "APM");
    env.json(&["milestone", "add", "v1", "-p", "APM"]);
    assert_eq!(
        env.json(&["milestone", "close", "v1", "-p", "APM"])["status"],
        "closed"
    );
    // Closing twice is a no-op.
    assert_eq!(
        env.json(&["milestone", "close", "v1", "-p", "APM"])["status"],
        "closed"
    );
    assert_eq!(
        env.json(&["milestone", "reopen", "v1", "-p", "APM"])["status"],
        "open"
    );
    env.apm()
        .args(["milestone", "close", "v1", "-p", "APM"])
        .assert()
        .success()
        .stdout("closed milestone v1\n");
    env.apm()
        .args(["milestone", "reopen", "v1", "-p", "APM"])
        .assert()
        .success()
        .stdout("reopened milestone v1\n");
    assert_eq!(
        env.json_err(&["milestone", "close", "v9", "-p", "APM"]),
        "no such milestone: v9"
    );
    assert_eq!(
        env.json_err(&["milestone", "reopen", "v9", "-p", "APM"]),
        "no such milestone: v9"
    );
}

#[test]
fn rm_keeps_tasks() {
    let env = Env::new();
    project(&env, "APM");
    env.json(&["milestone", "add", "v1", "-p", "APM"]);
    task(&env, "a", "todo", Some("v1"));
    let v = env.json(&["milestone", "rm", "v1", "-p", "APM"]);
    assert_eq!(v["name"], "v1");
    assert_eq!(v["progress"]["total"], 1);

    assert_eq!(
        env.json(&["show", "APM-1"])["milestone"],
        serde_json::Value::Null
    );
    assert_eq!(
        env.json_err(&["milestone", "show", "v1", "-p", "APM"]),
        "no such milestone: v1"
    );
    assert_eq!(
        env.json_err(&["milestone", "rm", "v1", "-p", "APM"]),
        "no such milestone: v1"
    );

    env.json(&["milestone", "add", "v2", "-p", "APM"]);
    env.apm()
        .args(["milestone", "rm", "v2", "-p", "APM"])
        .assert()
        .success()
        .stdout("removed milestone v2\n");
}

#[test]
fn names_are_trimmed() {
    let env = Env::new();
    project(&env, "APM");
    assert_eq!(
        env.json(&["milestone", "add", " v1 ", "-p", "APM"])["name"],
        "v1"
    );
    assert_eq!(
        env.json_err(&["milestone", "add", "v1", "-p", "APM"]),
        "milestone already exists: v1"
    );
    env.json(&["milestone", "add", "v2", "-p", "APM"]);
    assert_eq!(
        env.json_err(&["milestone", "edit", "v2", "-p", "APM", "--rename", " v1"]),
        "milestone already exists: v1"
    );
    assert_eq!(
        env.json(&["milestone", "edit", "v2", "-p", "APM", "--rename", "v3 "])["name"],
        "v3"
    );
    assert_eq!(
        env.json(&["milestone", "list", "-p", "APM"])
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn unknown_project_errors() {
    let env = Env::new();
    for cmd in ["show", "close", "reopen", "rm"] {
        assert_eq!(
            env.json_err(&["milestone", cmd, "v1", "-p", "nope"]),
            "no such project: NOPE"
        );
    }
    assert_eq!(
        env.json_err(&["milestone", "edit", "v1", "-p", "NOPE", "-d", "x"]),
        "no such project: NOPE"
    );
}

#[test]
fn progress_counts_leaf_tasks_only() {
    let env = Env::new();
    project(&env, "APM");
    env.json(&["milestone", "add", "v1", "-p", "APM"]);
    // APM-1 has three subtasks in v1 (one done, one cancelled): it counts through them.
    task(&env, "round", "doing", Some("v1"));
    for (title, status) in [("a", "done"), ("b", "todo"), ("c", "cancelled")] {
        env.json(&[
            "add", title, "--parent", "APM-1", "-m", "v1", "--status", status,
        ]);
    }
    // APM-5's only subtask is outside v1: APM-5 is the unit of work here.
    task(&env, "solo", "done", Some("v1"));
    env.json(&["add", "elsewhere", "--parent", "APM-5"]);
    // A nested round: APM-7 > APM-8 > APM-9, all in v1; only APM-9 counts.
    task(&env, "outer", "todo", Some("v1"));
    env.json(&["add", "middle", "--parent", "APM-7", "-m", "v1"]);
    env.json(&[
        "add", "inner", "--parent", "APM-8", "-m", "v1", "--status", "done",
    ]);
    let v = env.json(&["milestone", "show", "v1", "-p", "APM"]);
    assert_eq!(
        v["progress"],
        serde_json::json!({"total": 4, "done": 3, "cancelled": 1})
    );
    // Every task in the milestone is still listed.
    assert_eq!(v["items"].as_array().unwrap().len(), 8);
    assert_eq!(
        env.json(&["milestone", "list", "-p", "APM"])[0]["progress"]["total"],
        4
    );
}
