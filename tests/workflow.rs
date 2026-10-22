//! Cross-slice tests: projects, tasks, milestones, comments and attributes together.

mod common;

use common::Env;
use serde_json::{Value, json};

/// The `id`s of a JSON array of items.
fn ids(v: &Value) -> Vec<&str> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].as_str().unwrap())
        .collect()
}

/// An env with project APM.
fn env_apm() -> Env {
    let env = Env::new();
    env.json(&["project", "add", "APM", "agent-pm"]);
    env
}

/// Split a guide example line into args: whitespace-separated, `"..."` quoted, `#` starts a
/// comment.
fn shell_words(line: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let (mut quoted, mut started) = (false, false);
    for c in line.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                started = true;
            }
            '#' if !quoted && !started => break,
            c if c.is_whitespace() && !quoted => {
                if started {
                    words.push(std::mem::take(&mut word));
                    started = false;
                }
            }
            c => {
                word.push(c);
                started = true;
            }
        }
    }
    assert!(!quoted, "unbalanced quote in guide line: {line}");
    if started {
        words.push(word);
    }
    words
}

/// The example lines of a guide section, as args without the leading `apm`. The section runs
/// from its heading to the next unindented line; every non-blank line in it must be a command.
fn guide_examples(guide: &str, section: &str) -> Vec<Vec<String>> {
    let mut lines = guide.lines().skip_while(|l| *l != section);
    assert!(lines.next().is_some(), "guide has no {section} section");
    let examples: Vec<Vec<String>> = lines
        .take_while(|l| l.is_empty() || l.starts_with(' '))
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let words = shell_words(l);
            assert_eq!(
                words.first().map(String::as_str),
                Some("apm"),
                "{section}: {l}"
            );
            words.into_iter().skip(1).collect()
        })
        .collect();
    assert!(!examples.is_empty(), "{section} has no examples");
    examples
}

#[test]
fn shell_words_splits_like_the_guide_means() {
    assert_eq!(
        shell_words(r#"  apm add "a b" -p APM   # note"#),
        ["apm", "add", "a b", "-p", "APM"]
    );
    assert_eq!(shell_words(r#"apm x "" y"#), ["apm", "x", "", "y"]);
}

#[test]
fn every_guide_example_runs_in_order() {
    let env = Env::new();
    let guide = env.json(&["guide"])["guide"].as_str().unwrap().to_string();
    let workflow = guide_examples(&guide, "WORKFLOW");
    let attrs = guide_examples(&guide, "CUSTOM ATTRIBUTES");
    for args in workflow.iter().chain(&attrs) {
        let out = env.apm().arg("--json").args(args).output().unwrap();
        assert!(
            out.status.success(),
            "apm {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice::<Value>(&out.stdout)
            .unwrap_or_else(|e| panic!("apm {}: stdout is not JSON: {e}", args.join(" ")));
    }
}

#[test]
fn full_workflow() {
    let env = Env::new();
    env.json(&["project", "add", "APM", "agent-pm", "-d", "the tracker"]);
    env.json(&["milestone", "add", "v1", "-p", "APM", "--due", "2026-12-01"]);
    let t = env.json(&[
        "add",
        "parse args",
        "-p",
        "APM",
        "-m",
        "v1",
        "-a",
        "alice",
        "--priority",
        "high",
        "--due",
        "2026-11-01",
    ]);
    assert_eq!(t["id"], "APM-1");
    assert_eq!(t["milestone"], "v1");
    let sub = env.json(&["add", "flag tests", "--parent", "APM-1"]);
    assert_eq!(sub["id"], "APM-2");
    assert_eq!(sub["parent"], "APM-1");

    assert_eq!(env.json(&["start", "APM-1"])["status"], "doing");
    env.json(&["--as", "bob", "comment", "APM-1", "clap derive works"]);
    assert_eq!(env.json(&["done", "APM-2"])["status"], "done");

    // List filters.
    assert_eq!(ids(&env.json(&["list", "-p", "APM"])), ["APM-1"]);
    assert_eq!(
        ids(&env.json(&["list", "-p", "APM", "-a", "alice"])),
        ["APM-1"]
    );
    assert_eq!(
        ids(&env.json(&["list", "-p", "APM", "--status", "done,cancelled"])),
        ["APM-2"]
    );
    assert_eq!(ids(&env.json(&["list", "-m", "v1", "--all"])), ["APM-1"]);
    assert_eq!(
        ids(&env.json(&["list", "--parent", "APM-1", "--all"])),
        ["APM-2"]
    );

    // Show: fields, subtasks, comments.
    let v = env.json(&["show", "APM-1"]);
    assert_eq!(v["status"], "doing");
    assert_eq!(v["due"], "2026-11-01");
    assert_eq!(ids(&v["subtasks"]), ["APM-2"]);
    assert_eq!(v["comments"][0]["author"], "bob");
    assert_eq!(v["comments"][0]["body"], "clap derive works");

    // Edit with --unset.
    let v = env.json(&["edit", "APM-1", "--priority", "urgent", "--unset", "due"]);
    assert_eq!(v["priority"], "urgent");
    assert_eq!(v["due"], Value::Null);

    // Milestone progress counts the milestone's tasks (the subtask has none).
    let m = env.json(&["milestone", "list", "-p", "APM"]);
    assert_eq!(
        m[0]["progress"],
        json!({"total": 1, "done": 0, "cancelled": 0})
    );
    env.json(&["done", "APM-1"]);
    let m = env.json(&["milestone", "list", "-p", "APM"]);
    assert_eq!(m[0]["progress"]["done"], 1);

    // Custom attributes: --set, --where, --unset.
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
    env.json(&["attr", "add", "points", "-p", "APM", "--type", "number"]);
    let t = env.json(&[
        "add",
        "cache layer",
        "-p",
        "APM",
        "--set",
        "size=m",
        "--set",
        "points=3",
    ]);
    assert_eq!(t["id"], "APM-3");
    assert_eq!(t["attributes"], json!({"size": "m", "points": 3}));
    env.json(&["add", "small one", "-p", "APM", "--set", "size=s"]);
    assert_eq!(
        ids(&env.json(&["list", "-p", "APM", "--where", "size=m"])),
        ["APM-3"]
    );
    assert_eq!(
        ids(&env.json(&["list", "--where", "points=3.0"])),
        ["APM-3"]
    );
    let v = env.json(&["edit", "APM-3", "--unset", "points", "--set", "size=l"]);
    assert_eq!(v["attributes"], json!({"size": "l"}));
    assert_eq!(
        ids(&env.json(&["list", "-p", "APM", "--where", "points=3"])),
        Vec::<&str>::new()
    );

    // Close the milestone.
    let m = env.json(&["milestone", "close", "v1", "-p", "APM"]);
    assert_eq!(m["status"], "closed");
    assert_eq!(env.json(&["milestone", "list", "-p", "APM"]), json!([]));
}

#[test]
fn list_where_errors() {
    let env = env_apm();
    env.json(&["attr", "add", "points", "-p", "APM", "--type", "number"]);
    assert_eq!(
        env.json_err(&["list", "-p", "APM", "--where", "nope=1"]),
        "unknown attribute: nope"
    );
    assert_eq!(
        env.json_err(&["list", "--where", "nope=1"]),
        "unknown attribute: nope"
    );
    assert_eq!(
        env.json_err(&["list", "-p", "APM", "--where", "points=lots"]),
        "invalid value for points: expected a number, got `lots`"
    );
    env.apm()
        .args(["--json", "list", "--where", "novalue"])
        .assert()
        .code(2);
}

#[test]
fn add_with_bad_set_creates_nothing_and_uses_no_number() {
    let env = env_apm();
    env.json(&["attr", "add", "points", "-p", "APM", "--type", "number"]);
    assert_eq!(
        env.json_err(&["add", "x", "-p", "APM", "--set", "points=abc"]),
        "invalid value for points: expected a number, got `abc`"
    );
    assert_eq!(
        env.json_err(&["add", "x", "-p", "APM", "--set", "nope=1"]),
        "unknown attribute: nope"
    );
    assert_eq!(env.json(&["list", "--all"]), json!([]));
    assert_eq!(env.json(&["add", "y", "-p", "APM"])["id"], "APM-1");
}

#[test]
fn show_has_comments_and_typed_attributes() {
    let env = env_apm();
    for (name, kind) in [
        ("points", "number"),
        ("ratio", "number"),
        ("flag", "checkbox"),
        ("when", "date"),
        ("note", "text"),
    ] {
        env.json(&["attr", "add", name, "-p", "APM", "--type", kind]);
    }
    env.json(&[
        "add",
        "t",
        "-p",
        "APM",
        "--set",
        "points=5",
        "--set",
        "ratio=0.5",
        "--set",
        "flag=yes",
        "--set",
        "when=2026-10-22",
        "--set",
        "note=42",
    ]);
    env.json(&["comment", "APM-1", "first"]);
    env.json(&["--as", "bob", "comment", "APM-1", "second"]);
    let v = env.json(&["show", "APM-1"]);
    assert_eq!(
        v["attributes"],
        json!({"points": 5, "ratio": 0.5, "flag": true, "when": "2026-10-22", "note": "42"})
    );
    assert!(v["attributes"]["points"].is_i64());
    let comments: Vec<(&str, &str)> = v["comments"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| (c["author"].as_str().unwrap(), c["body"].as_str().unwrap()))
        .collect();
    assert_eq!(comments, [("tester", "first"), ("bob", "second")]);
    // The same values in `list`.
    assert_eq!(env.json(&["list"])[0]["attributes"], v["attributes"]);

    let out = env.apm().args(["show", "APM-1"]).assert().success();
    let text = String::from_utf8_lossy(&out.get_output().stdout).to_string();
    assert!(text.contains("attr points=5\n"));
    assert!(text.contains("attr flag=true\n"));
    assert!(text.contains("bob: second\n"));
}

#[test]
fn milestone_rm_leaves_tasks_without_milestone() {
    let env = env_apm();
    env.json(&["milestone", "add", "v1", "-p", "APM"]);
    env.json(&["add", "a", "-p", "APM", "-m", "v1"]);
    env.json(&["milestone", "rm", "v1", "-p", "APM"]);
    let v = env.json(&["show", "APM-1"]);
    assert_eq!(v["milestone"], Value::Null);
    assert_eq!(
        env.json_err(&["list", "-p", "APM", "-m", "v1"]),
        "no such milestone: v1"
    );
}

#[test]
fn project_rm_cascades_everything() {
    let env = env_apm();
    env.json(&["project", "add", "WEB", "web"]);
    for key in ["APM", "WEB"] {
        env.json(&["milestone", "add", "v1", "-p", key]);
        env.json(&["attr", "add", "points", "-p", key, "--type", "number"]);
        env.json(&["add", "t", "-p", key, "-m", "v1", "--set", "points=1"]);
        env.json(&["add", "sub", "--parent", &format!("{key}-1")]);
        env.json(&["comment", &format!("{key}-1"), "hi"]);
    }
    env.json(&["project", "rm", "APM"]);

    assert_eq!(ids(&env.json(&["list", "--all"])), ["WEB-1", "WEB-2"]);
    // Only WEB's rows are left in every table.
    let conn = env.conn();
    for table in [
        "milestones",
        "attributes",
        "items",
        "item_values",
        "comments",
    ] {
        let n: i64 = conn
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .unwrap();
        let want = if table == "items" { 2 } else { 1 };
        assert_eq!(n, want, "{table}");
    }
    // A new APM starts empty, numbered from 1.
    env.json(&["project", "add", "APM", "again"]);
    assert_eq!(
        env.json(&["milestone", "list", "-p", "APM", "--all"]),
        json!([])
    );
    assert_eq!(env.json(&["attr", "list", "-p", "APM"]), json!([]));
    assert_eq!(env.json(&["add", "new", "-p", "APM"])["id"], "APM-1");
}

#[test]
fn attr_rm_removes_values_from_show() {
    let env = env_apm();
    env.json(&["attr", "add", "points", "-p", "APM", "--type", "number"]);
    env.json(&["attr", "add", "flag", "-p", "APM", "--type", "checkbox"]);
    env.json(&[
        "add", "t", "-p", "APM", "--set", "points=2", "--set", "flag=no",
    ]);
    env.json(&["attr", "rm", "points", "-p", "APM"]);
    assert_eq!(
        env.json(&["show", "APM-1"])["attributes"],
        json!({"flag": false})
    );
    assert_eq!(
        env.json_err(&["list", "--where", "points=2"]),
        "unknown attribute: points"
    );
}

#[test]
fn guide_examples_reads_whole_sections() {
    let guide = "A\n  apm one\n\n  apm two  # x\nB\n  apm three\n";
    assert_eq!(guide_examples(guide, "A"), [vec!["one"], vec!["two"]]);
}

#[test]
#[should_panic(expected = "A:   note")]
fn guide_examples_rejects_non_commands() {
    guide_examples("A\n  apm one\n  note\n", "A");
}
