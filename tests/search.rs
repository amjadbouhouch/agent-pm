mod common;

use common::Env;
use serde_json::{Value, json};

/// APM with APM-1 and a log, WEB with WEB-1; pages in both.
fn seed(env: &Env) {
    env.json(&["project", "add", "APM", "agent-pm"]);
    env.json(&["project", "add", "WEB", "web"]);
    env.json(&["add", "first", "-p", "APM"]);
    env.json(&["add", "web task", "-p", "WEB"]);
    env.json(&[
        "page",
        "add",
        "APM:notes",
        "Notes",
        "-b",
        "the busy timeout covers most writes",
    ]);
    env.json(&[
        "page",
        "add",
        "APM:busy",
        "Busy timeout",
        "-b",
        "see the notes",
    ]);
    env.json(&[
        "page",
        "add",
        "WEB:deploy",
        "Deploy",
        "-s",
        "busy timeout on the web too",
    ]);
    env.json(&[
        "--as",
        "bob",
        "comment",
        "APM-1",
        "busy_timeout does not cover the WAL switch",
    ]);
    env.json(&[
        "comment",
        "APM-1",
        "--kind",
        "question",
        "Is the timeout busy?",
    ]);
    env.json(&["comment", "WEB-1", "unrelated"]);
}

fn ids(hits: &Value) -> Vec<String> {
    hits.as_array()
        .unwrap()
        .iter()
        .map(|h| match h["kind"].as_str().unwrap() {
            "page" => h["id"].as_str().unwrap().to_string(),
            _ => format!("{} #{}", h["task"].as_str().unwrap(), h["comment"]),
        })
        .collect()
}

#[test]
fn search_finds_pages_and_log_entries() {
    let env = Env::new();
    seed(&env);
    let hits = env.json(&["search", "busy timeout", "-p", "apm"]);
    let found = ids(&hits);
    assert_eq!(found.len(), 4, "{found:?}");
    // A title hit ranks above a body hit.
    let pos = |id: &str| found.iter().position(|f| f == id).unwrap();
    assert!(pos("APM:busy") < pos("APM:notes"), "{found:?}");
    assert_eq!(found[0], "APM:busy");
    let log = hits
        .as_array()
        .unwrap()
        .iter()
        .find(|h| h["kind"] == "log" && h["author"] == "bob")
        .unwrap();
    assert_eq!(log["task"], "APM-1");
    assert_eq!(log["entry"], "note");
    assert!(log["comment"].is_i64());
    assert!(log["created_at"].is_string());
    assert_eq!(
        log["snippet"],
        "[busy]_[timeout] does not cover the WAL switch"
    );
    let page = &hits[0];
    assert_eq!(page["title"], "Busy timeout");
    assert!(page["snippet"].as_str().unwrap().contains("[Busy]"));
    let keys: Vec<&String> = page.as_object().unwrap().keys().collect();
    assert_eq!(keys, ["id", "kind", "snippet", "title"]); // serde_json sorts keys
    // Without -p: every project; --limit cuts the best N.
    assert_eq!(ids(&env.json(&["search", "busy timeout"])).len(), 5);
    assert_eq!(
        ids(&env.json(&["search", "busy timeout", "--limit", "1"])),
        ["APM:busy"]
    );
    // Case, accents and word order do not matter; every word must match.
    assert_eq!(
        ids(&env.json(&["search", "TIMEOUT Busy", "-p", "APM"])).len(),
        4
    );
    assert_eq!(
        ids(&env.json(&["search", "busy nothing-like-this"])),
        Vec::<String>::new()
    );
    env.json(&["page", "add", "APM:cafe", "Caf\u{e9} cr\u{e8}me", "-b", "x"]);
    assert_eq!(ids(&env.json(&["search", "CAFE creme"])), ["APM:cafe"]);
    assert_eq!(
        env.json_err(&["search", "x", "-p", "NOPE"]),
        "no such project: NOPE"
    );
}

#[test]
fn search_never_errors_on_user_text() {
    let env = Env::new();
    seed(&env);
    for q in [
        "\"",
        "-",
        ":",
        "*",
        "(",
        ")",
        "busy\"",
        "\"busy",
        "-busy",
        "busy:timeout",
        "busy*",
        "(busy",
        "busy OR",
        "NOT busy",
        "NEAR(busy timeout)",
        "^busy",
        "busy AND -timeout",
        "' ; DROP TABLE pages; --",
        "\u{e9}\u{301}",
        "\u{1f600}",
        "",
    ] {
        let out = env.apm().args(["--json", "search", q]).assert().success();
        let v: Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
        assert!(v.is_array(), "{q}");
        env.apm().args(["search", q]).assert().success();
    }
    // Operators are words: `busy OR` needs both, `-busy` finds busy.
    assert!(ids(&env.json(&["search", "busy OR"])).is_empty());
    assert_eq!(ids(&env.json(&["search", "-busy", "-p", "APM"])).len(), 4);
    // Punctuation inside a token keeps its words side by side: not the question
    // "Is the timeout busy?".
    assert_eq!(
        ids(&env.json(&["search", "busy:timeout", "-p", "APM"])).len(),
        3
    );
    assert_eq!(env.json(&["search", "\" - : * ("]), json!([]));
    assert_eq!(count_pages(&env), 3);
}

fn count_pages(env: &Env) -> i64 {
    env.conn()
        .query_row("SELECT count(*) FROM pages", [], |r| r.get(0))
        .unwrap()
}

#[test]
fn search_follows_edits_and_removals() {
    let env = Env::new();
    seed(&env);
    env.json(&["page", "edit", "APM:notes", "-b", "pangolin"]);
    assert!(!ids(&env.json(&["search", "busy", "-p", "APM"])).contains(&"APM:notes".to_string()));
    assert_eq!(ids(&env.json(&["search", "pangolin"])), ["APM:notes"]);
    env.json(&["page", "edit", "APM:notes", "--rename", "animals"]);
    assert_eq!(ids(&env.json(&["search", "pangolin"])), ["APM:animals"]);
    env.json(&["page", "rm", "APM:animals"]);
    assert!(ids(&env.json(&["search", "pangolin"])).is_empty());
    env.json(&["rm", "APM-1"]);
    assert_eq!(ids(&env.json(&["search", "switch"])), Vec::<String>::new());
    env.json(&["project", "edit", "WEB", "--key", "SITE"]);
    assert_eq!(
        ids(&env.json(&["search", "deploy", "-p", "WEB"])),
        ["SITE:deploy"]
    );
}

#[test]
fn search_text_output() {
    let env = Env::new();
    seed(&env);
    let out = env.apm().args(["search", "wal switch"]).assert().success();
    let text = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    let id: i64 = env.json(&["search", "wal switch"])[0]["comment"]
        .as_i64()
        .unwrap();
    assert_eq!(
        text,
        format!("log  APM-1 #{id}  bob note: busy_timeout does not cover the [WAL] [switch]\n")
    );
    let out = env.apm().args(["search", "deploy"]).assert().success();
    let text = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    assert!(text.starts_with("page  WEB:deploy  Deploy: "), "{text}");
    env.apm()
        .args(["search", "nothing"])
        .assert()
        .success()
        .stdout("");
    env.apm()
        .args(["--json", "search", "x", "--limit", "0"])
        .assert()
        .code(2);
}

#[test]
fn search_lists_pages_then_log_entries() {
    let env = Env::new();
    env.json(&["project", "add", "APM", "agent-pm"]);
    env.json(&["add", "first", "-p", "APM"]);
    // Log entries that match better than the page on their own index still come after it.
    let once = env.json(&[
        "comment",
        "APM-1",
        "zebra crossing and other long words here",
    ]);
    let thrice = env.json(&["comment", "APM-1", "zebra zebra zebra"]);
    env.json(&[
        "page",
        "add",
        "APM:misc",
        "Misc",
        "-s",
        "a zebra somewhere",
        "-b",
        "long text",
    ]);
    env.json(&["page", "add", "APM:zebra", "Zebra", "-b", "x"]);
    let log = |c: &Value| format!("APM-1 #{}", c["id"]);
    assert_eq!(
        ids(&env.json(&["search", "zebra"])),
        ["APM:zebra", "APM:misc", &log(&thrice), &log(&once)]
    );
    // --limit counts pages and log entries together.
    assert_eq!(
        ids(&env.json(&["search", "zebra", "--limit", "3"])),
        ["APM:zebra", "APM:misc", &log(&thrice)]
    );
    assert_eq!(
        ids(&env.json(&["search", "zebra", "--limit", "1"])),
        ["APM:zebra"]
    );
    let out = env.apm().args(["search", "--help"]).assert().success();
    let help = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    for line in [
        "every word must match",
        "OR, NOT, `*`, quotes and `-` are plain text",
        "busy_timeout finds \"busy timeout\"",
        "pages first, then log entries",
        "Only pages and task log entries of project KEY",
    ] {
        assert!(help.contains(line), "{line} not in:\n{help}");
    }
}

#[test]
fn search_tokens_are_phrases() {
    let env = Env::new();
    seed(&env);
    let apm = |q: &str| ids(&env.json(&["search", q, "-p", "APM"]));
    // Separate words match anywhere; joined by punctuation they must be adjacent, in order.
    assert_eq!(apm("timeout busy").len(), 4);
    assert_eq!(apm("busy_timeout").len(), 3);
    assert_eq!(apm("timeout_busy").len(), 1); // "Is the timeout busy?"
    assert_eq!(apm("switch:wal").len(), 0);
    assert_eq!(apm("wal:switch").len(), 1);
    // Quotes in a token are doubled, not syntax; tokens without a letter or digit go.
    assert_eq!(apm("\"busy\" - * ( \"timeout").len(), 4);
    assert_eq!(apm("bu\"sy"), Vec::<String>::new());
}
