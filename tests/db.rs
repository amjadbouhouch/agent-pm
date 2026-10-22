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
    assert_eq!(v["schema_version"], 2);
    assert!(env.db_path().exists());
    // Reopening an up-to-date database is a no-op.
    assert_eq!(env.json(&["db"])["schema_version"], 2);
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
fn usage_errors_are_json_under_json_flag() {
    let env = Env::new();
    let out = env.apm().args(["--json", "add"]).assert().code(2);
    let v: serde_json::Value = serde_json::from_slice(&out.get_output().stderr).unwrap();
    assert!(v["error"].as_str().unwrap().contains("required"));
}

#[test]
fn usage_errors_are_text_without_json_flag() {
    let env = Env::new();
    env.apm().arg("add").assert().code(2);
}

#[test]
fn guide_does_not_touch_the_database() {
    let env = Env::new();
    env.apm().arg("guide").assert().success();
    assert!(!env.db_path().exists());
}
