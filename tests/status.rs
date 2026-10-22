mod common;

use common::Env;
use serde_json::{Value, json};

/// Text-mode stdout of a successful run.
fn text(env: &Env, args: &[&str]) -> String {
    let out = env.apm().args(args).assert().success();
    String::from_utf8(out.get_output().stdout.clone()).unwrap()
}

/// The `id`s of a JSON array of items.
fn ids(v: &Value) -> Vec<String> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].as_str().unwrap().to_string())
        .collect()
}

/// The `name`s of `status list -p <project>`, in order.
fn names(env: &Env, project: &str) -> Vec<String> {
    env.json(&["status", "list", "-p", project])
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["name"].as_str().unwrap().to_string())
        .collect()
}

/// An env with project APM.
fn env_apm() -> Env {
    let env = Env::new();
    env.json(&["project", "add", "APM", "Agent PM"]);
    env
}

// ---- status commands

#[test]
fn new_projects_get_the_default_statuses() {
    let env = env_apm();
    let v = env.json(&["status", "list", "-p", "apm"]);
    assert_eq!(
        v,
        json!([
            {"name": "todo", "project": "APM", "category": "todo", "position": 1, "tasks": 0},
            {"name": "doing", "project": "APM", "category": "active", "position": 2, "tasks": 0},
            {"name": "blocked", "project": "APM", "category": "active", "position": 3, "tasks": 0},
            {"name": "done", "project": "APM", "category": "done", "position": 4, "tasks": 0},
            {"name": "cancelled", "project": "APM", "category": "cancelled", "position": 5, "tasks": 0},
        ])
    );
    assert_eq!(
        text(&env, &["status", "list", "-p", "APM"]).lines().next(),
        Some("NAME       CATEGORY   TASKS")
    );
}

#[test]
fn add_appends_or_inserts_at_a_position() {
    let env = env_apm();
    let s = env.json(&[
        "status",
        "add",
        "review",
        "-p",
        "APM",
        "--category",
        "active",
    ]);
    assert_eq!(
        s,
        json!({"name": "review", "project": "APM", "category": "active", "position": 6, "tasks": 0})
    );
    let s = env.json(&[
        "status",
        "add",
        "brainstorming",
        "-p",
        "APM",
        "--category",
        "todo",
        "--position",
        "1",
    ]);
    assert_eq!(s["position"], 1);
    // Past the end means last.
    env.json(&[
        "status",
        "add",
        "archived",
        "-p",
        "APM",
        "--category",
        "done",
        "--position",
        "99",
    ]);
    assert_eq!(
        names(&env, "APM"),
        [
            "brainstorming",
            "todo",
            "doing",
            "blocked",
            "done",
            "cancelled",
            "review",
            "archived"
        ]
    );
    let positions: Vec<i64> = env
        .json(&["status", "list", "-p", "APM"])
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["position"].as_i64().unwrap())
        .collect();
    assert_eq!(positions, [1, 2, 3, 4, 5, 6, 7, 8]);
    assert_eq!(
        text(
            &env,
            &["status", "add", "qa", "-p", "APM", "--category", "active"]
        ),
        "added status qa\n"
    );
}

#[test]
fn add_errors() {
    let env = env_apm();
    let add = |name: &str, extra: &[&str]| {
        let mut args = vec!["status", "add", name, "-p", "APM", "--category", "todo"];
        args.extend_from_slice(extra);
        env.json_err(&args)
    };
    assert_eq!(add("Doing", &[]), "status already exists: Doing");
    assert_eq!(add(" ", &[]), "status name cannot be empty");
    assert_eq!(add("a,b", &[]), "status name may not contain `,`: a,b");
    assert_eq!(
        add("x", &["--position", "0"]),
        "invalid position 0: must be 1 or more"
    );
    assert_eq!(
        env.json_err(&["status", "add", "x", "-p", "NOPE", "--category", "todo"]),
        "no such project: NOPE"
    );
    env.apm()
        .args([
            "--json",
            "status",
            "add",
            "x",
            "-p",
            "APM",
            "--category",
            "later",
        ])
        .assert()
        .code(2);
    assert_eq!(names(&env, "APM").len(), 5);
}

#[test]
fn edit_renames_recategorizes_and_moves() {
    let env = env_apm();
    env.json(&["add", "a", "-p", "APM", "--status", "doing"]);
    let s = env.json(&[
        "status",
        "edit",
        "DOING",
        "-p",
        "APM",
        "--rename",
        "in progress",
        "--position",
        "1",
    ]);
    assert_eq!(
        s,
        json!({"name": "in progress", "project": "APM", "category": "active", "position": 1, "tasks": 1})
    );
    // Tasks follow the rename.
    assert_eq!(env.json(&["show", "APM-1"])["status"], "in progress");
    assert_eq!(names(&env, "APM")[..2], ["in progress", "todo"]);
    // A case-only rename of the same status is allowed.
    env.json(&[
        "status",
        "edit",
        "in progress",
        "-p",
        "APM",
        "--rename",
        "In Progress",
    ]);
    // Recategorizing to done hides its tasks from list.
    env.json(&[
        "status",
        "edit",
        "in progress",
        "-p",
        "APM",
        "--category",
        "done",
    ]);
    assert_eq!(env.json(&["list"]), json!([]));
    assert_eq!(env.json(&["show", "APM-1"])["status_category"], "done");
    assert_eq!(
        text(
            &env,
            &["status", "edit", "todo", "-p", "APM", "--position", "9"]
        ),
        "updated status todo\n"
    );
    assert_eq!(names(&env, "APM").last().unwrap(), "todo");
}

#[test]
fn edit_errors() {
    let env = env_apm();
    assert_eq!(
        env.json_err(&["status", "edit", "todo", "-p", "APM"]),
        "nothing to change: pass --rename, --category or --position"
    );
    assert_eq!(
        env.json_err(&["status", "edit", "nope", "-p", "APM", "--rename", "x"]),
        "no such status: nope"
    );
    assert_eq!(
        env.json_err(&["status", "edit", "todo", "-p", "APM", "--rename", "Done"]),
        "status already exists: Done"
    );
    assert_eq!(
        env.json_err(&["status", "edit", "todo", "-p", "APM", "--position=-1"]),
        "invalid position -1: must be 1 or more"
    );
    assert_eq!(names(&env, "APM")[0], "todo");
}

#[test]
fn rm_unused_status_closes_the_gap() {
    let env = env_apm();
    let s = env.json(&["status", "rm", "blocked", "-p", "APM"]);
    assert_eq!(s["name"], "blocked");
    assert_eq!(names(&env, "APM"), ["todo", "doing", "done", "cancelled"]);
    assert_eq!(env.json(&["status", "list", "-p", "APM"])[3]["position"], 4);
    assert_eq!(
        text(&env, &["status", "rm", "doing", "-p", "APM"]),
        "removed status doing\n"
    );
}

#[test]
fn rm_used_status_needs_to() {
    let env = env_apm();
    env.json(&["add", "a", "-p", "APM", "--status", "doing"]);
    env.json(&["add", "b", "-p", "APM", "--status", "doing"]);
    assert_eq!(
        env.json_err(&["status", "rm", "doing", "-p", "APM"]),
        "status doing is used by 2 task(s); pass --to <status>"
    );
    assert_eq!(
        env.json_err(&["status", "rm", "doing", "-p", "APM", "--to", "Doing"]),
        "--to must name another status than doing"
    );
    assert_eq!(
        env.json_err(&["status", "rm", "doing", "-p", "APM", "--to", "nope"]),
        "no such status: nope"
    );
    assert_eq!(
        text(
            &env,
            &["status", "rm", "doing", "-p", "APM", "--to", "blocked"]
        ),
        "removed status doing (2 task(s) moved to blocked)\n"
    );
    let all = env.json(&["list", "--all"]);
    assert_eq!(all[0]["status"], "blocked");
    assert_eq!(all[1]["status"], "blocked");
}

#[test]
fn a_project_keeps_one_status() {
    let env = env_apm();
    for s in ["doing", "blocked", "done", "cancelled"] {
        env.json(&["status", "rm", s, "-p", "APM"]);
    }
    assert_eq!(
        env.json_err(&["status", "rm", "todo", "-p", "APM"]),
        "cannot remove the last status of APM"
    );
}

// ---- tasks with custom statuses

#[test]
fn tasks_take_any_status_of_their_project() {
    let env = env_apm();
    env.json(&[
        "status",
        "add",
        "Needs Rework",
        "-p",
        "APM",
        "--category",
        "active",
    ]);
    let i = env.json(&["add", "a", "-p", "APM", "--status", "needs rework"]);
    assert_eq!(i["status"], "Needs Rework");
    assert_eq!(i["status_category"], "active");
    let i = env.json(&["edit", "APM-1", "--status", "DONE"]);
    assert_eq!(i["status"], "done");
    assert_eq!(i["status_category"], "done");
    assert_eq!(
        env.json_err(&["add", "b", "-p", "APM", "--status", "later"]),
        "no such status: later"
    );
    assert_eq!(
        env.json_err(&["edit", "APM-1", "--status", "later"]),
        "no such status: later"
    );
    // Another project's status is unknown here.
    env.json(&["project", "add", "WEB", "web"]);
    env.json(&["status", "add", "design", "-p", "WEB", "--category", "todo"]);
    assert_eq!(
        env.json_err(&["edit", "APM-1", "--status", "design"]),
        "no such status: design"
    );
}

#[test]
fn new_tasks_get_the_first_todo_status() {
    let env = env_apm();
    env.json(&[
        "status",
        "add",
        "inbox",
        "-p",
        "APM",
        "--category",
        "todo",
        "--position",
        "1",
    ]);
    assert_eq!(env.json(&["add", "a", "-p", "APM"])["status"], "inbox");
    env.json(&["status", "rm", "inbox", "-p", "APM", "--to", "doing"]);
    env.json(&["status", "rm", "todo", "-p", "APM"]);
    assert_eq!(
        env.json_err(&["add", "b", "-p", "APM"]),
        "project APM has no todo-category status"
    );
    // An explicit status still works.
    assert_eq!(
        env.json(&["add", "b", "-p", "APM", "--status", "doing"])["id"],
        "APM-2"
    );
}

#[test]
fn shortcuts_follow_categories() {
    let env = env_apm();
    env.json(&[
        "status",
        "add",
        "started",
        "-p",
        "APM",
        "--category",
        "active",
        "--position",
        "2",
    ]);
    env.json(&[
        "status",
        "add",
        "shipped",
        "-p",
        "APM",
        "--category",
        "done",
        "--position",
        "1",
    ]);
    env.json(&["add", "a", "-p", "APM"]);
    assert_eq!(env.json(&["start", "APM-1"])["status"], "started");
    assert_eq!(env.json(&["done", "APM-1"])["status"], "shipped");
    assert_eq!(env.json(&["block", "APM-1"])["status"], "blocked");
    env.json(&["status", "rm", "blocked", "-p", "APM", "--to", "started"]);
    assert_eq!(
        env.json_err(&["block", "APM-1"]),
        "project APM has no status named blocked"
    );
    for s in ["started", "doing"] {
        env.json(&["status", "rm", s, "-p", "APM", "--to", "todo"]);
    }
    assert_eq!(
        env.json_err(&["start", "APM-1"]),
        "project APM has no active-category status"
    );
}

#[test]
fn list_status_filters_by_name() {
    let env = env_apm();
    env.json(&["project", "add", "WEB", "web"]);
    env.json(&[
        "status",
        "add",
        "review",
        "-p",
        "WEB",
        "--category",
        "active",
    ]);
    env.json(&[
        "status",
        "add",
        "shipped",
        "-p",
        "WEB",
        "--category",
        "done",
    ]);
    env.json(&["add", "a", "-p", "APM", "--status", "doing"]);
    env.json(&["add", "b", "-p", "WEB", "--status", "doing"]);
    env.json(&["add", "c", "-p", "WEB", "--status", "review"]);
    env.json(&["add", "d", "-p", "WEB", "--status", "shipped"]);
    // Across projects a name matches in every project that has it, case-insensitively.
    assert_eq!(
        ids(&env.json(&["list", "--status", "DOING"])),
        ["APM-1", "WEB-1"]
    );
    assert_eq!(
        ids(&env.json(&["list", "--status", "review, shipped"])),
        ["WEB-2", "WEB-3"]
    );
    // With a project the name must exist there.
    assert_eq!(
        env.json_err(&["list", "-p", "APM", "--status", "review"]),
        "no such status: review"
    );
    assert_eq!(
        ids(&env.json(&["list", "-p", "WEB", "--status", "Review"])),
        ["WEB-2"]
    );
    // Custom done-category statuses are hidden unless --all.
    assert_eq!(ids(&env.json(&["list"])), ["APM-1", "WEB-1", "WEB-2"]);
}

#[test]
fn project_show_counts_by_the_projects_statuses() {
    let env = env_apm();
    env.json(&[
        "status",
        "add",
        "idea",
        "-p",
        "APM",
        "--category",
        "todo",
        "--position",
        "1",
    ]);
    env.json(&["status", "rm", "blocked", "-p", "APM"]);
    env.json(&["add", "a", "-p", "APM", "--status", "idea"]);
    env.json(&["add", "b", "-p", "APM", "--status", "done"]);
    env.json(&["add", "c", "--parent", "APM-1", "--status", "done"]);
    let counts = &env.json(&["project", "show", "APM"])["counts"];
    assert_eq!(
        counts,
        &json!({"idea": 1, "todo": 0, "doing": 0, "done": 2, "cancelled": 0})
    );
    // The JSON object lists them in status order (serde_json's Value would sort the keys,
    // so read the raw output).
    let out = env
        .apm()
        .args(["--json", "project", "show", "APM"])
        .assert()
        .success();
    let raw = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    let at = |k: &str| raw.find(&format!("\"{k}\":")).unwrap();
    let order: Vec<usize> = ["idea", "todo", "doing", "done", "cancelled"]
        .iter()
        .map(|k| at(k))
        .collect();
    assert!(order.is_sorted(), "{raw}");
    assert!(
        text(&env, &["project", "show", "APM"])
            .contains("tasks: idea=1 todo=0 doing=0 done=2 cancelled=0\n")
    );
}

#[test]
fn milestone_progress_counts_by_category() {
    let env = env_apm();
    env.json(&["milestone", "add", "v1", "-p", "APM"]);
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
        "wontfix",
        "-p",
        "APM",
        "--category",
        "cancelled",
    ]);
    for s in ["todo", "shipped", "done", "wontfix", "cancelled"] {
        env.json(&["add", s, "-p", "APM", "-m", "v1", "--status", s]);
    }
    let m = env.json(&["milestone", "show", "v1", "-p", "APM"]);
    assert_eq!(
        m["progress"],
        json!({"total": 3, "done": 2, "cancelled": 2})
    );
    assert_eq!(m["items"][1]["status"], "shipped");
}

#[test]
fn project_rm_removes_its_statuses() {
    let env = env_apm();
    env.json(&["add", "a", "-p", "APM"]);
    env.json(&["project", "rm", "APM"]);
    let n: i64 = env
        .conn()
        .query_row("SELECT count(*) FROM statuses", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 0);
    env.json(&["project", "add", "APM", "again"]);
    assert_eq!(names(&env, "APM").len(), 5);
}
