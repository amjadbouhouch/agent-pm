# AGENTS.md

`apm` (agent-pm): one small executable that agents and humans use to track projects,
milestones, tasks, subtasks, assignees and custom attributes. A mini Notion for the terminal.

## Principles

- **Small binary.** Rust + bundled SQLite, release profile tuned for size. Check the size of
  `target/release/apm(.exe)` after changes that add code or crates; report it if it grows by
  more than ~10%. No new crate without the orchestrator's (or owner's) OK.
- **Agents are the main user.** Every command accepts `--json` and prints stable JSON on stdout;
  errors go to stderr (`error: <msg>`, or `{"error": "<msg>"}` with `--json`) with exit code 1.
  Text output stays short and grep-friendly. Never prompt: no interactive confirmations.
- **Simple over clever.** Plain SQL with `rusqlite`, no ORM, no async, no global state.

## Commands

```sh
cargo fmt --check                                  # format
cargo clippy --all-targets -- -D warnings          # lint
cargo test                                         # unit + integration tests
cargo build --release                              # binary in target/release/
```

All four must pass before a commit is "done".

Windows note: this machine uses the GNU toolchain (`stable-x86_64-pc-windows-gnu`) with WinLibs
GCC on PATH, because the MSVC build tools need admin. Linux/macOS use the default toolchain.

## Layout

```
src/main.rs        parse args, run, print errors
src/cli.rs         top-level clap definition (global flags, command enum)
src/cmd/mod.rs     Ctx {conn, json, actor} and dispatch
src/cmd/<group>.rs one module per command group: its clap args, its `run`, and
                   functions other groups call (e.g. milestone::resolve, comment::for_item)
src/db.rs          db path, schema migrations, shared lookups (project_id, item KEY-N)
src/model.rs       Status, Priority, name=value parsing, date check
src/output.rs      JSON printing, errors, text tables
tests/             integration tests (run the real binary on a temp DB)
```

## Conventions

- Errors: return `crate::error::Result`; build with `err("...")`. Messages are lowercase and
  name the thing (`no such project: APM`).
- Ids: projects have an uppercase KEY; tasks are `KEY-N` (per-project sequence, never reused).
  Keys are case-insensitive on input.
- Database: `--db` > `$APM_DB` > `~/.agent-pm/apm.db`. Schema changes are a **new** entry
  appended to `MIGRATIONS` in `db.rs`; never edit a released one. Foreign keys are on.
  Timestamps are UTC ISO text written by SQLite defaults; set `updated_at` on every update.
- Actor: `--as` > `$APM_AGENT` > OS user. Stored as `created_by` and comment author.
- Writes return the written object: under `--json` the full object, otherwise one line
  (`created APM-3`).
- Tests: every command gets integration tests in `tests/<group>.rs` using `tests/common`
  (`Env::new()`, `env.json(&[...])`, `env.json_err(&[...])`). Test behaviour through the CLI,
  JSON output first. Pure helpers get unit tests next to them.

## Workflow

- Merge target: `main`. Work on a branch; the orchestrator reviews and merges locally
  (`git merge --no-ff`) and pushes `main`. Workers don't push `main`.
- Commits: short imperative subject (`add milestone commands`).
