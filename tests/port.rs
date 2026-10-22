mod common;

use common::Env;
use serde_json::Value;

/// Run each command (without --json) against `env`, asserting success.
fn run_all(env: &Env, cmds: &[&[&str]]) {
    for args in cmds {
        env.apm().args(*args).assert().success();
    }
}

/// Two projects with everything an export carries: custom statuses, milestones, attributes
/// of every type, subtasks two deep, tags, every link kind (one across projects), comments,
/// and history including moves both ways. APM-5 is deleted, so next_seq is past a gap.
fn rich(env: &Env) {
    run_all(
        env,
        &[
            &["project", "add", "APM", "agent-pm", "-d", "the tracker"],
            &["project", "add", "OPS", "ops"],
            &[
                "status",
                "add",
                "review",
                "-p",
                "APM",
                "--category",
                "active",
                "--position",
                "3",
            ],
            &[
                "status",
                "edit",
                "doing",
                "-p",
                "APM",
                "--rename",
                "in progress",
            ],
            &[
                "milestone",
                "add",
                "v1",
                "-p",
                "APM",
                "--due",
                "2026-12-01T09:00+02:00",
            ],
            &[
                "milestone",
                "add",
                "v0",
                "-p",
                "APM",
                "--due",
                "2026-01-01",
                "-d",
                "first",
            ],
            &["milestone", "close", "v0", "-p", "APM"],
            &[
                "attr",
                "add",
                "size",
                "-p",
                "APM",
                "--type",
                "select",
                "--options",
                "s,m,l",
            ],
            &["attr", "add", "points", "-p", "APM", "--type", "number"],
            &["attr", "add", "note", "-p", "APM", "--type", "text"],
            &["attr", "add", "start", "-p", "APM", "--type", "date"],
            &["attr", "add", "at", "-p", "APM", "--type", "datetime"],
            &["attr", "add", "ok", "-p", "APM", "--type", "checkbox"],
            &["attr", "add", "size", "-p", "OPS", "--type", "text"],
            &[
                "add",
                "parse args",
                "-p",
                "APM",
                "-m",
                "v1",
                "-a",
                "alice",
                "--priority",
                "high",
                "--due",
                "2026-11-20T17:00+01:00",
                "-b",
                "line one\nline two",
                "--set",
                "size=m",
                "--set",
                "points=2.5",
                "--set",
                "note= spaced ",
                "--set",
                "start=2026-11-01",
                "--set",
                "at=2026-11-02T10:00Z",
                "--set",
                "ok=yes",
                "--tag",
                "perf",
                "--tag",
                "wip",
            ],
            &["add", "flag tests", "--parent", "APM-1"],
            &["add", "edge cases", "--parent", "APM-2", "-m", "v0"],
            &["add", "docs", "-p", "APM", "--due", "2026-12-24"],
            &["add", "ops thing", "-p", "OPS", "--set", "size=big"],
            &["add", "ops other", "-p", "OPS"],
            &["add", "to delete", "-p", "APM"],
            &["rm", "APM-5"],
            &["move", "OPS-1", "APM"],
            &["move", "APM-4", "OPS"],
            &[
                "edit",
                "APM-1",
                "--status",
                "review",
                "--priority",
                "urgent",
            ],
            &["start", "APM-2"],
            &["done", "APM-3"],
            &["edit", "APM-3", "--tag", "x:y"],
            &["comment", "APM-1", "first comment"],
            &["comment", "APM-1", "second\nmultiline"],
            &["comment", "APM-2", "on a subtask"],
            &["link", "APM-1", "blocks", "APM-2"],
            &["link", "APM-2", "relates", "APM-3"],
            &["link", "APM-3", "duplicates", "APM-1"],
            &["link", "APM-1", "blocks", "OPS-3"],
            &["link", "OPS-2", "relates", "APM-6"],
        ],
    );
}

fn write(env: &Env, name: &str, text: &str) -> String {
    let path = env.dir.path().join(name);
    std::fs::write(&path, text).unwrap();
    path.to_str().unwrap().to_string()
}

/// `apm export` stdout, parsed.
fn export(env: &Env, args: &[&str]) -> Value {
    let out = env.apm().arg("export").args(args).assert().success();
    serde_json::from_slice(&out.get_output().stdout).unwrap()
}

fn without_time(mut v: Value) -> Value {
    v.as_object_mut().unwrap().remove("exported_at");
    v
}

fn project_keys(env: &Env) -> Vec<String> {
    env.json(&["project", "list"])
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["key"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn round_trip_is_identical() {
    let src = Env::new();
    rich(&src);
    let first = export(&src, &[]);
    assert_eq!(first["apm_export"], 1);
    let keys: Vec<&str> = first["projects"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["key"].as_str().unwrap())
        .collect();
    assert_eq!(keys, ["APM", "OPS"]);
    let apm = &first["projects"][0];
    assert_eq!(apm["next_seq"], 7);
    let statuses: Vec<&str> = apm["statuses"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        statuses,
        [
            "todo",
            "in progress",
            "review",
            "blocked",
            "done",
            "cancelled"
        ]
    );
    let t1 = &apm["tasks"][0];
    assert_eq!(t1["id"], "APM-1");
    assert_eq!(t1["values"]["note"], " spaced ");
    assert_eq!(t1["values"]["ok"], "true");
    assert_eq!(t1["values"]["at"], "2026-11-02T10:00:00Z");
    assert_eq!(apm["tasks"][2]["parent"], "APM-2");
    assert_eq!(first["links"].as_array().unwrap().len(), 5);
    // The moved-in task keeps its history, including the move.
    let moved = &apm["tasks"][3];
    assert_eq!(moved["id"], "APM-6");
    assert!(
        moved["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["kind"] == "moved" && e["old"] == "OPS-1")
    );
    // Not exported: aliases.
    let text = serde_json::to_string(&first).unwrap();
    assert!(!text.contains("alias"));

    let file = write(&src, "export.json", &text);
    let dst = Env::new();
    let res = dst.json(&["import", &file]);
    assert_eq!(res["dry_run"], false);
    assert_eq!(res["links"], 5);
    assert_eq!(res["projects"][0]["key"], "APM");
    assert_eq!(res["projects"][0]["tasks"], 4);
    let second = export(&dst, &[]);
    assert_eq!(without_time(first), without_time(second));

    // Same task ids, same views.
    for args in [
        &["show", "APM-1"][..],
        &["show", "APM-3", "--tree"],
        &["history", "APM-6"],
        &["list", "--all"],
        &["milestone", "list", "-p", "APM", "--all"],
        &["attr", "list", "-p", "APM"],
        &["status", "list", "-p", "APM"],
        &["tags"],
    ] {
        assert_eq!(src.json(args), dst.json(args), "{args:?}");
    }
    // Numbering continues after the gap.
    assert_eq!(dst.json(&["add", "new", "-p", "APM"])["id"], "APM-7");
}

#[test]
fn export_one_project_keeps_only_its_links() {
    let env = Env::new();
    rich(&env);
    let v = export(&env, &["-p", "ops"]);
    assert_eq!(v["projects"].as_array().unwrap().len(), 1);
    assert_eq!(v["projects"][0]["key"], "OPS");
    assert_eq!(v["links"], serde_json::json!([]));
    let v = export(&env, &["-p", "APM"]);
    let links = v["links"].as_array().unwrap();
    assert_eq!(links.len(), 3);
    assert_eq!(
        links[0],
        serde_json::json!({"from": "APM-1", "kind": "blocks", "to": "APM-2"})
    );
    assert!(
        env.json_err(&["export", "-p", "NOPE"])
            .contains("no such project: NOPE")
    );
}

#[test]
fn export_to_file() {
    let env = Env::new();
    rich(&env);
    let path = env.dir.path().join("out.json");
    let path_s = path.to_str().unwrap();
    let res = env.json(&["export", "-p", "APM", "-o", path_s]);
    assert_eq!(res["projects"], serde_json::json!(["APM"]));
    assert_eq!(res["tasks"], 4);
    let file: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(
        without_time(file),
        without_time(export(&env, &["-p", "APM"]))
    );
    let out = env.apm().args(["export", "-o", path_s]).assert().success();
    let line = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    assert!(
        line.starts_with("exported APM, OPS (6 tasks) to "),
        "{line}"
    );
}

#[test]
fn import_under_a_new_key() {
    let src = Env::new();
    rich(&src);
    let out = src.apm().args(["export", "-p", "APM"]).assert().success();
    let text = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    let file = write(&src, "apm.json", &text);

    // Into the same database, next to the original.
    let res = src.json(&["import", &file, "--key", "copy"]);
    assert_eq!(res["projects"][0]["key"], "COPY");
    assert_eq!(res["projects"][0]["source_key"], "APM");
    let task = src.json(&["show", "COPY-3"]);
    assert_eq!(task["parent"], "COPY-2");
    assert_eq!(task["milestone"], "v0");
    let links = src.json(&["show", "COPY-1"])["links"].clone();
    assert_eq!(links["blocks"][0]["id"], "COPY-2");
    assert_eq!(links["duplicated_by"][0]["id"], "COPY-3");
    // The original is untouched.
    assert_eq!(src.json(&["show", "APM-3"])["parent"], "APM-2");

    let out = src
        .apm()
        .args(["import", &file, "--key", "COPY2"])
        .assert()
        .success();
    let line = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    assert_eq!(line, "imported COPY2 from APM (4 tasks)\n");

    let all = src.apm().args(["export"]).assert().success();
    let file = write(
        &src,
        "all.json",
        &String::from_utf8_lossy(&all.get_output().stdout),
    );
    let e = src.json_err(&["import", &file, "--key", "X"]);
    assert!(e.contains("needs a file with one project"), "{e}");
    let e = src.json_err(&["import", &file, "--key", "1X"]);
    assert!(e.contains("needs a file with one project"), "{e}");
}

#[test]
fn import_refuses_an_existing_key_and_writes_nothing() {
    let src = Env::new();
    rich(&src);
    let out = src.apm().arg("export").assert().success();
    let file = write(
        &src,
        "all.json",
        &String::from_utf8_lossy(&out.get_output().stdout),
    );

    let dst = Env::new();
    run_all(&dst, &[&["project", "add", "OPS", "existing"]]);
    let e = dst.json_err(&["import", &file]);
    assert_eq!(e, "project already exists: OPS");
    // APM came first in the file, and is not there either.
    assert_eq!(project_keys(&dst), ["OPS"]);
    assert!(
        dst.json_err(&["import", &file, "--dry-run"])
            .contains("already exists")
    );
}

#[test]
fn import_claims_an_alias() {
    let src = Env::new();
    run_all(
        &src,
        &[&["project", "add", "APM", "a"], &["add", "t", "-p", "APM"]],
    );
    let out = src.apm().arg("export").assert().success();
    let file = write(
        &src,
        "a.json",
        &String::from_utf8_lossy(&out.get_output().stdout),
    );

    let dst = Env::new();
    run_all(
        &dst,
        &[
            &["project", "add", "APM", "old"],
            &["add", "old task", "-p", "APM"],
            &["project", "edit", "APM", "--key", "OLD"],
        ],
    );
    let res = dst.json(&["import", &file]);
    assert_eq!(res["projects"][0]["claimed_alias_from"], "OLD");
    assert_eq!(dst.json(&["show", "APM-1"])["title"], "t");
    assert_eq!(dst.json(&["show", "OLD-1"])["title"], "old task");
}

#[test]
fn dry_run_writes_nothing() {
    let src = Env::new();
    rich(&src);
    let out = src.apm().arg("export").assert().success();
    let file = write(
        &src,
        "all.json",
        &String::from_utf8_lossy(&out.get_output().stdout),
    );

    let dst = Env::new();
    let res = dst.json(&["import", &file, "--dry-run"]);
    assert_eq!(res["dry_run"], true);
    assert_eq!(res["projects"][1]["key"], "OPS");
    assert!(project_keys(&dst).is_empty());
    let out = dst
        .apm()
        .args(["import", &file, "--dry-run"])
        .assert()
        .success();
    assert_eq!(
        String::from_utf8_lossy(&out.get_output().stdout),
        "would import APM (4 tasks)\nwould import OPS (2 tasks)\n"
    );
}

#[test]
fn import_reads_stdin() {
    let src = Env::new();
    run_all(
        &src,
        &[&["project", "add", "APM", "a"], &["add", "t", "-p", "APM"]],
    );
    let out = src.apm().arg("export").assert().success();
    let text = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    let dst = Env::new();
    let res = dst.json_stdin(&["import", "-"], &text);
    assert_eq!(res["projects"][0]["tasks"], 1);
    assert_eq!(dst.json(&["show", "APM-1"])["title"], "t");
}

/// Breaks one thing in a valid export.
type Breaker = dyn Fn(&mut Value);

/// A small valid export, as JSON, for the error tests to break.
fn small(env: &Env) -> Value {
    run_all(
        env,
        &[
            &["project", "add", "APM", "a"],
            &["milestone", "add", "v1", "-p", "APM"],
            &[
                "attr",
                "add",
                "size",
                "-p",
                "APM",
                "--type",
                "select",
                "--options",
                "s,m",
            ],
            &["add", "one", "-p", "APM", "--set", "size=s"],
            &["add", "two", "--parent", "APM-1", "-m", "v1"],
            &["link", "APM-1", "blocks", "APM-2"],
        ],
    );
    export(env, &[])
}

#[test]
fn import_rejects_broken_files() {
    let env = Env::new();
    let good = small(&env);
    let dst = Env::new();
    let err_for = |text: &str| dst.json_err(&["import", &write(&dst, "bad.json", text)]);
    let err_with = |f: &dyn Fn(&mut Value)| {
        let mut v = good.clone();
        f(&mut v);
        err_for(&v.to_string())
    };

    let text = good.to_string();
    assert!(err_for(&text[..text.len() / 2]).starts_with("invalid export file: "));
    assert!(err_for("").contains("import file is empty"));
    assert!(err_for("[1, 2]").contains("not an apm export"));
    let cases: Vec<(&Breaker, &str)> = vec![
        (
            &|v| v["apm_export"] = 2.into(),
            "unsupported export version 2",
        ),
        (
            &|v| v["projects"][0]["tasks"][0]["status"] = "nope".into(),
            "APM-1: no such status: nope",
        ),
        (
            &|v| v["projects"][0]["tasks"][1]["milestone"] = "v9".into(),
            "APM-2: no such milestone: v9",
        ),
        (
            &|v| v["projects"][0]["tasks"][1]["parent"] = "APM-9".into(),
            "no such parent in the file: APM-9",
        ),
        (
            &|v| v["projects"][0]["tasks"][0]["parent"] = "APM-2".into(),
            "parent cycle",
        ),
        (
            &|v| v["projects"][0]["tasks"][0]["values"]["size"] = "xl".into(),
            "invalid value for size",
        ),
        (
            &|v| v["projects"][0]["tasks"][0]["values"]["nope"] = "1".into(),
            "unknown attribute: nope",
        ),
        (
            &|v| v["projects"][0]["tasks"][0]["id"] = "OPS-1".into(),
            "not of the form APM-N",
        ),
        (
            &|v| v["projects"][0]["tasks"][0]["id"] = "APM-2".into(),
            "task appears twice: APM-2",
        ),
        (
            &|v| v["projects"][0]["next_seq"] = 2.into(),
            "APM-2 is not below next_seq 2",
        ),
        (
            &|v| v["projects"][0]["tasks"][0]["created_at"] = "yesterday".into(),
            "invalid timestamp",
        ),
        (
            &|v| v["projects"][0]["tasks"][0]["tags"] = serde_json::json!(["Bad Tag"]),
            "invalid tag",
        ),
        (
            &|v| v["projects"][0]["tasks"][0]["priority"] = "huge".into(),
            "invalid priority: huge",
        ),
        (
            &|v| v["projects"][0]["statuses"][0]["category"] = "later".into(),
            "invalid status category",
        ),
        (
            &|v| v["projects"][0]["key"] = "apm".into(),
            "must be uppercase",
        ),
        (
            &|v| v["links"][0]["to"] = "APM-7".into(),
            "no such task in the file: APM-7",
        ),
        (
            &|v| v["links"][0]["kind"] = "follows".into(),
            "invalid link kind",
        ),
        (
            &|v| v["projects"][0]["surprise"] = 1.into(),
            "unknown field `surprise`",
        ),
        (
            &|v| {
                v["projects"][0].as_object_mut().unwrap().remove("tasks");
            },
            "missing field `tasks`",
        ),
    ];
    for (f, want) in cases {
        let e = err_with(f);
        assert!(e.contains(want), "want `{want}`, got `{e}`");
    }
    assert!(project_keys(&dst).is_empty());
    // The good file still imports.
    let res = dst.json(&["import", &write(&dst, "good.json", &good.to_string())]);
    assert_eq!(res["projects"][0]["tasks"], 2);
}

#[test]
fn import_normalizes_hand_written_values() {
    let env = Env::new();
    let mut v = small(&env);
    v["projects"][0]["attributes"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({
            "name": "n", "type": "number", "options": [], "created_at": "2026-01-01T00:00:00Z"
        }));
    v["projects"][0]["tasks"][0]["values"]["n"] = "2.0".into();
    let dst = Env::new();
    dst.json(&["import", &write(&dst, "v.json", &v.to_string())]);
    assert_eq!(dst.json(&["show", "APM-1"])["attributes"]["n"], 2);
}

#[test]
fn backup_copies_the_database() {
    let env = Env::new();
    rich(&env);
    let res = env.json(&["backup"]);
    let path = res["path"].as_str().unwrap().to_string();
    let name = std::path::Path::new(&path)
        .file_name()
        .unwrap()
        .to_str()
        .unwrap();
    assert!(name.starts_with("apm-") && name.ends_with("Z.db"), "{name}");
    assert_eq!(
        std::path::Path::new(&path).parent().unwrap(),
        env.dir.path().join("backups")
    );
    let copy = |args: &[&str]| -> Value {
        let out = env
            .apm()
            .args(["--db", &path, "--json"])
            .args(args)
            .assert()
            .success();
        serde_json::from_slice(&out.get_output().stdout).unwrap()
    };
    assert_eq!(
        copy(&["db"])["schema_version"],
        env.json(&["db"])["schema_version"]
    );
    assert_eq!(copy(&["list", "--all"]), env.json(&["list", "--all"]));

    let target = env.dir.path().join("explicit.db");
    let target = target.to_str().unwrap();
    let res = env.json(&["backup", target]);
    assert_eq!(res["path"], target);
    assert_eq!(res["pruned"], serde_json::json!([]));
    assert_eq!(
        env.json_err(&["backup", target]),
        format!("file exists: {target}")
    );
}

#[test]
fn backup_keep_prunes_old_backups_only() {
    let env = Env::new();
    run_all(&env, &[&["project", "add", "APM", "a"]]);
    let dir = env.dir.path().join("backups");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("apm-0000-old.db"), "x").unwrap();
    std::fs::write(dir.join("mine.db"), "x").unwrap();
    let first = env.json(&["backup"])["path"].as_str().unwrap().to_string();
    let second = env.json(&["backup"])["path"].as_str().unwrap().to_string();
    let res = env.json(&["backup", "--keep", "2"]);
    let pruned: Vec<&str> = res["pruned"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p.as_str().unwrap())
        .collect();
    assert_eq!(pruned.len(), 2, "{pruned:?}");
    assert!(pruned[0].ends_with("apm-0000-old.db"));
    assert_eq!(pruned[1], first);
    let mut left: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    left.sort();
    assert_eq!(left.len(), 3);
    assert!(left.contains(&"mine.db".to_string()));
    assert!(second.ends_with(&left[0]));

    // --keep is for the default dir; 0 would delete the new backup.
    env.apm()
        .args(["backup", "x.db", "--keep", "1"])
        .assert()
        .code(2);
    env.apm().args(["backup", "--keep", "0"]).assert().code(2);
}
