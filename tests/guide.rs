mod common;

use common::Env;

fn guide_text(env: &Env) -> String {
    let out = env.apm().arg("guide").assert().success();
    String::from_utf8(out.get_output().stdout.clone()).unwrap()
}

/// Top-level command names, read from the `Commands:` section of `apm --help`.
fn commands(env: &Env) -> Vec<String> {
    let out = env.apm().arg("--help").assert().success();
    let help = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    help.lines()
        .skip_while(|l| !l.starts_with("Commands:"))
        .skip(1)
        .take_while(|l| l.starts_with("  "))
        .filter_map(|l| l.split_whitespace().next())
        .filter(|c| *c != "help")
        .map(str::to_string)
        .collect()
}

#[test]
fn guide_mentions_every_command() {
    let env = Env::new();
    let text = guide_text(&env);
    let cmds = commands(&env);
    assert!(cmds.len() >= 14, "parsed too few commands: {cmds:?}");
    for c in cmds {
        assert!(
            text.split(|ch: char| !ch.is_ascii_alphanumeric())
                .any(|w| w == c),
            "guide does not mention `{c}`"
        );
    }
}

#[test]
fn guide_is_compact() {
    let env = Env::new();
    // Compact enough to read in one go; v2 (statuses, links, tags, history, move) needed
    // ~100, v4 (context, log kinds, pages, search) needs 142.
    assert!(guide_text(&env).lines().count() <= 150);
}

#[test]
fn guide_json() {
    let env = Env::new();
    let v = env.json(&["guide"]);
    assert_eq!(v["guide"].as_str().unwrap(), guide_text(&env));
    assert!(!env.db_path().exists());
}

#[test]
fn guide_agents_snippet() {
    let env = Env::new();
    let out = env.apm().args(["guide", "--agents"]).assert().success();
    let text = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert!(lines.len() <= 10, "{text}");
    assert!(lines[0].starts_with("## "));
    for needle in [
        "apm context -p KEY --json",
        "apm context KEY-N --json",
        "--kind handoff -",
        "--kind question",
        "apm page add",
        "apm search",
        "APM_AGENT",
    ] {
        assert!(text.contains(needle), "{needle} not in:\n{text}");
    }
    assert!(text.is_ascii());
    assert_eq!(env.json(&["guide", "--agents"])["agents"], text);
    assert!(!env.db_path().exists());
}

#[test]
fn guide_has_a_knowledge_section() {
    let env = Env::new();
    let text = guide_text(&env);
    assert!(text.contains("\nKNOWLEDGE "));
    assert!(text.contains("\n  Every command has --help.\n"));
    for needle in [
        "apm context -p APM",
        "--kind handoff",
        "--answer",
        "apm page verify",
    ] {
        assert!(text.contains(needle), "{needle}");
    }
}
