# agent-pm

A command-line project manager built by agents, for agents — a mini Notion-style
tracker you drive from the terminal.

## Idea

Agents need somewhere durable to plan and track work: projects, tasks, who owns
what, and where it stands. `agent-pm` gives them a small, predictable CLI for that,
with output that is easy for both agents and humans to read.

`apm` is one small binary with SQLite built in. Every command takes `--json` and
prints stable JSON, so agents can parse it; without it, output is short plain text.

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
apm --json show APM-1
```

Data lives in `~/.agent-pm/apm.db`; override with `--db PATH` or `$APM_DB`.

## Commands

| Command | What it does |
| --- | --- |
| `project add\|list\|show\|edit\|rm` | Projects, each with a short KEY (`APM`) |
| `add`, `list`, `show`, `edit`, `rm` | Tasks and subtasks, referenced as `KEY-N` |
| `start`, `done`, `block` | Status shortcuts |
| `milestone add\|list\|show\|edit\|close\|reopen\|rm` | Milestones with progress |
| `attr add\|list\|rm` | Custom attributes (text, number, select, date, checkbox), set with `--set`, filtered with `--where` |
| `comment KEY-N TEXT` | Comment on a task |
| `guide` | Compact usage guide for agents |
| `db` | Database path and schema version |

Run `apm guide` for the full workflow and the JSON contract, and `apm <command> --help`
for every flag.

## Attributes

Every item comes with a set of **default attributes** (title, status, priority,
assignee, due date, milestone, parent, created/updated timestamps).

On top of those, a project can declare **custom attributes** when it needs more —
Notion-style properties such as text, number, select, date, or checkbox — without
changing the tool itself.

## License

TBD
