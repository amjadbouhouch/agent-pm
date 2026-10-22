mod common;

use common::Env;
use serde_json::{Value, json};

fn ids(v: &Value) -> Vec<String> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].as_str().unwrap().to_string())
        .collect()
}

fn env_apm() -> Env {
    let env = Env::new();
    env.json(&["project", "add", "APM", "Agent PM"]);
    env.json(&["project", "add", "OPS", "Ops"]);
    env
}

#[test]
fn add_normalizes_and_sorts_tags() {
    let env = env_apm();
    let t = env.json(&[
        "add",
        "t",
        "-p",
        "APM",
        "--tag",
        " UI ",
        "--tag",
        "area:db.v2_x-y",
        "--tag",
        "ui",
    ]);
    assert_eq!(t["tags"], json!(["area:db.v2_x-y", "ui"]));
    let plain = env.json(&["add", "u", "-p", "APM"]);
    assert_eq!(plain["tags"], json!([]));
    assert_eq!(
        env.json(&["show", "APM-1"])["tags"],
        json!(["area:db.v2_x-y", "ui"])
    );
    assert_eq!(
        env.json(&["list"])[0]["tags"],
        json!(["area:db.v2_x-y", "ui"])
    );
}

#[test]
fn bad_tags_are_refused() {
    let env = env_apm();
    let long = "a".repeat(41);
    for bad in ["", "  ", "a b", "café", "x/y", "#x", long.as_str()] {
        assert_eq!(
            env.json_err(&["add", "t", "-p", "APM", "--tag", bad]),
            format!("invalid tag `{bad}`: 1-40 of a-z, 0-9, _ : . -")
        );
    }
    // Nothing was created.
    assert_eq!(env.json(&["list", "--all"]), json!([]));
    env.json(&["add", "t", "-p", "APM", "--tag", &"a".repeat(40)]);
    assert!(
        env.json_err(&["edit", "APM-1", "--tag", "a b"])
            .starts_with("invalid tag")
    );
    assert!(
        env.json_err(&["edit", "APM-1", "--untag", "a b"])
            .starts_with("invalid tag")
    );
    assert!(
        env.json_err(&["list", "--tag", "a b"])
            .starts_with("invalid tag")
    );
}

#[test]
fn edit_tags_and_untags() {
    let env = env_apm();
    env.json(&["add", "t", "-p", "APM", "--tag", "a", "--tag", "b"]);
    let t = env.json(&["edit", "APM-1", "--tag", "C", "--untag", "A"]);
    assert_eq!(t["tags"], json!(["b", "c"]));
    // Adding a present tag or removing an absent one changes nothing.
    let t = env.json(&["edit", "APM-1", "--tag", "b", "--untag", "zzz"]);
    assert_eq!(t["tags"], json!(["b", "c"]));
    assert_eq!(
        env.json_err(&["edit", "APM-1", "--tag", "x", "--untag", "X"]),
        "cannot both tag and untag x"
    );
    assert_eq!(env.json(&["show", "APM-1"])["tags"], json!(["b", "c"]));
}

#[test]
fn show_text_prints_tags() {
    let env = env_apm();
    env.json(&["add", "t", "-p", "APM", "--tag", "b", "--tag", "a"]);
    let out = env.apm().args(["show", "APM-1"]).assert().success();
    let text = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    assert!(text.contains("\ntags: a, b\n"), "{text}");
}

#[test]
fn list_by_tags_all_must_match() {
    let env = env_apm();
    env.json(&["add", "a", "-p", "APM", "--tag", "bug", "--tag", "ui"]);
    env.json(&["add", "b", "-p", "APM", "--tag", "bug"]);
    env.json(&["add", "c", "-p", "OPS", "--tag", "ui"]);
    assert_eq!(
        ids(&env.json(&["list", "--tag", "BUG"])),
        ["APM-1", "APM-2"]
    );
    assert_eq!(ids(&env.json(&["list", "--tag", "ui"])), ["APM-1", "OPS-1"]);
    assert_eq!(
        ids(&env.json(&["list", "--tag", "bug", "--tag", "ui"])),
        ["APM-1"]
    );
    assert_eq!(
        ids(&env.json(&["list", "--tag", "ui", "-p", "OPS"])),
        ["OPS-1"]
    );
    assert_eq!(env.json(&["list", "--tag", "nope"]), json!([]));
    // Closed tasks stay hidden unless --all.
    env.json(&["done", "APM-2"]);
    assert_eq!(ids(&env.json(&["list", "--tag", "bug"])), ["APM-1"]);
    assert_eq!(
        ids(&env.json(&["list", "--tag", "bug", "--all"])),
        ["APM-1", "APM-2"]
    );
}

#[test]
fn tags_command_counts() {
    let env = env_apm();
    assert_eq!(env.json(&["tags"]), json!([]));
    env.json(&["add", "a", "-p", "APM", "--tag", "ui", "--tag", "bug"]);
    env.json(&["add", "b", "-p", "APM", "--tag", "bug", "--tag", "api"]);
    env.json(&["add", "c", "-p", "OPS", "--tag", "ui", "--tag", "zz"]);
    env.json(&["add", "d", "-p", "OPS", "--tag", "ui"]);
    // Count desc, then name.
    assert_eq!(
        env.json(&["tags"]),
        json!([
            {"tag": "ui", "count": 3},
            {"tag": "bug", "count": 2},
            {"tag": "api", "count": 1},
            {"tag": "zz", "count": 1},
        ])
    );
    assert_eq!(
        env.json(&["tags", "-p", "apm"]),
        json!([
            {"tag": "bug", "count": 2},
            {"tag": "api", "count": 1},
            {"tag": "ui", "count": 1},
        ])
    );
    assert_eq!(
        env.json_err(&["tags", "-p", "NOPE"]),
        "no such project: NOPE"
    );
    env.apm()
        .args(["tags", "-p", "OPS"])
        .assert()
        .success()
        .stdout("TAG  COUNT\nui   2\nzz   1\n");
}

#[test]
fn tag_events() {
    let env = env_apm();
    env.json(&[
        "--as", "alice", "add", "t", "-p", "APM", "--tag", "b", "--tag", "a",
    ]);
    env.json(&[
        "--as", "bob", "edit", "APM-1", "--tag", "c", "--untag", "a", "--untag", "nope",
    ]);
    env.json(&["edit", "APM-1", "--tag", "c"]);
    let h = env.json(&["history", "APM-1"]);
    let got: Vec<(String, String, Value, Value)> = h
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            (
                e["actor"].as_str().unwrap().to_string(),
                e["kind"].as_str().unwrap().to_string(),
                e["old"].clone(),
                e["new"].clone(),
            )
        })
        .collect();
    let e = |a: &str, k: &str, o: Value, n: Value| (a.to_string(), k.to_string(), o, n);
    assert_eq!(
        got,
        [
            e("alice", "created", Value::Null, json!("t")),
            e("alice", "tagged", Value::Null, json!("a")),
            e("alice", "tagged", Value::Null, json!("b")),
            e("bob", "tagged", Value::Null, json!("c")),
            e("bob", "untagged", json!("a"), Value::Null),
        ]
    );
}
