# AGENTS.md

`apm` (agent-pm): one small executable that agents and humans use to track projects,
milestones, tasks, subtasks, assignees, tags, links, history, custom statuses and custom
attributes. A mini Notion for the terminal.

## Principles

- **Small binary.** Rust + bundled SQLite, release profile tuned for size. Check the size of
  `target/release/apm(.exe)` after changes that add code or crates; report it if it grows by
  more than ~10%. No new crate without the orchestrator's (or owner's) OK.
- **Agents are the main user.** Every command accepts `--json` and prints stable JSON on stdout;
  errors go to stderr (`error: <msg>`, or `{"error": "<msg>"}` with `--json`) with exit code 1;
  invalid arguments exit 2 (clap's usage text, or the same JSON shape under `--json`).
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
                   functions other groups call (e.g. milestone::resolve, comment::for_item):
                   project, item (add/list/show/edit/rm + shortcuts), milestone, attr,
                   comment, status (custom statuses), link (link/unlink, blockers),
                   tag (tags, `apm tags`), history (`apm history`), mv (`apm move`),
                   guide (+ guide.txt), db
src/db.rs          db path, schema migrations, SQL functions, shared lookups (project_id,
                   item KEY-N, current_key), record_event
src/model.rs       Category, Priority, name=value parsing, date and date-time (`parse_when`)
src/output.rs      JSON printing, errors, text tables
tests/             integration tests (run the real binary on a temp DB)
install.sh         installer for release binaries (POSIX sh)
scripts/           version-updater.cjs for commit-and-tag-version
```

## Conventions

- Errors: return `crate::error::Result`; build with `err("...")`. Messages are lowercase and
  name the thing (`no such project: APM`).
- Ids: projects have an uppercase KEY (1-10 letters/digits, `db::check_project_key`); tasks are
  `KEY-N` (per-project sequence, never reused, `db::parse_item_key`). Case-insensitive on input.
  Old ids keep resolving: a re-keyed project's old KEY via `project_aliases`, a moved task's old
  `KEY-N` via `item_aliases`. Always look up through `db::project_id` / `db::item`; output shows
  current ids only: `db::current_key` when you only have a row id (queries may still join
  `p.key || '-' || i.seq`). A live key wins: creating or re-keying to a KEY drops the alias of
  that name (`db::claim_project_key`), so a key is never both live and an alias.
- History: every task mutation records events with `db::record_event` inside its write
  transaction (`db::write_tx`), one per real change (diff before/after, no-ops record
  nothing). Pick an `EventKind`; a new kind is appended to the enum with a doc comment saying
  what `field`/`old`/`new` hold. Values are stored as displayed (status name, KEY-N).
- Text matching: SQLite's `lower`/`LIKE` fold ASCII only; use the `fold(text)` SQL function
  (registered in `db.rs`) for case-insensitive matching of user text.
- Text output is ASCII (`->`, `...`); user data is printed as given.
  `text_output_is_ascii` in `tests/workflow.rs` checks it.
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
- Commits: [Conventional Commits](https://www.conventionalcommits.org) with a short imperative
  subject (`feat: add milestone commands`, `fix(item): ...`, `docs: ...`). The release tool
  derives the version and changelog from them, so a wrong prefix ships a wrong version.

## Releasing

```sh
# first release only: tags the current 0.1.0 without bumping
npx commit-and-tag-version --first-release
git push --follow-tags origin main

# every later release
npx commit-and-tag-version --dry-run          # preview the bump and changelog
npx commit-and-tag-version                    # bump, write CHANGELOG.md, commit, tag vX.Y.Z
git push --follow-tags origin main            # the tag push triggers release.yml
```

There is no package.json; `.versionrc.json` points the
tool at `Cargo.toml` and `Cargo.lock` through `scripts/version-updater.cjs`, which rewrites
only the `[package]` version and the lockfile's `apm` entry, so `--locked` builds keep
passing after a bump.

While the version is below 1.0.0, `feat:` bumps the **patch** and only `feat!:` /
`BREAKING CHANGE:` bumps the minor. Override with `--release-as minor`.

`release.yml` runs fmt, clippy and the tests in a `test` job that `build` depends on, so a
tag whose tests fail publishes nothing. `build` compiles on native runners and uploads
`apm-<os>-<arch>` (`apm-windows-x64.exe` on Windows) plus a `.sha256` for each; `publish`
creates the GitHub release. A manual run (`workflow_dispatch` with `tag`) rebuilds an
existing tag. `ci.yml` covers pull requests and manual runs only, on Linux, macOS and Windows.

`install.sh` downloads the asset for the platform from `uname`, verifies the checksum, runs
`--version` before replacing anything, and offers to edit the shell profile (prompting via
`/dev/tty`, since stdin is the script under `curl | sh`). `--uninstall` removes only the
binary: `~/.agent-pm` also holds the database. `tests/release_assets.rs` runs it against a
stubbed `uname` for all four platforms and asserts every asset it requests is in
`release.yml`, and that Cargo.toml, Cargo.lock and `apm --version` agree.
