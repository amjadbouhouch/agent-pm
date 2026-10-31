mod common;

use common::Env;
use serde_json::{Value, json};

/// Project APM with tasks APM-1 and APM-2.
fn seed(env: &Env) {
    env.json(&["project", "add", "APM", "agent-pm"]);
    env.json(&["add", "first", "-p", "APM"]);
    env.json(&["add", "second", "-p", "APM"]);
}

fn count(env: &Env, sql: &str) -> i64 {
    env.conn().query_row(sql, [], |r| r.get(0)).unwrap()
}

/// FTS5's own check that `pages_fts` matches `pages` exactly.
fn fts_consistent(env: &Env) {
    env.conn()
        .execute(
            "INSERT INTO pages_fts (pages_fts, rank) VALUES ('integrity-check', 1)",
            [],
        )
        .unwrap();
}

/// The task's events of a kind, as (old, new).
fn events(env: &Env, task: &str, kind: &str) -> Vec<(Value, Value)> {
    env.json(&["history", task])
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["kind"] == kind)
        .map(|e| (e["old"].clone(), e["new"].clone()))
        .collect()
}

#[test]
fn add_returns_the_page() {
    let env = Env::new();
    seed(&env);
    let v = env.json_stdin(
        &[
            "page",
            "add",
            "apm:Storage",
            "Storage",
            "-s",
            "SQLite file, WAL",
            "-b",
            "-",
            "--link",
            "APM-2",
            "--link",
            "apm-1",
            "--link",
            "APM-2",
            "--pin",
        ],
        "# Storage\n\nOne writer per tx.\n",
    );
    assert_eq!(v["id"], "APM:storage");
    assert_eq!(v["project"], "APM");
    assert_eq!(v["slug"], "storage");
    assert_eq!(v["title"], "Storage");
    assert_eq!(v["summary"], "SQLite file, WAL");
    assert_eq!(v["body"], "# Storage\n\nOne writer per tx.");
    assert_eq!(v["pinned"], true);
    assert_eq!(v["version"], 1);
    assert_eq!(v["tasks"], json!(["APM-1", "APM-2"]));
    assert_eq!(v["created_by"], "tester");
    assert_eq!(v["updated_by"], "tester");
    assert!(v["created_at"].is_string());
    assert_eq!(v["updated_at"], v["created_at"]);
    assert_eq!(v["verified_by"], Value::Null);
    assert_eq!(v["verified_at"], Value::Null);
    assert_eq!(v["stale"], false);
    assert_eq!(env.json(&["page", "show", "APM:storage"]), v);

    // Minimal page; text line.
    env.apm()
        .args(["page", "add", "APM:conv", "Conventions"])
        .assert()
        .success()
        .stdout("created APM:conv\n");
    let v = env.json(&["page", "show", "apm:CONV"]);
    assert_eq!(
        (v["summary"].clone(), v["body"].clone()),
        (json!(""), json!(""))
    );
    assert_eq!(v["tasks"], json!([]));
    assert_eq!(v["pinned"], false);
}

#[test]
fn add_errors() {
    let env = Env::new();
    seed(&env);
    env.json(&["page", "add", "APM:storage", "Storage"]);
    let e = |args: &[&str]| env.json_err(args);
    assert_eq!(
        e(&["page", "add", "APM:Storage", "again"]),
        "page already exists: APM:storage"
    );
    assert_eq!(
        e(&["page", "add", "APM-3", "x"]),
        "invalid page reference `APM-3`, expected KEY:slug like APM:storage"
    );
    assert_eq!(
        e(&["page", "add", "APM:wal_mode", "x"]),
        "invalid page reference `APM:wal_mode`, expected KEY:slug like APM:storage"
    );
    assert_eq!(e(&["page", "add", "NOPE:x", "x"]), "no such project: NOPE");
    assert_eq!(e(&["page", "add", "APM:x", " "]), "title cannot be empty");
    assert_eq!(
        e(&["page", "add", "APM:x", "x", "-s", &"s".repeat(201)]),
        "summary is 201 characters, at most 200: put detail in the body"
    );
    assert_eq!(
        e(&["page", "add", "APM:x", "x", "-s", "a\nb"]),
        "summary must be one line: put detail in the body"
    );
    assert_eq!(
        e(&["page", "add", "APM:x", "x", "--link", "APM-9"]),
        "no such item: APM-9"
    );
    // Nothing half-written.
    assert_eq!(count(&env, "SELECT count(*) FROM pages"), 1);
    assert_eq!(count(&env, "SELECT count(*) FROM page_versions"), 1);
    env.apm()
        .args(["--json", "page", "add", "APM:x"])
        .assert()
        .code(2);
}

#[test]
fn show_errors_and_old_versions() {
    let env = Env::new();
    seed(&env);
    env.json(&[
        "page",
        "add",
        "APM:storage",
        "Storage",
        "-s",
        "one",
        "-b",
        "v1 body",
    ]);
    env.json(&[
        "--as",
        "bob",
        "page",
        "edit",
        "APM:storage",
        "-b",
        "v2 body",
        "--title",
        "Disk",
    ]);
    let v1 = env.json(&["page", "show", "APM:storage", "--version", "1"]);
    assert_eq!(v1["version"], 1);
    assert_eq!(v1["title"], "Storage");
    assert_eq!(v1["body"], "v1 body");
    assert_eq!(v1["summary"], "one");
    let v2 = env.json(&["page", "show", "APM:storage"]);
    assert_eq!(
        (
            v2["version"].clone(),
            v2["title"].clone(),
            v2["updated_by"].clone()
        ),
        (json!(2), json!("Disk"), json!("bob"))
    );
    assert_eq!(
        env.json(&["page", "show", "APM:storage", "--version", "2"]),
        v2
    );
    assert_eq!(
        env.json_err(&["page", "show", "APM:storage", "--version", "3"]),
        "no such version: APM:storage v3 (latest is v2)"
    );
    assert_eq!(
        env.json_err(&["page", "show", "APM:nope"]),
        "no such page: APM:nope"
    );
    env.apm()
        .args(["--json", "page", "show", "APM:storage", "--version", "0"])
        .assert()
        .code(2);

    let out = env
        .apm()
        .args(["page", "show", "APM:storage", "--version", "1"])
        .assert()
        .success();
    let text = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines[0], "APM:storage  Storage");
    assert_eq!(lines[1], "summary: one");
    assert!(lines[2].starts_with("version: 1 (") && lines[2].ends_with(" by tester)"));
    assert_eq!(lines[3], "pinned: no");
    assert_eq!(&lines[4..], ["", "  v1 body"]);
}

#[test]
fn every_content_edit_is_a_version() {
    let env = Env::new();
    seed(&env);
    env.json(&["page", "add", "APM:storage", "Storage", "-b", "a"]);
    env.json(&["page", "edit", "APM:storage", "-s", "gist"]);
    env.json(&["page", "edit", "APM:storage", "--title", "Disk"]);
    let v = env.json_stdin(&["page", "edit", "APM:storage", "-b", "-"], "new body\n");
    assert_eq!(v["version"], 4);
    assert_eq!(v["body"], "new body");
    // Pin, links, rename and verify make no version.
    env.json(&["page", "edit", "APM:storage", "--pin", "--link", "APM-1"]);
    env.json(&["page", "edit", "APM:storage", "--rename", "disk"]);
    env.json(&["page", "verify", "APM:disk"]);
    assert_eq!(env.json(&["page", "show", "APM:disk"])["version"], 4);
    let history = env.json(&["page", "history", "APM:disk"]);
    let rows: Vec<(i64, &str, &str)> = history
        .as_array()
        .unwrap()
        .iter()
        .map(|v| {
            (
                v["version"].as_i64().unwrap(),
                v["title"].as_str().unwrap(),
                v["summary"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        rows,
        [
            (1, "Storage", ""),
            (2, "Storage", "gist"),
            (3, "Disk", "gist"),
            (4, "Disk", "gist")
        ]
    );
    assert_eq!(history[0]["author"], "tester");
    assert!(history[0]["at"].is_string());
    // The stored versions are the full content, the last one equal to the page.
    let body: String = env
        .conn()
        .query_row(
            "SELECT body FROM page_versions WHERE version = 4",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(body, "new body");
    fts_consistent(&env);
    let out = env
        .apm()
        .args(["page", "history", "APM:disk"])
        .assert()
        .success();
    let text = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    assert!(text.starts_with("V   AT"), "{text}");
    assert!(text.contains("v4  "));
}

#[test]
fn edit_rejects_a_stale_expect_version() {
    let env = Env::new();
    seed(&env);
    env.json(&["page", "add", "APM:storage", "Storage", "-b", "mine"]);
    // Two agents read v1; bob edits first.
    let v = env.json(&[
        "--as",
        "bob",
        "page",
        "edit",
        "APM:storage",
        "-b",
        "bob's",
        "--expect-version",
        "1",
    ]);
    assert_eq!(v["version"], 2);
    assert_eq!(
        env.json_err(&[
            "page",
            "edit",
            "APM:storage",
            "-b",
            "alice's",
            "--expect-version",
            "1"
        ]),
        "APM:storage is at version 2, not 1: someone changed it since you read it; \
         re-read it (apm page show APM:storage) and edit again"
    );
    env.apm()
        .args([
            "page",
            "edit",
            "APM:storage",
            "--pin",
            "--expect-version",
            "1",
        ])
        .assert()
        .code(1);
    let v = env.json(&["page", "show", "APM:storage"]);
    assert_eq!(
        (v["body"].clone(), v["pinned"].clone()),
        (json!("bob's"), json!(false))
    );
    assert_eq!(count(&env, "SELECT count(*) FROM page_versions"), 2);
    // With the current version, it goes through.
    let v = env.json(&[
        "page",
        "edit",
        "APM:storage",
        "-b",
        "merged",
        "--expect-version",
        "2",
    ]);
    assert_eq!(v["version"], 3);
}

#[test]
fn edit_appends() {
    let env = Env::new();
    seed(&env);
    env.json(&["page", "add", "APM:log", "Log"]);
    // On an empty body the text is the body; then each section follows a blank line.
    let v = env.json(&["page", "edit", "APM:log", "--append", "first"]);
    assert_eq!(v["body"], "first");
    let v = env.json_stdin(
        &["--as", "bob", "page", "edit", "APM:log", "--append", "-"],
        "## second\nline\n",
    );
    assert_eq!(v["body"], "first\n\n## second\nline");
    assert_eq!(v["version"], 3);
    assert_eq!(v["updated_by"], "bob");
    assert_eq!(
        env.json_err(&["page", "edit", "APM:log", "--append", "  "]),
        "text to append cannot be empty"
    );
    // --append replaces nothing, so it cannot go with -b.
    env.apm()
        .args([
            "--json", "page", "edit", "APM:log", "--append", "x", "-b", "y",
        ])
        .assert()
        .code(2);
    fts_consistent(&env);
}

#[test]
fn noop_edit_is_unchanged() {
    let env = Env::new();
    seed(&env);
    env.json(&[
        "page",
        "add",
        "APM:storage",
        "Storage",
        "-s",
        "gist",
        "-b",
        "body",
        "--pin",
        "--link",
        "APM-1",
    ]);
    let conn = env.conn();
    conn.execute_batch(
        "UPDATE pages SET updated_at = '2000-01-01T00:00:00Z';
         UPDATE items SET updated_at = '2000-01-01T00:00:00Z';",
    )
    .unwrap();
    drop(conn);
    let before = env.json(&["page", "show", "APM:storage"]);
    let events_before = env.json(&["history", "APM-1"]);
    env.apm()
        .args([
            "page",
            "edit",
            "apm:storage",
            "--title",
            "Storage",
            "-s",
            "gist",
            "-b",
            "body",
            "--pin",
            "--rename",
            "Storage",
            "--link",
            "APM-1",
            "--unlink",
            "APM-2",
        ])
        .assert()
        .success()
        .stdout("unchanged APM:storage\n");
    assert_eq!(env.json(&["page", "show", "APM:storage"]), before);
    assert_eq!(env.json(&["history", "APM-1"]), events_before);
    assert_eq!(
        env.json(&["show", "APM-1"])["updated_at"],
        "2000-01-01T00:00:00Z"
    );
    assert_eq!(count(&env, "SELECT count(*) FROM page_versions"), 1);
    assert_eq!(
        env.json_err(&["page", "edit", "APM:storage"]),
        "nothing to change: pass --title, -s, -b, --append, --rename, --pin, --unpin, --link or --unlink"
    );
    assert_eq!(
        env.json_err(&[
            "page",
            "edit",
            "APM:storage",
            "--link",
            "APM-2",
            "--unlink",
            "apm-2"
        ]),
        "cannot both link and unlink APM-2"
    );
}

#[test]
fn links_record_events_on_the_task() {
    let env = Env::new();
    seed(&env);
    env.json(&["page", "add", "APM:storage", "Storage", "--link", "APM-1"]);
    assert_eq!(
        events(&env, "APM-1", "paged"),
        [(Value::Null, json!("APM:storage"))]
    );
    env.conn()
        .execute("UPDATE items SET updated_at = '2000-01-01T00:00:00Z'", [])
        .unwrap();
    let v = env.json(&[
        "page",
        "edit",
        "APM:storage",
        "--link",
        "APM-2",
        "--unlink",
        "APM-1",
    ]);
    assert_eq!(v["tasks"], json!(["APM-2"]));
    assert_eq!(
        events(&env, "APM-1", "unpaged"),
        [(json!("APM:storage"), Value::Null)]
    );
    assert_eq!(
        events(&env, "APM-2", "paged"),
        [(Value::Null, json!("APM:storage"))]
    );
    for task in ["APM-1", "APM-2"] {
        assert_ne!(
            env.json(&["show", task])["updated_at"],
            "2000-01-01T00:00:00Z"
        );
    }
    // Linking the page has no effect on the page's own updated_at (not a content edit).
    let page = env.json(&["page", "show", "APM:storage"]);
    assert_eq!(page["version"], 1);
    // History text.
    let out = env.apm().args(["history", "APM-1"]).assert().success();
    let text = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    assert!(text.contains(" tester paged APM:storage\n"), "{text}");
    assert!(text.contains(" tester unpaged APM:storage\n"), "{text}");
    // page list --task.
    let ids = |v: Value| -> Vec<String> {
        v.as_array()
            .unwrap()
            .iter()
            .map(|p| p["id"].as_str().unwrap().to_string())
            .collect()
    };
    assert_eq!(
        ids(env.json(&["page", "list", "--task", "APM-2"])),
        ["APM:storage"]
    );
    assert!(ids(env.json(&["page", "list", "--task", "APM-1"])).is_empty());
}

#[test]
fn rename_keeps_links() {
    let env = Env::new();
    seed(&env);
    env.json(&[
        "page",
        "add",
        "APM:storage",
        "Storage",
        "--link",
        "APM-1",
        "--link",
        "APM-2",
    ]);
    let v = env.json(&["page", "edit", "APM:storage", "--rename", "Disk"]);
    assert_eq!(v["id"], "APM:disk");
    assert_eq!(v["slug"], "disk");
    assert_eq!(v["tasks"], json!(["APM-1", "APM-2"]));
    assert_eq!(v["version"], 1);
    assert_eq!(
        env.json_err(&["page", "show", "APM:storage"]),
        "no such page: APM:storage"
    );
    env.json(&["page", "add", "APM:other", "Other"]);
    assert_eq!(
        env.json_err(&["page", "edit", "APM:other", "--rename", "disk"]),
        "page already exists: APM:disk"
    );
    assert!(
        env.json_err(&["page", "edit", "APM:other", "--rename=-x"])
            .starts_with("invalid page slug `-x`")
    );
    // Links added in the same edit name the new id.
    env.json(&["add", "third", "-p", "APM"]);
    env.json(&[
        "page", "edit", "APM:disk", "--rename", "drive", "--link", "APM-3",
    ]);
    assert_eq!(
        events(&env, "APM-3", "paged"),
        [(Value::Null, json!("APM:drive"))]
    );
}

#[test]
fn old_project_key_resolves_pages() {
    let env = Env::new();
    seed(&env);
    env.json(&["page", "add", "APM:storage", "Storage", "--link", "APM-1"]);
    env.json(&["project", "edit", "APM", "--key", "OPS"]);
    let v = env.json(&["page", "show", "APM:storage"]);
    assert_eq!(v["id"], "OPS:storage");
    assert_eq!(v["project"], "OPS");
    assert_eq!(v["tasks"], json!(["OPS-1"]));
    let v = env.json(&["page", "edit", "apm:storage", "-s", "moved"]);
    assert_eq!(v["id"], "OPS:storage");
    // A new page under the old key lands in the project as it is now.
    assert_eq!(
        env.json(&["page", "add", "APM:conv", "Conv"])["id"],
        "OPS:conv"
    );
    assert_eq!(
        env.json_err(&["page", "add", "apm:Storage", "dup"]),
        "page already exists: OPS:storage"
    );
    assert_eq!(
        env.json(&["page", "list", "-p", "APM"])
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn verify_and_staleness() {
    let env = Env::new();
    seed(&env);
    env.json(&[
        "page",
        "add",
        "APM:old",
        "Old",
        "-b",
        "first line\nsecond",
        "--pin",
    ]);
    env.json(&["page", "add", "APM:fresh", "Fresh", "-s", "kept"]);
    env.json(&["page", "add", "APM:checked", "Checked"]);
    // Make two of them 31 days old; one of those was verified 31 days ago too.
    env.conn()
        .execute_batch(
            "UPDATE pages SET updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-31 days')
             WHERE slug IN ('old', 'checked');
             UPDATE pages SET verified_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-31 days'),
                              verified_by = 'bob'
             WHERE slug = 'checked';",
        )
        .unwrap();
    let stale = |env: &Env| -> Vec<String> {
        env.json(&["page", "list", "--stale"])
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["id"].as_str().unwrap().to_string())
            .collect()
    };
    assert_eq!(stale(&env), ["APM:checked", "APM:old"]);
    assert_eq!(env.json(&["page", "show", "APM:old"])["stale"], true);
    let out = env
        .apm()
        .args(["page", "show", "APM:old"])
        .assert()
        .success();
    let text = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    assert!(
        text.contains(
            "stale: not edited or verified for 30 days; re-read it, then apm page verify \
             APM:old (still true) or apm page edit APM:old (fix it)"
        ),
        "{text}"
    );

    // Verify: still true; no new version, not a content edit.
    let before = env.json(&["page", "show", "APM:checked"]);
    let v = env.json(&["--as", "carol", "page", "verify", "apm:checked"]);
    assert_eq!(v["verified_by"], "carol");
    assert_ne!(v["verified_at"], before["verified_at"]);
    assert_eq!(v["updated_at"], before["updated_at"]);
    assert_eq!(v["version"], 1);
    assert_eq!(v["stale"], false);
    env.apm()
        .args(["page", "verify", "APM:checked"])
        .assert()
        .success()
        .stdout("verified APM:checked\n");
    // An edit makes a page fresh too.
    env.json(&["page", "edit", "APM:old", "--append", "still true"]);
    assert!(stale(&env).is_empty());

    // list: all, pinned; JSON summaries fall back to the body's first line.
    let list = env.json(&["page", "list", "-p", "APM"]);
    assert_eq!(
        list[2],
        json!({
            "id": "APM:old", "title": "Old", "summary": "first line", "pinned": true,
            "version": 2, "updated_at": list[2]["updated_at"],
            "verified_at": null, "stale": false
        })
    );
    assert_eq!(list[1]["summary"], "kept");
    let pinned = env.json(&["page", "list", "--pinned"]);
    assert_eq!(pinned.as_array().unwrap().len(), 1);
    assert_eq!(
        env.json_err(&["page", "list", "-p", "NOPE"]),
        "no such project: NOPE"
    );
    let out = env.apm().args(["page", "list"]).assert().success();
    let text = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines[0], "ID           V   CHECKED     FLAGS   TITLE");
    assert!(lines[3].starts_with("APM:old      v2  20") && lines[3].ends_with("  pinned  Old"));
    assert_eq!(
        env.json_err(&["page", "verify", "APM:nope"]),
        "no such page: APM:nope"
    );
}

#[test]
fn rm_removes_versions_links_and_index_rows() {
    let env = Env::new();
    seed(&env);
    env.json(&[
        "page",
        "add",
        "APM:storage",
        "Storage",
        "-b",
        "aardvark",
        "--link",
        "APM-1",
    ]);
    env.json(&["page", "edit", "APM:storage", "--append", "more aardvark"]);
    env.json(&["page", "add", "APM:keep", "Keep", "-b", "aardvark too"]);
    let v = env.json(&["page", "rm", "apm:storage"]);
    assert_eq!(v["id"], "APM:storage");
    assert_eq!(v["tasks"], json!(["APM-1"]));
    assert_eq!(
        env.json_err(&["page", "show", "APM:storage"]),
        "no such page: APM:storage"
    );
    assert_eq!(count(&env, "SELECT count(*) FROM pages"), 1);
    assert_eq!(count(&env, "SELECT count(*) FROM page_versions"), 1);
    assert_eq!(count(&env, "SELECT count(*) FROM page_links"), 0);
    assert_eq!(
        count(
            &env,
            "SELECT count(*) FROM pages_fts WHERE pages_fts MATCH 'aardvark'"
        ),
        1
    );
    fts_consistent(&env);
    // The task records the unlink.
    assert_eq!(
        events(&env, "APM-1", "unpaged"),
        [(json!("APM:storage"), Value::Null)]
    );
    env.apm()
        .args(["page", "rm", "APM:keep"])
        .assert()
        .success()
        .stdout("deleted APM:keep\n");
    assert_eq!(
        env.json_err(&["page", "rm", "APM:keep"]),
        "no such page: APM:keep"
    );
}

#[test]
fn page_help_says_what_things_are_for() {
    let env = Env::new();
    let help = |args: &[&str]| -> String {
        let out = env.apm().args(args).assert().success();
        String::from_utf8(out.get_output().stdout.clone()).unwrap()
    };
    let top = help(&["--help"]);
    assert!(top.contains("knowledge that outlives a task"), "{top}");
    let add = help(&["page", "add", "--help"]);
    for line in [
        "the gist, shown in `apm context`",
        "it shows in the context of every task of its project",
    ] {
        assert!(add.contains(line), "{line} not in:\n{add}");
    }
    let edit = help(&["page", "edit", "--help"]);
    assert!(
        edit.contains("so you never overwrite someone else's change"),
        "{edit}"
    );
    let page = help(&["page", "--help"]);
    assert!(page.contains("re-read and still true"), "{page}");
}
