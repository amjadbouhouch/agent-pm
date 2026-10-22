# agent-pm

A command-line project manager built by agents, for agents — a mini Notion-style
tracker you drive from the terminal.

## Idea

Agents need somewhere durable to plan and track work: projects, tasks, who owns
what, and where it stands. `agent-pm` gives them a small, predictable CLI for that,
with output that is easy for both agents and humans to read.

`apm` is one small binary with SQLite built in. Every command takes `--json` and
prints stable JSON, so agents can parse it; without it, output is short plain text.

## Features

- Projects, milestones (with progress), tasks and subtasks, assignees, priorities, comments
- Custom statuses per project ("brainstorming", "needs re-work", ...), grouped in categories
- Custom attributes: text, number, select, date, datetime, checkbox
- Due dates as dates or date-times with a zone, stored in UTC; `--overdue`, due ranges
- Links between tasks (`blocks`, `relates`, `duplicates`), with `list --ready` / `--blocked`
- Tags, filtered with `list --tag`, counted with `apm tags`
- Search (titles, bodies, comments), multi-key sort, limit
- History: every change to a task, who made it and when
- Export/import projects as JSON (same task ids and history), and one-command backups
- Move tasks and re-key projects (old ids keep resolving); rename statuses, milestones and
  attributes

## Install

macOS and Linux (x64, arm64):

```sh
curl -fsSL https://raw.githubusercontent.com/amjadbouhouch/agent-pm/main/install.sh | sh
```

This puts `apm` in `~/.agent-pm/bin` and offers to add it to your `PATH`. Re-run it to
upgrade. Options: `--version v0.1.0`, `--dir DIR`, `--no-modify-path`, `--uninstall`
(keeps your database); `sh install.sh --help` lists them all.

Windows (x64; PowerShell 5.1 or 7):

```powershell
irm https://raw.githubusercontent.com/amjadbouhouch/agent-pm/main/install.ps1 | iex
```

This puts `apm.exe` in `%USERPROFILE%\.agent-pm\bin` and offers to add it to your user
`PATH` (no admin). Re-run it to upgrade. `irm | iex` takes no parameters; to pass them,
run the script as a scriptblock:

```powershell
& ([scriptblock]::Create((irm https://raw.githubusercontent.com/amjadbouhouch/agent-pm/main/install.ps1))) -Version v0.3.0
```

Parameters: `-Version vX.Y.Z`, `-Dir DIR`, `-NoModifyPath`, `-Uninstall` (removes `apm.exe`
and its PATH entry, keeps your database), `-Help`. The `irm | iex` form reads the same
settings from `APM_VERSION`, `APM_INSTALL_DIR` and `APM_MODIFY_PATH` (`yes`/`no`); a
`$env:APM_VERSION` you set stays set for the rest of the session and pins later upgrades,
so clear it afterwards (`Remove-Item Env:APM_VERSION`). A downloaded copy runs as
`powershell -ExecutionPolicy Bypass -File .\install.ps1 -Version v0.3.0`.

Or build it with Rust (also the way on musl/Alpine):

```sh
cargo install --git https://github.com/amjadbouhouch/agent-pm
```

## Quick start

```sh
export APM_AGENT=alice                    # who is acting (default: OS user)
apm project add APM "agent-pm"
apm milestone add v1 -p APM --due 2026-12-01
apm add "parse args" -p APM -m v1 -a alice --priority high
apm add "flag tests" --parent APM-1       # subtask
apm start APM-1
apm comment APM-1 "clap derive works"
apm list -p APM
apm list --search "clap" --sort -priority,due --limit 5
apm list --overdue                        # open tasks due before now
apm --json show APM-1
apm add "cache layer" -p APM --tag perf
apm link APM-1 blocks APM-3               # also relates, duplicates
apm list -p APM --ready                   # open tasks with no open blocker
apm history APM-3
```

Data lives in `~/.agent-pm/apm.db`; override with `--db PATH` or `$APM_DB`.

## Commands

| Command | What it does |
| --- | --- |
| `project add\|list\|show\|edit\|rm` | Projects, each with a short KEY (`APM`); `edit --key` re-keys |
| `add`, `list`, `show`, `edit`, `rm` | Tasks and subtasks, referenced as `KEY-N` |
| `list` filters | `-p`, `--status`, `-a`, `-m`, `--parent`, `--where`, `--tag`, `--ready`, `--blocked`, `--search`, `--due-before`, `--due-after`, `--overdue`; `--sort`, `--limit`, `--all` |
| `start`, `done`, `block` | Status shortcuts |
| `move KEY-N PROJECT` | Move a task and its subtasks to another project, under new numbers |
| `status list\|add\|edit\|rm` | Custom statuses per project |
| `milestone add\|list\|show\|edit\|close\|reopen\|rm` | Milestones with progress |
| `attr add\|edit\|list\|rm` | Custom attributes, set with `--set`, filtered with `--where` |
| `comment KEY-N TEXT` | Comment on a task |
| `link\|unlink A KIND B` | Links between tasks, also across projects: `blocks` (no cycles), `relates`, `duplicates` |
| `tags`, `add/edit --tag`, `edit --untag` | Lowercase tags and their counts |
| `history KEY-N` | A task's change history, oldest first |
| `export`, `import FILE\|-`, `backup [PATH]` | Projects as JSON, back in another database; a copy of the database |
| `guide` | Compact usage guide for agents |
| `db` | Database path and schema version |

Run `apm guide` for the full workflow and the JSON contract, and `apm <command> --help`
for every flag.

## Concepts

A **project** has a short KEY (`APM`); its tasks are numbered `APM-1`, `APM-2`, ... and a
number is never reused. Tasks can have subtasks, a milestone, tags, comments and links to
other tasks, in any project.

Every task has **default attributes** (title, body, status, priority, assignee, due,
milestone, parent, created/updated timestamps). On top of those, a project can declare
**custom attributes** when it needs more (Notion-style text, number, select, date, datetime
or checkbox properties) without changing the tool itself.

**Statuses** are per project: every project starts with todo, doing, blocked, done and
cancelled, and can add, rename, reorder or remove its own. Each status belongs to a
**category** (todo, active, done, cancelled), which decides what `list` hides (done and
cancelled), how milestone progress counts, and where `start`/`done` go.

A task is **blocked** while another open task `blocks` it; `list --ready` shows open tasks
with no open blocker. Every change to a task is recorded in its **history** with the actor
and time.

**Ids keep working.** Moving a task gives it a new number in the target project and
re-keying a project changes every task id, but the old `KEY-N` and the old KEY still
resolve. Output always shows the current ids.

Due dates (tasks and milestones) and `datetime` attributes take a date (`2026-11-20`) or a
date-time with a zone (`2026-11-20T17:00+01:00`, `...Z`), stored in UTC. Date-times without a
zone are rejected. In `list --due-before/--due-after`, a date bound compares the UTC date
part of a due; `--overdue` is open tasks due before now (date dues: before today, UTC).

## Backup and move data

`apm backup` copies the whole database, safely while agents write to it, to
`~/.agent-pm/backups/apm-<UTC time>.db` (or `backups/` next to a `--db`/`$APM_DB` file) and
prints the path; `--keep 7` then deletes all but the 7 newest backups it made there (other
files are left alone). Pass a path to write elsewhere. A backup is a normal database:
`apm --db <backup> list` reads it.

To move projects to another machine or database, export them and import the file:

```sh
apm export -p APM -o apm.json        # all projects without -p; stdout without -o
apm --db other.db import apm.json    # or: apm export | apm --db other.db import -
apm import apm.json --key APM2       # a one-project file under another key
```

The file is versioned JSON (`"apm_export": 1`) with each project's statuses, milestones,
attributes and tasks (same `KEY-N`, values, tags, comments, history) and the links between
exported tasks. Import is all or nothing: it checks the whole file first, refuses a key that
already exists, and `--dry-run` shows what it would create. Old-id aliases and comment ids are
not carried over.

## License

TBD
