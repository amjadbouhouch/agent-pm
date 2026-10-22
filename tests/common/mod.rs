//! Shared test harness: every test gets its own temporary database.
#![allow(dead_code)]

use assert_cmd::Command;
use serde_json::Value;
use tempfile::TempDir;

pub struct Env {
    pub dir: TempDir,
}

impl Env {
    pub fn new() -> Self {
        Env {
            dir: tempfile::tempdir().unwrap(),
        }
    }

    pub fn db_path(&self) -> std::path::PathBuf {
        self.dir.path().join("apm.db")
    }

    /// `apm` bound to this env's database, acting as `tester`.
    pub fn apm(&self) -> Command {
        let mut cmd = Command::cargo_bin("apm").unwrap();
        cmd.env("APM_DB", self.db_path())
            .env("APM_AGENT", "tester")
            .env_remove("USER")
            .env_remove("USERNAME");
        cmd
    }

    /// Run `apm --json <args>`, assert success, parse stdout.
    pub fn json(&self, args: &[&str]) -> Value {
        let out = self.apm().arg("--json").args(args).assert().success();
        serde_json::from_slice(&out.get_output().stdout).unwrap()
    }

    /// Direct connection to the (migrated) test database, for seeding rows a test needs
    /// that another command group creates. Prefer the CLI when the command exists.
    pub fn conn(&self) -> rusqlite::Connection {
        self.apm().arg("db").assert().success();
        let conn = rusqlite::Connection::open(self.db_path()).unwrap();
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();
        conn
    }

    /// Run `apm --json <args>`, assert failure, return the error message.
    pub fn json_err(&self, args: &[&str]) -> String {
        let out = self.apm().arg("--json").args(args).assert().failure();
        let v: Value = serde_json::from_slice(&out.get_output().stderr).unwrap();
        v["error"].as_str().unwrap().to_string()
    }
}
