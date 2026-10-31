//! Moving data out and in: `export` (projects as versioned JSON), `import` (recreate them in
//! another database) and `backup` (a consistent copy of the whole file).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use clap::{Args, ValueEnum};
use rusqlite::types::Type;
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::attr::{self, AttrType};
use super::link::{self, LinkKind};
use super::{Ctx, milestone, status, tag};
use crate::error::{AppError, Result, err};
use crate::model::{self, Category, CommentKind, Priority, When};
use crate::{db, input, output};

/// The export format this apm writes (`apm_export`). Bump it on any change to the shape
/// below, and keep reading the old ones (`parse`): 1 had no log kinds, pages or page links.
const VERSION: i64 = 2;

/// The longest page summary, in characters (as `apm page` allows).
const SUMMARY_MAX: usize = 200;

#[derive(Args, Debug)]
pub struct ExportArgs {
    /// Project to export (repeatable; default: all)
    #[arg(short, long = "project", value_name = "KEY")]
    pub projects: Vec<String>,
    /// Write to FILE instead of stdout
    #[arg(short, long, value_name = "FILE")]
    pub output: Option<PathBuf>,
}

#[derive(Args, Debug)]
pub struct ImportArgs {
    /// Export file, or - to read stdin
    pub file: String,
    /// Import a single-project file under this KEY
    #[arg(long, value_name = "NEW")]
    pub key: Option<String>,
    /// Check the file and print what would be created, without writing
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Args, Debug)]
pub struct BackupArgs {
    /// Target file (default: backups/apm-<UTC time>.db next to the database)
    pub path: Option<PathBuf>,
    /// Keep only the N newest backups apm made in the default backup dir
    #[arg(long, value_name = "N", conflicts_with = "path",
          value_parser = clap::value_parser!(u32).range(1..))]
    pub keep: Option<u32>,
}

// The export file. Field order is the output order; lists are ordered as documented in
// AGENTS.md, so exporting the same data twice gives the same text.

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Export {
    apm_export: i64,
    exported_at: String,
    projects: Vec<Project>,
    links: Vec<Link>,
    page_links: Vec<PageLink>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Project {
    key: String,
    name: String,
    description: String,
    next_seq: i64,
    created_at: String,
    updated_at: String,
    statuses: Vec<Status>,
    milestones: Vec<Milestone>,
    attributes: Vec<Attribute>,
    pages: Vec<Page>,
    tasks: Vec<Task>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Status {
    name: String,
    category: Category,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Milestone {
    name: String,
    description: String,
    due: Option<String>,
    status: MilestoneStatus,
    created_at: String,
    updated_at: String,
}

/// `milestones.status`.
#[derive(Clone, Copy, Debug, ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum MilestoneStatus {
    Open,
    Closed,
}

impl MilestoneStatus {
    fn as_str(self) -> &'static str {
        match self {
            MilestoneStatus::Open => "open",
            MilestoneStatus::Closed => "closed",
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Attribute {
    name: String,
    #[serde(rename = "type")]
    kind: AttrType,
    options: Vec<String>,
    created_at: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Task {
    id: String,
    parent: Option<String>,
    title: String,
    body: String,
    status: String,
    priority: Priority,
    assignee: Option<String>,
    due: Option<String>,
    milestone: Option<String>,
    created_by: String,
    created_at: String,
    updated_at: String,
    /// Attribute name -> stored value.
    values: BTreeMap<String, String>,
    tags: Vec<String>,
    comments: Vec<Comment>,
    events: Vec<Event>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Comment {
    author: String,
    kind: CommentKind,
    /// For an answer: the 1-based position of its question in the task's comments.
    answers: Option<i64>,
    body: String,
    created_at: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Event {
    actor: String,
    at: String,
    kind: String,
    field: Option<String>,
    old: Option<String>,
    new: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Link {
    from: String,
    kind: LinkKind,
    to: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Page {
    slug: String,
    title: String,
    summary: String,
    body: String,
    pinned: bool,
    version: i64,
    created_by: String,
    created_at: String,
    updated_by: String,
    updated_at: String,
    verified_by: Option<String>,
    verified_at: Option<String>,
    /// Every version, 1 to `version`; the last is the page's current content.
    versions: Vec<PageVersion>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PageVersion {
    version: i64,
    title: String,
    summary: String,
    body: String,
    author: String,
    at: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PageLink {
    /// `KEY:slug`
    page: String,
    /// `KEY-N`
    task: String,
}

// ---- export ----

pub fn export(ctx: &Ctx, args: ExportArgs) -> Result<()> {
    // One read transaction: a consistent snapshot while agents keep writing.
    let tx = ctx.conn.unchecked_transaction()?;
    let mut ids = Vec::new();
    for key in &args.projects {
        let id = db::project_id(&tx, key)?;
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    if args.projects.is_empty() {
        let mut stmt = tx.prepare("SELECT id FROM projects")?;
        ids = stmt
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
    }
    let mut projects = ids
        .iter()
        .map(|&id| export_project(&tx, id))
        .collect::<Result<Vec<_>>>()?;
    projects.sort_by(|a, b| a.key.cmp(&b.key));
    let data = Export {
        apm_export: VERSION,
        exported_at: tx.query_row(&format!("SELECT {}", db::NOW_SQL), [], |r| r.get(0))?,
        links: export_links(&tx, &ids)?,
        page_links: export_page_links(&tx, &ids)?,
        projects,
    };
    tx.finish()?;
    let Some(path) = args.output else {
        return output::json(&data);
    };
    write_whole(&path, &(serde_json::to_string_pretty(&data)? + "\n"))?;
    let keys: Vec<&str> = data.projects.iter().map(|p| p.key.as_str()).collect();
    let tasks: usize = data.projects.iter().map(|p| p.tasks.len()).sum();
    let pages: usize = data.projects.iter().map(|p| p.pages.len()).sum();
    let summary = json!({
        "path": path.display().to_string(),
        "projects": keys,
        "tasks": tasks,
        "pages": pages,
        "links": data.links.len(),
        "page_links": data.page_links.len(),
    });
    let line = format!(
        "exported {} ({tasks} tasks) to {}",
        if keys.is_empty() {
            "nothing".to_string()
        } else {
            keys.join(", ")
        },
        path.display()
    );
    output::written(ctx.json, &summary, &line)
}

/// Write `text` to a temporary file next to `path`, then rename it over `path`: a failed
/// write leaves no half-written file behind.
fn write_whole(path: &Path, text: &str) -> Result<()> {
    let name = path
        .file_name()
        .ok_or_else(|| err(format!("not a file path: {}", path.display())))?;
    let mut tmp_name = std::ffi::OsString::from(".");
    tmp_name.push(name);
    tmp_name.push(format!(".tmp-{}", std::process::id()));
    let tmp = path.with_file_name(tmp_name);
    let res = std::fs::write(&tmp, text).and_then(|()| std::fs::rename(&tmp, path));
    if let Err(e) = res {
        let _ = std::fs::remove_file(&tmp);
        return Err(err(format!("cannot write {}: {e}", path.display())));
    }
    Ok(())
}

/// Column `i` of a row, parsed as one of apm's names for an enum (`active`, `high`, ...).
fn named<T: ValueEnum>(r: &rusqlite::Row, i: usize) -> rusqlite::Result<T> {
    let s: String = r.get(i)?;
    T::from_str(&s, false)
        .map_err(|e| rusqlite::Error::FromSqlConversionFailure(i, Type::Text, e.into()))
}

fn export_project(conn: &Connection, id: i64) -> Result<Project> {
    let (key, name, description, next_seq, created_at, updated_at) = conn.query_row(
        "SELECT key, name, description, next_seq, created_at, updated_at
         FROM projects WHERE id = ?1",
        params![id],
        |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
            ))
        },
    )?;
    let statuses = rows(
        conn,
        "SELECT name, category FROM statuses WHERE project_id = ?1 ORDER BY position, id",
        id,
        |r| {
            Ok(Status {
                name: r.get(0)?,
                category: named(r, 1)?,
            })
        },
    )?;
    let milestones = rows(
        conn,
        "SELECT name, description, due, status, created_at, updated_at
         FROM milestones WHERE project_id = ?1 ORDER BY id",
        id,
        |r| {
            Ok(Milestone {
                name: r.get(0)?,
                description: r.get(1)?,
                due: r.get(2)?,
                status: named(r, 3)?,
                created_at: r.get(4)?,
                updated_at: r.get(5)?,
            })
        },
    )?;
    let attributes = rows(
        conn,
        "SELECT name, type, options, created_at FROM attributes WHERE project_id = ?1 ORDER BY id",
        id,
        |r| Ok((r.get(0)?, named(r, 1)?, r.get::<_, String>(2)?, r.get(3)?)),
    )?
    .into_iter()
    .map(|(name, kind, options, created_at)| {
        Ok(Attribute {
            name,
            kind,
            options: serde_json::from_str(&options)?,
            created_at,
        })
    })
    .collect::<Result<_>>()?;
    let pages = export_pages(conn, id)?;
    let tasks = rows(
        conn,
        "SELECT i.id, p.key || '-' || i.seq, pp.key || '-' || pa.seq, i.title, i.body, s.name,
                i.priority, i.assignee, i.due, m.name, i.created_by, i.created_at, i.updated_at
         FROM items i
         JOIN projects p ON p.id = i.project_id
         JOIN statuses s ON s.id = i.status_id
         LEFT JOIN items pa ON pa.id = i.parent_id
         LEFT JOIN projects pp ON pp.id = pa.project_id
         LEFT JOIN milestones m ON m.id = i.milestone_id
         WHERE i.project_id = ?1 ORDER BY i.seq",
        id,
        |r| {
            Ok((
                r.get::<_, i64>(0)?,
                Task {
                    id: r.get(1)?,
                    parent: r.get(2)?,
                    title: r.get(3)?,
                    body: r.get(4)?,
                    status: r.get(5)?,
                    priority: named(r, 6)?,
                    assignee: r.get(7)?,
                    due: r.get(8)?,
                    milestone: r.get(9)?,
                    created_by: r.get(10)?,
                    created_at: r.get(11)?,
                    updated_at: r.get(12)?,
                    values: BTreeMap::new(),
                    tags: Vec::new(),
                    comments: Vec::new(),
                    events: Vec::new(),
                },
            ))
        },
    )?
    .into_iter()
    .map(|(item_id, task)| {
        Ok(Task {
            values: attr::stored_values(conn, item_id)?,
            tags: tag::for_item(conn, item_id)?,
            comments: export_comments(conn, item_id)?,
            events: rows(
                conn,
                "SELECT actor, at, kind, field, old, new FROM events WHERE item_id = ?1 ORDER BY id",
                item_id,
                |r| {
                    Ok(Event {
                        actor: r.get(0)?,
                        at: r.get(1)?,
                        kind: r.get(2)?,
                        field: r.get(3)?,
                        old: r.get(4)?,
                        new: r.get(5)?,
                    })
                },
            )?,
            ..task
        })
    })
    .collect::<Result<_>>()?;
    Ok(Project {
        key,
        name,
        description,
        next_seq,
        created_at,
        updated_at,
        statuses,
        milestones,
        attributes,
        pages,
        tasks,
    })
}

/// A task's comments, oldest first; an answer points at its question by position (1-based),
/// since the file has no comment ids.
fn export_comments(conn: &Connection, item_id: i64) -> Result<Vec<Comment>> {
    let all = rows(
        conn,
        "SELECT id, author, kind, answers, body, created_at FROM comments
         WHERE item_id = ?1 ORDER BY id",
        item_id,
        |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, Option<i64>>(3)?,
                Comment {
                    author: r.get(1)?,
                    kind: r.get(2)?,
                    answers: None,
                    body: r.get(4)?,
                    created_at: r.get(5)?,
                },
            ))
        },
    )?;
    let position: HashMap<i64, i64> = all
        .iter()
        .enumerate()
        .map(|(n, (id, _, _))| (*id, n as i64 + 1))
        .collect();
    Ok(all
        .into_iter()
        .map(|(_, answers, c)| Comment {
            answers: answers.and_then(|id| position.get(&id).copied()),
            ..c
        })
        .collect())
}

/// A project's pages by slug, each with its versions.
fn export_pages(conn: &Connection, project_id: i64) -> Result<Vec<Page>> {
    rows(
        conn,
        "SELECT id, slug, title, summary, body, pinned, version, created_by, created_at,
                updated_by, updated_at, verified_by, verified_at
         FROM pages WHERE project_id = ?1 ORDER BY slug",
        project_id,
        |r| {
            Ok((
                r.get::<_, i64>(0)?,
                Page {
                    slug: r.get(1)?,
                    title: r.get(2)?,
                    summary: r.get(3)?,
                    body: r.get(4)?,
                    pinned: r.get(5)?,
                    version: r.get(6)?,
                    created_by: r.get(7)?,
                    created_at: r.get(8)?,
                    updated_by: r.get(9)?,
                    updated_at: r.get(10)?,
                    verified_by: r.get(11)?,
                    verified_at: r.get(12)?,
                    versions: Vec::new(),
                },
            ))
        },
    )?
    .into_iter()
    .map(|(page_id, page)| {
        Ok(Page {
            versions: rows(
                conn,
                "SELECT version, title, summary, body, author, at FROM page_versions
                 WHERE page_id = ?1 ORDER BY version",
                page_id,
                |r| {
                    Ok(PageVersion {
                        version: r.get(0)?,
                        title: r.get(1)?,
                        summary: r.get(2)?,
                        body: r.get(3)?,
                        author: r.get(4)?,
                        at: r.get(5)?,
                    })
                },
            )?,
            ..page
        })
    })
    .collect()
}

/// Links whose both ends are in the exported projects.
fn export_links(conn: &Connection, project_ids: &[i64]) -> Result<Vec<Link>> {
    let mut stmt = conn.prepare(
        "SELECT pf.key || '-' || f.seq, l.kind, pt.key || '-' || t.seq, f.project_id, t.project_id
         FROM links l
         JOIN items f ON f.id = l.from_id JOIN projects pf ON pf.id = f.project_id
         JOIN items t ON t.id = l.to_id JOIN projects pt ON pt.id = t.project_id
         ORDER BY pf.key, f.seq, l.kind, pt.key, t.seq",
    )?;
    let all = stmt
        .query_map([], |r| {
            Ok((
                Link {
                    from: r.get(0)?,
                    kind: named(r, 1)?,
                    to: r.get(2)?,
                },
                r.get::<_, i64>(3)?,
                r.get::<_, i64>(4)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(all
        .into_iter()
        .filter(|(_, from, to)| project_ids.contains(from) && project_ids.contains(to))
        .map(|(link, _, _)| link)
        .collect())
}

/// Page links whose page and task are both in the exported projects.
fn export_page_links(conn: &Connection, project_ids: &[i64]) -> Result<Vec<PageLink>> {
    let mut stmt = conn.prepare(
        "SELECT pp.key || ':' || pg.slug, pt.key || '-' || i.seq, pg.project_id, i.project_id
         FROM page_links l
         JOIN pages pg ON pg.id = l.page_id JOIN projects pp ON pp.id = pg.project_id
         JOIN items i ON i.id = l.item_id JOIN projects pt ON pt.id = i.project_id
         ORDER BY pp.key, pg.slug, pt.key, i.seq",
    )?;
    let all = stmt
        .query_map([], |r| {
            Ok((
                PageLink {
                    page: r.get(0)?,
                    task: r.get(1)?,
                },
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(all
        .into_iter()
        .filter(|(_, page, task)| project_ids.contains(page) && project_ids.contains(task))
        .map(|(link, _, _)| link)
        .collect())
}

/// Every row of a query with one `?1` parameter.
fn rows<T>(
    conn: &Connection,
    sql: &str,
    id: i64,
    f: impl FnMut(&rusqlite::Row) -> rusqlite::Result<T>,
) -> Result<Vec<T>> {
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(params![id], f)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

// ---- import ----

pub fn import(ctx: &Ctx, args: ImportArgs) -> Result<()> {
    let text = match args.file.as_str() {
        "-" => input::read_text(Some(args.file.clone()), None, &input::IMPORT)?,
        file => input::read_text(None, Some(Path::new(file)), &input::IMPORT)?,
    }
    .unwrap_or_default();
    let mut data = parse(&text)?;
    let new_key = match args.key.as_deref() {
        Some(key) if data.projects.len() != 1 => {
            return Err(err(format!(
                "--key {key} needs a file with one project; this one has {}",
                data.projects.len()
            )));
        }
        Some(key) => Some(db::check_project_key(key)?),
        None => None,
    };
    check(&mut data)?;
    // Everything in one write transaction: a failure (or --dry-run) leaves the database as it was.
    let tx = db::write_tx(&ctx.conn)?;
    let mut ids = HashMap::new();
    let mut page_ids = HashMap::new();
    let mut done = Vec::new();
    for project in &data.projects {
        let key = new_key.clone().unwrap_or_else(|| project.key.clone());
        let claimed = import_project(&tx, project, &key, &mut ids, &mut page_ids)?;
        done.push((project, key, claimed));
    }
    let mut stmt = tx.prepare("INSERT INTO links (from_id, to_id, kind) VALUES (?1, ?2, ?3)")?;
    for link in &data.links {
        stmt.execute(params![ids[&link.from], ids[&link.to], link.kind.as_str()])?;
    }
    drop(stmt);
    let mut stmt = tx.prepare("INSERT INTO page_links (page_id, item_id) VALUES (?1, ?2)")?;
    for link in &data.page_links {
        stmt.execute(params![page_ids[&link.page], ids[&link.task]])?;
    }
    drop(stmt);
    if args.dry_run {
        tx.rollback()?;
    } else {
        tx.commit()?;
    }

    let verb = if args.dry_run {
        "would import"
    } else {
        "imported"
    };
    let mut lines = Vec::new();
    let mut projects = Vec::new();
    for (p, key, claimed) in done {
        let comments: usize = p.tasks.iter().map(|t| t.comments.len()).sum();
        let events: usize = p.tasks.iter().map(|t| t.events.len()).sum();
        let versions: usize = p.pages.iter().map(|g| g.versions.len()).sum();
        let mut line = format!("{verb} {key}");
        if *key != p.key {
            line.push_str(&format!(" (from {})", p.key));
        }
        line.push_str(&format!(
            ": {} tasks, {} statuses, {} milestones, {} attributes, {comments} comments, \
             {events} events, {} pages ({versions} versions)",
            p.tasks.len(),
            p.statuses.len(),
            p.milestones.len(),
            p.attributes.len(),
            p.pages.len(),
        ));
        let mut value = json!({
            "key": key,
            "source_key": p.key,
            "name": p.name,
            "tasks": p.tasks.len(),
            "statuses": p.statuses.len(),
            "milestones": p.milestones.len(),
            "attributes": p.attributes.len(),
            "comments": comments,
            "events": events,
            "pages": p.pages.len(),
            "page_versions": versions,
        });
        if let Some(from) = claimed {
            line.push_str(&format!(
                " (key was an alias of {from}; its old ids no longer resolve)"
            ));
            value["claimed_alias_from"] = from.into();
        }
        lines.push(line);
        projects.push(value);
    }
    lines.push(format!(
        "{verb} {} links, {} page links",
        data.links.len(),
        data.page_links.len()
    ));
    let summary = json!({
        "dry_run": args.dry_run,
        "projects": projects,
        "links": data.links.len(),
        "page_links": data.page_links.len(),
    });
    output::written(ctx.json, &summary, &lines.join("\n"))
}

/// Parse an export file: an object with a supported `apm_export` version, then the full shape.
fn parse(text: &str) -> Result<Export> {
    let mut value: Value =
        serde_json::from_str(text).map_err(|e| err(format!("invalid export file: {e}")))?;
    match value.get("apm_export").map(Value::as_i64) {
        None => return Err(err("not an apm export: no apm_export field")),
        Some(Some(1)) => upgrade_v1(&mut value)?,
        Some(Some(VERSION)) => {}
        Some(_) => {
            return Err(err(format!(
                "unsupported export version {}; this apm reads 1 to {VERSION}",
                value["apm_export"]
            )));
        }
    }
    serde_json::from_value(value).map_err(|e| err(format!("invalid export file: {e}")))
}

/// Bring a version 1 file to the current shape: every comment is a note, and there are no
/// pages or page links. The fields version 1 did not have stay unknown in a version 1 file.
fn upgrade_v1(value: &mut Value) -> Result<()> {
    fn add(obj: &mut serde_json::Map<String, Value>, field: &str, v: Value) -> Result<()> {
        if obj.contains_key(field) {
            return Err(err(format!(
                "invalid export file: unknown field `{field}` in apm_export 1"
            )));
        }
        obj.insert(field.to_string(), v);
        Ok(())
    }
    // Leave anything of the wrong shape alone: deserializing reports it.
    fn list(v: Option<&mut Value>) -> impl Iterator<Item = &mut Value> {
        v.and_then(Value::as_array_mut).into_iter().flatten()
    }
    let Some(top) = value.as_object_mut() else {
        return Ok(());
    };
    add(top, "page_links", json!([]))?;
    for p in list(top.get_mut("projects")) {
        let Some(p) = p.as_object_mut() else { continue };
        add(p, "pages", json!([]))?;
        for t in list(p.get_mut("tasks")) {
            for c in list(t.get_mut("comments")) {
                if let Some(c) = c.as_object_mut() {
                    add(c, "kind", json!("note"))?;
                    add(c, "answers", Value::Null)?;
                }
            }
        }
    }
    Ok(())
}

/// Validate the file before anything is written, with the rules of the commands that create
/// the same things, and put values, tags and select options in their stored form. Every
/// reference (parent, status, milestone, attribute, link end) must point into the file.
/// Database conflicts (an existing key) are found by the write itself, inside the transaction.
fn check(data: &mut Export) -> Result<()> {
    let mut keys = HashSet::new();
    // Every task and page in the file, as KEY-N and KEY:slug (the file's keys).
    let mut all_tasks = HashSet::new();
    let mut all_pages = HashSet::new();
    for p in &mut data.projects {
        let pkey = p.key.clone();
        let at = |e: AppError| err(format!("project {pkey}: {e}"));
        if db::check_project_key(&p.key)? != p.key {
            return Err(err(format!("project key must be uppercase: {}", p.key)));
        }
        if !keys.insert(p.key.clone()) {
            return Err(err(format!("project {} appears twice", p.key)));
        }
        if p.name.trim().is_empty() {
            return Err(at(err("project name cannot be empty")));
        }
        if p.next_seq < 1 {
            return Err(at(err(format!("invalid next_seq {}", p.next_seq))));
        }
        check_time(&p.created_at).map_err(at)?;
        check_time(&p.updated_at).map_err(at)?;

        if p.statuses.is_empty() {
            return Err(at(err("no statuses")));
        }
        let mut statuses = HashSet::new();
        for s in &p.statuses {
            if status::clean_name(&s.name).map_err(at)? != s.name {
                return Err(at(err(format!("invalid status name: `{}`", s.name))));
            }
            if !statuses.insert(s.name.to_ascii_lowercase()) {
                return Err(at(err(format!("status appears twice: {}", s.name))));
            }
        }
        let mut milestones = HashSet::new();
        for m in &p.milestones {
            if milestone::clean_name(&m.name).map_err(at)? != m.name {
                return Err(at(err(format!("invalid milestone name: `{}`", m.name))));
            }
            if !milestones.insert(m.name.clone()) {
                return Err(at(err(format!("milestone appears twice: {}", m.name))));
            }
            if let Some(due) = &m.due {
                check_due(due).map_err(at)?;
            }
            check_time(&m.created_at).map_err(at)?;
            check_time(&m.updated_at).map_err(at)?;
        }
        let mut attrs = HashMap::new();
        for a in &mut p.attributes {
            if attr::check_name(&a.name).map_err(at)? != a.name {
                return Err(at(err(format!("invalid attribute name: `{}`", a.name))));
            }
            a.options = attr::clean_options(a.kind, &a.options).map_err(at)?;
            check_time(&a.created_at).map_err(at)?;
            if attrs
                .insert(a.name.clone(), (a.kind, a.options.clone()))
                .is_some()
            {
                return Err(at(err(format!("attribute appears twice: {}", a.name))));
            }
        }

        let mut slugs = HashSet::new();
        for page in &p.pages {
            let id = format!("{}:{}", p.key, page.slug);
            check_page(page).map_err(|e| err(format!("page {id}: {e}")))?;
            if !slugs.insert(page.slug.clone()) {
                return Err(at(err(format!("page appears twice: {id}"))));
            }
            all_pages.insert(id);
        }

        // Task numbers first, so a parent may come after its subtask.
        let mut parents = HashMap::new();
        for t in &p.tasks {
            let seq = task_seq(&p.key, &t.id).map_err(at)?;
            if seq >= p.next_seq {
                return Err(at(err(format!(
                    "{} is not below next_seq {}",
                    t.id, p.next_seq
                ))));
            }
            if parents.insert(seq, t.parent.clone()).is_some() {
                return Err(at(err(format!("task appears twice: {}", t.id))));
            }
            all_tasks.insert(t.id.clone());
        }
        for t in &mut p.tasks {
            let id = t.id.clone();
            let at = |e: AppError| err(format!("{id}: {e}"));
            if let Some(parent) = &t.parent {
                let seq = task_seq(&p.key, parent).map_err(at)?;
                if !parents.contains_key(&seq) {
                    return Err(at(err(format!("no such parent in the file: {parent}"))));
                }
            }
            if t.title.trim().is_empty() {
                return Err(at(err("title cannot be empty")));
            }
            if !statuses.contains(&t.status.to_ascii_lowercase()) {
                return Err(at(err(format!("no such status: {}", t.status))));
            }
            if let Some(due) = &t.due {
                check_due(due).map_err(at)?;
            }
            if let Some(m) = &t.milestone
                && !milestones.contains(m)
            {
                return Err(at(err(format!("no such milestone: {m}"))));
            }
            for (name, value) in &mut t.values {
                let (kind, options) = attrs
                    .get(name)
                    .ok_or_else(|| at(err(format!("unknown attribute: {name}"))))?;
                *value = attr::normalize_value(*kind, options, value.as_str())
                    .map_err(|why| at(err(format!("invalid value for {name}: {why}"))))?;
            }
            t.tags = t
                .tags
                .iter()
                .map(|s| tag::normalize(s))
                .collect::<Result<_>>()
                .map_err(at)?;
            t.tags.sort();
            t.tags.dedup();
            check_time(&t.created_at).map_err(at)?;
            check_time(&t.updated_at).map_err(at)?;
            for (n, c) in t.comments.iter().enumerate() {
                check_time(&c.created_at).map_err(at)?;
                check_answers(&t.comments, n).map_err(at)?;
            }
            for e in &t.events {
                if db::EventKind::parse(&e.kind).is_none() {
                    return Err(at(err(format!("unknown event kind: {}", e.kind))));
                }
                check_time(&e.at).map_err(at)?;
            }
        }
        // A parent chain must end: walk up at most as many steps as there are tasks.
        for t in &p.tasks {
            let mut parent = t.parent.clone();
            for _ in 0..=p.tasks.len() {
                let Some(id) = parent else { break };
                parent = parents[&task_seq(&p.key, &id)?].clone();
            }
            if parent.is_some() {
                return Err(at(err(format!("parent cycle at {}", t.id))));
            }
        }
    }

    // Links, as `link` checks them, each against the ones before it.
    let mut seen = HashSet::new();
    let mut blocks: HashMap<&str, Vec<&str>> = HashMap::new();
    for l in &data.links {
        let name = format!("link {} {} {}", l.from, l.kind.as_str(), l.to);
        for end in [&l.from, &l.to] {
            if !all_tasks.contains(end) {
                return Err(err(format!("{name}: no such task in the file: {end}")));
            }
        }
        if l.from == l.to {
            return Err(err(format!("{name}: a task cannot link to itself")));
        }
        // `relates` is symmetric: A relates B and B relates A are the same link.
        let (a, b) = match l.kind {
            LinkKind::Relates if l.to < l.from => (&l.to, &l.from),
            _ => (&l.from, &l.to),
        };
        if !seen.insert((a, l.kind.as_str(), b)) {
            return Err(err(format!("{name}: appears twice")));
        }
        if l.kind == LinkKind::Blocks {
            let cycle = link::blocks_cycle(l.from.as_str(), l.to.as_str(), |id| {
                Ok(blocks.get(id).cloned().unwrap_or_default())
            })?;
            if let Some(path) = cycle {
                return Err(err(format!("blocks cycle: {}", path.join(" blocks "))));
            }
            blocks.entry(&l.from).or_default().push(&l.to);
        }
    }

    let mut seen = HashSet::new();
    for l in &data.page_links {
        let name = format!("page link {} {}", l.page, l.task);
        if !all_pages.contains(&l.page) {
            return Err(err(format!("{name}: no such page in the file: {}", l.page)));
        }
        if !all_tasks.contains(&l.task) {
            return Err(err(format!("{name}: no such task in the file: {}", l.task)));
        }
        if !seen.insert((&l.page, &l.task)) {
            return Err(err(format!("{name}: appears twice")));
        }
    }
    Ok(())
}

/// Comment `n` (0-based) of a task: only an answer answers, and it answers an earlier
/// question of the same task (`answers` is that question's 1-based position).
fn check_answers(comments: &[Comment], n: usize) -> Result<()> {
    let c = &comments[n];
    let Some(q) = c.answers else {
        return Ok(());
    };
    let at = n + 1;
    if c.kind != CommentKind::Answer {
        return Err(err(format!(
            "comment {at} is a {}, only an answer has `answers`",
            c.kind.as_str()
        )));
    }
    if q < 1 || q as usize >= at {
        return Err(err(format!(
            "comment {at} answers comment {q}, which is not an earlier comment of this task"
        )));
    }
    let kind = comments[q as usize - 1].kind;
    if kind != CommentKind::Question {
        return Err(err(format!(
            "comment {at} answers comment {q}, which is a {}, not a question",
            kind.as_str()
        )));
    }
    Ok(())
}

/// A page as `apm page` would have written it: a lowercase slug, a title, a short summary,
/// and every version from 1 to `version`, the last one being the page's content.
fn check_page(page: &Page) -> Result<()> {
    if db::check_slug(&page.slug)? != page.slug {
        return Err(err(format!("page slug must be lowercase: {}", page.slug)));
    }
    check_content(&page.title, &page.summary)?;
    for t in [&page.created_at, &page.updated_at] {
        check_time(t)?;
    }
    match (&page.verified_by, &page.verified_at) {
        (Some(_), Some(at)) => check_time(at)?,
        (None, None) => {}
        _ => return Err(err("verified_by and verified_at go together")),
    }
    if page.version < 1 {
        return Err(err(format!("invalid version {}", page.version)));
    }
    for (i, v) in page.versions.iter().enumerate() {
        let want = i as i64 + 1;
        if v.version != want {
            return Err(err(format!(
                "versions must run 1 to {} in order: expected version {want}, found {}",
                page.version, v.version
            )));
        }
        let at = |e: AppError| err(format!("version {want}: {e}"));
        check_content(&v.title, &v.summary).map_err(at)?;
        check_time(&v.at).map_err(at)?;
    }
    if page.versions.len() as i64 != page.version {
        return Err(err(format!(
            "versions must run 1 to {}: the file has {}",
            page.version,
            page.versions.len()
        )));
    }
    let last = &page.versions[page.versions.len() - 1];
    if (&last.title, &last.summary, &last.body) != (&page.title, &page.summary, &page.body) {
        return Err(err(format!(
            "version {} differs from the page's title, summary or body",
            page.version
        )));
    }
    Ok(())
}

/// A page's (or page version's) title and summary.
fn check_content(title: &str, summary: &str) -> Result<()> {
    if title.trim().is_empty() {
        return Err(err("title cannot be empty"));
    }
    let n = summary.chars().count();
    if n > SUMMARY_MAX {
        return Err(err(format!(
            "summary is {n} characters, the limit is {SUMMARY_MAX}"
        )));
    }
    Ok(())
}

/// The number N of a task id `KEY-N` that must belong to project `key`.
fn task_seq(key: &str, id: &str) -> Result<i64> {
    match db::parse_item_key(id)? {
        (k, n) if k == key && db::item_key(&k, n) == id => Ok(n),
        _ => Err(err(format!("task id {id} is not of the form {key}-N"))),
    }
}

/// A due as stored: a date or a UTC date-time.
fn check_due(due: &str) -> Result<()> {
    if model::normalize_when(due)? != due {
        return Err(err(format!("due not in stored form: {due}")));
    }
    Ok(())
}

/// A timestamp as SQLite writes them: `YYYY-MM-DDTHH:MM:SSZ`.
fn check_time(s: &str) -> Result<()> {
    match model::parse_when(s) {
        Ok(When::DateTime(t)) if t == s => Ok(()),
        _ => Err(err(format!(
            "invalid timestamp `{s}`, expected YYYY-MM-DDTHH:MM:SSZ"
        ))),
    }
}

/// Create one project from a checked file under `key`, adding its tasks to `ids` (file
/// `KEY-N` -> new row id) and its pages to `page_ids` (file `KEY:slug` -> new row id) for the
/// links. Returns the project whose alias `key` was, if any.
fn import_project(
    tx: &Connection,
    p: &Project,
    key: &str,
    ids: &mut HashMap<String, i64>,
    page_ids: &mut HashMap<String, i64>,
) -> Result<Option<String>> {
    let claimed = db::claim_project_key(tx, key, None)?;
    db::unique(
        tx.execute(
            "INSERT INTO projects (key, name, description, next_seq, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                key,
                p.name,
                p.description,
                p.next_seq,
                p.created_at,
                p.updated_at
            ],
        ),
        || err(format!("project already exists: {key}")),
    )?;
    let project_id = tx.last_insert_rowid();

    let mut statuses = HashMap::new();
    for (i, s) in p.statuses.iter().enumerate() {
        tx.execute(
            "INSERT INTO statuses (project_id, name, category, position) VALUES (?1, ?2, ?3, ?4)",
            params![project_id, s.name, s.category.as_str(), i as i64 + 1],
        )?;
        statuses.insert(s.name.to_ascii_lowercase(), tx.last_insert_rowid());
    }
    let mut milestones = HashMap::new();
    for m in &p.milestones {
        tx.execute(
            "INSERT INTO milestones (project_id, name, description, due, status, created_at,
                                     updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                project_id,
                m.name,
                m.description,
                m.due,
                m.status.as_str(),
                m.created_at,
                m.updated_at
            ],
        )?;
        milestones.insert(m.name.as_str(), tx.last_insert_rowid());
    }
    let mut attrs = HashMap::new();
    for a in &p.attributes {
        tx.execute(
            "INSERT INTO attributes (project_id, name, type, options, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                project_id,
                a.name,
                a.kind.as_str(),
                serde_json::to_string(&a.options)?,
                a.created_at
            ],
        )?;
        attrs.insert(a.name.as_str(), tx.last_insert_rowid());
    }
    // The FTS triggers index each page as it is inserted.
    for page in &p.pages {
        tx.execute(
            "INSERT INTO pages (project_id, slug, title, summary, body, pinned, version,
                                created_by, created_at, updated_by, updated_at, verified_by,
                                verified_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                project_id,
                page.slug,
                page.title,
                page.summary,
                page.body,
                page.pinned,
                page.version,
                page.created_by,
                page.created_at,
                page.updated_by,
                page.updated_at,
                page.verified_by,
                page.verified_at
            ],
        )?;
        let page_id = tx.last_insert_rowid();
        page_ids.insert(format!("{}:{}", p.key, page.slug), page_id);
        for v in &page.versions {
            tx.execute(
                "INSERT INTO page_versions (page_id, version, title, summary, body, author, at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    page_id, v.version, v.title, v.summary, v.body, v.author, v.at
                ],
            )?;
        }
    }

    // check() put values and tags in their stored form.
    let mut seq_ids = HashMap::new();
    for t in &p.tasks {
        let seq = task_seq(&p.key, &t.id)?;
        let milestone_id = t.milestone.as_deref().map(|m| milestones[m]);
        tx.execute(
            "INSERT INTO items (project_id, seq, milestone_id, title, body, status_id, priority,
                                assignee, due, created_by, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                project_id,
                seq,
                milestone_id,
                t.title,
                t.body,
                statuses[&t.status.to_ascii_lowercase()],
                t.priority.as_str(),
                t.assignee,
                t.due,
                t.created_by,
                t.created_at,
                t.updated_at
            ],
        )?;
        let item_id = tx.last_insert_rowid();
        seq_ids.insert(seq, item_id);
        ids.insert(t.id.clone(), item_id);
        for (name, value) in &t.values {
            tx.execute(
                "INSERT INTO item_values (item_id, attribute_id, value) VALUES (?1, ?2, ?3)",
                params![item_id, attrs[name.as_str()], value],
            )?;
        }
        for tag in &t.tags {
            tx.execute(
                "INSERT INTO tags (item_id, tag) VALUES (?1, ?2)",
                params![item_id, tag],
            )?;
        }
        // check() made every `answers` the position of an earlier comment.
        let mut comment_ids = Vec::new();
        for c in &t.comments {
            let answers = c.answers.map(|n| comment_ids[n as usize - 1]);
            tx.execute(
                "INSERT INTO comments (item_id, kind, answers, author, body, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    item_id,
                    c.kind.as_str(),
                    answers,
                    c.author,
                    c.body,
                    c.created_at
                ],
            )?;
            comment_ids.push(tx.last_insert_rowid());
        }
        for e in &t.events {
            tx.execute(
                "INSERT INTO events (item_id, actor, at, kind, field, old, new)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![item_id, e.actor, e.at, e.kind, e.field, e.old, e.new],
            )?;
        }
    }
    // Parents last: a subtask may come before its parent in number order.
    for t in &p.tasks {
        if let Some(parent) = &t.parent {
            tx.execute(
                "UPDATE items SET parent_id = ?1 WHERE id = ?2",
                params![
                    seq_ids[&task_seq(&p.key, parent)?],
                    seq_ids[&task_seq(&p.key, &t.id)?]
                ],
            )?;
        }
    }
    Ok(claimed)
}

// ---- backup ----

pub fn backup(ctx: &Ctx, db_path: &Path, args: BackupArgs) -> Result<()> {
    let dir = db_path
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
        .join("backups");
    let target = match args.path {
        Some(path) => path,
        None => {
            std::fs::create_dir_all(&dir)
                .map_err(|e| err(format!("cannot create backup dir: {}: {e}", dir.display())))?;
            // Milliseconds, so two backups in one second do not collide; sorts by time.
            let stamp: String =
                ctx.conn
                    .query_row("SELECT strftime('%Y-%m-%dT%H-%M-%fZ', 'now')", [], |r| {
                        r.get(0)
                    })?;
            dir.join(format!("apm-{stamp}.db"))
        }
    };
    if target.exists() {
        return Err(err(format!("file exists: {}", target.display())));
    }
    let target_text = target
        .to_str()
        .ok_or_else(|| err(format!("path is not UTF-8: {}", target.display())))?;
    // A consistent copy, even of a database other agents are writing (WAL included).
    ctx.conn.execute("VACUUM INTO ?1", params![target_text])?;
    let path = target.display().to_string();
    if !ctx.json {
        println!("{path}");
    }
    // The backup exists now: a failure to prune is a warning, not an error.
    let mut pruned = Vec::new();
    let mut warning = None;
    if let Some(keep) = args.keep {
        warning = prune(&dir, keep as usize, &mut pruned).err();
    }
    let pruned: Vec<String> = pruned.iter().map(|p| p.display().to_string()).collect();
    if ctx.json {
        output::json(&json!({ "path": path, "pruned": pruned }))?;
    } else {
        for p in &pruned {
            println!("pruned {p}");
        }
    }
    if let Some(e) = warning {
        output::print_warning(&e.0, ctx.json);
    }
    Ok(())
}

/// Delete all but the `keep` newest backups `backup` made in `dir` (names sort by time),
/// adding each deleted file to `pruned`. Other files are left alone.
fn prune(dir: &Path, keep: usize, pruned: &mut Vec<PathBuf>) -> Result<()> {
    let cannot = |what: &str, path: &Path, e: std::io::Error| {
        err(format!("cannot {what}: {}: {e}", path.display()))
    };
    let mut backups = Vec::new();
    for entry in std::fs::read_dir(dir).map_err(|e| cannot("read backup dir", dir, e))? {
        let path = entry.map_err(|e| cannot("read backup dir", dir, e))?.path();
        let ours = path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(is_backup_name);
        if ours && path.is_file() {
            backups.push(path);
        }
    }
    backups.sort();
    let old = backups.len().saturating_sub(keep);
    for path in backups.drain(..old) {
        std::fs::remove_file(&path).map_err(|e| cannot("prune backup", &path, e))?;
        pruned.push(path);
    }
    Ok(())
}

/// A name `backup` generates: `apm-YYYY-MM-DDTHH-MM-SS.mmmZ.db`.
fn is_backup_name(name: &str) -> bool {
    const SHAPE: &[u8] = b"apm-0000-00-00T00-00-00.000Z.db";
    name.len() == SHAPE.len()
        && name.bytes().zip(SHAPE).all(|(c, &s)| {
            if s == b'0' {
                c.is_ascii_digit()
            } else {
                c == s
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_ids_belong_to_their_project() {
        assert_eq!(task_seq("APM", "APM-3").unwrap(), 3);
        assert!(task_seq("APM", "OPS-3").is_err());
        assert!(task_seq("APM", "apm-3").is_err());
        assert!(task_seq("APM", "APM-03").is_err());
    }

    #[test]
    fn timestamps_and_dues_must_be_in_stored_form() {
        assert!(check_time("2026-10-22T14:12:00Z").is_ok());
        assert!(check_time("2026-10-22T14:12Z").is_err());
        assert!(check_time("2026-10-22").is_err());
        assert!(check_due("2026-10-22").is_ok());
        assert!(check_due("2026-10-22T16:12:00+02:00").is_err());
    }

    #[test]
    fn backup_names() {
        assert!(is_backup_name("apm-2026-10-22T14-12-00.123Z.db"));
        assert!(!is_backup_name("apm-old.db"));
        assert!(!is_backup_name("apm-2026-10-22T14-12-00.123Z.db-wal"));
        assert!(!is_backup_name("apm-2026-10-22T14-12-00Z.db"));
        assert!(!is_backup_name("apm-2026-1x-22T14-12-00.123Z.db"));
    }
}
