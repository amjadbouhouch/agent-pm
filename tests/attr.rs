mod common;

use common::Env;
use serde_json::json;

/// An env with project APM (no `project add` on this branch yet, so raw SQL).
fn env() -> Env {
    let env = Env::new();
    env.conn()
        .execute(
            "INSERT INTO projects (key, name) VALUES ('APM', 'agent-pm')",
            [],
        )
        .unwrap();
    env
}

#[test]
fn add_returns_the_attribute() {
    let env = env();
    let v = env.json(&["attr", "add", "points", "-p", "apm", "--type", "number"]);
    assert_eq!(
        v,
        json!({"name": "points", "project": "APM", "type": "number", "options": []})
    );
    let v = env.json(&[
        "attr",
        "add",
        "size",
        "-p",
        "APM",
        "--type",
        "select",
        "--options",
        "s, m,l",
    ]);
    assert_eq!(v["type"], "select");
    assert_eq!(v["options"], json!(["s", "m", "l"]));
}

#[test]
fn add_prints_one_line() {
    let env = env();
    let out = env
        .apm()
        .args(["attr", "add", "done-ok", "-p", "APM", "--type", "checkbox"])
        .assert()
        .success();
    assert_eq!(
        String::from_utf8_lossy(&out.get_output().stdout).trim(),
        "added attribute done-ok"
    );
}

#[test]
fn add_every_type() {
    let env = env();
    for t in ["text", "number", "date", "checkbox"] {
        let name = format!("a-{t}");
        assert_eq!(
            env.json(&["attr", "add", &name, "-p", "APM", "--type", t])["type"],
            t
        );
    }
    let list = env.json(&["attr", "list", "-p", "APM"]);
    assert_eq!(list.as_array().unwrap().len(), 4);
}

#[test]
fn add_rejects_bad_names() {
    let env = env();
    let add = |name: &str| env.json_err(&["attr", "add", name, "-p", "APM", "--type", "text"]);
    assert_eq!(add("  "), "attribute name is empty");
    assert_eq!(add("status"), "built-in field: status");
    assert_eq!(add("Due"), "built-in field: Due");
    assert!(add("a=b").contains("may not contain `=`"));
    env.json(&["attr", "add", "note", "-p", "APM", "--type", "text"]);
    assert_eq!(add("note"), "attribute already exists: note");
    assert_eq!(add(" note "), "attribute already exists: note");
}

#[test]
fn add_trims_name() {
    let env = env();
    let v = env.json(&["attr", "add", " note ", "-p", "APM", "--type", "text"]);
    assert_eq!(v["name"], "note");
    env.json(&["attr", "rm", "note", "-p", "APM"]);
}

#[test]
fn add_checks_options() {
    let env = env();
    let add = |args: &[&str]| {
        let mut all = vec!["attr", "add", "x", "-p", "APM"];
        all.extend_from_slice(args);
        env.json_err(&all)
    };
    assert_eq!(add(&["--type", "select"]), "a select needs --options");
    assert_eq!(
        add(&["--type", "text", "--options", "a"]),
        "--options is only for select attributes"
    );
    assert_eq!(
        add(&["--type", "select", "--options", "a,,b"]),
        "empty option in --options"
    );
    assert_eq!(
        add(&["--type", "select", "--options", "a,b,a"]),
        "duplicate option: a"
    );
}

#[test]
fn add_rejects_unknown_project_and_type() {
    let env = env();
    assert_eq!(
        env.json_err(&["attr", "add", "x", "-p", "nope", "--type", "text"]),
        "no such project: NOPE"
    );
    env.apm()
        .args(["--json", "attr", "add", "x", "-p", "APM", "--type", "money"])
        .assert()
        .code(2);
}

#[test]
fn same_name_in_two_projects() {
    let env = env();
    env.conn()
        .execute("INSERT INTO projects (key, name) VALUES ('WEB', 'web')", [])
        .unwrap();
    env.json(&["attr", "add", "points", "-p", "APM", "--type", "number"]);
    let v = env.json(&["attr", "add", "points", "-p", "WEB", "--type", "text"]);
    assert_eq!(v["project"], "WEB");
}

#[test]
fn list_in_creation_order() {
    let env = env();
    assert_eq!(env.json(&["attr", "list", "-p", "APM"]), json!([]));
    env.json(&["attr", "add", "zeta", "-p", "APM", "--type", "text"]);
    env.json(&[
        "attr",
        "add",
        "alpha",
        "-p",
        "APM",
        "--type",
        "select",
        "--options",
        "x,y",
    ]);
    let list = env.json(&["attr", "list", "-p", "APM"]);
    assert_eq!(
        list,
        json!([
            {"name": "zeta", "project": "APM", "type": "text", "options": []},
            {"name": "alpha", "project": "APM", "type": "select", "options": ["x", "y"]},
        ])
    );
    let out = env
        .apm()
        .args(["attr", "list", "-p", "APM"])
        .assert()
        .success();
    let text = String::from_utf8_lossy(&out.get_output().stdout).to_string();
    assert!(text.starts_with("NAME"));
    assert!(text.contains("alpha  select  x,y"));
    assert_eq!(
        env.json_err(&["attr", "list", "-p", "NOPE"]),
        "no such project: NOPE"
    );
}

#[test]
fn rm_deletes_attribute_and_values() {
    let env = env();
    env.json(&["attr", "add", "note", "-p", "APM", "--type", "text"]);
    let conn = env.conn();
    conn.execute_batch(
        "INSERT INTO items (project_id, seq, title, created_by) VALUES (1, 1, 't', 'x');
         INSERT INTO item_values (item_id, attribute_id, value)
             SELECT 1, id, 'hello' FROM attributes WHERE name = 'note';",
    )
    .unwrap();
    let v = env.json(&["attr", "rm", "note", "-p", "apm"]);
    assert_eq!(v["name"], "note");
    assert_eq!(env.json(&["attr", "list", "-p", "APM"]), json!([]));
    let left: i64 = conn
        .query_row("SELECT count(*) FROM item_values", [], |r| r.get(0))
        .unwrap();
    assert_eq!(left, 0);
    assert_eq!(
        env.json_err(&["attr", "rm", "note", "-p", "APM"]),
        "no such attribute: note"
    );
}

#[test]
fn rm_prints_one_line() {
    let env = env();
    env.json(&["attr", "add", "note", "-p", "APM", "--type", "text"]);
    let out = env
        .apm()
        .args(["attr", "rm", "note", "-p", "APM"])
        .assert()
        .success();
    assert_eq!(
        String::from_utf8_lossy(&out.get_output().stdout).trim(),
        "removed attribute note"
    );
}

#[test]
fn errors_are_text_without_json() {
    let env = env();
    let out = env
        .apm()
        .args(["attr", "rm", "nope", "-p", "APM"])
        .assert()
        .code(1);
    assert_eq!(
        String::from_utf8_lossy(&out.get_output().stderr).trim(),
        "error: no such attribute: nope"
    );
}
