mod common;

use common::Env;

/// Project APM with task APM-1.
fn seed(env: &Env) {
    env.json(&["project", "add", "APM", "agent-pm"]);
    env.json(&["add", "a task", "-p", "APM"]);
}

#[test]
fn comment_returns_the_comment() {
    let env = Env::new();
    seed(&env);
    let v = env.json(&["comment", "apm-1", "looks good"]);
    assert!(v["id"].is_i64());
    assert_eq!(v["item"], "APM-1");
    assert_eq!(v["author"], "tester");
    assert_eq!(v["kind"], "note");
    assert_eq!(v["answers"], serde_json::Value::Null);
    assert_eq!(v["body"], "looks good");
    assert!(v["created_at"].is_string());

    let v = env.json(&["--as", "bot", "comment", "APM-1", "second"]);
    assert_eq!(v["author"], "bot");

    let comments = &env.json(&["show", "APM-1"])["comments"];
    let rows: Vec<(&str, &str)> = comments
        .as_array()
        .unwrap()
        .iter()
        .map(|c| (c["author"].as_str().unwrap(), c["body"].as_str().unwrap()))
        .collect();
    assert_eq!(rows, [("tester", "looks good"), ("bot", "second")]);
}

#[test]
fn comment_text_output() {
    let env = Env::new();
    seed(&env);
    env.apm()
        .args(["comment", "apm-1", "hi"])
        .assert()
        .success()
        .stdout("commented on APM-1\n");
}

#[test]
fn comment_errors() {
    let env = Env::new();
    seed(&env);
    assert_eq!(
        env.json_err(&["comment", "APM-1", "  "]),
        "comment text cannot be empty"
    );
    assert_eq!(
        env.json_err(&["comment", "APM-9", "x"]),
        "no such item: APM-9"
    );
    assert_eq!(
        env.json_err(&["comment", "NOPE-1", "x"]),
        "no such project: NOPE"
    );
    assert!(
        env.json_err(&["comment", "APM", "x"])
            .starts_with("invalid item reference")
    );
    env.apm()
        .args(["--json", "comment", "APM-1"])
        .assert()
        .code(2);
    env.apm()
        .args(["comment", "APM-1", ""])
        .assert()
        .code(1)
        .stderr("error: comment text cannot be empty\n");
}

#[test]
fn comment_bumps_item_updated_at() {
    let env = Env::new();
    seed(&env);
    env.conn()
        .execute("UPDATE items SET updated_at = '2000-01-01T00:00:00Z'", [])
        .unwrap();
    env.json(&["comment", "APM-1", "ping"]);
    let updated: String = env
        .conn()
        .query_row("SELECT updated_at FROM items", [], |r| r.get(0))
        .unwrap();
    assert_ne!(updated, "2000-01-01T00:00:00Z");
}

#[test]
fn comment_from_stdin_or_a_file() {
    let env = Env::new();
    seed(&env);
    let v = env.json_stdin(&["comment", "APM-1", "-"], "**done**\n\n- tests pass\n");
    assert_eq!(v["body"], "**done**\n\n- tests pass");
    let path = env.dir.path().join("note.md");
    std::fs::write(&path, "from a file\n").unwrap();
    let v = env.json(&["comment", "APM-1", "--file", path.to_str().unwrap()]);
    assert_eq!(v["body"], "from a file");
    let bodies: Vec<serde_json::Value> = env.json(&["show", "APM-1"])["comments"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["body"].clone())
        .collect();
    assert_eq!(bodies, ["**done**\n\n- tests pass", "from a file"]);
}

#[test]
fn comment_input_errors() {
    let env = Env::new();
    seed(&env);
    // Neither text nor --file, or both: usage errors.
    env.apm()
        .args(["--json", "comment", "APM-1"])
        .assert()
        .code(2);
    env.apm()
        .args(["--json", "comment", "APM-1", "x", "--file", "f.md"])
        .assert()
        .code(2);
    let out = env
        .apm()
        .args(["--json", "comment", "APM-1", "-"])
        .write_stdin("\n\n")
        .assert()
        .code(1);
    let v: serde_json::Value = serde_json::from_slice(&out.get_output().stderr).unwrap();
    assert_eq!(v["error"], "comment text cannot be empty");
    assert!(
        env.json_err(&["comment", "APM-1", "--file", "no/such.md"])
            .starts_with("cannot read no/such.md: ")
    );
    assert_eq!(
        env.json(&["show", "APM-1"])["comments"],
        serde_json::json!([])
    );
}

/// Project APM with tasks APM-1 and APM-2, a question on each; returns their ids.
fn seed_questions(env: &Env) -> (i64, i64) {
    seed(env);
    env.json(&["add", "other task", "-p", "APM"]);
    let q1 = env.json(&["comment", "APM-1", "--kind", "question", "WAL on NFS?"]);
    let q2 = env.json(&["comment", "APM-2", "--kind", "question", "retry too?"]);
    (q1["id"].as_i64().unwrap(), q2["id"].as_i64().unwrap())
}

#[test]
fn comment_kinds() {
    let env = Env::new();
    let (q, _) = seed_questions(&env);
    let h = env.json_stdin(
        &["comment", "APM-1", "--kind", "handoff", "-"],
        "State: done\nNext: test\n",
    );
    assert_eq!(h["kind"], "handoff");
    assert_eq!(h["answers"], serde_json::Value::Null);
    assert_eq!(h["body"], "State: done\nNext: test");
    let n = env.json(&["comment", "APM-1", "--kind", "note", "plain"]);
    assert_eq!(n["kind"], "note");

    // --answer implies the kind; --kind answer with it says the same.
    let q = q.to_string();
    let a = env.json(&["--as", "bob", "comment", "APM-1", "--answer", &q, "no"]);
    assert_eq!(a["kind"], "answer");
    assert_eq!(a["answers"], q.parse::<i64>().unwrap());
    assert_eq!(a["author"], "bob");
    let a2 = env.json(&[
        "comment", "apm-1", "--kind", "answer", "--answer", &q, "agreed",
    ]);
    assert_eq!(a2["kind"], "answer");
    // A question may get several answers.
    assert_eq!(a2["answers"], a["answers"]);

    // show returns the same objects, oldest first.
    let comments = env.json(&["show", "APM-1"])["comments"].clone();
    let kinds: Vec<&str> = comments
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["question", "handoff", "note", "answer", "answer"]);
    assert_eq!(comments[3], a);
}

#[test]
fn comment_kind_text_output() {
    let env = Env::new();
    let (q, _) = seed_questions(&env);
    let run = |args: &[&str], out: String| {
        env.apm().args(args).assert().success().stdout(out);
    };
    run(
        &["comment", "APM-1", "--kind", "handoff", "h"],
        "handoff on APM-1\n".into(),
    );
    run(
        &["comment", "APM-1", "--kind", "note", "n"],
        "commented on APM-1\n".into(),
    );
    run(
        &["comment", "APM-1", "--kind", "question", "?"],
        format!("asked on APM-1 (#{})\n", q + 4),
    );
    run(
        &["comment", "APM-1", "--answer", &q.to_string(), "a"],
        format!("answered #{q} on APM-1\n"),
    );
}

#[test]
fn show_marks_log_kinds() {
    let env = Env::new();
    let (q, _) = seed_questions(&env);
    env.json(&["comment", "APM-1", "a note"]);
    env.json(&["comment", "APM-1", "--kind", "handoff", "State: x\nNext: y"]);
    env.json(&["comment", "APM-1", "--answer", &q.to_string(), "yes"]);
    let out = env.apm().args(["show", "APM-1"]).assert().success();
    let text = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    let log: Vec<String> = text
        .split("\ncomments:\n")
        .nth(1)
        .unwrap()
        .lines()
        .map(|l| {
            // Drop the timestamp: `  <time> <rest>`.
            let l = l.trim_start();
            match l.split_once(' ') {
                Some((t, rest)) if t.ends_with('Z') => rest.to_string(),
                _ => l.to_string(),
            }
        })
        .collect();
    assert_eq!(
        log,
        [
            format!("tester [question #{q}]: WAL on NFS?"),
            "tester: a note".to_string(),
            "tester [handoff]: State: x".to_string(),
            "Next: y".to_string(),
            format!("tester [answer #{q}]: yes"),
        ]
    );
}

#[test]
fn answer_errors() {
    let env = Env::new();
    let (q1, q2) = seed_questions(&env);
    let note = env.json(&["comment", "APM-1", "a note"])["id"]
        .as_i64()
        .unwrap();
    let handoff = env.json(&["comment", "APM-1", "--kind", "handoff", "h"])["id"]
        .as_i64()
        .unwrap();
    let answer = env.json(&["comment", "APM-1", "--answer", &q1.to_string(), "a"])["id"]
        .as_i64()
        .unwrap();
    let before = env.json(&["show", "APM-1"])["comments"].clone();
    let ans =
        |task: &str, id: i64| env.json_err(&["comment", task, "--answer", &id.to_string(), "x"]);

    assert_eq!(
        ans("APM-1", note),
        format!("comment #{note} is not a question (it is a note)")
    );
    assert_eq!(
        ans("APM-1", handoff),
        format!("comment #{handoff} is not a question (it is a handoff)")
    );
    assert_eq!(
        ans("APM-1", answer),
        format!("comment #{answer} is not a question (it is an answer)")
    );
    assert_eq!(
        ans("APM-1", q2),
        format!("comment #{q2} is on APM-2, not APM-1; answer it there")
    );
    assert_eq!(ans("APM-1", 999), "no such comment: #999");
    assert_eq!(ans("APM-9", q1), "no such item: APM-9");
    assert_eq!(
        env.json_err(&[
            "comment",
            "APM-1",
            "--kind",
            "handoff",
            "--answer",
            &q1.to_string(),
            "x"
        ]),
        "--answer makes the comment an answer, not a handoff; drop --kind"
    );
    // Usage errors: --kind answer needs --answer; unknown kinds; a non-numeric id.
    for args in [
        &["--json", "comment", "APM-1", "--kind", "answer", "x"][..],
        &["--json", "comment", "APM-1", "--kind", "todo", "x"],
        &["--json", "comment", "APM-1", "--answer", "#3", "x"],
    ] {
        env.apm().args(args).assert().code(2);
    }
    env.apm()
        .args(["comment", "APM-1", "--answer", "999", "x"])
        .assert()
        .code(1)
        .stderr("error: no such comment: #999\n");
    // Nothing was written.
    assert_eq!(env.json(&["show", "APM-1"])["comments"], before);
}

#[test]
fn comment_events_record_the_kind() {
    let env = Env::new();
    let (q, _) = seed_questions(&env);
    env.json(&["comment", "APM-1", "plain"]);
    env.json(&["comment", "APM-1", "--answer", &q.to_string(), "yes"]);
    env.json(&["comment", "APM-1", "--kind", "handoff", "bye"]);
    let events: Vec<(serde_json::Value, serde_json::Value)> = env
        .json(&["history", "APM-1"])
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["kind"] == "commented")
        .map(|e| (e["field"].clone(), e["new"].clone()))
        .collect();
    let s = |v: &str| serde_json::Value::from(v);
    let null = serde_json::Value::Null;
    assert_eq!(
        events,
        [
            (s("question"), s("WAL on NFS?")),
            (null, s("plain")),
            (s("answer"), s("yes")),
            (s("handoff"), s("bye")),
        ]
    );
}

#[test]
fn comment_help_says_what_each_kind_is_for() {
    let env = Env::new();
    let out = env.apm().args(["comment", "--help"]).assert().success();
    let help = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    for line in [
        "What the next agent needs to pick this task up: state, next steps",
        "Something you could not settle; it stays open until someone answers it",
        "The answer to a question on this task",
        "--answer <ID>",
    ] {
        assert!(help.contains(line), "{line} not in:\n{help}");
    }
}
