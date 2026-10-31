use clap::Args;

use crate::error::Result;
use crate::output;

/// The guide is static text compiled into the binary.
const GUIDE: &str = include_str!("guide.txt");

/// Lines to paste into an agent's instructions file (AGENTS.md, CLAUDE.md, ...).
const AGENTS: &str = "\
## Task tracking and shared memory: apm
- Set APM_AGENT to your name. Fresh session on a project: `apm context -p KEY --json`.
- Starting or resuming a task (also after context compaction): `apm context KEY-N --json`
  (no id? `apm context` finds the task you claimed; `apm next -p KEY --context` claims one).
- Log findings as you go: `apm comment KEY-N \"...\"`; ask with `--kind question`.
- Before you stop or hand over: `apm comment KEY-N --kind handoff -` (state, next, decisions + why, where).
- Knowledge that outlives the task (decisions, architecture, conventions, gotchas): `apm page add/edit`.
- Before re-deriving something: `apm search \"words\" -p KEY`. Details: `apm guide`.
";

#[derive(Args, Debug)]
pub struct GuideArgs {
    /// Print only the lines to paste into an agent's instructions file (AGENTS.md,
    /// CLAUDE.md), so every session knows to use apm
    #[arg(long)]
    pub agents: bool,
}

/// `apm guide`: a compact usage guide for agents. Never opens the database.
pub fn run(json: bool, args: &GuideArgs) -> Result<()> {
    let (key, text) = if args.agents {
        ("agents", AGENTS)
    } else {
        ("guide", GUIDE)
    };
    if json {
        output::json(&serde_json::json!({ key: text }))
    } else {
        print!("{text}");
        Ok(())
    }
}
