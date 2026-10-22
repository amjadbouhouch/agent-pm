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
    // Keys keep their declared order: project fields, then counts in status order.
    let out = env
        .apm()
        .args(["--json", "project", "show", "APM"])
        .assert()
        .success();
    let raw = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    let at = |k: &str| raw.find(&format!("\"{k}\"")).unwrap();
    assert!(at("key") < at("name") && at("name") < at("description"));
    assert!(at("updated_at") < at("counts"));
    assert!(at("todo") < at("doing") && at("done") < at("cancelled"));
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
    let left = env.json(&["list", "--all"]);
    assert_eq!(left.as_array().unwrap().len(), 1);
    assert_eq!(left[0]["id"], "OTHER-1");
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

fn keys(env: &Env) -> Vec<String> {
    env.json(&["project", "list"])
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["key"].as_str().unwrap().to_string())
        .collect()
}

/// APM with tasks APM-1, APM-2 (subtask of 1) and a milestone.
fn env_tasks() -> Env {
    let env = Env::new();
    env.json(&["project", "add", "APM", "agent-pm"]);
    env.json(&["milestone", "add", "v1", "-p", "APM"]);
    env.json(&["add", "a", "-p", "APM", "-m", "v1"]);
    env.json(&["add", "b", "--parent", "APM-1"]);
    env
}

// ---- project edit --key

#[test]
fn rekey_moves_every_task_id() {
    let env = env_tasks();
    let p = env.json(&["project", "edit", "apm", "--key", "ops"]);
    assert_eq!(p["key"], "OPS");
    assert!(p.get("claimed_alias_from").is_none());
    assert_eq!(p["name"], "agent-pm");
    assert_eq!(keys(&env), ["OPS"]);
    let i = env.json(&["show", "OPS-2"]);
    assert_eq!(i["parent"], "OPS-1");
    assert_eq!(i["project"], "OPS");
    // The old key still finds the project and its tasks; output shows current ids.
    assert_eq!(env.json(&["project", "show", "APM"])["key"], "OPS");
    assert_eq!(env.json(&["show", "APM-1"])["id"], "OPS-1");
    assert_eq!(env.json(&["list", "-p", "apm"])[1]["id"], "OPS-2");
    assert_eq!(env.json(&["add", "c", "-p", "APM"])["id"], "OPS-3");
    assert_eq!(env.json(&["edit", "APM-2", "--title", "x"])["id"], "OPS-2");
    assert_eq!(env.json(&["comment", "APM-1", "hi"])["item"], "OPS-1");
    assert_eq!(
        env.json(&["milestone", "show", "v1", "-p", "APM"])["project"],
        "OPS"
    );
    assert_eq!(
        env.json(&["status", "list", "-p", "APM"])[0]["project"],
        "OPS"
    );
    assert_eq!(
        text(
            &env,
            &["project", "edit", "OPS", "--key", "WEB", "--name", "web"]
        ),
        "updated WEB\n"
    );
}

#[test]
fn rekey_twice_and_back() {
    let env = env_tasks();
    env.json(&["project", "edit", "APM", "--key", "OPS"]);
    env.json(&["project", "edit", "OPS", "--key", "WEB"]);
    for old in ["APM-2", "OPS-2", "WEB-2"] {
        assert_eq!(env.json(&["show", old])["id"], "WEB-2", "{old}");
    }
    // Back to a former key: it becomes live again, the others stay aliases. Its own alias:
    // nothing claimed from another project.
    let p = env.json(&["project", "edit", "OPS", "--key", "APM"]);
    assert!(p.get("claimed_alias_from").is_none());
    assert_eq!(keys(&env), ["APM"]);
    for old in ["APM-2", "OPS-2", "WEB-2"] {
        assert_eq!(env.json(&["show", old])["id"], "APM-2", "{old}");
    }
    // Re-keying to the current key is a no-op.
    assert_eq!(
        env.json(&["project", "edit", "APM", "--key", "apm"])["key"],
        "APM"
    );
}

#[test]
fn a_live_key_wins_over_an_alias() {
    let env = env_tasks();
    env.json(&["project", "edit", "APM", "--key", "OPS"]);
    // A new project may take the old key; old ids then mean the new project, and it says so.
    let p = env.json(&["project", "add", "APM", "fresh"]);
    assert_eq!(p["claimed_alias_from"], "OPS");
    assert_eq!(keys(&env), ["APM", "OPS"]);
    assert_eq!(env.json(&["project", "show", "APM"])["name"], "fresh");
    assert_eq!(env.json_err(&["show", "APM-1"]), "no such item: APM-1");
    assert_eq!(env.json(&["add", "new", "-p", "APM"])["id"], "APM-1");
    assert_eq!(env.json(&["show", "OPS-1"])["title"], "a");
    // Re-keying to another project's alias takes the alias too.
    env.json(&["project", "add", "WEB", "web"]);
    env.json(&["project", "edit", "WEB", "--key", "OLD"]);
    assert_eq!(
        text(&env, &["project", "edit", "OPS", "--key", "WEB"]),
        "updated WEB (key was an alias of OLD; its old ids no longer resolve)\n"
    );
    assert_eq!(env.json(&["project", "show", "WEB"])["name"], "agent-pm");
    assert_eq!(env.json(&["project", "show", "OLD"])["name"], "web");
    assert_eq!(env.json(&["show", "OPS-1"])["id"], "WEB-1");
}

#[test]
fn rekey_errors() {
    let env = env_tasks();
    env.json(&["project", "add", "WEB", "web"]);
    assert_eq!(
        env.json_err(&["project", "edit", "APM", "--key", "web"]),
        "project already exists: WEB"
    );
    assert!(
        env.json_err(&["project", "edit", "APM", "--key", "A-B"])
            .contains("invalid project key `A-B`")
    );
    assert_eq!(
        env.json_err(&["project", "edit", "APM"]),
        "nothing to change: pass --key, --name or --description"
    );
    assert_eq!(
        env.json_err(&["project", "edit", "NOPE", "--key", "X"]),
        "no such project: NOPE"
    );
    // A failed rename (bad name after a good key) changes nothing.
    env.json_err(&["project", "edit", "APM", "--key", "OPS", "--name", " "]);
    assert_eq!(keys(&env), ["APM", "WEB"]);
    assert_eq!(env.json_err(&["show", "OPS-1"]), "no such project: OPS");
}

#[test]
fn moves_and_rekeys_combine() {
    let env = env_tasks();
    env.json(&["project", "add", "WEB", "web"]);
    env.json(&["move", "APM-1", "WEB"]);
    env.json(&["project", "edit", "APM", "--key", "OPS"]);
    env.json(&["project", "edit", "WEB", "--key", "SITE"]);
    for old in ["APM-1", "OPS-1", "WEB-1", "SITE-1"] {
        assert_eq!(env.json(&["show", old])["id"], "SITE-1", "{old}");
    }
    assert_eq!(env.json(&["show", "APM-2"])["parent"], "SITE-1");
    // Move it back through its oldest id.
    assert_eq!(env.json(&["move", "APM-1", "APM"])["id"], "OPS-3");
}

#[test]
fn project_rm_removes_its_aliases() {
    let env = env_tasks();
    env.json(&["project", "add", "WEB", "web"]);
    env.json(&["move", "APM-1", "WEB"]);
    env.json(&["project", "edit", "APM", "--key", "OPS"]);
    env.json(&["project", "rm", "APM"]);
    let conn = env.conn();
    for table in ["project_aliases", "item_aliases"] {
        let n: i64 = conn
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0, "{table}");
    }
    assert_eq!(env.json_err(&["show", "APM-1"]), "no such project: APM");
}
