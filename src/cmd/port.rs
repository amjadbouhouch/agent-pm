//! Moving data out and in: `export` (projects as versioned JSON), `import` (recreate them in
//! another database) and `backup` (a consistent copy of the whole file).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use clap::{Args, ValueEnum};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{Ctx, attr, milestone, status, tag};
use crate::error::{Result, err};
use crate::model::{self, Category, Priority, When};
use crate::{db, input, output};

/// The export format this apm writes and reads (`apm_export`). Bump it on any change to the
/// shape below, and keep reading the old one.
const VERSION: i64 = 1;

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
    /// Check the file and print what would be imported, without writing
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Args, Debug)]
pub struct BackupArgs {
    /// Target file (default: backups/apm-<UTC time>.db next to the database)
    pub path: Option<PathBuf>,
    /// Keep only the N newest apm-*.db files in the default backup dir
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
    tasks: Vec<Task>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Status {
    name: String,
    category: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Milestone {
    name: String,
    description: String,
    due: Option<String>,
    status: String,
    created_at: String,
    updated_at: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Attribute {
    name: String,
    #[serde(rename = "type")]
    kind: String,
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
    priority: String,
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
    kind: String,
    to: String,
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
        projects,
    };
    tx.finish()?;
    let Some(path) = args.output else {
        return output::json(&data);
    };
    let text = serde_json::to_string_pretty(&data)? + "\n";
    std::fs::write(&path, text)
        .map_err(|e| err(format!("cannot write {}: {e}", path.display())))?;
    let keys: Vec<&str> = data.projects.iter().map(|p| p.key.as_str()).collect();
    let tasks: usize = data.projects.iter().map(|p| p.tasks.len()).sum();
    let summary = json!({
        "path": path.display().to_string(),
        "projects": keys,
        "tasks": tasks,
        "links": data.links.len(),
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
                category: r.get(1)?,
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
                status: r.get(3)?,
                created_at: r.get(4)?,
                updated_at: r.get(5)?,
            })
        },
    )?;
    let attributes = rows(
        conn,
        "SELECT name, type, options, created_at FROM attributes WHERE project_id = ?1 ORDER BY id",
        id,
        |r| Ok((r.get(0)?, r.get(1)?, r.get::<_, String>(2)?, r.get(3)?)),
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
                    priority: r.get(6)?,
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
            comments: rows(
                conn,
                "SELECT author, body, created_at FROM comments WHERE item_id = ?1 ORDER BY id",
                item_id,
                |r| {
                    Ok(Comment {
                        author: r.get(0)?,
                        body: r.get(1)?,
                        created_at: r.get(2)?,
                    })
                },
            )?,
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
        tasks,
    })
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
                    kind: r.get(1)?,
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
    let data = parse(&text)?;
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
    check(&data)?;
    // Everything in one write transaction: a failure (or --dry-run) leaves the database as it was.
    let tx = db::write_tx(&ctx.conn)?;
    let mut ids = HashMap::new();
    let mut done = Vec::new();
    for project in &data.projects {
        let key = new_key.clone().unwrap_or_else(|| project.key.clone());
        let claimed = import_project(&tx, project, &key, &mut ids)?;
        done.push((project, key, claimed));
    }
    let mut stmt = tx.prepare("INSERT INTO links (from_id, to_id, kind) VALUES (?1, ?2, ?3)")?;
    for link in &data.links {
        stmt.execute(params![ids[&link.from], ids[&link.to], link.kind])?;
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
    for (project, key, claimed) in done {
        let mut line = format!("{verb} {key}");
        if *key != project.key {
            line.push_str(&format!(" from {}", project.key));
        }
        line.push_str(&format!(" ({} tasks)", project.tasks.len()));
        let mut value = json!({
            "key": key,
            "source_key": project.key,
            "name": project.name,
            "tasks": project.tasks.len(),
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
    let summary = json!({
        "dry_run": args.dry_run,
        "projects": projects,
        "links": data.links.len(),
    });
    if lines.is_empty() {
        lines.push(format!("{verb} nothing"));
    }
    output::written(ctx.json, &summary, &lines.join("\n"))
}

/// Parse an export file: an object with a supported `apm_export` version, then the full shape.
fn parse(text: &str) -> Result<Export> {
    let value: Value =
        serde_json::from_str(text).map_err(|e| err(format!("invalid export file: {e}")))?;
    match value.get("apm_export").map(Value::as_i64) {
        None => Err(err("not an apm export: no apm_export field")),
        Some(Some(VERSION)) => {
            serde_json::from_value(value).map_err(|e| err(format!("invalid export file: {e}")))
        }
        Some(_) => Err(err(format!(
            "unsupported export version {}; this apm reads {VERSION}",
            value["apm_export"]
        ))),
    }
}

/// Validate the file before anything is written: names and values as the commands that
/// create them check them, and every reference (parent, status, milestone, attribute, link
/// end) points at something in the file. Database conflicts (an existing key) are found by
/// the write itself, inside the transaction.
fn check(data: &Export) -> Result<()> {
    let mut keys = HashSet::new();
    // Every task in the file, as KEY-N (the file's keys).
    let mut all_tasks = HashSet::new();
    for p in &data.projects {
        let at = |e: crate::error::AppError| err(format!("project {}: {e}", p.key));
        if db::check_project_key(&p.key)? != p.key {
            return Err(err(format!("project key must be uppercase: {}", p.key)));
        }
        if !keys.insert(p.key.as_str()) {
            return Err(err(format!("project {} appears twice", p.key)));
        }
        if p.name.trim().is_empty() {
            return Err(at(err("project name cannot be empty")));
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
            Category::from_str(&s.category, false)
                .map_err(|_| at(err(format!("invalid status category: {}", s.category))))?;
            if !statuses.insert(s.name.to_ascii_lowercase()) {
                return Err(at(err(format!("status appears twice: {}", s.name))));
            }
        }
        let mut milestones = HashSet::new();
        for m in &p.milestones {
            if milestone::clean_name(&m.name).map_err(at)? != m.name {
                return Err(at(err(format!("invalid milestone name: `{}`", m.name))));
            }
            if !milestones.insert(m.name.as_str()) {
                return Err(at(err(format!("milestone appears twice: {}", m.name))));
            }
            if let Some(due) = &m.due {
                check_due(due).map_err(at)?;
            }
            if m.status != "open" && m.status != "closed" {
                return Err(at(err(format!("invalid milestone status: {}", m.status))));
            }
            check_time(&m.created_at).map_err(at)?;
            check_time(&m.updated_at).map_err(at)?;
        }
        let mut attrs = HashMap::new();
        for a in &p.attributes {
            attr::check_decl(&a.name, &a.kind, &a.options).map_err(at)?;
            if attrs.insert(a.name.as_str(), a).is_some() {
                return Err(at(err(format!("attribute appears twice: {}", a.name))));
            }
            check_time(&a.created_at).map_err(at)?;
        }

        // Task numbers first, so a parent may come after its subtask.
        let mut seqs = HashMap::new();
        for t in &p.tasks {
            let seq = task_seq(&p.key, &t.id).map_err(at)?;
            if seq >= p.next_seq {
                return Err(at(err(format!(
                    "{} is not below next_seq {}",
                    t.id, p.next_seq
                ))));
            }
            if seqs.insert(seq, t.parent.as_deref()).is_some() {
                return Err(at(err(format!("task appears twice: {}", t.id))));
            }
            all_tasks.insert(t.id.as_str());
        }
        for t in &p.tasks {
            let at = |e: crate::error::AppError| err(format!("{}: {e}", t.id));
            if let Some(parent) = &t.parent {
                let seq = task_seq(&p.key, parent).map_err(at)?;
                if !seqs.contains_key(&seq) {
                    return Err(at(err(format!("no such parent in the file: {parent}"))));
                }
            }
            if t.title.trim().is_empty() {
                return Err(at(err("title cannot be empty")));
            }
            if !statuses.contains(&t.status.to_ascii_lowercase()) {
                return Err(at(err(format!("no such status: {}", t.status))));
            }
            Priority::from_str(&t.priority, false)
                .map_err(|_| at(err(format!("invalid priority: {}", t.priority))))?;
            if let Some(due) = &t.due {
                check_due(due).map_err(at)?;
            }
            if let Some(m) = &t.milestone
                && !milestones.contains(m.as_str())
            {
                return Err(at(err(format!("no such milestone: {m}"))));
            }
            for (name, value) in &t.values {
                let a = attrs
                    .get(name.as_str())
                    .ok_or_else(|| at(err(format!("unknown attribute: {name}"))))?;
                attr::normalize_value(name, &a.kind, &a.options, value).map_err(at)?;
            }
            for tag in &t.tags {
                tag::normalize(tag).map_err(at)?;
            }
            check_time(&t.created_at).map_err(at)?;
            check_time(&t.updated_at).map_err(at)?;
            for c in &t.comments {
                check_time(&c.created_at).map_err(at)?;
            }
            for e in &t.events {
                if e.kind.is_empty() {
                    return Err(at(err("event without a kind")));
                }
                check_time(&e.at).map_err(at)?;
            }
        }
        // A parent chain must end: walk up at most as many steps as there are tasks.
        for t in &p.tasks {
            let mut parent = t.parent.as_deref();
            for _ in 0..=p.tasks.len() {
                let Some(id) = parent else { break };
                parent = seqs[&task_seq(&p.key, id)?];
            }
            if parent.is_some() {
                return Err(at(err(format!("parent cycle at {}", t.id))));
            }
        }
    }
    for l in &data.links {
        let name = format!("link {} {} {}", l.from, l.kind, l.to);
        for end in [&l.from, &l.to] {
            if !all_tasks.contains(end.as_str()) {
                return Err(err(format!("{name}: no such task in the file: {end}")));
            }
        }
        super::link::LinkKind::from_str(&l.kind, false)
            .map_err(|_| err(format!("{name}: invalid link kind")))?;
        if l.from == l.to {
            return Err(err(format!("{name}: a task cannot link to itself")));
        }
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
/// `KEY-N` -> new row id) for the links. Returns the project whose alias `key` was, if any.
fn import_project(
    tx: &Connection,
    p: &Project,
    key: &str,
    ids: &mut HashMap<String, i64>,
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
            params![project_id, s.name, s.category, i as i64 + 1],
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
                m.status,
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
                a.kind,
                serde_json::to_string(&a.options)?,
                a.created_at
            ],
        )?;
        attrs.insert(a.name.as_str(), (tx.last_insert_rowid(), a));
    }

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
                t.priority,
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
            let (attribute_id, a) = attrs[name.as_str()];
            let value = attr::normalize_value(name, &a.kind, &a.options, value)?;
            tx.execute(
                "INSERT INTO item_values (item_id, attribute_id, value) VALUES (?1, ?2, ?3)",
                params![item_id, attribute_id, value],
            )?;
        }
        for tag in &t.tags {
            tx.execute(
                "INSERT OR IGNORE INTO tags (item_id, tag) VALUES (?1, ?2)",
                params![item_id, tag::normalize(tag)?],
            )?;
        }
        for c in &t.comments {
            tx.execute(
                "INSERT INTO comments (item_id, author, body, created_at) VALUES (?1, ?2, ?3, ?4)",
                params![item_id, c.author, c.body, c.created_at],
            )?;
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
            std::fs::create_dir_all(&dir)?;
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
    let pruned = match args.keep {
        Some(keep) => prune(&dir, keep as usize)?,
        None => Vec::new(),
    };
    let path = target.display().to_string();
    let pruned: Vec<String> = pruned.iter().map(|p| p.display().to_string()).collect();
    let mut line = path.clone();
    for p in &pruned {
        line.push_str(&format!("\npruned {p}"));
    }
    output::written(ctx.json, &json!({ "path": path, "pruned": pruned }), &line)
}

/// Delete all but the `keep` newest `apm-*.db` files in `dir` (names sort by time).
fn prune(dir: &Path, keep: usize) -> Result<Vec<PathBuf>> {
    let mut backups: Vec<PathBuf> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("apm-") && n.ends_with(".db"))
        })
        .collect();
    backups.sort();
    let old = backups.len().saturating_sub(keep);
    let pruned: Vec<PathBuf> = backups.drain(..old).collect();
    for p in &pruned {
        std::fs::remove_file(p)?;
    }
    Ok(pruned)
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
}
