//! `apm context`: what earlier agents learned, in one call and a few KB. For a task: the task,
//! its latest handoff, recent log, open questions, subtasks, links and pages. For a project:
//! pinned pages, open questions and the latest handoff of every in-flight or paused task.
//! Every list is capped (the rest counted) and every log text cut, so the output is bounded.

use clap::Args;
use rusqlite::{Connection, params};
use serde::Serialize;

use super::comment::{self, Comment};
use super::page::{self, PageSummary};
use super::{Ctx, claim, item, link, project};
use crate::db;
use crate::error::{Result, err};
use crate::model::{Category, CommentKind};
use crate::output;
use clap::ValueEnum;

/// Log entries shown by default (`--log`).
pub const DEFAULT_LOG: u32 = 5;
/// Text cuts, in characters: a log entry (and a handoff in project context), an open
/// question, a task's latest handoff.
const LOG_CUT: usize = 280;
const QUESTION_CUT: usize = 400;
const HANDOFF_CUT: usize = 2000;
/// List caps; what is left out is counted in `more_*`.
const PAGES_MAX: usize = 8;
const QUESTIONS_MAX: usize = 5;
const SUBTASKS_MAX: usize = 8;
const LINKS_MAX: usize = 8;
const PROJECT_QUESTIONS_MAX: usize = 6;
const ACTIVE_MAX: usize = 6;
const PAUSED_MAX: usize = 4;

#[derive(Args, Debug)]
pub struct ContextArgs {
    /// Task id, e.g. APM-12; without one, the task you hold a live claim on
    #[arg(conflicts_with = "project")]
    pub key: Option<String>,
    /// A project's context instead, for a fresh session: pinned pages, open questions and the
    /// latest handoff of every in-flight or paused task
    #[arg(short, long, value_name = "KEY")]
    pub project: Option<String>,
    /// How many recent log entries to show besides the latest handoff, each cut at 280
    /// characters (0-50)
    #[arg(long, value_name = "N", default_value_t = DEFAULT_LOG,
          value_parser = clap::value_parser!(u32).range(0..=50))]
    pub log: u32,
}

/// A log entry as context shows it: the comment, its body cut to a maximum length.
#[derive(Serialize, Debug)]
pub struct Entry {
    #[serde(flatten)]
    comment: Comment,
    /// The body was cut (full text: `apm show KEY-N`)
    truncated: bool,
}

impl Entry {
    fn cut(mut comment: Comment, max: usize) -> Entry {
        let truncated = comment.body.chars().count() > max;
        if truncated {
            let cut: String = comment.body.chars().take(max).collect();
            comment.body = format!("{}...", cut.trim_end());
        }
        Entry { comment, truncated }
    }
}

/// A task as one line.
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
    /// The newest handoff (cut at 2000 characters)
    handoff: Option<Entry>,
    /// The newest `--log N` other entries (not the handoff or a listed open question),
    /// oldest first, each cut at 280 characters
    log: Vec<Entry>,
    /// How many such entries the task has in all
    log_total: i64,
    /// Unanswered questions on the task and its subtasks, oldest first, cut at 400 characters
    questions: Vec<Entry>,
    more_questions: i64,
    /// Direct subtasks, by number
    subtasks: Vec<Line>,
    more_subtasks: i64,
    links: link::Links,
    more_links: i64,
    pages: Vec<PageSummary>,
    more_pages: i64,
    hints: Vec<String>,
}

/// A task in project context: in flight (active category) or paused (not active, but its
/// newest log entry is a handoff).
#[derive(Serialize, Debug)]
struct Story {
    id: String,
    status: String,
    title: String,
    claimed_by: Option<String>,
    paused: bool,
    /// Its newest handoff, cut at 280 characters
    handoff: Option<Entry>,
}

/// `apm context -p KEY`.
#[derive(Serialize, Debug)]
struct ProjectContext {
    project: project::Project,
    pages: Vec<PageSummary>,
    more_pages: i64,
    /// Unanswered questions on the project's open tasks, oldest first, cut at 400 characters
    questions: Vec<Entry>,
    more_questions: i64,
    /// In-flight tasks, most recently updated first, then paused ones
    tasks: Vec<Story>,
    more_active: i64,
    more_paused: i64,
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
    match claim::live_claims(conn, actor)?.as_slice() {
        [(id, _)] => Ok(*id),
        [] => Err(err(format!(
            "{actor} holds no claim: pass a task (apm context KEY-N) or a project \
             (apm context -p KEY)"
        ))),
        many => {
            let ids: Vec<&str> = many.iter().map(|(_, k)| k.as_str()).collect();
            Err(err(format!(
                "{actor} holds {} claims ({}): pass one (apm context KEY-N)",
                many.len(),
                ids.join(", ")
            )))
        }
    }
}

/// SQL: the task ?1 and all its subtasks, at any depth.
const TREE: &str = "WITH RECURSIVE down(id) AS (
                        SELECT ?1 UNION SELECT i.id FROM items i JOIN down ON i.parent_id = down.id
                    ) SELECT id FROM down";

/// The task context of a task (also `next --context`).
pub fn for_task(conn: &Connection, item_id: i64, log: u32) -> Result<TaskContext> {
    let task = item::get(conn, item_id)?;
    let handoff = comment::latest(conn, item_id, CommentKind::Handoff)?;
    let (questions, total) = comment::open_questions(conn, TREE, &[&item_id], QUESTIONS_MAX)?;
    let more_questions = total - questions.len() as i64;
    // The log: the task's other entries (not the handoff or open questions shown above).
    let shown: Vec<i64> = handoff.iter().chain(&questions).map(|c| c.id).collect();
    let (recent, log_total) = comment::recent(conn, item_id, &shown, log)?;
    let log = recent.into_iter().map(|c| Entry::cut(c, LOG_CUT)).collect();
    let questions: Vec<Entry> = questions
        .into_iter()
        .map(|c| Entry::cut(c, QUESTION_CUT))
        .collect();
    let handoff = handoff.map(|c| Entry::cut(c, HANDOFF_CUT));

    let mut stmt = conn.prepare(&format!(
        "SELECT p.key || '-' || i.seq, s.name, i.title FROM items i
         JOIN statuses s ON s.id = i.status_id JOIN projects p ON p.id = i.project_id
         WHERE i.parent_id = ?1 ORDER BY i.seq LIMIT {SUBTASKS_MAX}"
    ))?;
    let subtasks: Vec<Line> = stmt
        .query_map(params![item_id], |r| {
            Ok(Line {
                id: r.get(0)?,
                status: r.get(1)?,
                title: r.get(2)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    let all_subtasks: i64 = conn.query_row(
        "SELECT count(*) FROM items WHERE parent_id = ?1",
        params![item_id],
        |r| r.get(0),
    )?;
    let (links, more_links) = cap_links(link::for_item(conn, item_id)?);
    let (pages, more) = page::for_context(conn, item_id)?;
    let (pages, cut) = capped(pages, PAGES_MAX);

    let id = &task.id;
    let mut hints = Vec::new();
    if !comment::any(conn, item_id)? {
        hints.push(format!(
            "nobody logged on this task yet: apm comment {id} \"...\" as you learn things"
        ));
    }
    if !questions.is_empty() {
        hints.push(format!(
            "open questions: answer with apm comment {} --answer ID \"...\"",
            answer_on(&questions)
        ));
    }
    hints.extend(stale_hint(&pages));
    hints.push(format!(
        "before you stop: apm comment {id} --kind handoff - (state, next, decisions + why, where)"
    ));
    hints.push(format!(
        "learned something that outlives this task? apm page add {}:<slug> \"Title\" -s \"gist\" \
         -b - --link {id}",
        task.project
    ));
    Ok(TaskContext {
        more_subtasks: all_subtasks - subtasks.len() as i64,
        task,
        handoff,
        log,
        log_total,
        questions,
        more_questions,
        subtasks,
        links,
        more_links,
        pages,
        more_pages: more + cut,
        hints,
    })
}

/// The first `max` of a list, and how many were left out.
fn capped<T>(mut v: Vec<T>, max: usize) -> (Vec<T>, i64) {
    let more = v.len().saturating_sub(max) as i64;
    v.truncate(max);
    (v, more)
}

/// At most LINKS_MAX links over all groups (in group order), and how many were left out.
fn cap_links(mut links: link::Links) -> (link::Links, i64) {
    let mut room = LINKS_MAX;
    let mut more = 0;
    for group in [
        &mut links.blocked_by,
        &mut links.blocks,
        &mut links.relates,
        &mut links.duplicates,
        &mut links.duplicated_by,
    ] {
        let keep = group.len().min(room);
        more += (group.len() - keep) as i64;
        group.truncate(keep);
        room -= keep;
    }
    (links, more)
}

/// Where to answer the listed questions: their task when they are all on one, else `KEY-N`
/// (an answer goes on the question's own task).
fn answer_on(questions: &[Entry]) -> &str {
    match questions.split_first() {
        Some((first, rest)) if rest.iter().all(|q| q.comment.item == first.comment.item) => {
            &first.comment.item
        }
        _ => "KEY-N",
    }
}

/// The hint for the first stale page listed, if any.
fn stale_hint(pages: &[PageSummary]) -> Option<String> {
    let p = pages.iter().find(|p| p.stale)?;
    Some(format!(
        "stale pages: re-read, then apm page verify {} (still true) or apm page edit {}",
        p.id, p.id
    ))
}

/// The closed categories (done, cancelled) as an SQL list.
fn closed_categories() -> String {
    let closed: Vec<String> = Category::value_variants()
        .iter()
        .filter(|c| c.is_closed())
        .map(|c| format!("'{}'", c.as_str()))
        .collect();
    closed.join(", ")
}

/// The project's statuses of a category, by position, as one `--status` value.
fn status_list(conn: &Connection, project_id: i64, categories: &str) -> Result<String> {
    let mut stmt = conn.prepare(&format!(
        "SELECT name FROM statuses WHERE project_id = ?1 AND category IN ({categories})
         ORDER BY position"
    ))?;
    let names = stmt
        .query_map(params![project_id], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?
        .join(",");
    Ok(if names.contains(' ') {
        format!("\"{names}\"")
    } else {
        names
    })
}

/// Project tasks matching a condition over `items i` / `statuses s`, by an order: the first
/// `max` as stories, and how many match in all.
fn stories(
    conn: &Connection,
    project_id: i64,
    cond: &str,
    order: &str,
    max: usize,
    paused: bool,
) -> Result<(Vec<Story>, i64)> {
    let filter = format!("i.project_id = ?1 AND {cond}");
    let mut stmt = conn.prepare(&format!(
        "SELECT i.id FROM items i JOIN statuses s ON s.id = i.status_id
         WHERE {filter} ORDER BY {order} LIMIT {max}"
    ))?;
    let ids = stmt
        .query_map(params![project_id], |r| r.get::<_, i64>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let total: i64 = conn.query_row(
        &format!(
            "SELECT count(*) FROM items i JOIN statuses s ON s.id = i.status_id WHERE {filter}"
        ),
        params![project_id],
        |r| r.get(0),
    )?;
    let stories = ids
        .into_iter()
        .map(|id| {
            let i = item::get(conn, id)?;
            Ok(Story {
                handoff: comment::latest(conn, id, CommentKind::Handoff)?
                    .map(|c| Entry::cut(c, LOG_CUT)),
                claimed_by: i.claim.map(|c| c.by),
                id: i.id,
                status: i.status,
                title: i.title,
                paused,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok((stories, total))
}

fn for_project(conn: &Connection, project_id: i64) -> Result<ProjectContext> {
    let project = project::get(conn, project_id)?;
    let (pages, more) = page::for_project(conn, project_id)?;
    let (pages, cut) = capped(pages, PAGES_MAX);
    let (questions, total) = comment::open_questions(
        conn,
        &format!(
            "SELECT i.id FROM items i JOIN statuses s ON s.id = i.status_id
             WHERE i.project_id = ?1 AND s.category NOT IN ({})",
            closed_categories()
        ),
        &[&project_id],
        PROJECT_QUESTIONS_MAX,
    )?;
    let more_questions = total - questions.len() as i64;
    let questions: Vec<Entry> = questions
        .into_iter()
        .map(|c| Entry::cut(c, QUESTION_CUT))
        .collect();
    let (mut tasks, active) = stories(
        conn,
        project_id,
        "s.category = 'active'",
        "i.updated_at DESC, i.seq",
        ACTIVE_MAX,
        false,
    )?;
    let more_active = active - tasks.len() as i64;
    // Paused: open, not active, and the newest log entry is a handoff.
    let (paused, paused_total) = stories(
        conn,
        project_id,
        "s.category = 'todo' AND (SELECT c.kind FROM comments c WHERE c.item_id = i.id
                                  ORDER BY c.id DESC LIMIT 1) = 'handoff'",
        "i.updated_at DESC, i.seq",
        PAUSED_MAX,
        true,
    )?;
    let more_paused = paused_total - paused.len() as i64;
    tasks.extend(paused);

    let key = &project.key;
    let mut hints = vec![
        format!("pick up work: apm next -p {key} --context (claims the best ready task)"),
        format!("one task in full: apm context {key}-N"),
    ];
    if more_active > 0 {
        hints.push(format!(
            "all in-flight tasks: apm list -p {key} --status {}",
            status_list(conn, project_id, "'active'")?
        ));
    }
    if more_paused > 0 {
        hints.push(format!(
            "all waiting tasks: apm list -p {key} --status {}",
            status_list(conn, project_id, "'todo'")?
        ));
    }
    if !questions.is_empty() {
        hints.push(format!(
            "open questions: answer with apm comment {} --answer ID \"...\"",
            answer_on(&questions)
        ));
    }
    hints.extend(stale_hint(&pages));
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
        more_active,
        more_paused,
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

/// `  #ID TIME AUTHOR[ mark][ on KEY-N]: first line`, continuation lines indented under it.
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

/// The questions section: a header naming where to answer, each question, what was cut.
fn print_questions(questions: &[Entry], more: i64, task: Option<&str>) {
    if questions.is_empty() {
        return;
    }
    println!(
        "\nopen questions (answer: apm comment {} --answer ID TEXT):",
        answer_on(questions)
    );
    for q in questions {
        let c = &q.comment;
        print_entry(
            c,
            (Some(c.item.as_str()) != task).then_some(c.item.as_str()),
        );
        if q.truncated {
            println!("    (cut; full: apm show {})", c.item);
        }
    }
    if more > 0 {
        println!("  + {more} more");
    }
}

fn print_lines(rows: Vec<Vec<String>>) {
    for line in output::aligned(&rows) {
        println!("  {line}");
    }
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
    print_lines(rows);
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
        let cut = if h.truncated {
            format!("; cut, full: apm show {}", t.id)
        } else {
            String::new()
        };
        println!(
            "\nhandoff ({} {}{cut}):",
            h.comment.created_at, h.comment.author
        );
        item::print_text("  ", h.comment.body.lines());
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
    print_questions(&c.questions, c.more_questions, Some(&t.id));
    if !c.subtasks.is_empty() {
        println!("\nsubtasks:");
        print_lines(
            c.subtasks
                .iter()
                .map(|s| vec![s.id.clone(), s.status.clone(), s.title.clone()])
                .collect(),
        );
        if c.more_subtasks > 0 {
            println!("  + {} more: apm show {} --tree", c.more_subtasks, t.id);
        }
    }
    for (label, linked) in c.links.groups() {
        if !linked.is_empty() {
            println!("\n{label}:");
            print_lines(
                linked
                    .iter()
                    .map(|l| vec![l.id.clone(), l.status.clone(), l.title.clone()])
                    .collect(),
            );
        }
    }
    if c.more_links > 0 {
        println!("  + {} more links: apm show {}", c.more_links, t.id);
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
    let (active, paused): (Vec<&Story>, Vec<&Story>) = c.tasks.iter().partition(|t| !t.paused);
    for (label, list, more) in [
        ("in flight", active, c.more_active),
        ("paused with a handoff", paused, c.more_paused),
    ] {
        if list.is_empty() {
            continue;
        }
        println!("\n{label} (latest handoff; full: apm context ID):");
        for t in list {
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
        if more > 0 {
            println!("  + {more} more (see hints)");
        }
    }
    print_questions(&c.questions, c.more_questions, None);
    print_hints(&c.hints);
}
