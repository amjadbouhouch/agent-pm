//! Projects: a KEY, a name and a description. Tasks live in a project.

use clap::{Subcommand, ValueEnum};
use rusqlite::{Connection, Transaction, TransactionBehavior, params};
use serde::{Serialize, Serializer};

use super::Ctx;
use crate::db;
use crate::error::{Result, err};
use crate::model::Status;
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

/// `show`: the project plus its task counts.
#[derive(Serialize)]
struct Detail {
    #[serde(flatten)]
    project: Project,
    counts: Counts,
}

/// Task counts per status, in `Status` order (a JSON object).
struct Counts(Vec<(&'static str, i64)>);

impl Serialize for Counts {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.collect_map(self.0.iter().map(|(k, v)| (k, v)))
    }
}

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
    db::unique(
        ctx.conn.execute(
            "INSERT INTO projects (key, name, description) VALUES (?1, ?2, ?3)",
            params![key, name, description.unwrap_or("")],
        ),
        || err(format!("project already exists: {key}")),
    )?;
    let id = ctx.conn.last_insert_rowid();
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
    let detail = Detail {
        project: get(&ctx.conn, id)?,
        counts: counts(&ctx.conn, id)?,
    };
    if ctx.json {
        return output::json(&detail);
    }
    let project = &detail.project;
    println!("{}  {}", project.key, project.name);
    if !project.description.is_empty() {
        println!("description: {}", project.description);
    }
    let counts: Vec<String> = detail
        .counts
        .0
        .iter()
        .map(|(s, n)| format!("{s}={n}"))
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
        &format!(
            "UPDATE projects SET name = coalesce(?1, name), description = coalesce(?2, description),
             updated_at = {} WHERE id = ?3",
            db::NOW_SQL
        ),
        params![name, description, id],
    )?;
    print_written(ctx, &get(&ctx.conn, id)?, "updated")
}

fn rm(ctx: &Ctx, key: &str) -> Result<()> {
    let tx = Transaction::new_unchecked(&ctx.conn, TransactionBehavior::Immediate)?;
    let id = db::project_id(&tx, key)?;
    let project = get(&tx, id)?;
    // Milestones, tasks, attributes, values and comments go with it (ON DELETE CASCADE).
    tx.execute("DELETE FROM projects WHERE id = ?1", params![id])?;
    tx.commit()?;
    print_written(ctx, &project, "deleted")
}

/// `created APM`, or the project as JSON.
fn print_written(ctx: &Ctx, project: &Project, verb: &str) -> Result<()> {
    output::written(ctx.json, project, &format!("{verb} {}", project.key))
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
fn counts(conn: &Connection, project_id: i64) -> Result<Counts> {
    let mut stmt =
        conn.prepare("SELECT count(*) FROM items WHERE project_id = ?1 AND status = ?2")?;
    let counts = Status::value_variants()
        .iter()
        .map(|s| {
            let n = stmt.query_row(params![project_id, s.as_str()], |r| r.get(0))?;
            Ok((s.as_str(), n))
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Counts(counts))
}
