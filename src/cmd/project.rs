//! Projects: a KEY, a name and a description. Tasks live in a project.

use clap::Subcommand;
use rusqlite::{Connection, params};
use serde::Serialize;
use serde_json::{Map, Value};

use super::Ctx;
use crate::db;
use crate::error::{Result, err};
use crate::output;

#[derive(Subcommand, Debug)]
pub enum ProjectCmd {
    /// Create a project with a short KEY (letters/digits, e.g. APM) used in task ids
    Add {
        key: String,
        name: String,
        #[arg(short, long)]
        description: Option<String>,
    },
    /// List projects
    List,
    /// Show a project with its task counts
    Show { key: String },
    /// Rename or redescribe a project
    Edit {
        key: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(short, long)]
        description: Option<String>,
    },
    /// Delete a project and everything in it
    Rm { key: String },
}

#[derive(Serialize, Debug)]
pub struct Project {
    pub key: String,
    pub name: String,
    pub description: String,
    pub created_at: String,
    pub updated_at: String,
}

const STATUSES: [&str; 5] = ["todo", "doing", "blocked", "done", "cancelled"];

pub fn run(ctx: &Ctx, cmd: ProjectCmd) -> Result<()> {
    match cmd {
        ProjectCmd::Add {
            key,
            name,
            description,
        } => add(ctx, &key, &name, description.as_deref()),
        ProjectCmd::List => list(ctx),
        ProjectCmd::Show { key } => show(ctx, &key),
        ProjectCmd::Edit {
            key,
            name,
            description,
        } => edit(ctx, &key, name.as_deref(), description.as_deref()),
        ProjectCmd::Rm { key } => rm(ctx, &key),
    }
}

fn check_name(name: &str) -> Result<()> {
    if name.trim().is_empty() {
        return Err(err("project name cannot be empty"));
    }
    Ok(())
}

fn add(ctx: &Ctx, key: &str, name: &str, description: Option<&str>) -> Result<()> {
    let key = db::check_project_key(key)?;
    check_name(name)?;
    if db::project_id(&ctx.conn, &key).is_ok() {
        return Err(err(format!("project already exists: {key}")));
    }
    ctx.conn.execute(
        "INSERT INTO projects (key, name, description) VALUES (?1, ?2, ?3)",
        params![key, name, description.unwrap_or("")],
    )?;
    let id = db::project_id(&ctx.conn, &key)?;
    print_written(ctx, &get(&ctx.conn, id)?, "created")
}

fn list(ctx: &Ctx) -> Result<()> {
    let mut stmt = ctx.conn.prepare("SELECT id FROM projects ORDER BY key")?;
    let ids = stmt
        .query_map([], |r| r.get::<_, i64>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let projects = ids
        .into_iter()
        .map(|id| get(&ctx.conn, id))
        .collect::<Result<Vec<_>>>()?;
    if ctx.json {
        return output::json(&projects);
    }
    let rows: Vec<Vec<String>> = projects
        .into_iter()
        .map(|p| vec![p.key, p.name, p.description])
        .collect();
    output::table(&["KEY", "NAME", "DESCRIPTION"], &rows);
    Ok(())
}

fn show(ctx: &Ctx, key: &str) -> Result<()> {
    let id = db::project_id(&ctx.conn, key)?;
    let project = get(&ctx.conn, id)?;
    let counts = counts(&ctx.conn, id)?;
    if ctx.json {
        let mut v = serde_json::to_value(&project)?;
        v["counts"] = Value::Object(counts);
        return output::json(&v);
    }
    println!("{}  {}", project.key, project.name);
    if !project.description.is_empty() {
        println!("description: {}", project.description);
    }
    let counts: Vec<String> = STATUSES
        .iter()
        .map(|s| format!("{s}={}", counts[*s]))
        .collect();
    println!("tasks: {}", counts.join(" "));
    println!("created: {}", project.created_at);
    println!("updated: {}", project.updated_at);
    Ok(())
}

fn edit(ctx: &Ctx, key: &str, name: Option<&str>, description: Option<&str>) -> Result<()> {
    let id = db::project_id(&ctx.conn, key)?;
    if name.is_none() && description.is_none() {
        return Err(err("nothing to change: pass --name or --description"));
    }
    if let Some(name) = name {
        check_name(name)?;
    }
    ctx.conn.execute(
        "UPDATE projects SET name = coalesce(?1, name), description = coalesce(?2, description),
         updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now') WHERE id = ?3",
        params![name, description, id],
    )?;
    print_written(ctx, &get(&ctx.conn, id)?, "updated")
}

fn rm(ctx: &Ctx, key: &str) -> Result<()> {
    let id = db::project_id(&ctx.conn, key)?;
    let project = get(&ctx.conn, id)?;
    // Milestones, tasks, attributes, values and comments go with it (ON DELETE CASCADE).
    ctx.conn
        .execute("DELETE FROM projects WHERE id = ?1", params![id])?;
    print_written(ctx, &project, "deleted")
}

/// `created APM`, or the project as JSON.
fn print_written(ctx: &Ctx, project: &Project, verb: &str) -> Result<()> {
    if ctx.json {
        return output::json(project);
    }
    println!("{verb} {}", project.key);
    Ok(())
}

/// A project by id.
pub fn get(conn: &Connection, id: i64) -> Result<Project> {
    Ok(conn.query_row(
        "SELECT key, name, description, created_at, updated_at FROM projects WHERE id = ?1",
        params![id],
        |r| {
            Ok(Project {
                key: r.get(0)?,
                name: r.get(1)?,
                description: r.get(2)?,
                created_at: r.get(3)?,
                updated_at: r.get(4)?,
            })
        },
    )?)
}

/// Task counts (subtasks included) for every status, zeros included.
fn counts(conn: &Connection, project_id: i64) -> Result<Map<String, Value>> {
    let mut counts: Map<String, Value> =
        STATUSES.iter().map(|s| (s.to_string(), 0.into())).collect();
    let mut stmt =
        conn.prepare("SELECT status, count(*) FROM items WHERE project_id = ?1 GROUP BY status")?;
    let rows = stmt.query_map(params![project_id], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
    })?;
    for row in rows {
        let (status, n) = row?;
        counts.insert(status, n.into());
    }
    Ok(counts)
}
