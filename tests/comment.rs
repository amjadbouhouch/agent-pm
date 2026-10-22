mod common;

use common::Env;

/// Seed project APM with task APM-1 (projects and tasks live in another slice).
fn seed(env: &Env) {
    env.conn()
        .execute_batch(
            "INSERT INTO projects (key, name) VALUES ('APM', 'agent-pm');
             INSERT INTO items (project_id, seq, title, created_by)
             VALUES ((SELECT id FROM projects WHERE key = 'APM'), 1, 'a task', 'tester');",
        )
        .unwrap();
}

#[test]
fn comment_returns_the_comment() {
    let env = Env::new();
    seed(&env);
    let v = env.json(&["comment", "apm-1", "looks good"]);
    assert!(v["id"].is_i64());
    assert_eq!(v["item"], "APM-1");
    assert_eq!(v["author"], "tester");
    assert_eq!(v["body"], "looks good");
    assert!(v["created_at"].is_string());

    let v = env.json(&["--as", "bot", "comment", "APM-1", "second"]);
    assert_eq!(v["author"], "bot");

    let rows: Vec<(String, String)> = {
        let conn = env.conn();
        let mut stmt = conn
            .prepare("SELECT author, body FROM comments ORDER BY id")
            .unwrap();
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    };
    assert_eq!(
        rows,
        [
            ("tester".to_string(), "looks good".to_string()),
            ("bot".to_string(), "second".to_string())
        ]
    );
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
