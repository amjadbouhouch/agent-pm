mod common;

use common::Env;
use serde_json::{Value, json};

/// Project APM with APM-1..APM-3; APM-4 is a subtask of APM-3.
fn seed(env: &Env) {
    env.json(&["project", "add", "APM", "agent-pm", "-d", "the tracker"]);
    env.json(&["add", "one", "-p", "APM"]);
    env.json(&["add", "two", "-p", "APM"]);
    env.json(&[
        "add",
        "Retry on SQLITE_BUSY",
        "-p",
        "APM",
        "-b",
        "Agents fail at once.",
    ]);
    env.json(&["add", "test WAL race", "--parent", "APM-3"]);
}

fn text(env: &Env, args: &[&str]) -> String {
    let out = env.apm().args(args).assert().success();
    String::from_utf8(out.get_output().stdout.clone()).unwrap()
}

fn ids(v: &Value) -> Vec<String> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|x| x["id"].as_str().unwrap().to_string())
        .collect()
}

/// Agent A works APM-3 (logs, asks, hands off, writes a page); agent B, who only knows the
/// task id, gets all of it from one call.
#[test]
fn a_second_agent_gets_what_the_first_learned() {
    let env = Env::new();
    seed(&env);
    let a = |args: &[&str]| env.json(&[&["--as", "alice"], args].concat());
    a(&["claim", "APM-3"]);
    a(&[
        "comment",
        "APM-3",
        "busy_timeout does not cover the WAL switch",
    ]);
    let q = a(&[
        "comment",
        "APM-4",
        "--kind",
        "question",
        "Does WAL work on NFS?",
    ]);
    a(&[
        "comment",
        "APM-3",
        "--kind",
        "question",
        "Should migrate retry too?",
    ]);
    env.json_stdin(
        &[
            "--as", "alice", "comment", "APM-3", "--kind", "handoff", "-",
        ],
        "State: retry loop in db::open done.\nNext: test in tests/migrate.rs.\n",
    );
    a(&["comment", "APM-3", "after the handoff"]);
    a(&[
        "page",
        "add",
        "APM:storage",
        "Storage",
        "-s",
        "SQLite file, WAL, one writer per tx",
        "-b",
        "long text",
        "--link",
        "APM-3",
    ]);
    a(&[
        "page",
        "add",
        "APM:conventions",
        "Conventions",
        "-b",
        "errors lowercase",
        "--pin",
    ]);
    a(&["page", "add", "APM:release", "Release", "-b", "unrelated"]);
    a(&[
        "comment",
        "APM-4",
        "--answer",
        &q["id"].to_string(),
        "No: rollback journal",
    ]);

    // B: a fresh actor that knows only APM-3.
    let c = env.json(&["--as", "bob", "context", "APM-3"]);
    assert_eq!(c["task"]["id"], "APM-3");
    assert_eq!(c["task"]["body"], "Agents fail at once.");
    assert_eq!(c["task"]["claim"]["by"], "alice");
    assert_eq!(c["handoff"]["kind"], "handoff");
    assert_eq!(c["handoff"]["author"], "alice");
    assert_eq!(
        c["handoff"]["body"],
        "State: retry loop in db::open done.\nNext: test in tests/migrate.rs."
    );
    // The log: every other entry of the task (not the handoff, not the subtask's).
    let log: Vec<&str> = c["log"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["body"].as_str().unwrap())
        .collect();
    assert_eq!(
        log,
        [
            "busy_timeout does not cover the WAL switch",
            "after the handoff"
        ]
    );
    assert_eq!(c["log_total"], 2);
    assert_eq!(c["log"][0]["truncated"], false);
    assert_eq!(c["log"][0]["kind"], "note");
    // Open questions of the task and its subtasks: the answered one is gone.
    let questions: Vec<(&str, &str)> = c["questions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|q| (q["item"].as_str().unwrap(), q["body"].as_str().unwrap()))
        .collect();
    assert_eq!(questions, [("APM-3", "Should migrate retry too?")]);
    assert_eq!(c["more_questions"], 0);
    assert_eq!(
        c["subtasks"],
        json!([{"id": "APM-4", "status": "todo", "title": "test WAL race"}])
    );
    assert_eq!(c["links"]["blocks"], json!([]));
    // Pages: linked first, then pinned; the rest counted.
    assert_eq!(ids(&c["pages"]), ["APM:storage", "APM:conventions"]);
    let p = &c["pages"][0];
    assert_eq!(p["summary"], "SQLite file, WAL, one writer per tx");
    assert_eq!(p["linked"], true);
    assert_eq!(c["pages"][1]["linked"], false);
    assert_eq!(c["pages"][1]["summary"], "errors lowercase");
    assert!(p.get("body").is_none());
    assert_eq!(c["more_pages"], 1);
    let hints: Vec<&str> = c["hints"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h.as_str().unwrap())
        .collect();
    assert_eq!(
        hints,
        [
            "open questions: answer with apm comment APM-3 --answer ID \"...\"",
            "before you stop: apm comment APM-3 --kind handoff - (state, next, decisions + why, where)",
            "learned something that outlives this task? apm page add APM:<slug> \"Title\" -s \"gist\" -b - --link APM-3",
        ]
    );
    // Reading changes nothing.
    assert_eq!(env.json(&["--as", "bob", "context", "APM-3"]), c);

    // The same as text.
    let t = text(&env, &["--as", "bob", "context", "APM-3"]);
    for line in [
        "APM-3  Retry on SQLITE_BUSY\n",
        "status: todo  priority: none\n",
        "claimed: alice until ",
        "\n  Agents fail at once.\n",
        "\nhandoff (",
        "\n  State: retry loop in db::open done.\n  Next: test in tests/migrate.rs.\n",
        "\nlog (2 of 2, oldest first; full: apm show APM-3):\n",
        " alice [question]: Should migrate retry too?\n",
        "\nopen questions (answer: apm comment APM-3 --answer ID TEXT):\n",
        "\nsubtasks:\n  APM-4  todo  test WAL race\n",
        "\npages (full text: apm page show ID):\n",
        "  APM:storage      Storage: SQLite file, WAL, one writer per tx  (linked, v1)\n",
        "  APM:conventions  Conventions: errors lowercase  (pinned, v1)\n",
        "  + 1 more pages in APM: apm page list -p APM, apm search WORDS -p APM\n",
        "\nhints:\n  open questions: ",
    ] {
        assert!(t.contains(line), "{line:?} not in:\n{t}");
    }
}

#[test]
fn a_fresh_task_says_nobody_logged_yet() {
    let env = Env::new();
    seed(&env);
    let c = env.json(&["context", "APM-1"]);
    assert_eq!(c["handoff"], Value::Null);
    assert_eq!(c["log"], json!([]));
    assert_eq!(c["log_total"], 0);
    assert_eq!(c["questions"], json!([]));
    assert_eq!(c["pages"], json!([]));
    assert_eq!(c["more_pages"], 0);
    assert_eq!(
        c["hints"][0],
        "nobody logged on this task yet: apm comment APM-1 \"...\" as you learn things"
    );
    // A question only on a subtask: answer it on that task.
    env.json(&["comment", "APM-4", "--kind", "question", "where?"]);
    let c = env.json(&["context", "APM-3"]);
    assert_eq!(
        c["hints"][1],
        "open questions: answer with apm comment KEY-N --answer ID \"...\""
    );
    let t = text(&env, &["context", "APM-3"]);
    assert!(t.contains("open questions (answer: apm comment KEY-N --answer ID TEXT):\n"));
    assert!(t.contains(" tester [question] on APM-4: where?\n"), "{t}");
    assert_eq!(env.json_err(&["context", "APM-9"]), "no such item: APM-9");
}

#[test]
fn context_without_an_id_is_the_claimed_task() {
    let env = Env::new();
    seed(&env);
    assert_eq!(
        env.json_err(&["context"]),
        "tester holds no claim: pass a task (apm context KEY-N) or a project (apm context -p KEY)"
    );
    env.json(&["claim", "APM-2"]);
    assert_eq!(env.json(&["context"])["task"]["id"], "APM-2");
    // Someone else's claim is not yours.
    assert!(
        env.json_err(&["--as", "bob", "context"])
            .starts_with("bob holds no claim")
    );
    env.json(&["claim", "APM-3"]);
    assert_eq!(
        env.json_err(&["context"]),
        "tester holds 2 claims (APM-2, APM-3): pass one (apm context KEY-N)"
    );
    // A released or closed claim no longer counts.
    env.json(&["release", "APM-2"]);
    env.json(&["done", "APM-3"]);
    assert!(
        env.json_err(&["context"])
            .starts_with("tester holds no claim")
    );
    env.apm()
        .args(["--json", "context", "APM-1", "-p", "APM"])
        .assert()
        .code(2);
}

#[test]
fn the_log_is_recent_and_cut() {
    let env = Env::new();
    seed(&env);
    for n in 1..=7 {
        env.json(&["comment", "APM-1", &format!("note {n}")]);
    }
    let long = format!("{}tail", "x".repeat(300));
    env.json(&["comment", "APM-1", &long]);
    env.json(&["comment", "APM-1", "--kind=handoff", "old handoff"]);
    env.json(&["comment", "APM-1", "--kind", "handoff", "new handoff"]);
    let c = env.json(&["context", "APM-1"]);
    assert_eq!(c["handoff"]["body"], "new handoff");
    // 5 newest of the 9 others (the older handoff is a log entry), oldest first.
    let bodies: Vec<String> = c["log"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["body"].as_str().unwrap().chars().take(11).collect())
        .collect();
    assert_eq!(
        bodies,
        ["note 5", "note 6", "note 7", "xxxxxxxxxxx", "old handoff"]
    );
    assert_eq!(c["log_total"], 9);
    let cut = &c["log"][3];
    assert_eq!(cut["truncated"], true);
    assert_eq!(cut["body"], format!("{}...", "x".repeat(280)));
    assert_eq!(c["log"][4]["truncated"], false);
    // --log N.
    let c = env.json(&["context", "APM-1", "--log", "1"]);
    assert_eq!(c["log"].as_array().unwrap().len(), 1);
    assert_eq!(c["log"][0]["body"], "old handoff");
    assert_eq!(
        env.json(&["context", "APM-1", "--log", "0"])["log"],
        json!([])
    );
    assert_eq!(
        env.json(&["context", "APM-1", "--log", "50"])["log"]
            .as_array()
            .unwrap()
            .len(),
        9
    );
    env.apm()
        .args(["--json", "context", "APM-1", "--log", "51"])
        .assert()
        .code(2);
    let t = text(&env, &["context", "APM-1"]);
    assert!(
        t.contains("\nlog (5 of 9, oldest first; full: apm show APM-1):\n"),
        "{t}"
    );
    assert!(
        t.contains(&format!(" tester: {}...\n", "x".repeat(280))),
        "{t}"
    );
}

#[test]
fn task_context_stays_small() {
    let env = Env::new();
    seed(&env);
    // Busy but ordinary: entries of ~180 characters, page summaries of 100.
    let words = "the quick brown fox jumps over the lazy dog ".repeat(4);
    for n in 0..50 {
        let kind = match n % 10 {
            0 => "question",
            5 => "handoff",
            _ => "note",
        };
        env.json(&["comment", "APM-3", "--kind", kind, &format!("{n} {words}")]);
    }
    let summary = "s".repeat(100);
    for n in 0..30 {
        let id = format!("APM:page-{n:02}");
        let mut args = vec![
            "page",
            "add",
            &id,
            "A page title",
            "-s",
            &summary,
            "-b",
            &words,
        ];
        // 4 linked, 3 pinned; the other 23 are only counted.
        if n % 8 == 0 {
            args.extend(["--link", "APM-3"]);
        }
        if n % 10 == 3 {
            args.push("--pin");
        }
        env.json(&args);
    }
    let out = env
        .apm()
        .args(["--json", "context", "APM-3"])
        .assert()
        .success();
    let json = out.get_output().stdout.len();
    let c: Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert_eq!(c["log"].as_array().unwrap().len(), 5);
    assert_eq!(c["log_total"], 44); // 50 - the handoff - 5 open questions
    assert_eq!(c["questions"].as_array().unwrap().len(), 5);
    assert_eq!(c["pages"].as_array().unwrap().len(), 7);
    assert_eq!(c["more_pages"], 23);
    let t = text(&env, &["context", "APM-3"]).len();
    assert!(json < 8 * 1024, "context JSON is {json} bytes");
    assert!(t < 8 * 1024, "context text is {t} bytes");
}

#[test]
fn project_context_for_a_fresh_session() {
    let env = Env::new();
    seed(&env);
    // In flight: APM-1 (claimed, handoff), APM-2 (no handoff), APM-3 (long handoff).
    env.json(&["--as", "alice", "next", "-p", "APM"]);
    env.json(&[
        "--as",
        "alice",
        "comment",
        "APM-1",
        "--kind",
        "handoff",
        "alice's state",
    ]);
    env.json(&["start", "APM-2"]);
    env.json(&["start", "APM-3"]);
    let long = "y".repeat(400);
    env.json(&["comment", "APM-3", "--kind", "handoff", &long]);
    env.json(&["comment", "APM-2", "--kind", "question", "open one"]);
    env.json(&[
        "comment",
        "APM-4",
        "--kind",
        "question",
        "on a todo subtask",
    ]);
    env.json(&["add", "closed", "-p", "APM"]);
    env.json(&["comment", "APM-5", "--kind", "question", "on a closed task"]);
    env.json(&["done", "APM-5"]);
    env.json(&[
        "page",
        "add",
        "APM:conventions",
        "Conventions",
        "-s",
        "errors lowercase",
        "--pin",
    ]);
    env.json(&["page", "add", "APM:storage", "Storage", "--link", "APM-1"]);

    let c = env.json(&["context", "-p", "apm"]);
    assert_eq!(c["project"]["key"], "APM");
    assert_eq!(c["project"]["description"], "the tracker");
    assert_eq!(ids(&c["pages"]), ["APM:conventions"]);
    assert_eq!(c["pages"][0]["linked"], false);
    assert_eq!(c["more_pages"], 1);
    let questions: Vec<&str> = c["questions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|q| q["body"].as_str().unwrap())
        .collect();
    assert_eq!(questions, ["open one", "on a todo subtask"]);
    let tasks = c["tasks"].as_array().unwrap();
    let mut got: Vec<(&str, Option<&str>)> = tasks
        .iter()
        .map(|t| (t["id"].as_str().unwrap(), t["claimed_by"].as_str()))
        .collect();
    got.sort();
    assert_eq!(
        got,
        [("APM-1", Some("alice")), ("APM-2", None), ("APM-3", None)]
    );
    let by_id = |id: &str| tasks.iter().find(|t| t["id"] == id).unwrap();
    assert_eq!(by_id("APM-1")["handoff"]["body"], "alice's state");
    assert_eq!(by_id("APM-2")["handoff"], Value::Null);
    assert_eq!(by_id("APM-3")["handoff"]["truncated"], true);
    assert_eq!(
        by_id("APM-3")["handoff"]["body"],
        format!("{}...", "y".repeat(280))
    );
    assert_eq!(c["more_tasks"], 0);
    assert_eq!(c["more_questions"], 0);
    assert_eq!(
        c["hints"][0],
        "pick up work: apm next -p APM --context (claims the best ready task)"
    );
    let t = text(&env, &["context", "-p", "APM"]);
    for line in [
        "APM  agent-pm\ndescription: the tracker\n",
        "  APM:conventions  Conventions: errors lowercase  (pinned, v1)\n",
        "\nin flight (latest handoff; full: apm context ID):\n",
        "  APM-1  doing  one  claimed by alice\n",
        "    handoff (",
        "      alice's state\n",
        "    no handoff yet\n",
        " tester [question] on APM-2: open one\n",
    ] {
        assert!(t.contains(line), "{line:?} not in:\n{t}");
    }
    assert_eq!(
        env.json_err(&["context", "-p", "NOPE"]),
        "no such project: NOPE"
    );
}

#[test]
fn project_context_stays_small() {
    let env = Env::new();
    env.json(&["project", "add", "APM", "agent-pm"]);
    // Busy but ordinary: entries of ~180 characters, page summaries of 100.
    let words = "the quick brown fox jumps over the lazy dog ".repeat(4);
    for n in 1..=25 {
        env.json(&["add", &format!("task {n}"), "-p", "APM", "-b", &words]);
        env.json(&["start", &format!("APM-{n}")]);
        env.json(&["comment", &format!("APM-{n}"), "--kind", "handoff", &words]);
        env.json(&["comment", &format!("APM-{n}"), "--kind", "question", &words]);
    }
    for n in 0..30 {
        let id = format!("APM:p{n}");
        env.json(&["page", "add", &id, "title", "-s", &"s".repeat(100), "--pin"]);
    }
    let out = env
        .apm()
        .args(["--json", "context", "-p", "APM"])
        .assert()
        .success();
    let size = out.get_output().stdout.len();
    let c: Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert_eq!(c["tasks"].as_array().unwrap().len(), 8);
    assert_eq!(c["more_tasks"], 17);
    assert_eq!(c["questions"].as_array().unwrap().len(), 8);
    assert_eq!(c["more_questions"], 17);
    assert_eq!(c["pages"].as_array().unwrap().len(), 10);
    let shown = c["pages"].as_array().unwrap().len() as i64;
    assert_eq!(shown + c["more_pages"].as_i64().unwrap(), 30);
    let t = text(&env, &["context", "-p", "APM"]).len();
    // Every list at its cap.
    assert!(size < 12 * 1024, "project context JSON is {size} bytes");
    assert!(t < 12 * 1024, "project context text is {t} bytes");
}

#[test]
fn next_with_context() {
    let env = Env::new();
    seed(&env);
    env.json(&["comment", "APM-1", "--kind", "handoff", "where I stopped"]);
    let v = env.json(&["--as", "bot", "next", "-p", "APM", "--context"]);
    let keys: Vec<&String> = v.as_object().unwrap().keys().collect();
    assert_eq!(keys, ["context", "renewed", "task"]);
    assert_eq!(v["task"]["id"], "APM-1");
    assert_eq!(v["renewed"], false);
    assert_eq!(v["context"]["task"]["id"], "APM-1");
    assert_eq!(v["context"]["task"]["claim"]["by"], "bot");
    assert_eq!(v["context"]["handoff"]["body"], "where I stopped");
    // Without --context: the shape as before.
    let v = env.json(&["--as", "bot2", "next", "-p", "APM"]);
    assert!(v.get("context").is_none());
    // Nothing ready: context is null, exit 3.
    for t in ["APM-2", "APM-3", "APM-4"] {
        env.json(&["done", t]);
    }
    let out = env
        .apm()
        .args(["--json", "--as", "bot3", "next", "-p", "APM", "--context"])
        .assert()
        .code(3);
    let v: Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert_eq!(v, json!({"task": null, "renewed": false, "context": null}));
    // Text: the claim line, then the context.
    let t = text(&env, &["--as", "bot", "next", "-p", "APM", "--context"]);
    assert!(t.starts_with("renewed APM-1 until "), "{t}");
    assert!(t.contains("\n\nAPM-1  one\n"), "{t}");
    assert!(t.contains("\nhandoff ("), "{t}");
}

#[test]
fn done_hints_at_an_empty_log() {
    let env = Env::new();
    seed(&env);
    let out = env.apm().args(["done", "APM-1"]).assert().success();
    let o = out.get_output();
    assert_eq!(String::from_utf8_lossy(&o.stdout), "updated APM-1 (done)\n");
    assert_eq!(
        String::from_utf8_lossy(&o.stderr),
        "hint: APM-1 has an empty log; leave what the next agent should know: \
         apm comment APM-1 --kind handoff -\n"
    );
    // Never under --json, and not when the task has a log.
    env.apm()
        .args(["--json", "done", "APM-2"])
        .assert()
        .success()
        .stderr("");
    env.json(&["comment", "APM-3", "done: see tests"]);
    env.apm()
        .args(["done", "APM-3"])
        .assert()
        .success()
        .stderr("");
    // start and block give no hint.
    env.apm()
        .args(["start", "APM-4"])
        .assert()
        .success()
        .stderr("");
}

#[test]
fn context_help_says_what_it_is_for() {
    let env = Env::new();
    let top = text(&env, &["--help"]);
    assert!(
        top.contains("START HERE: what earlier agents learned"),
        "{top}"
    );
    let help = text(&env, &["context", "--help"]);
    for line in [
        "the task you hold a live claim on",
        "for a fresh session",
        "each cut at 280",
    ] {
        assert!(help.contains(line), "{line} not in:\n{help}");
    }
    let next = text(&env, &["next", "--help"]);
    assert!(next.contains("claim and catch up in one call"), "{next}");
}

#[test]
fn context_lists_at_most_ten_pages() {
    let env = Env::new();
    seed(&env);
    for n in 0..12 {
        let id = format!("APM:p{n:02}");
        env.json(&["page", "add", &id, "t", "--link", "APM-1"]);
    }
    env.json(&["page", "add", "APM:pinned", "t", "--pin"]);
    env.json(&["page", "add", "APM:other", "t"]);
    let c = env.json(&["context", "APM-1"]);
    assert_eq!(c["pages"].as_array().unwrap().len(), 10);
    // Not shown: 2 linked, the pinned one, the other one.
    assert_eq!(c["more_pages"], 4);
}
