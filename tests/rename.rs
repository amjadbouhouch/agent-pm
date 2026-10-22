//! Renaming project keys and attributes; old keys keep resolving through aliases.
mod common;

use common::Env;
use serde_json::{Value, json};

/// Text-mode stdout of a successful run.
fn text(env: &Env, args: &[&str]) -> String {
    let out = env.apm().args(args).assert().success();
    String::from_utf8(out.get_output().stdout.clone()).unwrap()
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
fn env_apm() -> Env {
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
    let env = env_apm();
    let p = env.json(&["project", "edit", "apm", "--key", "ops"]);
    assert_eq!(p["key"], "OPS");
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
    let env = env_apm();
    env.json(&["project", "edit", "APM", "--key", "OPS"]);
    env.json(&["project", "edit", "OPS", "--key", "WEB"]);
    for old in ["APM-2", "OPS-2", "WEB-2"] {
        assert_eq!(env.json(&["show", old])["id"], "WEB-2", "{old}");
    }
    // Back to a former key: it becomes live again, the others stay aliases.
    env.json(&["project", "edit", "OPS", "--key", "APM"]);
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
    let env = env_apm();
    env.json(&["project", "edit", "APM", "--key", "OPS"]);
    // A new project may take the old key; old ids then mean the new project.
    env.json(&["project", "add", "APM", "fresh"]);
    assert_eq!(keys(&env), ["APM", "OPS"]);
    assert_eq!(env.json(&["project", "show", "APM"])["name"], "fresh");
    assert_eq!(env.json_err(&["show", "APM-1"]), "no such item: APM-1");
    assert_eq!(env.json(&["add", "new", "-p", "APM"])["id"], "APM-1");
    assert_eq!(env.json(&["show", "OPS-1"])["title"], "a");
    // Re-keying to another project's alias takes the alias too.
    env.json(&["project", "add", "WEB", "web"]);
    env.json(&["project", "edit", "WEB", "--key", "OLD"]);
    env.json(&["project", "edit", "OPS", "--key", "WEB"]);
    assert_eq!(env.json(&["project", "show", "WEB"])["name"], "agent-pm");
    assert_eq!(env.json(&["project", "show", "OLD"])["name"], "web");
    assert_eq!(env.json(&["show", "OPS-1"])["id"], "WEB-1");
}

#[test]
fn rekey_errors() {
    let env = env_apm();
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
    let env = env_apm();
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
    let env = env_apm();
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

// ---- attr edit --rename

#[test]
fn attr_rename_keeps_values() {
    let env = env_apm();
    env.json(&["attr", "add", "points", "-p", "APM", "--type", "number"]);
    env.json(&["edit", "APM-1", "--set", "points=3"]);
    env.json(&["project", "edit", "APM", "--key", "OPS"]);
    let a = env.json(&[
        "attr", "edit", "points", "-p", "APM", "--rename", "estimate",
    ]);
    // The project in the output is the current key, even when an alias was passed.
    assert_eq!(
        a,
        json!({"name": "estimate", "project": "OPS", "type": "number", "options": []})
    );
    assert_eq!(
        env.json(&["show", "OPS-1"])["attributes"],
        json!({"estimate": 3})
    );
    assert_eq!(
        env.json(&["list", "--where", "estimate=3"])[0]["id"],
        "OPS-1"
    );
    assert_eq!(
        env.json(&["attr", "list", "-p", "APM"])[0]["project"],
        "OPS"
    );
    assert_eq!(
        text(
            &env,
            &["attr", "edit", "estimate", "-p", "OPS", "--rename", "pts"]
        ),
        "renamed attribute estimate to pts\n"
    );
}

#[test]
fn attr_rename_errors() {
    let env = env_apm();
    env.json(&["attr", "add", "points", "-p", "APM", "--type", "number"]);
    env.json(&["attr", "add", "size", "-p", "APM", "--type", "text"]);
    let e = |name: &str, rename: &str| {
        env.json_err(&["attr", "edit", name, "-p", "APM", "--rename", rename])
    };
    assert_eq!(e("points", "size"), "attribute already exists: size");
    assert_eq!(e("points", "Status"), "built-in field: Status");
    assert_eq!(
        e("points", "a=b"),
        "attribute name may not contain `=`: a=b"
    );
    assert_eq!(e("points", " "), "attribute name is empty");
    assert_eq!(e("nope", "x"), "no such attribute: nope");
    assert_eq!(
        env.json_err(&["attr", "edit", "points", "-p", "APM"]),
        "nothing to change: pass --rename"
    );
    let names: Vec<Value> = env
        .json(&["attr", "list", "-p", "APM"])
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["name"].clone())
        .collect();
    assert_eq!(names, ["points", "size"]);
}
