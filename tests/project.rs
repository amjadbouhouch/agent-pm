mod common;

use common::Env;

/// Text-mode stdout of a successful run.
fn text(env: &Env, args: &[&str]) -> String {
    let out = env.apm().args(args).assert().success();
    String::from_utf8(out.get_output().stdout.clone()).unwrap()
}

#[test]
fn add_returns_the_project() {
    let env = Env::new();
    let p = env.json(&["project", "add", "apm", "Agent PM", "-d", "the tracker"]);
    assert_eq!(p["key"], "APM");
    assert_eq!(p["name"], "Agent PM");
    assert_eq!(p["description"], "the tracker");
    assert!(p["created_at"].as_str().unwrap().ends_with('Z'));
    assert_eq!(p["created_at"], p["updated_at"]);
}

#[test]
fn add_prints_one_line_without_json() {
    let env = Env::new();
    env.apm()
        .args(["project", "add", "APM", "Agent PM"])
        .assert()
        .success()
        .stdout("created APM\n");
}

#[test]
fn add_rejects_bad_and_duplicate_keys() {
    let env = Env::new();
    assert!(
        env.json_err(&["project", "add", "2X", "x"])
            .contains("invalid project key")
    );
    assert!(
        env.json_err(&["project", "add", "A-B", "x"])
            .contains("invalid project key")
    );
    assert!(
        env.json_err(&["project", "add", "APM", " "])
            .contains("name cannot be empty")
    );
    env.json(&["project", "add", "APM", "x"]);
    assert_eq!(
        env.json_err(&["project", "add", "apm", "y"]),
        "project already exists: APM"
    );
}

#[test]
fn list_is_ordered_by_key() {
    let env = Env::new();
    assert_eq!(env.json(&["project", "list"]), serde_json::json!([]));
    env.json(&["project", "add", "ZED", "Zed"]);
    env.json(&["project", "add", "APM", "Agent PM"]);
    let v = env.json(&["project", "list"]);
    let keys: Vec<&str> = v
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["key"].as_str().unwrap())
        .collect();
    assert_eq!(keys, ["APM", "ZED"]);

    let text = text(&env, &["project", "list"]);
    assert!(text.starts_with("KEY"));
    assert!(text.contains("APM  Agent PM"));
}

#[test]
fn show_counts_tasks_by_status() {
    let env = Env::new();
    env.json(&["project", "add", "APM", "Agent PM"]);
    let v = env.json(&["project", "show", "apm"]);
    assert_eq!(v["key"], "APM");
    for s in ["todo", "doing", "blocked", "done", "cancelled"] {
        assert_eq!(v["counts"][s], 0, "{s}");
    }
    env.json(&["add", "a", "-p", "APM"]);
    env.json(&["add", "b", "-p", "APM", "--status", "done"]);
    env.json(&["add", "c", "--parent", "APM-1"]);
    let v = env.json(&["project", "show", "APM"]);
    assert_eq!(v["counts"]["todo"], 2);
    assert_eq!(v["counts"]["done"], 1);

    assert!(text(&env, &["project", "show", "APM"]).contains("tasks: todo=2 doing=0"));
}

#[test]
fn edit_changes_name_and_description() {
    let env = Env::new();
    let before = env.json(&["project", "add", "APM", "Agent PM", "-d", "old"]);
    std::thread::sleep(std::time::Duration::from_millis(1100));
    let p = env.json(&["project", "edit", "apm", "--name", "New"]);
    assert_eq!(p["name"], "New");
    assert_eq!(p["description"], "old");
    assert_ne!(p["updated_at"], before["updated_at"]);
    let p = env.json(&["project", "edit", "APM", "-d", "new"]);
    assert_eq!(p["name"], "New");
    assert_eq!(p["description"], "new");
}

#[test]
fn edit_errors() {
    let env = Env::new();
    assert_eq!(
        env.json_err(&["project", "edit", "NOPE", "--name", "x"]),
        "no such project: NOPE"
    );
    env.json(&["project", "add", "APM", "x"]);
    assert!(
        env.json_err(&["project", "edit", "APM"])
            .contains("nothing to change")
    );
    assert!(
        env.json_err(&["project", "edit", "APM", "--name", ""])
            .contains("name cannot be empty")
    );
}

#[test]
fn rm_deletes_the_project_and_its_tasks() {
    let env = Env::new();
    env.json(&["project", "add", "APM", "x"]);
    env.json(&["project", "add", "OTHER", "y"]);
    env.json(&["add", "a", "-p", "APM"]);
    env.json(&["add", "b", "--parent", "APM-1"]);
    env.json(&["add", "keep", "-p", "OTHER"]);
    let p = env.json(&["project", "rm", "apm"]);
    assert_eq!(p["key"], "APM");
    assert_eq!(env.json_err(&["show", "APM-1"]), "no such project: APM");
    let n: i64 = env
        .conn()
        .query_row("SELECT count(*) FROM items", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 1);
    assert_eq!(
        env.json_err(&["project", "rm", "APM"]),
        "no such project: APM"
    );
}

#[test]
fn show_unknown_project() {
    let env = Env::new();
    assert_eq!(
        env.json_err(&["project", "show", "x"]),
        "no such project: X"
    );
    env.apm()
        .args(["project", "show", "x"])
        .assert()
        .code(1)
        .stderr("error: no such project: X\n");
}
