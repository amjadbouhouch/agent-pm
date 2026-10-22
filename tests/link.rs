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

/// Projects APM (tasks APM-1..4: a b c d) and OPS (OPS-1: z).
fn env_tasks() -> Env {
    let env = Env::new();
    env.json(&["project", "add", "APM", "Agent PM"]);
    env.json(&["project", "add", "OPS", "Ops"]);
    for t in ["a", "b", "c", "d"] {
        env.json(&["add", t, "-p", "APM"]);
    }
    env.json(&["add", "z", "-p", "OPS"]);
    env
}

/// The ids in one `links` group of `show KEY-N`.
fn group(env: &Env, key: &str, name: &str) -> Vec<String> {
    ids(&env.json(&["show", key])["links"][name])
}

#[test]
fn link_returns_the_link_and_show_lists_both_directions() {
    let env = env_tasks();
    let l = env.json(&["link", "apm-1", "blocks", "APM-2"]);
    assert_eq!(l, json!({"from": "APM-1", "kind": "blocks", "to": "APM-2"}));
    env.json(&["link", "APM-1", "duplicates", "APM-3"]);
    env.json(&["link", "APM-1", "relates", "OPS-1"]);
    env.json(&["link", "APM-4", "relates", "APM-1"]);

    let a = env.json(&["show", "APM-1"]);
    assert_eq!(
        a["links"]["blocks"],
        json!([{"id": "APM-2", "title": "b", "status": "todo"}])
    );
    assert_eq!(a["links"]["blocked_by"], json!([]));
    // relates is symmetric: stored once, shown on both, sorted by id.
    assert_eq!(ids(&a["links"]["relates"]), ["APM-4", "OPS-1"]);
    assert_eq!(ids(&a["links"]["duplicates"]), ["APM-3"]);
    assert_eq!(group(&env, "APM-2", "blocked_by"), ["APM-1"]);
    assert_eq!(group(&env, "APM-3", "duplicated_by"), ["APM-1"]);
    assert_eq!(group(&env, "OPS-1", "relates"), ["APM-1"]);
    assert_eq!(group(&env, "APM-4", "relates"), ["APM-1"]);

    // The entry shows the linked task's current status.
    env.json(&["done", "APM-2"]);
    assert_eq!(
        env.json(&["show", "APM-1"])["links"]["blocks"][0]["status"],
        "done"
    );
}

#[test]
fn link_text_output() {
    let env = env_tasks();
    env.apm()
        .args(["link", "APM-1", "blocks", "OPS-1"])
        .assert()
        .success()
        .stdout("linked APM-1 blocks OPS-1\n");
    let out = env.apm().args(["show", "OPS-1"]).assert().success();
    let text = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    assert!(text.contains("\nblocked by:\n  APM-1  todo  a\n"), "{text}");
    env.apm()
        .args(["unlink", "APM-1", "blocks", "OPS-1"])
        .assert()
        .success()
        .stdout("unlinked APM-1 blocks OPS-1\n");
}

#[test]
fn link_errors() {
    let env = env_tasks();
    assert_eq!(
        env.json_err(&["link", "APM-1", "blocks", "apm-1"]),
        "a task cannot link to itself: APM-1"
    );
    assert_eq!(
        env.json_err(&["link", "APM-1", "blocks", "APM-9"]),
        "no such item: APM-9"
    );
    env.json(&["link", "APM-1", "blocks", "APM-2"]);
    assert_eq!(
        env.json_err(&["link", "APM-1", "blocks", "APM-2"]),
        "link already exists: APM-1 blocks APM-2"
    );
    // relates matches either direction.
    env.json(&["link", "APM-3", "relates", "APM-4"]);
    assert_eq!(
        env.json_err(&["link", "APM-4", "relates", "APM-3"]),
        "link already exists: APM-4 relates APM-3"
    );
    // Other kinds between the same pair are separate links.
    env.json(&["link", "APM-1", "relates", "APM-2"]);
}

#[test]
fn unknown_kind_is_a_usage_error() {
    let env = env_tasks();
    let out = env
        .apm()
        .args(["--json", "link", "APM-1", "parent-of", "APM-2"])
        .assert()
        .code(2);
    let v: Value = serde_json::from_slice(&out.get_output().stderr).unwrap();
    assert!(v["error"].as_str().unwrap().contains("parent-of"), "{v}");
}

#[test]
fn blocks_cycles_are_refused_and_named() {
    let env = env_tasks();
    env.json(&["link", "APM-1", "blocks", "APM-2"]);
    assert_eq!(
        env.json_err(&["link", "APM-2", "blocks", "APM-1"]),
        "blocks cycle: APM-2 blocks APM-1 blocks APM-2"
    );
    env.json(&["link", "APM-2", "blocks", "OPS-1"]);
    assert_eq!(
        env.json_err(&["link", "OPS-1", "blocks", "APM-1"]),
        "blocks cycle: OPS-1 blocks APM-1 blocks APM-2 blocks OPS-1"
    );
    // A diamond is not a cycle; other kinds may point back.
    env.json(&["link", "APM-1", "blocks", "APM-3"]);
    env.json(&["link", "APM-3", "blocks", "OPS-1"]);
    env.json(&["link", "OPS-1", "relates", "APM-1"]);
    env.json(&["link", "OPS-1", "duplicates", "APM-1"]);
}

#[test]
fn unlink_removes_and_errors_when_missing() {
    let env = env_tasks();
    env.json(&["link", "APM-1", "blocks", "APM-2"]);
    env.json(&["link", "APM-1", "relates", "APM-3"]);
    assert_eq!(
        env.json_err(&["unlink", "APM-2", "blocks", "APM-1"]),
        "no such link: APM-2 blocks APM-1"
    );
    assert_eq!(
        env.json_err(&["unlink", "APM-1", "duplicates", "APM-2"]),
        "no such link: APM-1 duplicates APM-2"
    );
    let l = env.json(&["unlink", "APM-1", "blocks", "APM-2"]);
    assert_eq!(l, json!({"from": "APM-1", "kind": "blocks", "to": "APM-2"}));
    // relates can be removed from either end.
    env.json(&["unlink", "APM-3", "relates", "APM-1"]);
    let links = &env.json(&["show", "APM-1"])["links"];
    assert_eq!(links["blocks"], json!([]));
    assert_eq!(links["relates"], json!([]));
}

#[test]
fn links_go_with_their_task() {
    let env = env_tasks();
    env.json(&["link", "APM-1", "blocks", "APM-2"]);
    env.json(&["rm", "APM-1"]);
    assert_eq!(group(&env, "APM-2", "blocked_by"), Vec::<String>::new());
    assert_eq!(env.json(&["list", "--blocked"]), json!([]));
}

#[test]
fn ready_and_blocked() {
    let env = env_tasks();
    // APM-1 blocks APM-2 and APM-3; APM-4 blocks APM-3; OPS-1 is free.
    env.json(&["link", "APM-1", "blocks", "APM-2"]);
    env.json(&["link", "APM-1", "blocks", "APM-3"]);
    env.json(&["link", "APM-4", "blocks", "APM-3"]);
    env.json(&["link", "OPS-1", "relates", "APM-1"]);
    assert_eq!(
        ids(&env.json(&["list", "--ready"])),
        ["APM-1", "APM-4", "OPS-1"]
    );
    assert_eq!(ids(&env.json(&["list", "--blocked"])), ["APM-2", "APM-3"]);
    assert_eq!(
        ids(&env.json(&["list", "--blocked", "-p", "OPS"])),
        Vec::<String>::new()
    );

    // A closed blocker no longer blocks; done and cancelled both count as closed.
    env.json(&["done", "APM-1"]);
    assert_eq!(
        ids(&env.json(&["list", "--ready"])),
        ["APM-2", "APM-4", "OPS-1"]
    );
    assert_eq!(ids(&env.json(&["list", "--blocked"])), ["APM-3"]);
    env.json(&["edit", "APM-4", "--status", "cancelled"]);
    assert_eq!(ids(&env.json(&["list", "--blocked"])), Vec::<String>::new());
    // Ready/blocked tasks are open ones, even with --all.
    assert_eq!(
        ids(&env.json(&["list", "--ready", "--all"])),
        ["APM-2", "APM-3", "OPS-1"]
    );
    // They combine with other filters.
    assert_eq!(
        ids(&env.json(&["list", "--ready", "-p", "APM"])),
        ["APM-2", "APM-3"]
    );
    env.apm()
        .args(["list", "--ready", "--blocked"])
        .assert()
        .code(2);
}

#[test]
fn a_status_named_blocked_counts_as_blocked() {
    let env = env_tasks();
    env.json(&["block", "APM-2"]);
    assert_eq!(
        ids(&env.json(&["list", "--ready", "-p", "APM"])),
        ["APM-1", "APM-3", "APM-4"]
    );
    assert_eq!(ids(&env.json(&["list", "--blocked"])), ["APM-2"]);
    // The name is matched case-insensitively, like the `block` shortcut.
    env.json(&[
        "status", "edit", "blocked", "-p", "APM", "--rename", "Blocked",
    ]);
    assert_eq!(ids(&env.json(&["list", "--blocked"])), ["APM-2"]);
    // A blocked task that is also link-blocked is listed once.
    env.json(&["link", "APM-1", "blocks", "APM-2"]);
    assert_eq!(ids(&env.json(&["list", "--blocked"])), ["APM-2"]);
}

#[test]
fn custom_closed_statuses_release_blocked_tasks() {
    let env = env_tasks();
    env.json(&[
        "status",
        "add",
        "shipped",
        "-p",
        "APM",
        "--category",
        "done",
    ]);
    env.json(&[
        "status",
        "add",
        "review",
        "-p",
        "APM",
        "--category",
        "active",
    ]);
    env.json(&["link", "APM-1", "blocks", "APM-2"]);
    // An open custom status still blocks.
    env.json(&["edit", "APM-1", "--status", "review"]);
    assert_eq!(ids(&env.json(&["list", "--blocked"])), ["APM-2"]);
    // A custom done-category status does not, and the blocker leaves --ready too.
    env.json(&["edit", "APM-1", "--status", "shipped"]);
    assert_eq!(ids(&env.json(&["list", "--blocked"])), Vec::<String>::new());
    assert_eq!(
        ids(&env.json(&["list", "--ready", "-p", "APM"])),
        ["APM-2", "APM-3", "APM-4"]
    );
}

#[test]
fn link_and_unlink_bump_both_tasks() {
    let env = env_tasks();
    // Back-date both tasks so the bump is visible within the same second.
    env.conn()
        .execute("UPDATE items SET updated_at = '2000-01-01T00:00:00Z'", [])
        .unwrap();
    let updated = |key: &str| env.json(&["show", key])["updated_at"].clone();
    env.json(&["link", "APM-1", "relates", "OPS-1"]);
    assert_ne!(updated("APM-1"), "2000-01-01T00:00:00Z");
    assert_ne!(updated("OPS-1"), "2000-01-01T00:00:00Z");
    assert_eq!(updated("APM-2"), "2000-01-01T00:00:00Z");
    env.conn()
        .execute("UPDATE items SET updated_at = '2000-01-01T00:00:00Z'", [])
        .unwrap();
    env.json(&["unlink", "OPS-1", "relates", "APM-1"]);
    assert_ne!(updated("APM-1"), "2000-01-01T00:00:00Z");
    assert_ne!(updated("OPS-1"), "2000-01-01T00:00:00Z");
    // A failed link bumps nothing.
    env.conn()
        .execute("UPDATE items SET updated_at = '2000-01-01T00:00:00Z'", [])
        .unwrap();
    env.json_err(&["unlink", "APM-1", "relates", "OPS-1"]);
    assert_eq!(updated("APM-1"), "2000-01-01T00:00:00Z");
}

#[test]
fn link_events_on_both_tasks() {
    let env = env_tasks();
    env.json(&["--as", "alice", "link", "APM-1", "blocks", "OPS-1"]);
    env.json(&["--as", "bob", "unlink", "APM-1", "blocks", "OPS-1"]);
    env.json(&["link", "APM-2", "duplicates", "APM-1"]);
    let pick = |v: &Value| {
        v.as_array()
            .unwrap()
            .iter()
            .skip(1) // created
            .map(|e| {
                (
                    e["actor"].clone(),
                    e["kind"].clone(),
                    e["field"].clone(),
                    e["old"].clone(),
                    e["new"].clone(),
                )
            })
            .collect::<Vec<_>>()
    };
    let e = |a: &str, k: &str, f: &str, o: Option<&str>, n: Option<&str>| {
        (json!(a), json!(k), json!(f), json!(o), json!(n))
    };
    assert_eq!(
        pick(&env.json(&["history", "APM-1"])),
        [
            e("alice", "linked", "blocks", None, Some("OPS-1")),
            e("bob", "unlinked", "blocks", Some("OPS-1"), None),
            e("tester", "linked", "duplicated_by", None, Some("APM-2")),
        ]
    );
    assert_eq!(
        pick(&env.json(&["history", "OPS-1"])),
        [
            e("alice", "linked", "blocked_by", None, Some("APM-1")),
            e("bob", "unlinked", "blocked_by", Some("APM-1"), None),
        ]
    );
    // Failed links record nothing.
    env.json_err(&["link", "OPS-1", "duplicates", "OPS-1"]);
    assert_eq!(env.json(&["history", "OPS-1"]).as_array().unwrap().len(), 3);
}
