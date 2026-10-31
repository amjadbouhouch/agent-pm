//! Pages: knowledge that outlives a task (decisions and why, architecture, conventions,
//! gotchas). A page belongs to one project, is `KEY:slug`, keeps every content version, and can
//! be linked to any number of tasks. `apm search` finds pages and task log entries.

use std::collections::BTreeSet;
use std::path::PathBuf;

use clap::{Args, Subcommand};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use super::{Ctx, item};
use crate::db::{self, EventKind};
use crate::error::{AppError, Result, err};
use crate::{input, output};

/// A page not edited or verified for this many days is stale: re-read it, then
/// `apm page verify` (still true) or `apm page edit` (fix it).
pub const STALE_DAYS: i64 = 30;

/// A summary longer than this is rejected: it is what `apm context` shows for every page.
const SUMMARY_MAX: usize = 200;

/// Without a summary, context shows the body's first line, cut at this many characters.
const FALLBACK_MAX: usize = 120;

#[derive(Subcommand, Debug)]
pub enum PageCmd {
    /// Create a page: knowledge that stays true after a task closes
    Add(AddArgs),
    /// Show a page in full (--version N: an older version)
    #[command(disable_version_flag = true)]
    Show {
        /// Page id KEY:slug, e.g. APM:storage
        page: String,
        /// Show this version instead of the current one
        #[arg(long, value_name = "N", value_parser = clap::value_parser!(i64).range(1..))]
        version: Option<i64>,
    },
    /// Change a page; a content change (title, summary, body) makes a new version
    Edit(EditArgs),
    /// Mark a page as re-read and still true (a page unchecked for 30 days is stale)
    Verify {
        /// Page id KEY:slug, e.g. APM:storage
        page: String,
    },
    /// List pages (all projects unless -p)
    List(ListArgs),
    /// List a page's versions, oldest first
    History {
        /// Page id KEY:slug, e.g. APM:storage
        page: String,
    },
    /// Delete a page with its versions and task links
    Rm {
        /// Page id KEY:slug, e.g. APM:storage
        page: String,
    },
}

#[derive(Args, Debug)]
pub struct AddArgs {
    /// Page id KEY:slug: the project's KEY and a slug of a-z 0-9 - (e.g. APM:storage)
    pub page: String,
    pub title: String,
    /// One line, at most 200 characters: the gist, shown in `apm context` (detail goes in
    /// the body)
    #[arg(short, long)]
    pub summary: Option<String>,
    /// The page text, Markdown by convention; `-` reads stdin
    #[arg(short, long)]
    pub body: Option<String>,
    /// Read the body from a file
    #[arg(long, value_name = "PATH", conflicts_with = "body")]
    pub body_file: Option<PathBuf>,
    /// Link the page to task KEY-N (repeatable): it shows in that task's context
    #[arg(long = "link", value_name = "KEY-N")]
    pub links: Vec<String>,
    /// Pin the page: it shows in the context of every task of its project (conventions,
    /// architecture)
    #[arg(long)]
    pub pin: bool,
}

#[derive(Args, Debug)]
pub struct EditArgs {
    /// Page id KEY:slug, e.g. APM:storage
    pub page: String,
    #[arg(long)]
    pub title: Option<String>,
    /// New summary: one line, at most 200 characters ("" clears it)
    #[arg(short, long)]
    pub summary: Option<String>,
    /// New body, replacing the old one; `-` reads stdin
    #[arg(short, long, conflicts_with = "append")]
    pub body: Option<String>,
    /// Read the new body from a file
    #[arg(long, value_name = "PATH", conflicts_with_all = ["body", "append"])]
    pub body_file: Option<PathBuf>,
    /// Add a section at the end of the body (after a blank line) without rewriting it;
    /// `-` reads stdin. Safe when several agents write to one page
    #[arg(long, value_name = "TEXT|-")]
    pub append: Option<String>,
    /// Refuse the edit unless the page is still at version N (the one you read), so you never
    /// overwrite someone else's change
    #[arg(long, value_name = "N")]
    pub expect_version: Option<i64>,
    /// New slug (the page id becomes KEY:NEW; task links stay)
    #[arg(long, value_name = "NEW")]
    pub rename: Option<String>,
    /// Show the page in the context of every task of its project
    #[arg(long, conflicts_with = "unpin")]
    pub pin: bool,
    /// Stop showing the page in every task's context
    #[arg(long)]
    pub unpin: bool,
    /// Link the page to task KEY-N (repeatable)
    #[arg(long = "link", value_name = "KEY-N")]
    pub links: Vec<String>,
    /// Unlink the page from task KEY-N (repeatable)
    #[arg(long = "unlink", value_name = "KEY-N")]
    pub unlinks: Vec<String>,
}

#[derive(Args, Debug)]
pub struct ListArgs {
    #[arg(short, long)]
    pub project: Option<String>,
    /// Only pages linked to task KEY-N
    #[arg(long, value_name = "KEY-N")]
    pub task: Option<String>,
    /// Only pinned pages
    #[arg(long)]
    pub pinned: bool,
    /// Only stale pages (not edited or verified for 30 days)
    #[arg(long)]
    pub stale: bool,
}

#[derive(Args, Debug)]
pub struct SearchArgs {
    /// Words to find (all must match, in any order; case and accents are ignored)
    #[arg(allow_hyphen_values = true)]
    pub query: String,
    #[arg(short, long)]
    pub project: Option<String>,
    /// At most N results
    #[arg(long, value_name = "N", default_value_t = 10,
          value_parser = clap::value_parser!(u32).range(1..=100))]
    pub limit: u32,
}

#[derive(Serialize, Debug)]
pub struct Page {
    /// KEY:slug, with the project's current key
    pub id: String,
    pub project: String,
    pub slug: String,
    pub title: String,
    pub summary: String,
    pub body: String,
    pub pinned: bool,
    pub version: i64,
    /// Linked tasks, KEY-N, by key and number
    pub tasks: Vec<String>,
    pub created_by: String,
    pub created_at: String,
    /// Who made the last content change (the current version) and when
    pub updated_by: String,
    pub updated_at: String,
    pub verified_by: Option<String>,
    pub verified_at: Option<String>,
    /// Neither edited nor verified for STALE_DAYS days
    pub stale: bool,
}

/// A page as one line: what `page list` and `apm context` show.
#[derive(Serialize, Debug, Clone)]
pub struct PageSummary {
    pub id: String,
    pub title: String,
    /// The summary, or without one the body's first line (cut at 120 characters + `...`)
    pub summary: String,
    pub pinned: bool,
    /// Linked to the task (or one of its parents); only in `apm context`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub linked: Option<bool>,
    pub version: i64,
    pub updated_at: String,
    pub verified_at: Option<String>,
    pub stale: bool,
}

#[derive(Serialize, Debug)]
struct Version {
    version: i64,
    title: String,
    summary: String,
    author: String,
    at: String,
}

/// A search result, best first.
#[derive(Serialize, Debug)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum Hit {
    Page {
        id: String,
        title: String,
        snippet: String,
    },
    Log {
        task: String,
        comment: i64,
        /// The log entry's kind: note, handoff, question or answer
        entry: String,
        author: String,
        created_at: String,
        snippet: String,
    },
}

pub fn run(ctx: &Ctx, cmd: PageCmd) -> Result<()> {
    match cmd {
        PageCmd::Add(a) => add(ctx, a),
        PageCmd::Show { page, version } => show(ctx, &page, version),
        PageCmd::Edit(a) => edit(ctx, a),
        PageCmd::Verify { page } => verify(ctx, &page),
        PageCmd::List(a) => list(ctx, a),
        PageCmd::History { page } => history(ctx, &page),
        PageCmd::Rm { page } => rm(ctx, &page),
    }
}

/// SQL for "last checked" (edited or verified, whichever is later) older than STALE_DAYS.
/// `p` is the pages alias.
fn stale_sql() -> String {
    format!(
        "(max(p.updated_at, coalesce(p.verified_at, '')) < \
         strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-{STALE_DAYS} days'))"
    )
}

/// The row id of a page by its `KEY:slug` id (the key may be an old one: aliases resolve).
fn find(conn: &Connection, reference: &str) -> Result<i64> {
    let (key, slug) = db::parse_page_ref(reference)?;
    let project_id = db::project_id(conn, &key)?;
    conn.query_row(
        "SELECT id FROM pages WHERE project_id = ?1 AND slug = ?2",
        params![project_id, slug],
        |r| r.get(0),
    )
    .optional()?
    .ok_or_else(|| err(format!("no such page: {key}:{slug}")))
}

/// A page by row id.
pub fn get(conn: &Connection, id: i64) -> Result<Page> {
    let mut page = conn.query_row(
        &format!(
            "SELECT pr.key || ':' || p.slug, pr.key, p.slug, p.title, p.summary, p.body, p.pinned,
                    p.version, p.created_by, p.created_at, p.updated_by, p.updated_at,
                    p.verified_by, p.verified_at, {}
             FROM pages p JOIN projects pr ON pr.id = p.project_id WHERE p.id = ?1",
            stale_sql()
        ),
        params![id],
        |r| {
            Ok(Page {
                id: r.get(0)?,
                project: r.get(1)?,
                slug: r.get(2)?,
                title: r.get(3)?,
                summary: r.get(4)?,
                body: r.get(5)?,
                pinned: r.get(6)?,
                version: r.get(7)?,
                tasks: Vec::new(),
                created_by: r.get(8)?,
                created_at: r.get(9)?,
                updated_by: r.get(10)?,
                updated_at: r.get(11)?,
                verified_by: r.get(12)?,
                verified_at: r.get(13)?,
                stale: r.get(14)?,
            })
        },
    )?;
    let mut stmt = conn.prepare(
        "SELECT pr.key || '-' || i.seq FROM page_links l
         JOIN items i ON i.id = l.item_id JOIN projects pr ON pr.id = i.project_id
         WHERE l.page_id = ?1 ORDER BY pr.key, i.seq",
    )?;
    page.tasks = stmt
        .query_map(params![id], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    Ok(page)
}

/// A title is required and one line.
fn check_title(title: &str) -> Result<()> {
    if title.trim().is_empty() {
        return Err(err("title cannot be empty"));
    }
    if title.contains(['\n', '\r']) {
        return Err(err("title must be one line"));
    }
    Ok(())
}

/// A summary is one line of at most SUMMARY_MAX characters; detail belongs in the body.
fn check_summary(summary: &str) -> Result<()> {
    if summary.contains(['\n', '\r']) {
        return Err(err("summary must be one line: put detail in the body"));
    }
    let n = summary.chars().count();
    if n > SUMMARY_MAX {
        return Err(err(format!(
            "summary is {n} characters, at most {SUMMARY_MAX}: put detail in the body"
        )));
    }
    Ok(())
}

fn already_exists(id: &str) -> AppError {
    err(format!("page already exists: {id}"))
}

/// Link a page to a task: a `paged` event on the task and a bump of its `updated_at`.
/// Linking twice is a no-op; returns whether a link was added.
fn link(conn: &Connection, actor: &str, page_id: i64, page: &str, item_id: i64) -> Result<bool> {
    let added = conn.execute(
        "INSERT OR IGNORE INTO page_links (page_id, item_id) VALUES (?1, ?2)",
        params![page_id, item_id],
    )? > 0;
    if added {
        touch_task(conn, actor, item_id, EventKind::Paged, page)?;
    }
    Ok(added)
}

/// Unlink a page from a task (an absent link is a no-op); returns whether one was removed.
fn unlink(conn: &Connection, actor: &str, page_id: i64, page: &str, item_id: i64) -> Result<bool> {
    let removed = conn.execute(
        "DELETE FROM page_links WHERE page_id = ?1 AND item_id = ?2",
        params![page_id, item_id],
    )? > 0;
    if removed {
        touch_task(conn, actor, item_id, EventKind::Unpaged, page)?;
    }
    Ok(removed)
}

/// Record `paged` (new = page id) or `unpaged` (old = page id) on a task and bump it.
fn touch_task(
    conn: &Connection,
    actor: &str,
    item_id: i64,
    kind: EventKind,
    page: &str,
) -> Result<()> {
    let (old, new) = match kind {
        EventKind::Paged => (None, Some(page)),
        _ => (Some(page), None),
    };
    db::record_event(conn, item_id, actor, kind, None, old, new)?;
    conn.execute(
        &format!(
            "UPDATE items SET updated_at = {} WHERE id = ?1",
            db::NOW_SQL
        ),
        params![item_id],
    )?;
    Ok(())
}

/// Resolve task references to row ids, once each, in the order given.
fn tasks(conn: &Connection, refs: &[String]) -> Result<Vec<i64>> {
    let mut ids = Vec::new();
    for r in refs {
        let id = db::item(conn, r)?.id;
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    Ok(ids)
}

fn add(ctx: &Ctx, args: AddArgs) -> Result<()> {
    let (key, slug) = db::parse_page_ref(&args.page)?;
    check_title(&args.title)?;
    let summary = args.summary.unwrap_or_default();
    check_summary(&summary)?;
    let body = input::read_text(args.body, args.body_file.as_deref(), &input::PAGE_BODY)?
        .unwrap_or_default();
    let tx = db::write_tx(&ctx.conn)?;
    let project_id = db::project_id(&tx, &key)?;
    let current: String = tx.query_row(
        "SELECT key FROM projects WHERE id = ?1",
        params![project_id],
        |r| r.get(0),
    )?;
    let id_text = format!("{current}:{slug}");
    db::unique(
        tx.execute(
            "INSERT INTO pages (project_id, slug, title, summary, body, pinned, created_by,
                                updated_by)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
            params![
                project_id, slug, args.title, summary, body, args.pin, ctx.actor
            ],
        ),
        || already_exists(&id_text),
    )?;
    let id = tx.last_insert_rowid();
    save_version(&tx, id)?;
    for item_id in tasks(&tx, &args.links)? {
        link(&tx, &ctx.actor, id, &id_text, item_id)?;
    }
    tx.commit()?;
    let page = get(&ctx.conn, id)?;
    output::written(ctx.json, &page, &format!("created {}", page.id))
}

/// Copy the page's current content into `page_versions` as its current version.
fn save_version(conn: &Connection, id: i64) -> Result<()> {
    conn.execute(
        "INSERT INTO page_versions (page_id, version, title, summary, body, author, at)
         SELECT id, version, title, summary, body, updated_by, updated_at FROM pages
         WHERE id = ?1",
        params![id],
    )?;
    Ok(())
}

fn show(ctx: &Ctx, reference: &str, version: Option<i64>) -> Result<()> {
    let id = find(&ctx.conn, reference)?;
    let mut page = get(&ctx.conn, id)?;
    // An old version: its content, author and time; the rest is the page as it is now.
    let (mut by, mut at) = (page.updated_by.clone(), page.updated_at.clone());
    if let Some(n) = version.filter(|&n| n != page.version) {
        let v = ctx
            .conn
            .query_row(
                "SELECT title, summary, body, author, at FROM page_versions
                 WHERE page_id = ?1 AND version = ?2",
                params![id, n],
                |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, String>(4)?,
                    ))
                },
            )
            .optional()?
            .ok_or_else(|| {
                err(format!(
                    "no such version: {} v{n} (latest is v{})",
                    page.id, page.version
                ))
            })?;
        (page.title, page.summary, page.body, by, at) = v;
        page.version = n;
    }
    if ctx.json {
        return output::json(&page);
    }
    println!("{}  {}", page.id, page.title);
    if !page.summary.is_empty() {
        println!("summary: {}", page.summary);
    }
    println!("version: {} ({at} by {by})", page.version);
    if let (Some(at), Some(by)) = (&page.verified_at, &page.verified_by) {
        println!("verified: {at} by {by}");
    }
    println!("pinned: {}", if page.pinned { "yes" } else { "no" });
    if !page.tasks.is_empty() {
        println!("tasks: {}", page.tasks.join(", "));
    }
    if page.stale {
        println!(
            "stale: not edited or verified for {STALE_DAYS} days; re-read it, then \
             apm page verify {} (still true) or apm page edit {} (fix it)",
            page.id, page.id
        );
    }
    if !page.body.is_empty() {
        println!();
        item::print_text("  ", page.body.lines());
    }
    Ok(())
}

fn edit(ctx: &Ctx, mut args: EditArgs) -> Result<()> {
    let nothing = args.title.is_none()
        && args.summary.is_none()
        && args.body.is_none()
        && args.body_file.is_none()
        && args.append.is_none()
        && args.rename.is_none()
        && !args.pin
        && !args.unpin
        && args.links.is_empty()
        && args.unlinks.is_empty();
    if nothing {
        return Err(err(
            "nothing to change: pass --title, -s, -b, --append, --rename, --pin, --unpin, \
             --link or --unlink",
        ));
    }
    if let Some(t) = &args.title {
        check_title(t)?;
    }
    if let Some(s) = &args.summary {
        check_summary(s)?;
    }
    let slug = args.rename.as_deref().map(db::check_slug).transpose()?;
    args.body = input::read_text(
        args.body.take(),
        args.body_file.as_deref(),
        &input::PAGE_BODY,
    )?;
    let append = input::read_text(args.append.take(), None, &input::APPEND)?;
    if append.as_ref().is_some_and(|t| t.trim().is_empty()) {
        return Err(err(input::APPEND.empty));
    }

    let tx = db::write_tx(&ctx.conn)?;
    let id = find(&tx, &args.page)?;
    let before = get(&tx, id)?;
    if let Some(n) = args.expect_version.filter(|&n| n != before.version) {
        return Err(err(format!(
            "{} is at version {}, not {n}: someone changed it since you read it; \
             re-read it (apm page show {}) and edit again",
            before.id, before.version, before.id
        )));
    }
    let links = tasks(&tx, &args.links)?;
    let unlinks = tasks(&tx, &args.unlinks)?;
    if let Some(both) = links.iter().find(|id| unlinks.contains(id)) {
        return Err(err(format!(
            "cannot both link and unlink {}",
            db::current_key(&tx, *both)?
        )));
    }

    let title = args.title.unwrap_or_else(|| before.title.clone());
    let summary = args.summary.unwrap_or_else(|| before.summary.clone());
    let mut body = args.body.unwrap_or_else(|| before.body.clone());
    if let Some(text) = append {
        if !body.is_empty() {
            body.push_str("\n\n");
        }
        body.push_str(&text);
    }
    let mut changed = false;
    // Content: a new version, and the page counts as edited (updated_at/by).
    if (&title, &summary, &body) != (&before.title, &before.summary, &before.body) {
        tx.execute(
            &format!(
                "UPDATE pages SET title = ?2, summary = ?3, body = ?4, version = version + 1,
                 updated_by = ?5, updated_at = {} WHERE id = ?1",
                db::NOW_SQL
            ),
            params![id, title, summary, body, ctx.actor],
        )?;
        save_version(&tx, id)?;
        changed = true;
    }
    if let Some(slug) = slug.filter(|s| *s != before.slug) {
        let new_id = format!("{}:{slug}", before.project);
        db::unique(
            tx.execute(
                "UPDATE pages SET slug = ?2 WHERE id = ?1",
                params![id, slug],
            ),
            || already_exists(&new_id),
        )?;
        changed = true;
    }
    let pinned = if args.pin {
        Some(true)
    } else if args.unpin {
        Some(false)
    } else {
        None
    };
    if let Some(p) = pinned.filter(|&p| p != before.pinned) {
        tx.execute("UPDATE pages SET pinned = ?2 WHERE id = ?1", params![id, p])?;
        changed = true;
    }
    let page_id = get(&tx, id)?.id;
    for item_id in links {
        changed |= link(&tx, &ctx.actor, id, &page_id, item_id)?;
    }
    for item_id in unlinks {
        changed |= unlink(&tx, &ctx.actor, id, &page_id, item_id)?;
    }
    tx.commit()?;
    let page = get(&ctx.conn, id)?;
    let verb = if changed { "updated" } else { "unchanged" };
    output::written(ctx.json, &page, &format!("{verb} {}", page.id))
}

fn verify(ctx: &Ctx, reference: &str) -> Result<()> {
    let id = find(&ctx.conn, reference)?;
    ctx.conn.execute(
        &format!(
            "UPDATE pages SET verified_by = ?2, verified_at = {} WHERE id = ?1",
            db::NOW_SQL
        ),
        params![id, ctx.actor],
    )?;
    let page = get(&ctx.conn, id)?;
    output::written(ctx.json, &page, &format!("verified {}", page.id))
}

/// Page summaries matching a WHERE clause on `pages p` (and `pr`, its project), by project key
/// and slug. `linked` is left None.
fn summaries(
    conn: &Connection,
    filter: &str,
    args: &[&dyn rusqlite::ToSql],
) -> Result<Vec<PageSummary>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT pr.key || ':' || p.slug, p.title, p.summary, p.body, p.pinned, p.version,
                p.updated_at, p.verified_at, {}
         FROM pages p JOIN projects pr ON pr.id = p.project_id
         WHERE {filter} ORDER BY pr.key, p.slug",
        stale_sql()
    ))?;
    let rows = stmt.query_map(args, |r| {
        let summary: String = r.get(2)?;
        let body: String = r.get(3)?;
        Ok(PageSummary {
            id: r.get(0)?,
            title: r.get(1)?,
            summary: if summary.is_empty() {
                fallback_summary(&body)
            } else {
                summary
            },
            pinned: r.get(4)?,
            linked: None,
            version: r.get(5)?,
            updated_at: r.get(6)?,
            verified_at: r.get(7)?,
            stale: r.get(8)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// The body's first non-empty line, cut at FALLBACK_MAX characters (+ `...` when cut).
fn fallback_summary(body: &str) -> String {
    let line = body.lines().map(str::trim).find(|l| !l.is_empty());
    let line = line.unwrap_or_default();
    if line.chars().count() <= FALLBACK_MAX {
        return line.to_string();
    }
    let cut: String = line.chars().take(FALLBACK_MAX).collect();
    format!("{}...", cut.trim_end())
}

fn list(ctx: &Ctx, args: ListArgs) -> Result<()> {
    let conn = &ctx.conn;
    let project = args
        .project
        .as_deref()
        .map(|k| db::project_id(conn, k))
        .transpose()?;
    let task = args
        .task
        .as_deref()
        .map(|k| db::item(conn, k).map(|r| r.id))
        .transpose()?;
    let filter = format!(
        "(?1 IS NULL OR p.project_id = ?1)
         AND (?2 IS NULL OR EXISTS (SELECT 1 FROM page_links l
                                    WHERE l.page_id = p.id AND l.item_id = ?2))
         AND (NOT ?3 OR p.pinned) AND (NOT ?4 OR {})",
        stale_sql()
    );
    let pages = summaries(conn, &filter, &[&project, &task, &args.pinned, &args.stale])?;
    if ctx.json {
        return output::json(&pages);
    }
    let rows: Vec<Vec<String>> = pages
        .iter()
        .map(|p| {
            let checked = max_time(&p.updated_at, p.verified_at.as_deref());
            let flags: Vec<&str> = [(p.pinned, "pinned"), (p.stale, "stale")]
                .into_iter()
                .filter_map(|(on, f)| on.then_some(f))
                .collect();
            vec![
                p.id.clone(),
                format!("v{}", p.version),
                checked.get(..10).unwrap_or(checked).to_string(),
                if flags.is_empty() {
                    "-".to_string()
                } else {
                    flags.join(",")
                },
                p.title.clone(),
            ]
        })
        .collect();
    output::table(&["ID", "V", "CHECKED", "FLAGS", "TITLE"], &rows);
    Ok(())
}

/// The later of two ISO UTC timestamps (they compare as text).
fn max_time<'a>(a: &'a str, b: Option<&'a str>) -> &'a str {
    match b {
        Some(b) if b > a => b,
        _ => a,
    }
}

fn history(ctx: &Ctx, reference: &str) -> Result<()> {
    let id = find(&ctx.conn, reference)?;
    let mut stmt = ctx.conn.prepare(
        "SELECT version, title, summary, author, at FROM page_versions
         WHERE page_id = ?1 ORDER BY version",
    )?;
    let versions = stmt
        .query_map(params![id], |r| {
            Ok(Version {
                version: r.get(0)?,
                title: r.get(1)?,
                summary: r.get(2)?,
                author: r.get(3)?,
                at: r.get(4)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if ctx.json {
        return output::json(&versions);
    }
    let rows: Vec<Vec<String>> = versions
        .into_iter()
        .map(|v| vec![format!("v{}", v.version), v.at, v.author, v.title])
        .collect();
    output::table(&["V", "AT", "AUTHOR", "TITLE"], &rows);
    Ok(())
}

/// Delete a page; its versions and links go with it (ON DELETE CASCADE), and each linked
/// task records the unlink.
fn rm(ctx: &Ctx, reference: &str) -> Result<()> {
    let tx = db::write_tx(&ctx.conn)?;
    let id = find(&tx, reference)?;
    let page = get(&tx, id)?;
    let mut stmt = tx.prepare("SELECT item_id FROM page_links WHERE page_id = ?1")?;
    let linked = stmt
        .query_map(params![id], |r| r.get(0))?
        .collect::<rusqlite::Result<Vec<i64>>>()?;
    drop(stmt);
    for item_id in linked {
        unlink(&tx, &ctx.actor, id, &page.id, item_id)?;
    }
    tx.execute("DELETE FROM pages WHERE id = ?1", params![id])?;
    tx.commit()?;
    output::written(ctx.json, &page, &format!("deleted {}", page.id))
}

/// User text as an FTS5 query that cannot be a syntax error: every word (a run of letters
/// and digits) becomes a quoted string, all must match. None when there is no word.
fn fts_query(text: &str) -> Option<String> {
    let words: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| format!("\"{w}\""))
        .collect();
    (!words.is_empty()).then(|| words.join(" "))
}

/// A snippet on one line (bodies have line breaks).
fn one_line(s: String) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `apm search QUERY`: pages and task log entries, best match first (bm25; a page's title
/// weighs more than its summary, which weighs more than its body).
pub fn search(ctx: &Ctx, args: SearchArgs) -> Result<()> {
    let conn = &ctx.conn;
    let project = args
        .project
        .as_deref()
        .map(|k| db::project_id(conn, k))
        .transpose()?;
    let limit = i64::from(args.limit);
    let mut hits: Vec<(f64, Hit)> = Vec::new();
    if let Some(query) = fts_query(&args.query) {
        let mut stmt = conn.prepare(
            "SELECT bm25(pages_fts, 10.0, 5.0, 1.0), pr.key || ':' || p.slug, p.title,
                    snippet(pages_fts, -1, '[', ']', '...', 12)
             FROM pages_fts JOIN pages p ON p.id = pages_fts.rowid
             JOIN projects pr ON pr.id = p.project_id
             WHERE pages_fts MATCH ?1 AND (?2 IS NULL OR p.project_id = ?2)
             ORDER BY 1 LIMIT ?3",
        )?;
        let rows = stmt.query_map(params![query, project, limit], |r| {
            Ok((
                r.get(0)?,
                Hit::Page {
                    id: r.get(1)?,
                    title: r.get(2)?,
                    snippet: one_line(r.get(3)?),
                },
            ))
        })?;
        hits.extend(rows.collect::<rusqlite::Result<Vec<_>>>()?);
        let mut stmt = conn.prepare(
            "SELECT bm25(comments_fts), pr.key || '-' || i.seq, c.id, c.kind, c.author,
                    c.created_at, snippet(comments_fts, 0, '[', ']', '...', 12)
             FROM comments_fts JOIN comments c ON c.id = comments_fts.rowid
             JOIN items i ON i.id = c.item_id JOIN projects pr ON pr.id = i.project_id
             WHERE comments_fts MATCH ?1 AND (?2 IS NULL OR i.project_id = ?2)
             ORDER BY 1 LIMIT ?3",
        )?;
        let rows = stmt.query_map(params![query, project, limit], |r| {
            Ok((
                r.get(0)?,
                Hit::Log {
                    task: r.get(1)?,
                    comment: r.get(2)?,
                    entry: r.get(3)?,
                    author: r.get(4)?,
                    created_at: r.get(5)?,
                    snippet: one_line(r.get(6)?),
                },
            ))
        })?;
        hits.extend(rows.collect::<rusqlite::Result<Vec<_>>>()?);
    }
    // bm25: lower is better. Ties keep pages first.
    hits.sort_by(|a, b| a.0.total_cmp(&b.0));
    hits.truncate(args.limit as usize);
    let hits: Vec<Hit> = hits.into_iter().map(|(_, h)| h).collect();
    if ctx.json {
        return output::json(&hits);
    }
    let rows: Vec<Vec<String>> = hits
        .into_iter()
        .map(|h| match h {
            Hit::Page { id, title, snippet } => {
                vec!["page".into(), id, format!("{title}: {snippet}")]
            }
            Hit::Log {
                task,
                comment,
                entry,
                author,
                snippet,
                ..
            } => vec![
                "log".into(),
                format!("{task} #{comment}"),
                format!("{author} {entry}: {snippet}"),
            ],
        })
        .collect();
    for line in output::aligned(&rows) {
        println!("{line}");
    }
    Ok(())
}

/// The pages `apm context KEY-N` shows: those linked to the task or one of its parents
/// (`linked`), then the pinned pages of its project; plus how many other pages that project
/// has.
#[allow(dead_code)] // `apm context` (S4)
pub fn for_context(conn: &Connection, item_id: i64) -> Result<(Vec<PageSummary>, i64)> {
    let up = "WITH RECURSIVE up(id) AS (
                  SELECT ?1 UNION SELECT i.parent_id FROM items i JOIN up ON i.id = up.id
                  WHERE i.parent_id IS NOT NULL
              )";
    let linked =
        format!("p.id IN ({up} SELECT l.page_id FROM page_links l JOIN up ON l.item_id = up.id)");
    let project: i64 = conn.query_row(
        "SELECT project_id FROM items WHERE id = ?1",
        params![item_id],
        |r| r.get(0),
    )?;
    let mut pages = summaries(conn, &linked, &[&item_id])?;
    for p in &mut pages {
        p.linked = Some(true);
    }
    let pinned = summaries(
        conn,
        &format!("p.project_id = ?2 AND p.pinned AND NOT {linked}"),
        &[&item_id, &project],
    )?;
    pages.extend(pinned.into_iter().map(|p| PageSummary {
        linked: Some(false),
        ..p
    }));
    let shown: BTreeSet<&str> = pages.iter().map(|p| p.id.as_str()).collect();
    let all: i64 = conn.query_row(
        "SELECT count(*) FROM pages WHERE project_id = ?1",
        params![project],
        |r| r.get(0),
    )?;
    // Linked pages may belong to other projects: count only this project's shown pages.
    let key: String = conn.query_row(
        "SELECT key FROM projects WHERE id = ?1",
        params![project],
        |r| r.get(0),
    )?;
    let prefix = format!("{key}:");
    let shown_here = shown.iter().filter(|id| id.starts_with(&prefix)).count() as i64;
    Ok((pages, all - shown_here))
}

/// The pages `apm context -p KEY` shows: the project's pinned pages, plus how many other
/// pages it has.
#[allow(dead_code)] // `apm context -p` (S4)
pub fn for_project(conn: &Connection, project_id: i64) -> Result<(Vec<PageSummary>, i64)> {
    let pages = summaries(conn, "p.project_id = ?1 AND p.pinned", &[&project_id])?;
    let all: i64 = conn.query_row(
        "SELECT count(*) FROM pages WHERE project_id = ?1",
        params![project_id],
        |r| r.get(0),
    )?;
    let more = all - pages.len() as i64;
    Ok((pages, more))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fts_queries_quote_every_word() {
        assert_eq!(
            fts_query("busy timeout").as_deref(),
            Some("\"busy\" \"timeout\"")
        );
        assert_eq!(
            fts_query("\"x\" OR y* (z) -w a:b NEAR").as_deref(),
            Some("\"x\" \"OR\" \"y\" \"z\" \"w\" \"a\" \"b\" \"NEAR\"")
        );
        assert_eq!(fts_query("caf\u{e9}").as_deref(), Some("\"caf\u{e9}\""));
        assert_eq!(
            fts_query("busy_timeout").as_deref(),
            Some("\"busy\" \"timeout\"")
        );
        assert_eq!(fts_query(" \" - : * ( "), None);
        assert_eq!(fts_query(""), None);
    }

    #[test]
    fn fallback_summaries() {
        assert_eq!(fallback_summary("\n\n  first line  \nsecond"), "first line");
        assert_eq!(fallback_summary(""), "");
        let long = "x".repeat(130);
        assert_eq!(fallback_summary(&long), format!("{}...", "x".repeat(120)));
        assert_eq!(fallback_summary(&"y".repeat(120)), "y".repeat(120));
    }

    #[test]
    fn summaries_are_checked() {
        assert!(check_summary(&"s".repeat(200)).is_ok());
        assert_eq!(
            check_summary(&"s".repeat(201)).unwrap_err().to_string(),
            "summary is 201 characters, at most 200: put detail in the body"
        );
        assert!(check_summary("a\nb").is_err());
        assert!(check_summary("").is_ok());
    }

    /// A migrated in-memory database: projects APM (1) and WEB (2); APM-1 > APM-2 (subtask),
    /// APM-3, WEB-1; pages `(project, slug, pinned)` with ids 1..n.
    fn conn(pages: &[(i64, &str, bool)]) -> Connection {
        let conn = db::open(std::path::Path::new(":memory:")).unwrap();
        conn.execute_batch(
            "INSERT INTO projects (id, key, name) VALUES (1, 'APM', 'apm'), (2, 'WEB', 'web');
             INSERT INTO statuses (id, project_id, name, category, position)
                 VALUES (1, 1, 'todo', 'todo', 1), (2, 2, 'todo', 'todo', 1);
             INSERT INTO items (id, project_id, seq, parent_id, title, status_id, created_by)
                 VALUES (1, 1, 1, NULL, 'one', 1, 'a'), (2, 1, 2, 1, 'two', 1, 'a'),
                        (3, 1, 3, NULL, 'three', 1, 'a'), (4, 2, 1, NULL, 'web', 2, 'a');",
        )
        .unwrap();
        for (project, slug, pinned) in pages {
            conn.execute(
                "INSERT INTO pages (project_id, slug, title, body, pinned, created_by, updated_by)
                 VALUES (?1, ?2, ?2, 'the body', ?3, 'a', 'a')",
                params![project, slug, pinned],
            )
            .unwrap();
        }
        conn
    }

    fn ids(pages: &[PageSummary]) -> Vec<(&str, Option<bool>)> {
        pages.iter().map(|p| (p.id.as_str(), p.linked)).collect()
    }

    #[test]
    fn context_pages_are_linked_ones_then_pinned_ones() {
        let conn = conn(&[
            (1, "arch", true),     // 1: pinned
            (1, "storage", false), // 2: linked to APM-1 (the parent of APM-2)
            (1, "misc", false),    // 3: neither
            (2, "deploy", false),  // 4: WEB's, linked to APM-2
            (1, "conv", true),     // 5: pinned and linked to APM-2
            (1, "other", false),   // 6: neither
        ]);
        conn.execute_batch(
            "INSERT INTO page_links (page_id, item_id) VALUES (2, 1), (4, 2), (5, 2);",
        )
        .unwrap();
        let (pages, more) = for_context(&conn, 2).unwrap();
        assert_eq!(
            ids(&pages),
            [
                ("APM:conv", Some(true)),
                ("APM:storage", Some(true)),
                ("WEB:deploy", Some(true)),
                ("APM:arch", Some(false)),
            ]
        );
        assert_eq!(more, 2); // misc, other
        assert_eq!(pages[0].summary, "the body");
        // APM-3 sees only the pinned pages.
        let (pages, more) = for_context(&conn, 3).unwrap();
        assert_eq!(
            ids(&pages),
            [("APM:arch", Some(false)), ("APM:conv", Some(false))]
        );
        assert_eq!(more, 3);
        let (pages, more) = for_project(&conn, 1).unwrap();
        assert_eq!(ids(&pages), [("APM:arch", None), ("APM:conv", None)]);
        assert_eq!(more, 3);
        let (pages, more) = for_project(&conn, 2).unwrap();
        assert!(pages.is_empty());
        assert_eq!(more, 1);
    }
}
