//! Claims: `claim`, `release`, `next`, `claims`, `list --claimed/--unclaimed`, and closing a
//! task releasing its claim. Expiry is tested by back-dating `claim_expires`, not by sleeping.
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

/// Project APM with tasks APM-1..n titled t1..tn.
fn env_tasks(n: usize) -> Env {
    let env = Env::new();
    env.json(&["project", "add", "APM", "Agent PM"]);
    for i in 1..=n {
        env.json(&["add", &format!("t{i}"), "-p", "APM"]);
    }
    env
}

/// `apm --json --as <who> <args>`, asserting success.
fn as_(env: &Env, who: &str, args: &[&str]) -> Value {
    let mut all = vec!["--as", who];
    all.extend_from_slice(args);
    env.json(&all)
}

/// `apm --json --as <who> <args>`, asserting failure; the error message.
fn as_err(env: &Env, who: &str, args: &[&str]) -> String {
    let mut all = vec!["--as", who];
    all.extend_from_slice(args);
    env.json_err(&all)
}

/// The (kind, field, old) of a task's claim events, oldest first.
fn claim_events(env: &Env, seq: i64) -> Vec<(String, String, Option<String>, Option<String>)> {
    let conn = env.conn();
    let mut stmt = conn
        .prepare(
            "SELECT e.actor, e.kind, e.field, e.old FROM events e JOIN items i ON i.id = e.item_id
             WHERE i.seq = ?1 AND e.kind IN ('claimed', 'released') ORDER BY e.id",
        )
        .unwrap();
    stmt.query_map([seq], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

fn ev(
    actor: &str,
    kind: &str,
    field: Option<&str>,
    old: Option<&str>,
) -> (String, String, Option<String>, Option<String>) {
    (
        actor.to_string(),
        kind.to_string(),
        field.map(str::to_string),
        old.map(str::to_string),
    )
}

/// Make every claim on task `seq` expired a minute ago.
fn expire(env: &Env, seq: i64) {
    env.conn()
        .execute(
            "UPDATE items SET claim_expires = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-60 seconds')
             WHERE seq = ?1",
            [seq],
        )
        .unwrap();
}

/// Seconds between a task's claimed_at and claim_expires.
fn claim_secs(env: &Env, seq: i64) -> i64 {
    env.conn()
        .query_row(
            "SELECT CAST(round((julianday(claim_expires) - julianday(claimed_at)) * 86400) AS INTEGER)
             FROM items WHERE seq = ?1",
            [seq],
            |r| r.get(0),
        )
        .unwrap()
}

#[test]
fn claim_shows_in_json_show_and_history() {
    let env = env_tasks(1);
    assert_eq!(env.json(&["show", "APM-1"])["claim"], Value::Null);
    let v = as_(&env, "alice", &["claim", "apm-1"]);
    assert_eq!(v["id"], "APM-1");
    assert_eq!(v["claim"]["by"], "alice");
    assert!(v["claim"]["expires"].as_str().unwrap() > v["claim"]["at"].as_str().unwrap());
    assert_eq!(claim_secs(&env, 1), 2 * 3600);
    // A claim does not assign or start the task.
    assert_eq!(v["assignee"], Value::Null);
    assert_eq!(v["status"], "todo");
    assert_eq!(env.json(&["list"])[0]["claim"]["by"], "alice");

    let out = env.apm().args(["show", "APM-1"]).assert().success();
    let text = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    let line = format!(
        "claimed: alice until {}",
        v["claim"]["expires"].as_str().unwrap()
    );
    assert!(text.lines().any(|l| l == line), "{text}");

    let out = env
        .apm()
        .args(["--as", "alice", "claim", "APM-1", "--ttl", "30m"])
        .assert()
        .success();
    let text = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    assert!(text.starts_with("claimed APM-1 until "), "{text}");
    assert!(
        (1800..=1802).contains(&claim_secs(&env, 1)),
        "a renewal keeps claimed_at"
    );
    let h = env.json(&["history", "APM-1"]);
    let kinds: Vec<&str> = h
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["created", "claimed", "claimed"]);
}

#[test]
fn someone_elses_live_claim_needs_steal() {
    let env = env_tasks(1);
    as_(&env, "alice", &["claim", "APM-1"]);
    let e = as_err(&env, "bob", &["claim", "APM-1"]);
    assert!(e.starts_with("APM-1 is claimed by alice until "), "{e}");
    assert!(e.ends_with("; pass --steal to take it"), "{e}");
    let v = as_(&env, "bob", &["claim", "APM-1", "--steal"]);
    assert_eq!(v["claim"]["by"], "bob");
    assert_eq!(
        claim_events(&env, 1),
        [
            ev("alice", "claimed", None, None),
            ev("bob", "released", Some("stolen"), Some("alice")),
            ev("bob", "claimed", None, None),
        ]
    );
}

#[test]
fn an_expired_claim_is_ignored_everywhere() {
    let env = env_tasks(1);
    as_(&env, "alice", &["claim", "APM-1"]);
    expire(&env, 1);
    assert_eq!(env.json(&["show", "APM-1"])["claim"], Value::Null);
    assert_eq!(ids(&env.json(&["list", "--claimed"])), Vec::<String>::new());
    assert_eq!(ids(&env.json(&["list", "--unclaimed"])), ["APM-1"]);
    assert_eq!(env.json(&["claims"]), json!([]));
    // alice's release finds nothing to release.
    let out = env
        .apm()
        .args(["--as", "alice", "release", "APM-1"])
        .assert()
        .success();
    assert_eq!(
        String::from_utf8_lossy(&out.get_output().stdout),
        "unchanged APM-1\n"
    );
    // Bob claims it without --steal, and `next` would have offered it to him too.
    assert_eq!(as_(&env, "bob", &["claim", "APM-1"])["claim"]["by"], "bob");
    expire(&env, 1);
    assert_eq!(as_(&env, "carol", &["next"])["id"], "APM-1");
    assert_eq!(
        claim_events(&env, 1),
        [
            ev("alice", "claimed", None, None),
            ev("bob", "claimed", None, None),
            ev("carol", "claimed", None, None),
        ]
    );
}

#[test]
fn release_drops_only_your_claim_unless_forced() {
    let env = env_tasks(2);
    as_(&env, "alice", &["claim", "APM-1"]);
    let e = as_err(&env, "bob", &["release", "APM-1"]);
    assert_eq!(
        e,
        "APM-1 is claimed by alice, not bob; pass --force to release it"
    );
    let v = as_(&env, "alice", &["release", "APM-1"]);
    assert_eq!(v["claim"], Value::Null);
    as_(&env, "alice", &["claim", "APM-1"]);
    assert_eq!(
        as_(&env, "bob", &["release", "APM-1", "--force"])["claim"],
        Value::Null
    );
    // Nothing to release: not an error, nothing recorded, updated_at kept.
    let before = env.json(&["show", "APM-2"])["updated_at"].clone();
    let v = as_(&env, "bob", &["release", "APM-2"]);
    assert_eq!(v["updated_at"], before);
    assert_eq!(
        claim_events(&env, 1),
        [
            ev("alice", "claimed", None, None),
            ev("alice", "released", Some("manual"), Some("alice")),
            ev("alice", "claimed", None, None),
            ev("bob", "released", Some("manual"), Some("alice")),
        ]
    );
    assert!(claim_events(&env, 2).is_empty());
}

#[test]
fn closing_a_task_releases_its_claim_by_any_path() {
    let env = env_tasks(5);
    env.json(&[
        "status",
        "add",
        "shipped",
        "-p",
        "APM",
        "--category",
        "active",
    ]);
    for n in 1..=5 {
        as_(&env, "alice", &["claim", &format!("APM-{n}")]);
    }
    as_(&env, "bob", &["done", "APM-1"]);
    as_(&env, "bob", &["edit", "APM-2", "--status", "cancelled"]);
    env.json(&["edit", "APM-3", "--status", "shipped"]);
    env.json(&[
        "status",
        "edit",
        "shipped",
        "-p",
        "APM",
        "--category",
        "done",
    ]);
    env.json(&["edit", "APM-4", "--status", "doing"]);
    env.json(&["status", "rm", "doing", "-p", "APM", "--to", "done"]);
    let claimed = |n: i64| env.json(&["show", &format!("APM-{n}")])["claim"].clone();
    for n in 1..=4 {
        assert_eq!(claimed(n), Value::Null, "APM-{n}");
        let last = claim_events(&env, n).pop().unwrap();
        assert_eq!(
            (last.1, last.2, last.3),
            ("released".into(), Some("done".into()), Some("alice".into()))
        );
    }
    // Starting a task keeps its claim; closed tasks cannot be claimed.
    env.json(&["start", "APM-5"]);
    assert_eq!(claimed(5)["by"], "alice");
    assert_eq!(
        as_err(&env, "alice", &["claim", "APM-1"]),
        "cannot claim APM-1: it is done"
    );
    // The released event comes after the status change it follows from.
    let h = env.json(&["history", "APM-1"]);
    let tail: Vec<&str> = h
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["kind"].as_str().unwrap())
        .collect();
    assert_eq!(tail, ["created", "claimed", "changed", "released"]);
}

#[test]
fn moving_a_task_keeps_its_claim() {
    let env = env_tasks(1);
    env.json(&["project", "add", "OPS", "Ops"]);
    as_(&env, "alice", &["claim", "APM-1"]);
    env.json(&["move", "APM-1", "OPS"]);
    assert_eq!(env.json(&["show", "OPS-1"])["claim"]["by"], "alice");
}

#[test]
fn claims_and_list_filters() {
    let env = env_tasks(3);
    env.json(&["project", "add", "OPS", "Ops"]);
    env.json(&["add", "o1", "-p", "OPS"]);
    as_(&env, "alice", &["claim", "APM-3", "--ttl", "1h"]);
    as_(&env, "bob", &["claim", "APM-1", "--ttl", "3h"]);
    as_(&env, "alice", &["claim", "OPS-1", "--ttl", "1d"]);
    // Soonest expiry first.
    assert_eq!(ids(&env.json(&["claims"])), ["APM-3", "APM-1", "OPS-1"]);
    assert_eq!(
        ids(&as_(&env, "alice", &["claims", "--mine"])),
        ["APM-3", "OPS-1"]
    );
    assert_eq!(
        ids(&as_(&env, "alice", &["claims", "--mine", "-p", "apm"])),
        ["APM-3"]
    );
    assert_eq!(
        ids(&env.json(&["list", "--claimed", "-p", "APM"])),
        ["APM-1", "APM-3"]
    );
    assert_eq!(ids(&env.json(&["list", "--unclaimed"])), ["APM-2"]);
    let out = env.apm().args(["claims", "-p", "APM"]).assert().success();
    let text = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert!(lines[0].starts_with("ID     BY     EXPIRES"), "{text}");
    assert!(
        lines[1].starts_with("APM-3  alice  ") && lines[1].ends_with("  t3"),
        "{text}"
    );
    assert_eq!(lines.len(), 3, "{text}");
    env.apm()
        .args(["list", "--claimed", "--unclaimed"])
        .assert()
        .code(2);
}

#[test]
fn ttl_must_be_minutes_hours_or_days_up_to_7d() {
    let env = env_tasks(1);
    as_(&env, "a", &["claim", "APM-1", "--ttl", "7d"]);
    assert_eq!(claim_secs(&env, 1), 7 * 86400);
    for bad in ["0m", "8d", "90", "1w", "1.5h"] {
        env.apm()
            .args(["--json", "claim", "APM-1", "--ttl", bad])
            .assert()
            .code(2);
        env.apm().args(["next", "--ttl", bad]).assert().code(2);
    }
}

#[test]
fn next_picks_by_priority_then_due_then_id() {
    let env = Env::new();
    env.json(&["project", "add", "APM", "Agent PM"]);
    env.json(&["add", "low", "-p", "APM", "--priority", "low"]); // 1
    env.json(&[
        "add",
        "high late",
        "-p",
        "APM",
        "--priority",
        "high",
        "--due",
        "2026-12-01",
    ]); // 2
    env.json(&["add", "high none", "-p", "APM", "--priority", "high"]); // 3
    env.json(&[
        "add",
        "high soon",
        "-p",
        "APM",
        "--priority",
        "high",
        "--due",
        "2026-11-01",
    ]); // 4
    env.json(&[
        "add",
        "high soon too",
        "-p",
        "APM",
        "--priority",
        "high",
        "--due",
        "2026-11-01",
    ]); // 5
    let order: Vec<String> = (0..5)
        .map(|n| {
            as_(&env, &format!("bot{n}"), &["next"])["id"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect();
    assert_eq!(order, ["APM-4", "APM-5", "APM-2", "APM-3", "APM-1"]);
}

#[test]
fn next_claims_assigns_and_starts() {
    let env = env_tasks(1);
    env.json(&[
        "status",
        "add",
        "triage",
        "-p",
        "APM",
        "--category",
        "todo",
        "--position",
        "1",
    ]);
    env.json(&[
        "status",
        "add",
        "wip",
        "-p",
        "APM",
        "--category",
        "active",
        "--position",
        "2",
    ]);
    let v = as_(&env, "bot", &["next", "--ttl", "45m"]);
    assert_eq!(v["id"], "APM-1");
    assert_eq!(v["claim"]["by"], "bot");
    assert_eq!(v["assignee"], "bot");
    assert_eq!(v["status"], "wip", "the first active-category status");
    assert_eq!(claim_secs(&env, 1), 45 * 60);
    let h = env.json(&["history", "APM-1"]);
    let got: Vec<(String, String)> = h
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            (
                e["kind"].as_str().unwrap().into(),
                e["field"].as_str().unwrap_or("").into(),
            )
        })
        .collect();
    let want: Vec<(String, String)> = [
        ("created", ""),
        ("claimed", ""),
        ("changed", "assignee"),
        ("changed", "status"),
    ]
    .iter()
    .map(|(k, f)| (k.to_string(), f.to_string()))
    .collect();
    assert_eq!(got, want);
    // Its claimant gets it again (renewed); an active task keeps its status and assignee.
    let v = as_(&env, "bot", &["next"]);
    assert_eq!(
        (v["id"].as_str(), v["status"].as_str()),
        (Some("APM-1"), Some("wip"))
    );
    assert!(
        (7200..=7202).contains(&claim_secs(&env, 1)),
        "a renewal keeps claimed_at"
    );
}

#[test]
fn next_skips_what_is_not_ready_for_the_actor() {
    let env = env_tasks(7);
    env.json(&["link", "APM-1", "blocks", "APM-2"]); // APM-2 has an open blocker
    env.json(&["block", "APM-3"]); // in the status named blocked
    as_(&env, "alice", &["claim", "APM-4"]); // claimed by someone else
    env.json(&["edit", "APM-5", "-a", "alice"]); // assigned to someone else
    env.json(&["done", "APM-6"]); // closed
    env.json(&["edit", "APM-1", "-a", "bot"]); // assigned to me: fine
    let next = |who: &str| {
        as_(&env, who, &["next"])["id"]
            .as_str()
            .unwrap()
            .to_string()
    };
    // A task the actor already holds is still theirs: `next` offers it again.
    assert_eq!([next("bot"), next("bot")], ["APM-1", "APM-1"]);
    env.json(&["done", "APM-1"]);
    assert_eq!(next("bot"), "APM-2", "its blocker is closed");
    assert_eq!(next("carol"), "APM-7");
    assert_eq!(next("alice"), "APM-4", "her own claim");
    as_(&env, "alice", &["done", "APM-4"]);
    assert_eq!(next("alice"), "APM-5", "assigned to her");
    let out = env.apm().args(["--as", "dave", "next"]).assert().code(3);
    assert_eq!(
        String::from_utf8_lossy(&out.get_output().stdout),
        "nothing ready\n"
    );
}

#[test]
fn next_filters_by_project_tag_and_milestone() {
    let env = env_tasks(3);
    env.json(&["project", "add", "OPS", "Ops"]);
    env.json(&["milestone", "add", "v1", "-p", "APM"]);
    env.json(&["add", "o1", "-p", "OPS", "--priority", "urgent"]);
    env.json(&["edit", "APM-2", "--tag", "db"]);
    env.json(&["edit", "APM-3", "-m", "v1", "--tag", "db"]);
    assert_eq!(
        as_(&env, "a", &["next", "-p", "apm", "--tag", "db", "-m", "v1"])["id"],
        "APM-3"
    );
    assert_eq!(as_(&env, "b", &["next", "--tag", "DB"])["id"], "APM-2");
    assert_eq!(as_(&env, "c", &["next", "-p", "APM"])["id"], "APM-1");
    assert_eq!(as_(&env, "d", &["next"])["id"], "OPS-1");
    assert_eq!(
        as_err(&env, "e", &["next", "-m", "v9"]),
        "no such milestone: v9"
    );
    assert_eq!(
        as_err(&env, "e", &["next", "-p", "NOPE"]),
        "no such project: NOPE"
    );
}

#[test]
fn next_with_nothing_ready_exits_3() {
    let env = env_tasks(0);
    let out = env.apm().args(["--json", "next"]).assert().code(3);
    let v: Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert_eq!(v, json!({ "task": null }));
    assert!(out.get_output().stderr.is_empty());
    let out = env.apm().args(["next", "-p", "APM"]).assert().code(3);
    assert_eq!(
        String::from_utf8_lossy(&out.get_output().stdout),
        "nothing ready\n"
    );
    // Errors still exit 1.
    env.apm().args(["next", "-p", "NOPE"]).assert().code(1);
}

#[test]
fn next_text_is_one_line() {
    let env = env_tasks(1);
    let out = env.apm().args(["--as", "bot", "next"]).assert().success();
    let text = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    assert!(
        text.starts_with("claimed APM-1 until ") && text.ends_with(": t1\n"),
        "{text}"
    );
    assert_eq!(text.lines().count(), 1);
}

/// The point of claims: agents racing on `next` never get the same task.
#[test]
fn parallel_next_claims_each_task_once() {
    const TASKS: usize = 5;
    const AGENTS: usize = 8;
    for round in 0..3 {
        let env = env_tasks(TASKS);
        let db = env.db_path();
        let workers: Vec<_> = (0..AGENTS)
            .map(|n| {
                let db = db.clone();
                std::thread::spawn(move || {
                    std::process::Command::new(env!("CARGO_BIN_EXE_apm"))
                        .env("APM_DB", &db)
                        .args(["--json", "--as", &format!("agent{n}"), "next"])
                        .output()
                        .unwrap()
                })
            })
            .collect();
        let mut got = Vec::new();
        let mut empty = 0;
        for w in workers {
            let out = w.join().unwrap();
            let stderr = String::from_utf8_lossy(&out.stderr);
            let v: Value = serde_json::from_slice(&out.stdout).unwrap_or(Value::Null);
            match out.status.code() {
                Some(0) => got.push((
                    v["id"].as_str().unwrap().to_string(),
                    v["claim"]["by"].clone(),
                )),
                Some(3) => {
                    assert_eq!(v, json!({ "task": null }));
                    empty += 1;
                }
                code => panic!("round {round}: exit {code:?}: {stderr}"),
            }
        }
        let mut tasks: Vec<&String> = got.iter().map(|(id, _)| id).collect();
        tasks.sort();
        tasks.dedup();
        assert_eq!(tasks.len(), TASKS, "round {round}: {got:?}");
        assert_eq!(got.len(), TASKS, "round {round}: {got:?}");
        assert_eq!(empty, AGENTS - TASKS, "round {round}");
        // Each task holds exactly one claim, by the agent that got it.
        let claims = env.json(&["claims"]);
        for (id, by) in &got {
            let c = claims
                .as_array()
                .unwrap()
                .iter()
                .find(|c| c["id"] == id.as_str())
                .unwrap();
            assert_eq!(&c["claim"]["by"], by);
        }
        let n: i64 = env
            .conn()
            .query_row(
                "SELECT count(*) FROM events WHERE kind = 'claimed'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, TASKS as i64);
    }
}
