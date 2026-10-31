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
    assert_eq!(first["apm_export"], 2);
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
fn export_to_file_leaves_no_partial_file() {
    let env = Env::new();
    run_all(&env, &[&["project", "add", "APM", "a"]]);
    // A directory where the file should go: the rename fails, the temp file is removed.
    let target = env.dir.path().join("out.json");
    std::fs::create_dir(&target).unwrap();
    let e = env.json_err(&["export", "-o", target.to_str().unwrap()]);
    assert!(e.starts_with("cannot write "), "{e}");
    let names: Vec<String> = std::fs::read_dir(env.dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    assert!(!names.iter().any(|n| n.contains(".tmp-")), "{names:?}");
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
    assert!(
        line.starts_with(
            "imported COPY2 (from APM): 4 tasks, 6 statuses, 2 milestones, 6 attributes, \
             3 comments, "
        ),
        "{line}"
    );
    assert!(
        line.ends_with(" events, 0 pages (0 versions)\nimported 3 links, 0 page links\n"),
        "{line}"
    );

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

    let exported = export(&src, &[]);
    let events: usize = exported["projects"][0]["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["events"].as_array().unwrap().len())
        .sum();

    let dst = Env::new();
    let res = dst.json(&["import", &file, "--dry-run"]);
    assert_eq!(res["dry_run"], true);
    assert_eq!(res["links"], 5);
    let apm = &res["projects"][0];
    assert_eq!(apm["key"], "APM");
    for (field, n) in [
        ("tasks", 4),
        ("statuses", 6),
        ("milestones", 2),
        ("attributes", 6),
        ("comments", 3),
        ("events", events),
    ] {
        assert_eq!(apm[field], n, "{field}");
    }
    assert_eq!(res["projects"][1]["key"], "OPS");
    assert!(project_keys(&dst).is_empty());
    let out = dst
        .apm()
        .args(["import", &file, "--dry-run"])
        .assert()
        .success();
    let text = String::from_utf8_lossy(&out.get_output().stdout).to_string();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 3, "{text}");
    assert_eq!(
        lines[0],
        format!(
            "would import APM: 4 tasks, 6 statuses, 2 milestones, 6 attributes, 3 comments, \
             {events} events, 0 pages (0 versions)"
        )
    );
    assert!(lines[1].starts_with("would import OPS: 2 tasks, 5 statuses, 0 milestones, "));
    assert_eq!(lines[2], "would import 5 links, 0 page links");
    assert!(project_keys(&dst).is_empty());
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
            &|v| v["apm_export"] = 3.into(),
            "unsupported export version 3; this apm reads 1 to 2",
        ),
        (
            &|v| v["projects"][0]["tasks"][0]["status"] = "nope".into(),
            "APM-1: no such status: nope",
        ),
        (
            &|v| v["projects"][0]["tasks"][0]["events"][0]["kind"] = "bogus".into(),
            "APM-1: unknown event kind: bogus",
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
            "unknown variant `huge`",
        ),
        (
            &|v| v["projects"][0]["statuses"][0]["category"] = "later".into(),
            "unknown variant `later`",
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
            "unknown variant `follows`",
        ),
        (
            &|v| v["projects"][0]["milestones"][0]["status"] = "paused".into(),
            "unknown variant `paused`",
        ),
        (
            &|v| {
                v["projects"][0]["tasks"] = serde_json::json!([]);
                v["links"] = serde_json::json!([]);
                v["projects"][0]["next_seq"] = 0.into();
            },
            "project APM: invalid next_seq 0",
        ),
        (
            &|v| {
                let l = v["links"][0].clone();
                v["links"].as_array_mut().unwrap().push(l);
            },
            "link APM-1 blocks APM-2: appears twice",
        ),
        (
            &|v| {
                let links = v["links"].as_array_mut().unwrap();
                links.push(serde_json::json!({"from": "APM-1", "kind": "relates", "to": "APM-2"}));
                links.push(serde_json::json!({"from": "APM-2", "kind": "relates", "to": "APM-1"}));
            },
            "link APM-2 relates APM-1: appears twice",
        ),
        (
            &|v| {
                let links = v["links"].as_array_mut().unwrap();
                links.push(serde_json::json!({"from": "APM-2", "kind": "blocks", "to": "APM-1"}));
            },
            "blocks cycle: APM-2 blocks APM-1 blocks APM-2",
        ),
        (
            &|v| v["projects"][0]["attributes"][0]["options"] = serde_json::json!([" s", "s", "m"]),
            "duplicate option: s",
        ),
        (
            &|v| v["projects"][0]["attributes"][0]["options"] = serde_json::json!(["s", " ", "m"]),
            "empty option",
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
    v["projects"][0]["attributes"][0]["options"] = serde_json::json!([" s ", "m"]);
    v["projects"][0]["tasks"][0]["values"]["size"] = " s".into();
    v["projects"][0]["tasks"][0]["tags"] = serde_json::json!(["Perf", "perf ", "a"]);
    let dst = Env::new();
    dst.json(&["import", &write(&dst, "v.json", &v.to_string())]);
    let task = dst.json(&["show", "APM-1"]);
    assert_eq!(task["attributes"]["n"], 2);
    assert_eq!(task["attributes"]["size"], "s");
    assert_eq!(task["tags"], serde_json::json!(["a", "perf"]));
    let attrs = dst.json(&["attr", "list", "-p", "APM"]);
    assert_eq!(attrs[0]["options"], serde_json::json!(["s", "m"]));
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
    // Hand-named files are never pruned, even when they look like backups.
    std::fs::write(dir.join("apm-0000-old.db"), "x").unwrap();
    std::fs::write(dir.join("apm-2000-01-01T00-00-00.000Z.db"), "x").unwrap();
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
    assert!(pruned[0].ends_with("apm-2000-01-01T00-00-00.000Z.db"));
    assert_eq!(pruned[1], first);
    let mut left: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    left.sort();
    assert_eq!(left.len(), 4, "{left:?}");
    assert_eq!(left[0], "apm-0000-old.db");
    assert!(second.ends_with(&left[1]));
    assert_eq!(left[3], "mine.db");

    // Text: the backup path first, then what was pruned.
    let out = env.apm().args(["backup", "--keep", "1"]).assert().success();
    let text = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 3, "{text}");
    assert!(lines[0].contains("backups") && lines[0].ends_with("Z.db"));
    assert!(lines[1].starts_with("pruned ") && lines[1].ends_with(&left[1]));

    // --keep is for the default dir; 0 would delete the new backup.
    env.apm()
        .args(["backup", "x.db", "--keep", "1"])
        .assert()
        .code(2);
    env.apm().args(["backup", "--keep", "0"]).assert().code(2);
}

// ---- apm_export 2: log kinds, pages, page links ----

/// Pages with versions, inserted by SQL (as `apm page` stores them): `KEY:slug` ->
/// (pinned, verified, versions as (title, summary, body, author, at)); the last version is
/// the page's content.
type PageRow<'a> = (&'a str, bool, bool, &'a [Version<'a>]);
type Version<'a> = (&'a str, &'a str, &'a str, &'a str, &'a str);

fn add_pages(env: &Env, pages: &[PageRow], links: &[(&str, &str)]) {
    let conn = env.conn();
    for (id, pinned, verified, versions) in pages {
        let (key, slug) = id.split_once(':').unwrap();
        let first = versions[0];
        let last = versions[versions.len() - 1];
        conn.execute(
            "INSERT INTO pages (project_id, slug, title, summary, body, pinned, version,
                                created_by, created_at, updated_by, updated_at, verified_by,
                                verified_at)
             SELECT id, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13
             FROM projects WHERE key = ?1",
            rusqlite::params![
                key,
                slug,
                last.0,
                last.1,
                last.2,
                pinned,
                versions.len() as i64,
                first.3,
                first.4,
                last.3,
                last.4,
                verified.then_some("carol"),
                verified.then_some("2026-10-30T09:00:00Z"),
            ],
        )
        .unwrap();
        let page_id = conn.last_insert_rowid();
        for (n, (title, summary, body, author, at)) in versions.iter().enumerate() {
            conn.execute(
                "INSERT INTO page_versions (page_id, version, title, summary, body, author, at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                rusqlite::params![page_id, n as i64 + 1, title, summary, body, author, at],
            )
            .unwrap();
        }
    }
    for (page, task) in links {
        let (key, slug) = page.split_once(':').unwrap();
        let (tkey, seq) = task.split_once('-').unwrap();
        conn.execute(
            "INSERT INTO page_links (page_id, item_id)
             SELECT pg.id, i.id FROM pages pg, items i
             JOIN projects p ON p.id = pg.project_id
             JOIN projects q ON q.id = i.project_id
             WHERE p.key = ?1 AND pg.slug = ?2 AND q.key = ?3 AND i.seq = ?4",
            rusqlite::params![key, slug, tkey, seq.parse::<i64>().unwrap()],
        )
        .unwrap();
    }
}

/// On top of `rich`: every log kind on OPS-2 (the last task, so comment ids match after an
/// import) and pages in both projects, one with three versions, linked across projects.
fn knowledge(env: &Env) {
    let q =
        env.json(&["comment", "OPS-2", "--kind", "question", "which region?"])["id"].to_string();
    run_all(
        env,
        &[
            &["comment", "OPS-2", "a note in between"],
            &["comment", "OPS-2", "--answer", &q, "eu-west"],
            &[
                "comment",
                "OPS-2",
                "--kind",
                "handoff",
                "State: done\nNext: deploy",
            ],
        ],
    );
    add_pages(
        env,
        &[
            (
                "APM:storage",
                true,
                true,
                &[
                    ("Store", "", "v1 body", "alice", "2026-10-01T10:00:00Z"),
                    (
                        "Storage",
                        "SQLite",
                        "v2 body",
                        "bob",
                        "2026-10-02T10:00:00Z",
                    ),
                    (
                        "Storage",
                        "SQLite file, WAL, one writer per tx",
                        "rollback journal on network drives",
                        "bob",
                        "2026-10-03T10:00:00Z",
                    ),
                ],
            ),
            (
                "APM:conventions",
                false,
                false,
                &[("Conventions", "", "", "alice", "2026-10-01T09:00:00Z")],
            ),
            (
                "OPS:runbook",
                false,
                false,
                &[(
                    "Runbook",
                    "how to deploy",
                    "steps",
                    "dave",
                    "2026-10-05T09:00:00Z",
                )],
            ),
        ],
        &[
            ("OPS:runbook", "APM-2"),
            ("APM:storage", "OPS-3"),
            ("APM:storage", "APM-1"),
        ],
    );
}

/// `apm export` stdout as text.
fn export_text(env: &Env, args: &[&str]) -> String {
    let out = env.apm().arg("export").args(args).assert().success();
    String::from_utf8(out.get_output().stdout.clone()).unwrap()
}

/// An export's text with the `exported_at` line taken out.
fn without_time_line(text: &str) -> String {
    text.lines()
        .filter(|l| !l.trim_start().starts_with("\"exported_at\""))
        .collect::<Vec<_>>()
        .join("\n")
}

fn count(env: &Env, sql: &str) -> i64 {
    env.conn().query_row(sql, [], |r| r.get(0)).unwrap()
}

#[test]
fn round_trip_keeps_pages_and_log_kinds() {
    let src = Env::new();
    rich(&src);
    knowledge(&src);
    let first = export(&src, &[]);
    let ops2 = &first["projects"][1]["tasks"][0];
    assert_eq!(ops2["id"], "OPS-2");
    assert_eq!(
        ops2["comments"][2],
        serde_json::json!({
            "author": "tester", "kind": "answer", "answers": 1,
            "body": "eu-west", "created_at": ops2["comments"][2]["created_at"]
        })
    );
    let kinds: Vec<&str> = ops2["comments"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["question", "note", "answer", "handoff"]);
    // Pages by slug, after the attributes; versions by number; page links after links.
    let text = export_text(&src, &[]);
    let pos = |field: &str| text.find(&format!("\"{field}\": ")).unwrap();
    assert!(pos("attributes") < pos("pages") && pos("pages") < pos("tasks"));
    assert!(text.rfind("\"links\": ").unwrap() < pos("page_links"));
    assert!(text.contains(
        r#""kind": "answer",
              "answers": 1,
              "body": "eu-west","#
    ));
    let storage = &first["projects"][0]["pages"][1];
    assert_eq!(first["projects"][0]["pages"][0]["slug"], "conventions");
    assert_eq!(storage["slug"], "storage");
    assert_eq!(storage["pinned"], true);
    assert_eq!(storage["version"], 3);
    assert_eq!(storage["verified_by"], "carol");
    assert_eq!(storage["versions"].as_array().unwrap().len(), 3);
    assert_eq!(storage["versions"][0]["title"], "Store");
    assert_eq!(
        first["page_links"],
        serde_json::json!([
            {"page": "APM:storage", "task": "APM-1"},
            {"page": "APM:storage", "task": "OPS-3"},
            {"page": "OPS:runbook", "task": "APM-2"},
        ])
    );

    let file = write(&src, "export.json", &text);
    let dst = Env::new();
    let res = dst.json(&["import", &file]);
    assert_eq!(res["page_links"], 3);
    assert_eq!(res["projects"][0]["pages"], 2);
    assert_eq!(res["projects"][0]["page_versions"], 4);
    // Byte-identical, but for exported_at.
    assert_eq!(
        without_time_line(&export_text(&dst, &[])),
        without_time_line(&text)
    );
    // The answer points at the question's new id.
    assert_eq!(src.json(&["show", "OPS-2"]), dst.json(&["show", "OPS-2"]));
    let comments = dst.json(&["show", "OPS-2"])["comments"].clone();
    assert_eq!(comments[2]["answers"], comments[0]["id"]);
    // The FTS triggers indexed the imported pages and log entries.
    let hits = |sql: &str| count(&dst, sql);
    assert_eq!(
        hits("SELECT count(*) FROM pages_fts WHERE pages_fts MATCH '\"rollback\" \"journal\"'"),
        1
    );
    assert_eq!(
        hits("SELECT count(*) FROM pages_fts WHERE pages_fts MATCH 'deploy'"),
        1
    );
    assert_eq!(
        hits("SELECT count(*) FROM comments_fts WHERE comments_fts MATCH 'region'"),
        1
    );
}

/// What apm 0.3.0 exported (`apm_export` 1): comments without kinds, no pages.
const V1_FILE: &str = r#"{
  "apm_export": 1,
  "exported_at": "2026-10-31T19:47:54Z",
  "projects": [
    {
      "key": "APM", "name": "agent-pm", "description": "v1 fixture", "next_seq": 3,
      "created_at": "2026-10-31T19:47:54Z", "updated_at": "2026-10-31T19:47:54Z",
      "statuses": [
        {"name": "todo", "category": "todo"}, {"name": "doing", "category": "active"},
        {"name": "blocked", "category": "active"}, {"name": "done", "category": "done"},
        {"name": "cancelled", "category": "cancelled"}
      ],
      "milestones": [],
      "attributes": [],
      "tasks": [
        {
          "id": "APM-1", "parent": null, "title": "first task", "body": "body",
          "status": "todo", "priority": "none", "assignee": null, "due": null,
          "milestone": null, "created_by": "alice", "created_at": "2026-10-31T19:47:54Z",
          "updated_at": "2026-10-31T19:47:54Z", "values": {}, "tags": ["wip"],
          "comments": [
            {"author": "alice", "body": "a plain note", "created_at": "2026-10-31T19:47:54Z"},
            {"author": "alice", "body": "another", "created_at": "2026-10-31T19:47:54Z"}
          ],
          "events": [
            {"actor": "alice", "at": "2026-10-31T19:47:54Z", "kind": "created", "field": null,
             "old": null, "new": "first task"},
            {"actor": "alice", "at": "2026-10-31T19:47:54Z", "kind": "commented",
             "field": null, "old": null, "new": "a plain note"},
            {"actor": "alice", "at": "2026-10-31T19:47:54Z", "kind": "linked",
             "field": "blocks", "old": null, "new": "APM-2"}
          ]
        },
        {
          "id": "APM-2", "parent": "APM-1", "title": "second", "body": "", "status": "todo",
          "priority": "none", "assignee": null, "due": null, "milestone": null,
          "created_by": "alice", "created_at": "2026-10-31T19:47:54Z",
          "updated_at": "2026-10-31T19:47:54Z", "values": {}, "tags": [], "comments": [],
          "events": []
        }
      ]
    }
  ],
  "links": [{"from": "APM-1", "kind": "blocks", "to": "APM-2"}]
}"#;

#[test]
fn v1_file_still_imports() {
    let dst = Env::new();
    let res = dst.json(&["import", &write(&dst, "v1.json", V1_FILE)]);
    assert_eq!(res["projects"][0]["comments"], 2);
    assert_eq!(res["projects"][0]["pages"], 0);
    assert_eq!(res["page_links"], 0);
    let comments = dst.json(&["show", "APM-1"])["comments"].clone();
    for c in comments.as_array().unwrap() {
        assert_eq!(c["kind"], "note");
        assert_eq!(c["answers"], Value::Null);
    }
    assert_eq!(comments[0]["body"], "a plain note");
    assert_eq!(dst.json(&["show", "APM-2"])["parent"], "APM-1");
    let v = export(&dst, &[]);
    assert_eq!(v["apm_export"], 2);
    assert_eq!(v["projects"][0]["pages"], serde_json::json!([]));
    assert_eq!(v["page_links"], serde_json::json!([]));
    assert_eq!(v["links"].as_array().unwrap().len(), 1);

    // A version 1 file has none of the version 2 fields.
    let v1: Value = serde_json::from_str(V1_FILE).unwrap();
    let other = Env::new();
    let err_with = |f: &dyn Fn(&mut Value)| {
        let mut v = v1.clone();
        f(&mut v);
        other.json_err(&["import", &write(&other, "bad.json", &v.to_string())])
    };
    let cases: Vec<(&Breaker, &str)> = vec![
        (
            &|v| v["projects"][0]["tasks"][0]["comments"][0]["kind"] = "handoff".into(),
            "unknown field `kind` in apm_export 1",
        ),
        (
            &|v| v["projects"][0]["tasks"][0]["comments"][0]["answers"] = Value::Null,
            "unknown field `answers` in apm_export 1",
        ),
        (
            &|v| v["projects"][0]["pages"] = serde_json::json!([]),
            "unknown field `pages` in apm_export 1",
        ),
        (
            &|v| v["page_links"] = serde_json::json!([]),
            "unknown field `page_links` in apm_export 1",
        ),
        (
            &|v| v["projects"][0]["tasks"][0]["comments"][0]["mood"] = "ok".into(),
            "unknown field `mood`",
        ),
    ];
    for (f, want) in cases {
        let e = err_with(f);
        assert!(e.contains(want), "want `{want}`, got `{e}`");
    }
    assert!(project_keys(&other).is_empty());
}

/// `small` plus a question, its answer and a handoff on APM-1, and a page with two
/// versions linked to APM-2.
fn small_knowledge(env: &Env) -> Value {
    small(env);
    let q = env.json(&["comment", "APM-1", "--kind", "question", "why?"])["id"].to_string();
    run_all(
        env,
        &[
            &["comment", "APM-1", "--answer", &q, "because"],
            &["comment", "APM-1", "--kind", "handoff", "next: ship"],
        ],
    );
    add_pages(
        env,
        &[(
            "APM:notes",
            false,
            false,
            &[
                ("Notes", "", "one", "alice", "2026-10-01T10:00:00Z"),
                ("Notes", "short", "two", "bob", "2026-10-02T10:00:00Z"),
            ],
        )],
        &[("APM:notes", "APM-2")],
    );
    export(env, &[])
}

fn page(v: &mut Value) -> &mut Value {
    &mut v["projects"][0]["pages"][0]
}

fn comments(v: &mut Value) -> &mut Value {
    &mut v["projects"][0]["tasks"][0]["comments"]
}

#[test]
fn import_rejects_broken_pages_and_answers() {
    let env = Env::new();
    let good = small_knowledge(&env);
    let dst = Env::new();
    let err_with = |f: &dyn Fn(&mut Value)| {
        let mut v = good.clone();
        f(&mut v);
        dst.json_err(&["import", &write(&dst, "bad.json", &v.to_string())])
    };
    let cases: Vec<(&Breaker, &str)> = vec![
        (
            &|v| page(v)["slug"] = "my notes".into(),
            "page APM:my notes: invalid page slug `my notes`",
        ),
        (
            &|v| page(v)["slug"] = "Notes".into(),
            "page APM:Notes: page slug must be lowercase: Notes",
        ),
        (
            &|v| {
                let s = "x".repeat(201);
                page(v)["summary"] = s.clone().into();
                page(v)["versions"][1]["summary"] = s.into();
            },
            "page APM:notes: summary is 201 characters, at most 200: put detail in the body",
        ),
        (
            &|v| page(v)["versions"][0]["summary"] = "y".repeat(201).into(),
            "page APM:notes: version 1: summary is 201 characters",
        ),
        (
            &|v| page(v)["title"] = " ".into(),
            "page APM:notes: title cannot be empty",
        ),
        // The same content rules as `apm page`: one-line title and summary.
        (
            &|v| {
                page(v)["title"] = "Notes\nmore".into();
                page(v)["versions"][1]["title"] = "Notes\nmore".into();
            },
            "page APM:notes: title must be one line",
        ),
        (
            &|v| {
                page(v)["summary"] = "short\nmore".into();
                page(v)["versions"][1]["summary"] = "short\nmore".into();
            },
            "page APM:notes: summary must be one line: put detail in the body",
        ),
        (
            &|v| page(v)["versions"][0]["title"] = "Notes\r\nold".into(),
            "page APM:notes: version 1: title must be one line",
        ),
        (
            &|v| page(v)["versions"][0]["summary"] = "old\nsummary".into(),
            "page APM:notes: version 1: summary must be one line",
        ),
        (
            &|v| {
                let p = page(v)["versions"].as_array_mut().unwrap();
                p.remove(0);
            },
            "page APM:notes: versions must run 1 to 2 in order: expected version 1, found 2",
        ),
        (
            &|v| page(v)["versions"][1]["version"] = 3.into(),
            "page APM:notes: versions must run 1 to 2 in order: expected version 2, found 3",
        ),
        (
            &|v| page(v)["version"] = 3.into(),
            "page APM:notes: versions must run 1 to 3: the file has 2",
        ),
        (
            &|v| page(v)["versions"] = serde_json::json!([]),
            "page APM:notes: versions must run 1 to 2: the file has 0",
        ),
        (
            &|v| page(v)["body"] = "edited without a version".into(),
            "page APM:notes: version 2 differs from the page's title, summary or body",
        ),
        (
            &|v| page(v)["verified_at"] = "2026-10-03T10:00:00Z".into(),
            "page APM:notes: verified_by and verified_at go together",
        ),
        (
            &|v| page(v)["verified_by"] = "carol".into(),
            "page APM:notes: verified_by and verified_at go together",
        ),
        (
            &|v| page(v)["versions"][0]["title"] = "  ".into(),
            "page APM:notes: version 1: title cannot be empty",
        ),
        (
            &|v| page(v)["versions"][0]["at"] = "monday".into(),
            "page APM:notes: version 1: invalid timestamp",
        ),
        (
            &|v| page(v)["pinned"] = 1.into(),
            "invalid type: integer `1`, expected a boolean",
        ),
        (&|v| page(v)["owner"] = "x".into(), "unknown field `owner`"),
        (
            &|v| {
                let p = page(v).clone();
                v["projects"][0]["pages"].as_array_mut().unwrap().push(p);
            },
            "project APM: page appears twice: APM:notes",
        ),
        (
            &|v| v["page_links"][0]["page"] = "APM:nope".into(),
            "page link APM:nope APM-2: no such page in the file: APM:nope",
        ),
        (
            &|v| v["page_links"][0]["task"] = "APM-9".into(),
            "page link APM:notes APM-9: no such task in the file: APM-9",
        ),
        (
            &|v| {
                let l = v["page_links"][0].clone();
                v["page_links"].as_array_mut().unwrap().push(l);
            },
            "page link APM:notes APM-2: appears twice",
        ),
        // Comments of APM-1: 1 question, 2 answer (answers 1), 3 handoff.
        (
            &|v| comments(v)[1]["answers"] = 2.into(),
            "APM-1: comment 2 answers comment 2, which is not an earlier comment of this task",
        ),
        (
            &|v| comments(v)[1]["answers"] = 3.into(),
            "APM-1: comment 2 answers comment 3, which is not an earlier comment",
        ),
        (
            &|v| comments(v)[1]["answers"] = 0.into(),
            "APM-1: comment 2 answers comment 0",
        ),
        (
            &|v| {
                comments(v)[0]["kind"] = "note".into();
            },
            "APM-1: comment 2 answers comment 1, which is a note, not a question",
        ),
        (
            &|v| comments(v)[2]["answers"] = 1.into(),
            "APM-1: comment 3 is a handoff, only an answer has `answers`",
        ),
        (
            &|v| comments(v)[1]["answers"] = Value::Null,
            "APM-1: comment 2 is an answer but does not say which question it answers",
        ),
        (
            &|v| comments(v)[0]["kind"] = "rant".into(),
            "unknown variant `rant`",
        ),
        (
            &|v| {
                comments(v)[0].as_object_mut().unwrap().remove("kind");
            },
            "missing field `kind`",
        ),
        (
            &|v| {
                v.as_object_mut().unwrap().remove("page_links");
            },
            "missing field `page_links`",
        ),
    ];
    for (f, want) in cases {
        let e = err_with(f);
        assert!(e.contains(want), "want `{want}`, got `{e}`");
    }
    // Nothing was written.
    assert!(project_keys(&dst).is_empty());
    for table in ["pages", "page_versions", "page_links", "comments"] {
        assert_eq!(
            count(&dst, &format!("SELECT count(*) FROM {table}")),
            0,
            "{table}"
        );
    }
    // The good file still imports.
    let res = dst.json(&["import", &write(&dst, "good.json", &good.to_string())]);
    assert_eq!(res["projects"][0]["pages"], 1);
}

#[test]
fn import_under_a_new_key_rewrites_page_ids() {
    let src = Env::new();
    small_knowledge(&src);
    let file = write(&src, "apm.json", &export_text(&src, &["-p", "APM"]));

    // --dry-run counts pages and page links, and writes none.
    let res = src.json(&["import", &file, "--key", "COPY", "--dry-run"]);
    assert_eq!(res["projects"][0]["pages"], 1);
    assert_eq!(res["projects"][0]["page_versions"], 2);
    assert_eq!(res["page_links"], 1);
    assert_eq!(count(&src, "SELECT count(*) FROM pages"), 1);
    assert_eq!(count(&src, "SELECT count(*) FROM page_links"), 1);

    let out = src
        .apm()
        .args(["import", &file, "--key", "COPY"])
        .assert()
        .success();
    let text = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    assert!(
        text.ends_with(", 1 pages (2 versions)\nimported 1 links, 1 page links\n"),
        "{text}"
    );
    let copy = export(&src, &["-p", "COPY"]);
    assert_eq!(copy["projects"][0]["pages"][0]["slug"], "notes");
    assert_eq!(
        copy["page_links"],
        serde_json::json!([{"page": "COPY:notes", "task": "COPY-2"}])
    );
    let comments = src.json(&["show", "COPY-1"])["comments"].clone();
    assert_eq!(comments[1]["kind"], "answer");
    assert_eq!(comments[1]["answers"], comments[0]["id"]);
    // The original keeps its page and link.
    assert_eq!(
        export(&src, &["-p", "APM"])["page_links"],
        serde_json::json!([{"page": "APM:notes", "task": "APM-2"}])
    );
}

#[test]
fn export_one_project_keeps_only_its_page_links() {
    let env = Env::new();
    rich(&env);
    knowledge(&env);
    // OPS:runbook -> APM-2 and APM:storage -> OPS-3 cross projects: exported only with both.
    assert_eq!(
        export(&env, &["-p", "APM"])["page_links"],
        serde_json::json!([{"page": "APM:storage", "task": "APM-1"}])
    );
    let ops = export(&env, &["-p", "OPS"]);
    assert_eq!(ops["page_links"], serde_json::json!([]));
    assert_eq!(ops["projects"][0]["pages"][0]["slug"], "runbook");
    let path = env.dir.path().join("out.json");
    let res = env.json(&["export", "-o", path.to_str().unwrap()]);
    assert_eq!(res["pages"], 3);
    assert_eq!(res["page_links"], 3);
}
