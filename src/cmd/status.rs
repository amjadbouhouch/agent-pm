//! Custom statuses: per-project rows, ordered by position, each in a category.
//! Tasks reference a status by id, so renaming a status renames it on every task.

use clap::Subcommand;
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use super::{Ctx, claim, project};
use crate::db::{self, EventKind, write_tx};
use crate::error::{AppError, Result, err};
use crate::model::Category;
use crate::output;

#[derive(Subcommand, Debug)]
pub enum StatusCmd {
    /// List a project's statuses in order, with task counts
    List {
        #[arg(short, long)]
        project: String,
    },
    /// Add a status to a project (at the end unless --position)
    Add {
        name: String,
        #[arg(short, long)]
        project: String,
        #[arg(long, value_enum)]
        category: Category,
        /// 1-based place in the order; later statuses shift down
        #[arg(long)]
        position: Option<i64>,
    },
    /// Rename, recategorize or reorder a status
    Edit {
        name: String,
        #[arg(short, long)]
        project: String,
        /// New name (tasks follow)
        #[arg(long)]
        rename: Option<String>,
        #[arg(long, value_enum)]
        category: Option<Category>,
        /// New 1-based place in the order
        #[arg(long)]
        position: Option<i64>,
    },
    /// Delete a status; --to moves its tasks to another status first
    Rm {
        name: String,
        #[arg(short, long)]
        project: String,
        /// Status that takes over this status's tasks
        #[arg(long)]
        to: Option<String>,
    },
}

/// A status as printed: the `--json` status contract.
#[derive(Serialize, Debug)]
pub struct Status {
    #[serde(skip)]
    pub id: i64,
    pub name: String,
    pub project: String,
    pub category: String,
    pub position: i64,
    /// Tasks (subtasks included) in this status.
    pub tasks: i64,
}

/// The statuses a new project starts with, in order.
pub const DEFAULTS: &[(&str, Category)] = &[
    ("todo", Category::Todo),
    ("doing", Category::Active),
    ("blocked", Category::Active),
    ("done", Category::Done),
    ("cancelled", Category::Cancelled),
];

/// What `start`, `done` and `block` move a task to.
#[derive(Clone, Copy, Debug)]
pub enum Shortcut {
    /// The first `active` status by position.
    Start,
    /// The first `done` status by position.
    Done,
    /// The status named `blocked`.
    Block,
}

pub fn run(ctx: &Ctx, cmd: StatusCmd) -> Result<()> {
    match cmd {
        StatusCmd::List { project } => list(ctx, &project),
        StatusCmd::Add {
            name,
            project,
            category,
            position,
        } => add(ctx, &project, &name, category, position),
        StatusCmd::Edit {
            name,
            project,
            rename,
            category,
            position,
        } => edit(ctx, &project, &name, rename.as_deref(), category, position),
        StatusCmd::Rm { name, project, to } => rm(ctx, &project, &name, to.as_deref()),
    }
}

/// Give a new project the default statuses. Used by `project add`.
pub fn seed(conn: &Connection, project_id: i64) -> Result<()> {
    let mut stmt = conn.prepare(
        "INSERT INTO statuses (project_id, name, category, position) VALUES (?1, ?2, ?3, ?4)",
    )?;
    for (i, (name, category)) in DEFAULTS.iter().enumerate() {
        stmt.execute(params![project_id, name, category.as_str(), i as i64 + 1])?;
    }
    Ok(())
}

/// Status id for a name within a project, case-insensitive. Used by item add/edit/list.
pub fn resolve(conn: &Connection, project_id: i64, name: &str) -> Result<i64> {
    let name = name.trim();
    find(conn, project_id, name)?.ok_or_else(|| err(format!("no such status: {name}")))
}

/// Status id for a name within a project, case-insensitive, if it has one. Used by `move`.
pub fn find(conn: &Connection, project_id: i64, name: &str) -> Result<Option<i64>> {
    Ok(conn
        .query_row(
            "SELECT id FROM statuses WHERE project_id = ?1 AND name = ?2 COLLATE NOCASE",
            params![project_id, name],
            |r| r.get(0),
        )
        .optional()?)
}

/// The name, trimmed, if some project has a status by that name (case-insensitive); else a
/// `no such status` error. Used by `list --status` without a project.
pub fn check_known_name(conn: &Connection, name: &str) -> Result<String> {
    let name = name.trim();
    let known: bool = conn.query_row(
        "SELECT count(*) > 0 FROM statuses WHERE name = ?1 COLLATE NOCASE",
        params![name],
        |r| r.get(0),
    )?;
    if !known {
        return Err(err(format!("no such status: {name}")));
    }
    Ok(name.to_string())
}

/// The status a new task gets: the project's first `todo`-category status.
pub fn default_id(conn: &Connection, project_id: i64) -> Result<i64> {
    first(conn, project_id, Category::Todo)
}

/// The status a shortcut moves a task of this project to.
pub fn for_shortcut(conn: &Connection, project_id: i64, shortcut: Shortcut) -> Result<i64> {
    match shortcut {
        Shortcut::Start => first(conn, project_id, Category::Active),
        Shortcut::Done => first(conn, project_id, Category::Done),
        Shortcut::Block => find(conn, project_id, "blocked")?.ok_or_else(|| {
            err(format!(
                "project {} has no status named blocked",
                project_key(conn, project_id)
            ))
        }),
    }
}

/// The first status of a category by position, if the project has one. Used by `next`.
pub fn first_in(conn: &Connection, project_id: i64, category: Category) -> Result<Option<i64>> {
    Ok(conn
        .query_row(
            "SELECT id FROM statuses WHERE project_id = ?1 AND category = ?2
             ORDER BY position, id LIMIT 1",
            params![project_id, category.as_str()],
            |r| r.get(0),
        )
        .optional()?)
}

/// The first status of a category by position.
fn first(conn: &Connection, project_id: i64, category: Category) -> Result<i64> {
    first_in(conn, project_id, category)?.ok_or_else(|| {
        err(format!(
            "project {} has no {}-category status",
            project_key(conn, project_id),
            category.as_str()
        ))
    })
}

/// A project's key for messages.
fn project_key(conn: &Connection, project_id: i64) -> String {
    project::get(conn, project_id).map_or_else(|_| project_id.to_string(), |p| p.key)
}

/// A status with its task count, by id.
pub fn load(conn: &Connection, id: i64) -> Result<Status> {
    Ok(conn.query_row(
        "SELECT s.id, s.name, p.key, s.category, s.position,
                (SELECT count(*) FROM items i WHERE i.status_id = s.id)
         FROM statuses s JOIN projects p ON p.id = s.project_id
         WHERE s.id = ?1",
        params![id],
        |r| {
            Ok(Status {
                id: r.get(0)?,
                name: r.get(1)?,
                project: r.get(2)?,
                category: r.get(3)?,
                position: r.get(4)?,
                tasks: r.get(5)?,
            })
        },
    )?)
}

/// A project's status ids in order.
fn ordered_ids(conn: &Connection, project_id: i64) -> Result<Vec<i64>> {
    let mut stmt =
        conn.prepare("SELECT id FROM statuses WHERE project_id = ?1 ORDER BY position, id")?;
    let ids = stmt
        .query_map(params![project_id], |r| r.get(0))?
        .collect::<rusqlite::Result<Vec<i64>>>()?;
    Ok(ids)
}

/// Put status `id` at 1-based `position` (None: keep its place) and renumber the project's
/// statuses 1..n, so positions stay dense. Positions past the end mean "last".
fn place(conn: &Connection, project_id: i64, id: i64, position: Option<i64>) -> Result<()> {
    let mut ids = ordered_ids(conn, project_id)?;
    if let Some(position) = position {
        ids.retain(|&i| i != id);
        let at = usize::try_from(position - 1).unwrap_or(0).min(ids.len());
        ids.insert(at, id);
    }
    let mut stmt = conn.prepare("UPDATE statuses SET position = ?1 WHERE id = ?2")?;
    for (i, id) in ids.iter().enumerate() {
        stmt.execute(params![i as i64 + 1, id])?;
    }
    Ok(())
}

/// A new status name, trimmed. Commas are reserved: `list --status a,b` splits on them.
pub fn clean_name(name: &str) -> Result<&str> {
    let name = name.trim();
    if name.is_empty() {
        return Err(err("status name cannot be empty"));
    }
    if name.contains(',') {
        return Err(err(format!("status name may not contain `,`: {name}")));
    }
    Ok(name)
}

fn check_position(position: Option<i64>) -> Result<()> {
    match position {
        Some(p) if p < 1 => Err(err(format!("invalid position {p}: must be 1 or more"))),
        _ => Ok(()),
    }
}

fn already_exists(name: &str) -> AppError {
    err(format!("status already exists: {name}"))
}

fn list(ctx: &Ctx, project: &str) -> Result<()> {
    let project_id = db::project_id(&ctx.conn, project)?;
    let statuses = ordered_ids(&ctx.conn, project_id)?
        .into_iter()
        .map(|id| load(&ctx.conn, id))
        .collect::<Result<Vec<_>>>()?;
    if ctx.json {
        return output::json(&statuses);
    }
    let rows: Vec<Vec<String>> = statuses
        .into_iter()
        .map(|s| vec![s.name, s.category, s.tasks.to_string()])
        .collect();
    output::table(&["NAME", "CATEGORY", "TASKS"], &rows);
    Ok(())
}

fn add(
    ctx: &Ctx,
    project: &str,
    name: &str,
    category: Category,
    position: Option<i64>,
) -> Result<()> {
    let name = clean_name(name)?;
    check_position(position)?;
    let tx = write_tx(&ctx.conn)?;
    let project_id = db::project_id(&tx, project)?;
    // Appended last, then moved into place.
    db::unique(
        tx.execute(
            "INSERT INTO statuses (project_id, name, category, position)
             SELECT ?1, ?2, ?3, coalesce(max(position), 0) + 1 FROM statuses WHERE project_id = ?1",
            params![project_id, name, category.as_str()],
        ),
        || already_exists(name),
    )?;
    let id = tx.last_insert_rowid();
    place(&tx, project_id, id, position)?;
    tx.commit()?;
    let s = load(&ctx.conn, id)?;
    output::written(ctx.json, &s, &format!("added status {}", s.name))
}

fn edit(
    ctx: &Ctx,
    project: &str,
    name: &str,
    rename: Option<&str>,
    category: Option<Category>,
    position: Option<i64>,
) -> Result<()> {
    if rename.is_none() && category.is_none() && position.is_none() {
        return Err(err(
            "nothing to change: pass --rename, --category or --position",
        ));
    }
    let rename = rename.map(clean_name).transpose()?;
    check_position(position)?;
    let tx = write_tx(&ctx.conn)?;
    let project_id = db::project_id(&tx, project)?;
    let id = resolve(&tx, project_id, name)?;
    db::unique(
        tx.execute(
            "UPDATE statuses SET name = coalesce(?2, name), category = coalesce(?3, category)
             WHERE id = ?1",
            params![id, rename, category.map(Category::as_str)],
        ),
        || already_exists(rename.unwrap_or(name)),
    )?;
    place(&tx, project_id, id, position)?;
    if category.is_some_and(Category::is_closed) {
        // Its tasks are closed now: they keep no claims.
        let mut stmt = tx.prepare("SELECT id FROM items WHERE status_id = ?1")?;
        let ids = stmt
            .query_map(params![id], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<i64>>>()?;
        drop(stmt);
        for item_id in ids {
            if claim::release_if_closed(&tx, &ctx.actor, item_id)? {
                claim::touch(&tx, item_id)?;
            }
        }
    }
    tx.commit()?;
    let s = load(&ctx.conn, id)?;
    output::written(ctx.json, &s, &format!("updated status {}", s.name))
}

fn rm(ctx: &Ctx, project: &str, name: &str, to: Option<&str>) -> Result<()> {
    let tx = write_tx(&ctx.conn)?;
    let project_id = db::project_id(&tx, project)?;
    let id = resolve(&tx, project_id, name)?;
    let s = load(&tx, id)?;
    if ordered_ids(&tx, project_id)?.len() == 1 {
        return Err(err(format!(
            "cannot remove the last status of {}",
            s.project
        )));
    }
    let to = to.map(|t| resolve(&tx, project_id, t)).transpose()?;
    if to == Some(id) {
        return Err(err(format!(
            "--to must name another status than {}",
            s.name
        )));
    }
    let mut line = format!("removed status {}", s.name);
    match to {
        None if s.tasks > 0 => {
            return Err(err(format!(
                "status {} is used by {} task(s); pass --to <status>",
                s.name, s.tasks
            )));
        }
        None => {}
        Some(to) => {
            let target = load(&tx, to)?;
            move_tasks(&tx, &ctx.actor, &s, &target)?;
            if s.tasks > 0 {
                line = format!("{line} ({} task(s) moved to {})", s.tasks, target.name);
            }
        }
    }
    tx.execute("DELETE FROM statuses WHERE id = ?1", params![id])?;
    // Close the gap it leaves in the order.
    place(&tx, project_id, id, None)?;
    tx.commit()?;
    output::written(ctx.json, &s, &line)
}

/// Move every task from one status to another, recording each change.
fn move_tasks(tx: &Connection, actor: &str, from: &Status, to: &Status) -> Result<()> {
    let mut stmt = tx.prepare("SELECT id FROM items WHERE status_id = ?1")?;
    let ids = stmt
        .query_map(params![from.id], |r| r.get(0))?
        .collect::<rusqlite::Result<Vec<i64>>>()?;
    for item_id in ids {
        tx.execute(
            &format!(
                "UPDATE items SET status_id = ?1, updated_at = {} WHERE id = ?2",
                db::NOW_SQL
            ),
            params![to.id, item_id],
        )?;
        db::record_event(
            tx,
            item_id,
            actor,
            EventKind::Changed,
            Some("status"),
            Some(&from.name),
            Some(&to.name),
        )?;
        claim::release_if_closed(tx, actor, item_id)?;
    }
    Ok(())
}
