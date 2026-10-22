//! `list` search, sort, limit and due filters, and date-time values (task and milestone due,
//! `datetime` attributes).

mod common;

use common::Env;
use serde_json::Value;

/// The `id`s of a JSON array of items.
fn ids(v: &Value) -> Vec<String> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].as_str().unwrap().to_string())
        .collect()
}

/// `apm --json list <args>` as ids.
fn list(env: &Env, args: &[&str]) -> Vec<String> {
    let mut all = vec!["list"];
    all.extend_from_slice(args);
    ids(&env.json(&all))
}

/// An env with project APM.
fn env_apm() -> Env {
    let env = Env::new();
    env.json(&["project", "add", "APM", "Agent PM"]);
    env
}

/// Add a task to APM with extra flags; returns its id.
fn add(env: &Env, title: &str, flags: &[&str]) -> String {
    let mut args = vec!["add", title, "-p", "APM"];
    args.extend_from_slice(flags);
    env.json(&args)["id"].as_str().unwrap().to_string()
}

/// A UTC time from SQLite, e.g. `now_utc(&env, "%Y-%m-%d", "-1 days")`.
fn now_utc(env: &Env, format: &str, modifier: &str) -> String {
    env.conn()
        .query_row("SELECT strftime(?1, 'now', ?2)", [format, modifier], |r| {
            r.get(0)
        })
        .unwrap()
}

// ---- date-time values

#[test]
fn task_due_accepts_dates_and_datetimes() {
    let env = env_apm();
    let t = env.json(&["add", "a", "-p", "APM", "--due", "2026-11-01T09:30+02:00"]);
    assert_eq!(t["due"], "2026-11-01T07:30:00Z");
    let t = env.json(&["add", "b", "-p", "APM", "--due", "2026-11-01"]);
    assert_eq!(t["due"], "2026-11-01");
    let t = env.json(&["edit", "APM-2", "--due", "2026-12-31T23:30-01:00"]);
    assert_eq!(t["due"], "2027-01-01T00:30:00Z");
    let t = env.json(&["edit", "APM-2", "--due", "2026-12-31T23:30:15Z"]);
    assert_eq!(t["due"], "2026-12-31T23:30:15Z");
    assert!(
        env.json_err(&["add", "c", "-p", "APM", "--due", "2026-11-01T09:30"])
            .contains("add Z or an offset")
    );
    assert!(
        env.json_err(&["edit", "APM-1", "--due", "2026-11-01T24:00Z"])
            .contains("invalid date/time")
    );
    assert!(
        env.json_err(&["add", "c", "-p", "APM", "--due", "tomorrow"])
            .contains("invalid date/time")
    );
    // Nothing written by the failed add.
    assert_eq!(list(&env, &[]), ["APM-1", "APM-2"]);
    // History records the stored form.
    let new: String = env
        .conn()
        .query_row(
            "SELECT new FROM events WHERE field = 'due' ORDER BY id DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(new, "2026-12-31T23:30:15Z");
    // Text output keeps its shape: the stored string in the DUE column.
    let out = env.apm().args(["list"]).assert().success();
    let text = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    assert!(text.contains("2026-11-01T07:30:00Z"), "{text}");
}

#[test]
fn milestone_due_accepts_datetimes_and_sorts_dates_first() {
    let env = env_apm();
    let m = env.json(&[
        "milestone",
        "add",
        "late",
        "-p",
        "APM",
        "--due",
        "2026-12-01T10:00+01:00",
    ]);
    assert_eq!(m["due"], "2026-12-01T09:00:00Z");
    env.json(&[
        "milestone",
        "add",
        "day",
        "-p",
        "APM",
        "--due",
        "2026-12-01",
    ]);
    env.json(&["milestone", "add", "none", "-p", "APM"]);
    env.json(&[
        "milestone",
        "add",
        "early",
        "-p",
        "APM",
        "--due",
        "2026-11-30T23:59Z",
    ]);
    let names: Vec<String> = env
        .json(&["milestone", "list", "-p", "APM"])
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["name"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(names, ["early", "day", "late", "none"]);
    let m = env.json(&[
        "milestone",
        "edit",
        "day",
        "-p",
        "APM",
        "--due",
        "2026-12-02T00:30+02:00",
    ]);
    assert_eq!(m["due"], "2026-12-01T22:30:00Z");
    assert!(
        env.json_err(&[
            "milestone",
            "edit",
            "day",
            "-p",
            "APM",
            "--due",
            "2026-12-02T00:30"
        ])
        .contains("add Z or an offset")
    );
    assert!(
        env.json_err(&["milestone", "add", "x", "-p", "APM", "--due", "2026-02-30"])
            .contains("invalid date/time")
    );
    let m = env.json(&["milestone", "edit", "day", "-p", "APM", "--due", ""]);
    assert_eq!(m["due"], Value::Null);
}

#[test]
fn datetime_attribute_validates_and_normalizes() {
    let env = env_apm();
    let a = env.json(&["attr", "add", "starts", "-p", "APM", "--type", "datetime"]);
    assert_eq!(a["type"], "datetime");
    env.json(&["attr", "add", "on", "-p", "APM", "--type", "date"]);
    let t = env.json(&[
        "add",
        "a",
        "-p",
        "APM",
        "--set",
        "starts=2026-11-01T09:30+02:00",
    ]);
    assert_eq!(t["attributes"]["starts"], "2026-11-01T07:30:00Z");
    let t = env.json(&["add", "b", "-p", "APM", "--set", "starts=2026-11-01"]);
    assert_eq!(t["attributes"]["starts"], "2026-11-01");
    assert!(
        env.json_err(&["add", "c", "-p", "APM", "--set", "starts=2026-11-01T09:30"])
            .contains("invalid value for starts: date-time `2026-11-01T09:30` has no time zone")
    );
    assert!(
        env.json_err(&["edit", "APM-1", "--set", "starts=soon"])
            .starts_with("invalid value for starts: invalid date/time")
    );
    // A date attribute stays date-only.
    assert!(
        env.json_err(&["edit", "APM-1", "--set", "on=2026-11-01T09:30Z"])
            .contains("invalid value for on: invalid date")
    );
    // --where compares normalized values.
    assert_eq!(
        list(&env, &["--where", "starts=2026-11-01T08:30+01:00"]),
        ["APM-1"]
    );
    let attrs = env.json(&["attr", "list", "-p", "APM"]);
    assert_eq!(attrs[0]["type"], "datetime");
}

// ---- search

#[test]
fn search_matches_title_body_and_comments_case_insensitively() {
    let env = env_apm();
    add(&env, "Parse ARGS", &[]);
    add(&env, "other", &["-b", "the args parser"]);
    add(&env, "third", &[]);
    add(&env, "fourth", &[]);
    env.json(&["comment", "APM-3", "needs ARGS too"]);
    env.json(&["comment", "APM-4", "unrelated"]);
    assert_eq!(
        list(&env, &["--search", "args"]),
        ["APM-1", "APM-2", "APM-3"]
    );
    assert_eq!(list(&env, &["--search", "ARGS PARSER"]), ["APM-2"]);
    assert!(list(&env, &["--search", "nothing"]).is_empty());
    // Combines with other filters and the open-only default.
    env.json(&["done", "APM-1"]);
    assert_eq!(list(&env, &["--search", "args"]), ["APM-2", "APM-3"]);
    assert_eq!(
        list(&env, &["--search", "args", "--all", "-p", "APM"]),
        ["APM-1", "APM-2", "APM-3"]
    );
    // Several matching comments still list the task once.
    env.json(&["comment", "APM-3", "args again"]);
    assert_eq!(list(&env, &["--search", "args"]), ["APM-2", "APM-3"]);
}

#[test]
fn search_folds_unicode_case() {
    let env = env_apm();
    add(&env, "École du soir", &[]); // APM-1
    add(&env, "ecole", &[]); // APM-2: no accent, no match
    add(&env, "other", &["-b", "voir l'ÉCOLE"]); // APM-3
    add(&env, "third", &[]); // APM-4
    env.json(&["comment", "APM-4", "après l'école"]);
    assert_eq!(
        list(&env, &["--search", "école"]),
        ["APM-1", "APM-3", "APM-4"]
    );
    assert_eq!(
        list(&env, &["--search", "ÉCOLE"]),
        ["APM-1", "APM-3", "APM-4"]
    );
    assert_eq!(list(&env, &["--search", "APRÈS"]), ["APM-4"]);
}

#[test]
fn search_escapes_like_wildcards() {
    let env = env_apm();
    add(&env, "100% done", &[]);
    add(&env, "1000 done", &[]);
    add(&env, "snake_case", &[]);
    add(&env, "snakeXcase", &[]);
    add(&env, r"back\slash", &[]);
    assert_eq!(list(&env, &["--search", "100%"]), ["APM-1"]);
    assert_eq!(list(&env, &["--search", "%"]), ["APM-1"]);
    assert_eq!(list(&env, &["--search", "e_c"]), ["APM-3"]);
    assert_eq!(list(&env, &["--search", r"k\s"]), ["APM-5"]);
    assert_eq!(list(&env, &["--search", ""]).len(), 5);
}

// ---- sort and limit

#[test]
fn sort_by_each_key() {
    let env = env_apm();
    env.json(&["project", "add", "B", "b"]);
    add(&env, "beta", &["--priority", "low", "--due", "2026-11-02"]); // APM-1
    add(&env, "Alpha", &["--priority", "urgent"]); // APM-2
    add(
        &env,
        "gamma",
        &["--priority", "high", "--due", "2026-11-01T12:00Z"],
    ); // APM-3
    add(&env, "delta", &["--due", "2026-11-01"]); // APM-4
    env.json(&["add", "epsilon", "-p", "B", "--priority", "medium"]); // B-1
    env.json(&["start", "APM-3"]);
    env.json(&["block", "B-1"]);

    assert_eq!(list(&env, &[]), ["APM-1", "APM-2", "APM-3", "APM-4", "B-1"]);
    assert_eq!(list(&env, &["--sort", "id"]), list(&env, &[]));
    assert_eq!(
        list(&env, &["--sort", "-id"]),
        ["B-1", "APM-4", "APM-3", "APM-2", "APM-1"]
    );
    assert_eq!(
        list(&env, &["--sort", "title"]),
        ["APM-2", "APM-1", "APM-4", "B-1", "APM-3"]
    );
    assert_eq!(
        list(&env, &["--sort", "-priority"]),
        ["APM-2", "APM-3", "B-1", "APM-1", "APM-4"]
    );
    assert_eq!(
        list(&env, &["--sort", "priority"]),
        ["APM-4", "APM-1", "B-1", "APM-3", "APM-2"]
    );
    // A date sorts before a date-time on the same day; no due is last in both directions.
    assert_eq!(
        list(&env, &["--sort", "due"]),
        ["APM-4", "APM-3", "APM-1", "APM-2", "B-1"]
    );
    assert_eq!(
        list(&env, &["--sort", "-due"]),
        ["APM-1", "APM-3", "APM-4", "APM-2", "B-1"]
    );
    // Project, then status position (todo 1, doing 2, blocked 3), then id.
    assert_eq!(
        list(&env, &["--sort", "status"]),
        ["APM-1", "APM-2", "APM-4", "APM-3", "B-1"]
    );
    assert_eq!(
        list(&env, &["--sort", "-status"]),
        ["APM-3", "APM-1", "APM-2", "APM-4", "B-1"]
    );
    // Several keys: the first decides, the next breaks ties, id last.
    assert_eq!(
        list(&env, &["--sort", "status,-priority"]),
        ["APM-2", "APM-1", "APM-4", "APM-3", "B-1"]
    );
    assert_eq!(
        list(&env, &["--sort", "-status", "--sort", "title"]),
        ["APM-3", "APM-2", "APM-1", "APM-4", "B-1"]
    );
    // Keys are case-insensitive.
    assert_eq!(
        list(&env, &["--sort=-Priority"]),
        list(&env, &["--sort", "-priority"])
    );
}

#[test]
fn sort_by_timestamps_is_stable() {
    let env = env_apm();
    for t in ["a", "b", "c", "d"] {
        add(&env, t, &[]);
    }
    // Same timestamps everywhere: ties keep the id order, in both directions.
    let conn = env.conn();
    conn.execute_batch(
        "UPDATE items SET created_at = '2026-01-01T00:00:00Z', updated_at = '2026-01-01T00:00:00Z'",
    )
    .unwrap();
    for key in ["created", "-created", "updated", "-updated"] {
        assert_eq!(
            list(&env, &["--sort", key]),
            ["APM-1", "APM-2", "APM-3", "APM-4"],
            "{key}"
        );
    }
    conn.execute_batch(
        "UPDATE items SET created_at = '2026-01-02T00:00:00Z' WHERE seq IN (1, 3);
         UPDATE items SET updated_at = '2026-01-03T00:00:00Z' WHERE seq = 2;",
    )
    .unwrap();
    assert_eq!(
        list(&env, &["--sort", "created"]),
        ["APM-2", "APM-4", "APM-1", "APM-3"]
    );
    assert_eq!(
        list(&env, &["--sort", "-created"]),
        ["APM-1", "APM-3", "APM-2", "APM-4"]
    );
    assert_eq!(
        list(&env, &["--sort", "-updated"]),
        ["APM-2", "APM-1", "APM-3", "APM-4"]
    );
    assert_eq!(
        list(&env, &["--sort", "updated"]),
        ["APM-1", "APM-3", "APM-4", "APM-2"]
    );
}

#[test]
fn sort_errors() {
    let env = env_apm();
    assert_eq!(
        env.json_err(&["list", "--sort", "due,size"]),
        "unknown sort key: size (valid: id, created, updated, due, priority, status, title)"
    );
    assert!(
        env.json_err(&["list", "--sort", "-"])
            .starts_with("unknown sort key: -")
    );
    assert!(
        env.json_err(&["list", "--sort", ""])
            .starts_with("unknown sort key")
    );
    // `--sort` right before another flag: a usage error, not `--limit` taken as a key.
    let out = env
        .apm()
        .args(["--json", "list", "--sort", "--limit", "2"])
        .assert()
        .code(2);
    let err: Value = serde_json::from_slice(&out.get_output().stderr).unwrap();
    assert!(
        err["error"]
            .as_str()
            .unwrap()
            .contains("--sort needs a value before --limit"),
        "{err}"
    );
    assert_eq!(
        list(&env, &["--sort", "-due", "--limit", "2"]),
        Vec::<String>::new()
    );
}

#[test]
fn sort_by_status_groups_by_project() {
    let env = env_apm();
    env.json(&["project", "add", "B", "b"]);
    env.json(&[
        "status",
        "add",
        "triage",
        "-p",
        "B",
        "--category",
        "todo",
        "--position",
        "1",
    ]);
    add(&env, "a", &[]); // APM-1 todo (position 1)
    env.json(&["add", "b", "-p", "B"]); // B-1 triage (position 1)
    env.json(&["add", "c", "-p", "B", "--status", "todo"]); // B-2 todo (position 2 in B)
    add(&env, "d", &[]); // APM-2
    env.json(&["start", "APM-2"]); // doing (position 2 in APM)
    assert_eq!(
        list(&env, &["--sort", "status"]),
        ["APM-1", "APM-2", "B-1", "B-2"]
    );
    assert_eq!(
        list(&env, &["--sort", "-status"]),
        ["APM-2", "APM-1", "B-2", "B-1"]
    );
}

#[test]
fn limit_applies_after_sort_and_filters() {
    let env = env_apm();
    env.json(&[
        "attr",
        "add",
        "size",
        "-p",
        "APM",
        "--type",
        "select",
        "--options",
        "s,m",
    ]);
    add(&env, "a", &["--set", "size=s"]);
    add(&env, "b", &["--set", "size=m"]);
    add(&env, "c", &["--set", "size=s"]);
    add(&env, "d", &["--set", "size=s"]);
    assert_eq!(list(&env, &["--limit", "2"]), ["APM-1", "APM-2"]);
    assert_eq!(list(&env, &["--sort", "-id", "--limit", "1"]), ["APM-4"]);
    assert_eq!(list(&env, &["--limit", "10"]).len(), 4);
    // --where is applied before the limit.
    assert_eq!(
        list(
            &env,
            &["--where", "size=s", "--sort", "-id", "--limit", "2"]
        ),
        ["APM-4", "APM-3"]
    );
    // N >= 1 is checked by clap: exit 2, JSON error under --json.
    for bad in ["0", "-1", "x"] {
        env.apm()
            .args(["--json", "list", "--limit", bad])
            .assert()
            .code(2);
    }
}

// ---- due filters

#[test]
fn due_before_and_after() {
    let env = env_apm();
    add(&env, "a", &["--due", "2026-11-01"]); // APM-1
    add(&env, "b", &["--due", "2026-11-01T08:00Z"]); // APM-2
    add(&env, "c", &["--due", "2026-11-01T18:00Z"]); // APM-3
    add(&env, "d", &["--due", "2026-11-02"]); // APM-4
    add(&env, "e", &[]); // APM-5, never matches a due filter

    // Date bounds compare with the date part: strictly before / after the day.
    assert_eq!(
        list(&env, &["--due-before", "2026-11-01"]),
        Vec::<String>::new()
    );
    assert_eq!(
        list(&env, &["--due-before", "2026-11-02"]),
        ["APM-1", "APM-2", "APM-3"]
    );
    assert_eq!(list(&env, &["--due-after", "2026-11-01"]), ["APM-4"]);
    assert_eq!(
        list(&env, &["--due-after", "2026-10-31"]),
        ["APM-1", "APM-2", "APM-3", "APM-4"]
    );
    // Date-time bounds compare with the whole due; a date due is before any time on its day.
    assert_eq!(
        list(&env, &["--due-before", "2026-11-01T12:00Z"]),
        ["APM-1", "APM-2"]
    );
    assert_eq!(
        list(&env, &["--due-before", "2026-11-01T08:00Z"]),
        ["APM-1"]
    );
    assert_eq!(
        list(&env, &["--due-after", "2026-11-01T08:00Z"]),
        ["APM-3", "APM-4"]
    );
    // Bounds are normalized to UTC first: 13:00+02:00 is 11:00Z.
    assert_eq!(
        list(&env, &["--due-after", "2026-11-01T13:00+02:00"]),
        ["APM-3", "APM-4"]
    );
    // Both bounds: a window.
    assert_eq!(
        list(
            &env,
            &[
                "--due-after",
                "2026-11-01T00:00Z",
                "--due-before",
                "2026-11-02",
                "--sort",
                "-due"
            ]
        ),
        ["APM-3", "APM-2"]
    );
    assert!(
        env.json_err(&["list", "--due-before", "2026-11-01T09:00"])
            .contains("add Z or an offset")
    );
    assert!(
        env.json_err(&["list", "--due-after", "next week"])
            .contains("invalid date/time")
    );
}

#[test]
fn overdue_with_dates_and_datetimes() {
    let env = env_apm();
    let today = now_utc(&env, "%Y-%m-%d", "+0 days");
    let yesterday = now_utc(&env, "%Y-%m-%d", "-1 days");
    let minute_ago = now_utc(&env, "%Y-%m-%dT%H:%M:%SZ", "-1 minutes");
    let in_an_hour = now_utc(&env, "%Y-%m-%dT%H:%M:%SZ", "+1 hours");
    add(&env, "date yesterday", &["--due", &yesterday]); // APM-1
    add(&env, "date today", &["--due", &today]); // APM-2: not overdue until tomorrow
    add(&env, "a minute ago", &["--due", &minute_ago]); // APM-3
    add(&env, "in an hour", &["--due", &in_an_hour]); // APM-4
    add(&env, "no due", &[]); // APM-5
    add(&env, "old", &["--due", "2000-01-01T00:00Z"]); // APM-6, closed below
    add(&env, "cancelled", &["--due", "2000-01-01"]); // APM-7
    env.json(&["done", "APM-6"]);
    env.json(&["edit", "APM-7", "--status", "cancelled"]);

    assert_eq!(list(&env, &["--overdue"]), ["APM-1", "APM-3"]);
    // Open tasks only, even with --all or a closed --status.
    assert_eq!(list(&env, &["--overdue", "--all"]), ["APM-1", "APM-3"]);
    assert!(list(&env, &["--overdue", "--status", "done"]).is_empty());
    // Combines with sort, limit and other filters.
    assert_eq!(
        list(&env, &["--overdue", "--sort", "-due", "--limit", "1"]),
        ["APM-3"]
    );
    assert_eq!(list(&env, &["--overdue", "--search", "date"]), ["APM-1"]);
    // A date due is before every date-time on its day, so only the minute-old due is after
    // a bound two minutes ago, even just after midnight.
    let two_minutes_ago = now_utc(&env, "%Y-%m-%dT%H:%M:%SZ", "-2 minutes");
    assert_eq!(
        list(&env, &["--overdue", "--due-after", &two_minutes_ago]),
        ["APM-3"]
    );
}

#[test]
fn list_help_documents_the_new_flags() {
    let env = Env::new();
    let out = env.apm().args(["list", "--help"]).assert().success();
    let help = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    for flag in [
        "--search",
        "--sort",
        "--limit",
        "--due-before",
        "--due-after",
        "--overdue",
    ] {
        assert!(help.contains(flag), "{flag} missing from:\n{help}");
    }
    assert!(help.contains("date part"), "{help}");
}
