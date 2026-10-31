//! `apm context`: what earlier agents learned, in one call and a few KB. For a task: the task,
//! its latest handoff, recent log, open questions, subtasks, links and pages. For a project:
//! pinned pages, open questions and the latest handoff of every in-flight task.

use clap::Args;
use rusqlite::types::Value as SqlValue;
use rusqlite::{Connection, params};
use serde::Serialize;

use super::comment::{self, Comment};
use super::page::{self, PageSummary};
use super::{Ctx, claim, item, link, project};
use crate::db;
use crate::error::{Result, err};
use crate::model::CommentKind;
use crate::output;

/// Log entries (and handoffs in project context) are cut at this many characters.
const CUT: usize = 280;
/// Log entries shown by default (`--log`).
pub const DEFAULT_LOG: u32 = 5;
/// At most this many pages, open questions and in-flight tasks are listed; the rest are
/// counted. They keep a task's context under ~8 KB and a project's under ~10 KB.
const PAGES_MAX: usize = 10;
const QUESTIONS_MAX: usize = 10;
const PROJECT_QUESTIONS_MAX: usize = 8;
const TASKS_MAX: usize = 8;

#[derive(Args, Debug)]
pub struct ContextArgs {
    /// Task id, e.g. APM-12; without one, the task you hold a live claim on
    #[arg(conflicts_with = "project")]
    pub key: Option<String>,
    /// A project's context instead, for a fresh session: pinned pages, open questions and the
    /// latest handoff of every in-flight task
    #[arg(short, long, value_name = "KEY")]
    pub project: Option<String>,
    /// How many recent log entries to show besides the latest handoff, each cut at 280
    /// characters (0-50)
    #[arg(long, value_name = "N", default_value_t = DEFAULT_LOG,
          value_parser = clap::value_parser!(u32).range(0..=50))]
    pub log: u32,
}

/// A log entry as context shows it: the comment, cut at CUT characters.
#[derive(Serialize, Debug)]
pub struct Entry {
    #[serde(flatten)]
    comment: Comment,
    truncated: bool,
}

impl Entry {
    fn cut(mut comment: Comment) -> Entry {
        let truncated = comment.body.chars().count() > CUT;
        if truncated {
            let cut: String = comment.body.chars().take(CUT).collect();
            comment.body = format!("{}...", cut.trim_end());
        }
        Entry { comment, truncated }
    }
}

/// A subtask as one line.
#[derive(Serialize, Debug)]
struct Line {
    id: String,
    status: String,
    title: String,
}

/// `apm context KEY-N`.
#[derive(Serialize, Debug)]
pub struct TaskContext {
    task: item::Item,
    /// The newest handoff, in full
    handoff: Option<Comment>,
    /// The newest `--log N` other entries (not the handoff or a listed open question),
    /// oldest first
    log: Vec<Entry>,
    /// How many such entries the task has in all
    log_total: i64,
    /// Unanswered questions on the task and its subtasks, oldest first, in full
    questions: Vec<Comment>,
    more_questions: i64,
    subtasks: Vec<Line>,
    links: link::Links,
    pages: Vec<PageSummary>,
    more_pages: i64,
    hints: Vec<String>,
}

/// An in-flight task in project context.
#[derive(Serialize, Debug)]
struct InFlight {
    id: String,
    status: String,
    title: String,
    claimed_by: Option<String>,
    /// Its newest handoff, cut like log entries
    handoff: Option<Entry>,
}

/// `apm context -p KEY`.
#[derive(Serialize, Debug)]
struct ProjectContext {
    project: project::Project,
    pages: Vec<PageSummary>,
    more_pages: i64,
    /// Unanswered questions on the project's open tasks, oldest first
    questions: Vec<Comment>,
    more_questions: i64,
    /// Tasks in an active-category status, most recently updated first
    tasks: Vec<InFlight>,
    more_tasks: i64,
    hints: Vec<String>,
}

pub fn run(ctx: &Ctx, args: ContextArgs) -> Result<()> {
    if let Some(key) = &args.project {
        let c = for_project(&ctx.conn, db::project_id(&ctx.conn, key)?)?;
        if ctx.json {
            return output::json(&c);
        }
        print_project(&c);
        return Ok(());
    }
    let item_id = match &args.key {
        Some(key) => db::item(&ctx.conn, key)?.id,
        None => claimed(&ctx.conn, &ctx.actor)?,
    };
    let c = for_task(&ctx.conn, item_id, args.log)?;
    if ctx.json {
        return output::json(&c);
    }
    print_task(&c);
    Ok(())
}

/// The task the actor holds a live claim on: exactly one, else an error saying what to pass.
fn claimed(conn: &Connection, actor: &str) -> Result<i64> {
    let mut vals: Vec<SqlValue> = Vec::new();
    let mark = item::bind(&mut vals, actor.to_string());
    let items = item::query(
        conn,
        &format!("{} AND i.claimed_by = {mark}", claim::live()),
        &vals,
    )?;
    match items.as_slice() {
        [one] => Ok(one.row_id),
        [] => Err(err(format!(
            "{actor} holds no claim: pass a task (apm context KEY-N) or a project \
             (apm context -p KEY)"
        ))),
        many => {
            let ids: Vec<&str> = many.iter().map(|i| i.id.as_str()).collect();
            Err(err(format!(
                "{actor} holds {} claims ({}): pass one (apm context KEY-N)",
                many.len(),
                ids.join(", ")
            )))
        }
    }
}

/// The task context of a task (also `next --context`).
pub fn for_task(conn: &Connection, item_id: i64, log: u32) -> Result<TaskContext> {
    let task = item::get(conn, item_id)?;
    let handoff = comment::latest(conn, item_id, CommentKind::Handoff)?;
    let mut stmt = conn.prepare(
        "WITH RECURSIVE down(id) AS (
             SELECT ?1 UNION SELECT i.id FROM items i JOIN down ON i.parent_id = down.id
         ) SELECT id FROM down",
    )?;
    let tree = stmt
        .query_map(params![item_id], |r| r.get(0))?
        .collect::<rusqlite::Result<Vec<i64>>>()?;
    let (questions, more_questions) = capped(comment::open_questions(conn, &tree)?, QUESTIONS_MAX);

    // The log: the task's other entries (not the handoff or open questions shown above).
    let shown: Vec<i64> = handoff.iter().chain(&questions).map(|c| c.id).collect();
    let others: Vec<Comment> = comment::for_item(conn, item_id)?
        .into_iter()
        .filter(|c| !shown.contains(&c.id))
        .collect();
    let log_total = others.len() as i64;
    let skip = others.len().saturating_sub(log as usize);
    let log: Vec<Entry> = others.into_iter().skip(skip).map(Entry::cut).collect();

    let subtasks = item::query(conn, "i.parent_id = ?1", &[item_id.into()])?
        .into_iter()
        .map(|s| Line {
            id: s.id,
            status: s.status,
            title: s.title,
        })
        .collect();
    let (pages, more) = page::for_context(conn, item_id)?;
    let (pages, cut) = capped(pages, PAGES_MAX);
    let more_pages = more + cut;

    let id = &task.id;
    let mut hints = Vec::new();
    if handoff.is_none() && log_total == 0 {
        hints.push(format!(
            "nobody logged on this task yet: apm comment {id} \"...\" as you learn things"
        ));
    }
    if !questions.is_empty() {
        hints.push(format!(
            "open questions: answer with apm comment {} --answer ID \"...\"",
            answer_on(&questions, id)
        ));
    }
    if let Some(p) = pages.iter().find(|p| p.stale) {
        hints.push(format!(
            "stale pages: re-read, then apm page verify {} (still true) or apm page edit {}",
            p.id, p.id
        ));
    }
    hints.push(format!(
        "before you stop: apm comment {id} --kind handoff - (state, next, decisions + why, where)"
    ));
    hints.push(format!(
        "learned something that outlives this task? apm page add {}:<slug> \"Title\" -s \"gist\" \
         -b - --link {id}",
        task.project
    ));
    Ok(TaskContext {
        links: link::for_item(conn, item_id)?,
        task,
        handoff,
        log,
        log_total,
        questions,
        more_questions,
        subtasks,
        pages,
        more_pages,
        hints,
    })
}

/// Where to answer these questions: the task's id when they are all on it, else `KEY-N`
/// (an answer goes on the question's own task).
fn answer_on<'a>(questions: &[Comment], task: &'a str) -> &'a str {
    if questions.iter().all(|q| q.item == task) {
        task
    } else {
        "KEY-N"
    }
}

/// The first `max` of a list, and how many were left out.
fn capped<T>(mut v: Vec<T>, max: usize) -> (Vec<T>, i64) {
    let more = v.len().saturating_sub(max) as i64;
    v.truncate(max);
    (v, more)
}

fn for_project(conn: &Connection, project_id: i64) -> Result<ProjectContext> {
    let project = project::get(conn, project_id)?;
    let (pages, more) = page::for_project(conn, project_id)?;
    let (pages, cut) = capped(pages, PAGES_MAX);
    let mut vals: Vec<SqlValue> = vec![project_id.into()];
    let closed = item::closed_marks(&mut vals);
    let open = item::query(
        conn,
        &format!("i.project_id = ?1 AND s.category NOT IN ({closed})"),
        &vals,
    )?;
    let ids: Vec<i64> = open.iter().map(|i| i.row_id).collect();
    let (questions, more_questions) =
        capped(comment::open_questions(conn, &ids)?, PROJECT_QUESTIONS_MAX);
    let active = item::query_sorted(
        conn,
        "i.project_id = ?1 AND s.category = 'active'",
        &[project_id.into()],
        "i.updated_at DESC, i.seq",
    )?;
    let (active, more_tasks) = capped(active, TASKS_MAX);
    let tasks = active
        .into_iter()
        .map(|i| {
            Ok(InFlight {
                handoff: comment::latest(conn, i.row_id, CommentKind::Handoff)?.map(Entry::cut),
                claimed_by: i.claim.map(|c| c.by),
                id: i.id,
                status: i.status,
                title: i.title,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    let key = &project.key;
    let mut hints = vec![
        format!("pick up work: apm next -p {key} --context (claims the best ready task)"),
        format!("one task in full: apm context {key}-N"),
    ];
    if !questions.is_empty() {
        hints.push(format!(
            "open questions: answer with apm comment {key}-N --answer ID \"...\""
        ));
    }
    if let Some(p) = pages.iter().find(|p| p.stale) {
        hints.push(format!(
            "stale pages: re-read, then apm page verify {} (still true) or apm page edit {}",
            p.id, p.id
        ));
    }
    if pages.is_empty() {
        hints.push(format!(
            "no pinned pages: pin what every task needs (conventions, architecture): \
             apm page add {key}:<slug> \"Title\" -s \"gist\" -b - --pin"
        ));
    }
    Ok(ProjectContext {
        project,
        pages,
        more_pages: more + cut,
        questions,
        more_questions,
        tasks,
        more_tasks,
        hints,
    })
}

/// ` [handoff]`, ` [question]`, ` [answer #41]`; nothing for a note. (The entry's own id
/// leads the line.)
fn mark(c: &Comment) -> String {
    match (c.kind, c.answers) {
        (CommentKind::Note, _) => String::new(),
        (CommentKind::Answer, Some(q)) => format!(" [answer #{q}]"),
        (kind, _) => format!(" [{}]", kind.as_str()),
    }
}

/// `  #ID TIME AUTHOR[ mark]: first line`, continuation lines indented under it.
fn print_entry(c: &Comment, on: Option<&str>) {
    let mut lines = c.body.lines();
    let on = on.map(|t| format!(" on {t}")).unwrap_or_default();
    println!(
        "  #{} {} {}{}{on}: {}",
        c.id,
        c.created_at,
        c.author,
        mark(c),
        lines.next().unwrap_or_default()
    );
    item::print_text("    ", lines);
}

fn print_pages(pages: &[PageSummary], more: i64, key: &str) {
    if pages.is_empty() && more == 0 {
        return;
    }
    println!("\npages (full text: apm page show ID):");
    let rows: Vec<Vec<String>> = pages
        .iter()
        .map(|p| {
            let mut flags = Vec::new();
            if p.linked == Some(true) {
                flags.push("linked".to_string());
            }
            if p.pinned {
                flags.push("pinned".to_string());
            }
            flags.push(format!("v{}", p.version));
            if p.stale {
                flags.push("STALE".to_string());
            } else if let Some(v) = &p.verified_at {
                flags.push(format!("verified {}", v.get(..10).unwrap_or(v)));
            }
            let gist = if p.summary.is_empty() {
                p.title.clone()
            } else {
                format!("{}: {}", p.title, p.summary)
            };
            vec![p.id.clone(), format!("{gist}  ({})", flags.join(", "))]
        })
        .collect();
    for line in output::aligned(&rows) {
        println!("  {line}");
    }
    if more > 0 {
        println!(
            "  + {more} more pages in {key}: apm page list -p {key}, apm search WORDS -p {key}"
        );
    }
}

fn print_hints(hints: &[String]) {
    println!("\nhints:");
    for h in hints {
        println!("  {h}");
    }
}

/// The task context as text (also `next --context`).
pub fn print_task(c: &TaskContext) {
    let t = &c.task;
    println!("{}  {}", t.id, t.title);
    let mut fields = vec![
        format!("status: {}", t.status),
        format!("priority: {}", t.priority),
    ];
    for (name, value) in [
        ("assignee", &t.assignee),
        ("due", &t.due),
        ("milestone", &t.milestone),
        ("parent", &t.parent),
    ] {
        if let Some(v) = value {
            fields.push(format!("{name}: {v}"));
        }
    }
    println!("{}", fields.join("  "));
    if let Some(claim) = &t.claim {
        println!("claimed: {} until {}", claim.by, claim.expires);
    }
    if !t.body.is_empty() {
        println!();
        item::print_text("  ", t.body.lines());
    }
    if let Some(h) = &c.handoff {
        println!("\nhandoff ({} {}):", h.created_at, h.author);
        item::print_text("  ", h.body.lines());
    }
    if !c.log.is_empty() {
        println!(
            "\nlog ({} of {}, oldest first; full: apm show {}):",
            c.log.len(),
            c.log_total,
            t.id
        );
        for e in &c.log {
            print_entry(&e.comment, None);
        }
    }
    if !c.questions.is_empty() {
        println!(
            "\nopen questions (answer: apm comment {} --answer ID TEXT):",
            answer_on(&c.questions, &t.id)
        );
        for q in &c.questions {
            print_entry(q, (q.item != t.id).then_some(q.item.as_str()));
        }
        if c.more_questions > 0 {
            println!("  + {} more", c.more_questions);
        }
    }
    if !c.subtasks.is_empty() {
        println!("\nsubtasks:");
        let rows: Vec<Vec<String>> = c
            .subtasks
            .iter()
            .map(|s| vec![s.id.clone(), s.status.clone(), s.title.clone()])
            .collect();
        for line in output::aligned(&rows) {
            println!("  {line}");
        }
    }
    for (label, linked) in c.links.groups() {
        if !linked.is_empty() {
            println!("{label}:");
            let rows: Vec<Vec<String>> = linked
                .iter()
                .map(|l| vec![l.id.clone(), l.status.clone(), l.title.clone()])
                .collect();
            for line in output::aligned(&rows) {
                println!("  {line}");
            }
        }
    }
    print_pages(&c.pages, c.more_pages, &t.project);
    print_hints(&c.hints);
}

fn print_project(c: &ProjectContext) {
    let p = &c.project;
    println!("{}  {}", p.key, p.name);
    if !p.description.is_empty() {
        println!("description: {}", p.description);
    }
    print_pages(&c.pages, c.more_pages, &p.key);
    if !c.tasks.is_empty() {
        println!("\nin flight (latest handoff; full: apm context ID):");
        for t in &c.tasks {
            let by = t
                .claimed_by
                .as_ref()
                .map(|b| format!("  claimed by {b}"))
                .unwrap_or_default();
            println!("  {}  {}  {}{by}", t.id, t.status, t.title);
            match &t.handoff {
                Some(h) => {
                    println!(
                        "    handoff ({} {}):",
                        h.comment.created_at, h.comment.author
                    );
                    item::print_text("      ", h.comment.body.lines());
                }
                None => println!("    no handoff yet"),
            }
        }
        if c.more_tasks > 0 {
            println!(
                "  + {} more in flight: apm list -p {} --status ...",
                c.more_tasks, p.key
            );
        }
    }
    if !c.questions.is_empty() {
        println!("\nopen questions (answer: apm comment KEY-N --answer ID TEXT):");
        for q in &c.questions {
            print_entry(q, Some(&q.item));
        }
        if c.more_questions > 0 {
            println!("  + {} more", c.more_questions);
        }
    }
    print_hints(&c.hints);
}
