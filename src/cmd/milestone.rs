use clap::Subcommand;
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use super::Ctx;
use crate::db::{self, EventKind};
use crate::error::{Result, err};
use crate::{model, output};

#[derive(Subcommand, Debug)]
pub enum MilestoneCmd {
    /// Create a milestone in a project
    Add {
        name: String,
        #[arg(short, long)]
        project: String,
        #[arg(short, long)]
        description: Option<String>,
        /// Due date YYYY-MM-DD, or date-time YYYY-MM-DDTHH:MM[:SS] with Z or +HH:MM/-HH:MM
        #[arg(long)]
        due: Option<String>,
    },
    /// List a project's milestones with progress (open ones unless --all)
    List {
        #[arg(short, long)]
        project: String,
        #[arg(long)]
        all: bool,
    },
    /// Show a milestone and its tasks
    Show {
        name: String,
        #[arg(short, long)]
        project: String,
    },
    /// Rename, redescribe or re-date a milestone
    Edit {
        name: String,
        #[arg(short, long)]
        project: String,
        /// New name
        #[arg(long)]
        rename: Option<String>,
        #[arg(short, long)]
        description: Option<String>,
        /// New due date or date-time, as for add ("" clears it)
        #[arg(long)]
        due: Option<String>,
    },
    /// Mark a milestone closed
    Close {
        name: String,
        #[arg(short, long)]
        project: String,
    },
    /// Mark a closed milestone open again
    Reopen {
        name: String,
        #[arg(short, long)]
        project: String,
    },
    /// Delete a milestone (its tasks stay, without a milestone)
    Rm {
        name: String,
        #[arg(short, long)]
        project: String,
    },
}

#[derive(Serialize, Debug)]
pub struct Milestone {
    #[serde(skip)]
    pub id: i64,
    pub name: String,
    pub project: String,
    pub description: String,
    pub due: Option<String>,
    pub status: String,
    pub progress: Progress,
    pub created_at: String,
    pub updated_at: String,
}

/// By status category: `total` counts tasks that are not cancelled; `done` and `cancelled`
/// count tasks in done- and cancelled-category statuses.
#[derive(Serialize, Debug)]
pub struct Progress {
    pub total: i64,
    pub done: i64,
    pub cancelled: i64,
}

#[derive(Serialize, Debug)]
struct MilestoneItem {
    id: String,
    status: String,
    title: String,
    assignee: Option<String>,
}

pub fn run(ctx: &Ctx, cmd: MilestoneCmd) -> Result<()> {
    match cmd {
        MilestoneCmd::Add {
            name,
            project,
            description,
            due,
        } => add(ctx, &project, &name, description, due),
        MilestoneCmd::List { project, all } => list(ctx, &project, all),
        MilestoneCmd::Show { name, project } => show(ctx, &project, &name),
        MilestoneCmd::Edit {
            name,
            project,
            rename,
            description,
            due,
        } => edit(ctx, &project, &name, rename, description, due),
        MilestoneCmd::Close { name, project } => set_status(ctx, &project, &name, "closed"),
        MilestoneCmd::Reopen { name, project } => set_status(ctx, &project, &name, "open"),
        MilestoneCmd::Rm { name, project } => rm(ctx, &project, &name),
    }
}

/// Milestone id for a name within a project. Used by item add/edit/list.
pub fn resolve(conn: &Connection, project_id: i64, name: &str) -> Result<i64> {
    lookup(conn, project_id, name)?.ok_or_else(|| err(format!("no such milestone: {name}")))
}

/// Milestone id for a name within a project, if it has one. Used by `move`.
pub fn lookup(conn: &Connection, project_id: i64, name: &str) -> Result<Option<i64>> {
    Ok(conn
        .query_row(
            "SELECT id FROM milestones WHERE project_id = ?1 AND name = ?2",
            params![project_id, name],
            |r| r.get(0),
        )
        .optional()?)
}

/// A milestone with its progress, by id.
pub fn load(conn: &Connection, id: i64) -> Result<Milestone> {
    Ok(conn.query_row(
        "SELECT m.id, m.name, p.key, m.description, m.due, m.status, m.created_at, m.updated_at,
                COUNT(i.id) - COUNT(CASE WHEN s.category = 'cancelled' THEN 1 END),
                COUNT(CASE WHEN s.category = 'done' THEN 1 END),
                COUNT(CASE WHEN s.category = 'cancelled' THEN 1 END)
         FROM milestones m
         JOIN projects p ON p.id = m.project_id
         LEFT JOIN items i ON i.milestone_id = m.id
         LEFT JOIN statuses s ON s.id = i.status_id
         WHERE m.id = ?1
         GROUP BY m.id",
        params![id],
        |r| {
            Ok(Milestone {
                id: r.get(0)?,
                name: r.get(1)?,
                project: r.get(2)?,
                description: r.get(3)?,
                due: r.get(4)?,
                status: r.get(5)?,
                created_at: r.get(6)?,
                updated_at: r.get(7)?,
                progress: Progress {
                    total: r.get(8)?,
                    done: r.get(9)?,
                    cancelled: r.get(10)?,
                },
            })
        },
    )?)
}

/// Resolve `-p KEY` and a milestone name to its id.
fn find(conn: &Connection, project: &str, name: &str) -> Result<i64> {
    resolve(conn, db::project_id(conn, project)?, name)
}

/// A new milestone name, trimmed; empty is an error.
fn clean_name(name: &str) -> Result<&str> {
    let name = name.trim();
    if name.is_empty() {
        return Err(err("milestone name cannot be empty"));
    }
    Ok(name)
}

/// The UNIQUE(project_id, name) error, for `db::unique`.
fn already_exists(name: &str) -> crate::error::AppError {
    err(format!("milestone already exists: {name}"))
}

/// Print a written milestone: the full object under `--json`, else `<verb> milestone <name>`.
fn written(ctx: &Ctx, id: i64, verb: &str) -> Result<()> {
    let m = load(&ctx.conn, id)?;
    output::written(ctx.json, &m, &format!("{verb} milestone {}", m.name))
}

fn add(
    ctx: &Ctx,
    project: &str,
    name: &str,
    description: Option<String>,
    due: Option<String>,
) -> Result<()> {
    let project_id = db::project_id(&ctx.conn, project)?;
    let name = clean_name(name)?;
    let due = due.as_deref().map(model::normalize_when).transpose()?;
    db::unique(
        ctx.conn.execute(
            "INSERT INTO milestones (project_id, name, description, due) VALUES (?1, ?2, ?3, ?4)",
            params![project_id, name, description.unwrap_or_default(), due],
        ),
        || already_exists(name),
    )?;
    written(ctx, ctx.conn.last_insert_rowid(), "created")
}

fn list(ctx: &Ctx, project: &str, all: bool) -> Result<()> {
    let project_id = db::project_id(&ctx.conn, project)?;
    let mut stmt = ctx.conn.prepare(
        "SELECT id FROM milestones WHERE project_id = ?1 AND (?2 OR status = 'open')
         ORDER BY due IS NULL, due, id",
    )?;
    let ids = stmt
        .query_map(params![project_id, all], |r| r.get::<_, i64>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let milestones = ids
        .into_iter()
        .map(|id| load(&ctx.conn, id))
        .collect::<Result<Vec<_>>>()?;
    if ctx.json {
        return output::json(&milestones);
    }
    let rows: Vec<Vec<String>> = milestones
        .iter()
        .map(|m| {
            vec![
                m.name.clone(),
                m.status.clone(),
                m.due.clone().unwrap_or_else(|| "-".into()),
                format!("{}/{}", m.progress.done, m.progress.total),
            ]
        })
        .collect();
    output::table(&["NAME", "STATUS", "DUE", "DONE"], &rows);
    Ok(())
}

fn show(ctx: &Ctx, project: &str, name: &str) -> Result<()> {
    let m = load(&ctx.conn, find(&ctx.conn, project, name)?)?;
    let mut stmt = ctx.conn.prepare(
        "SELECT i.seq, s.name, i.title, i.assignee FROM items i
         JOIN statuses s ON s.id = i.status_id
         WHERE i.milestone_id = ?1 ORDER BY i.seq",
    )?;
    let items = stmt
        .query_map(params![m.id], |r| {
            Ok(MilestoneItem {
                id: db::item_key(&m.project, r.get(0)?),
                status: r.get(1)?,
                title: r.get(2)?,
                assignee: r.get(3)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if ctx.json {
        let mut v = serde_json::to_value(&m)?;
        v["items"] = serde_json::to_value(&items)?;
        return output::json(&v);
    }
    println!("{} ({}, {})", m.name, m.project, m.status);
    if let Some(due) = &m.due {
        println!("due: {due}");
    }
    println!("progress: {}/{} done", m.progress.done, m.progress.total);
    if !m.description.is_empty() {
        println!("{}", m.description);
    }
    let rows: Vec<Vec<String>> = items
        .into_iter()
        .map(|i| {
            vec![
                i.id,
                i.status,
                i.assignee.unwrap_or_else(|| "-".into()),
                i.title,
            ]
        })
        .collect();
    if !rows.is_empty() {
        println!();
    }
    output::table(&["ID", "STATUS", "ASSIGNEE", "TITLE"], &rows);
    Ok(())
}

fn edit(
    ctx: &Ctx,
    project: &str,
    name: &str,
    rename: Option<String>,
    description: Option<String>,
    due: Option<String>,
) -> Result<()> {
    if rename.is_none() && description.is_none() && due.is_none() {
        return Err(err(
            "nothing to change: pass --rename, --description or --due",
        ));
    }
    let project_id = db::project_id(&ctx.conn, project)?;
    let id = resolve(&ctx.conn, project_id, name)?;
    let rename = rename.as_deref().map(clean_name).transpose()?;
    // Some("") clears the due date.
    let due = due.map(|d| {
        if d.is_empty() {
            Ok(None)
        } else {
            model::normalize_when(&d).map(Some)
        }
    });
    let due = due.transpose()?;
    db::unique(
        ctx.conn.execute(
            &format!(
                "UPDATE milestones SET
                    name = COALESCE(?2, name),
                    description = COALESCE(?3, description),
                    due = CASE WHEN ?4 THEN ?5 ELSE due END,
                    updated_at = {}
                 WHERE id = ?1",
                db::NOW_SQL
            ),
            params![id, rename, description, due.is_some(), due.flatten()],
        ),
        || already_exists(rename.unwrap_or(name)),
    )?;
    written(ctx, id, "updated")
}

/// Close or reopen. Setting the status a milestone already has is a no-op, not an error.
fn set_status(ctx: &Ctx, project: &str, name: &str, status: &str) -> Result<()> {
    let id = find(&ctx.conn, project, name)?;
    ctx.conn.execute(
        &format!(
            "UPDATE milestones SET status = ?2, updated_at = {} WHERE id = ?1 AND status != ?2",
            db::NOW_SQL
        ),
        params![id, status],
    )?;
    written(
        ctx,
        id,
        if status == "closed" {
            "closed"
        } else {
            "reopened"
        },
    )
}

/// Delete a milestone. Its tasks stay, with no milestone; each records that change.
fn rm(ctx: &Ctx, project: &str, name: &str) -> Result<()> {
    let tx = db::write_tx(&ctx.conn)?;
    let m = load(&tx, find(&tx, project, name)?)?;
    let mut stmt = tx.prepare("SELECT id FROM items WHERE milestone_id = ?1")?;
    let ids = stmt
        .query_map(params![m.id], |r| r.get(0))?
        .collect::<rusqlite::Result<Vec<i64>>>()?;
    drop(stmt);
    for item_id in ids {
        let field = Some("milestone");
        db::record_event(
            &tx,
            item_id,
            &ctx.actor,
            EventKind::Changed,
            field,
            Some(&m.name),
            None,
        )?;
        tx.execute(
            &format!(
                "UPDATE items SET updated_at = {} WHERE id = ?1",
                db::NOW_SQL
            ),
            params![item_id],
        )?;
    }
    // ON DELETE SET NULL clears the tasks' milestone.
    tx.execute("DELETE FROM milestones WHERE id = ?1", params![m.id])?;
    tx.commit()?;
    output::written(ctx.json, &m, &format!("removed milestone {}", m.name))
}
