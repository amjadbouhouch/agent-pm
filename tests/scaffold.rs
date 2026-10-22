mod common;

use common::Env;

#[test]
fn version_prints() {
    let env = Env::new();
    env.apm().arg("--version").assert().success();
}

#[test]
fn db_is_created_and_migrated() {
    let env = Env::new();
    let v = env.json(&["db"]);
    assert_eq!(v["schema_version"], 1);
    assert!(env.db_path().exists());
    // Reopening an up-to-date database is a no-op.
    assert_eq!(env.json(&["db"])["schema_version"], 1);
}

#[test]
fn db_flag_overrides_env() {
    let env = Env::new();
    let other = env.dir.path().join("sub").join("other.db");
    let v = env.json(&["--db", other.to_str().unwrap(), "db"]);
    assert_eq!(v["path"], other.to_str().unwrap());
    assert!(other.exists());
}

#[test]
fn errors_are_json_on_stderr() {
    let env = Env::new();
    let msg = env.json_err(&["comment", "nope", "hi"]);
    assert!(!msg.is_empty());
}

#[test]
fn guide_does_not_touch_the_database() {
    let env = Env::new();
    env.apm().arg("guide").assert().success();
    assert!(!env.db_path().exists());
}
