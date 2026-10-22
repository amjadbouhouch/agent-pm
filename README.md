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

Windows: download `apm-windows-x64.exe` from the
[latest release](https://github.com/amjadbouhouch/agent-pm/releases/latest), rename it to
`apm.exe` and put it on your `PATH`.

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

## Agent loop

Several agents can work one project without picking the same task:

```sh
export APM_AGENT=bot
apm --json next -p APM          # claim, assign and start the best ready task
# ... work ...
apm comment APM-4 "draft in docs/notes.md"
apm claim APM-4 --ttl 4h        # renew the claim (default 2h, max 7d)
apm done APM-4                  # closing a task releases its claim
```

`next` picks, in one transaction, the open task with the highest priority (then soonest due,
then id) that has no open blocker, is not in a status named blocked, is not claimed by
another agent and is unassigned or yours. When nothing is ready it prints `{"task": null}`
and exits with code 3. A claim expires on its own; `release` drops it early, `claim --steal`
takes over someone else's, and `apm claims` lists the live ones.

## Commands

| Command | What it does |
| --- | --- |
| `project add\|list\|show\|edit\|rm` | Projects, each with a short KEY (`APM`); `edit --key` re-keys |
| `add`, `list`, `show`, `edit`, `rm` | Tasks and subtasks, referenced as `KEY-N` |
| `list` filters | `-p`, `--status`, `-a`, `-m`, `--parent`, `--where`, `--tag`, `--ready`, `--blocked`, `--search`, `--due-before`, `--due-after`, `--overdue`; `--sort`, `--limit`, `--all` |
| `start`, `done`, `block` | Status shortcuts |
| `move KEY-N PROJECT` | Move a task and its subtasks to another project, under new numbers |
| `next`, `claim`, `release`, `claims` | Claim work so agents don't collide; `list --claimed/--unclaimed` |
| `status list\|add\|edit\|rm` | Custom statuses per project |
| `milestone add\|list\|show\|edit\|close\|reopen\|rm` | Milestones with progress |
| `attr add\|edit\|list\|rm` | Custom attributes, set with `--set`, filtered with `--where` |
| `comment KEY-N TEXT` | Comment on a task |
| `link\|unlink A KIND B` | Links between tasks, also across projects: `blocks` (no cycles), `relates`, `duplicates` |
| `tags`, `add/edit --tag`, `edit --untag` | Lowercase tags and their counts |
| `history KEY-N` | A task's change history, oldest first |
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

## License

TBD
