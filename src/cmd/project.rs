//! Projects: a KEY, a name and a description. Tasks live in a project.

use clap::Subcommand;
use rusqlite::{Connection, params};
use serde::{Serialize, Serializer};

use super::{Ctx, status};
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
    /// Rename, redescribe or re-key a project (the old key keeps working)
    Edit {
        key: String,
        /// New KEY; task ids follow (APM-3 becomes OPS-3), old ids keep resolving
        #[arg(long = "key", value_name = "NEW")]
        new_key: Option<String>,
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

/// Task counts per status, in the project's status order (a JSON object).
struct Counts(Vec<(String, i64)>);

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
            new_key,
            name,
            description,
        } => edit(
            ctx,
            &key,
            new_key.as_deref(),
            name.as_deref(),
            description.as_deref(),
        ),
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
    let tx = db::write_tx(&ctx.conn)?;
    let claimed = db::claim_project_key(&tx, &key, None)?;
    db::unique(
        tx.execute(
            "INSERT INTO projects (key, name, description) VALUES (?1, ?2, ?3)",
            params![key, name, description.unwrap_or("")],
        ),
        || err(format!("project already exists: {key}")),
    )?;
    let id = tx.last_insert_rowid();
    status::seed(&tx, id)?;
    tx.commit()?;
    written(ctx, id, "created", claimed)
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

fn edit(
    ctx: &Ctx,
    key: &str,
    new_key: Option<&str>,
    name: Option<&str>,
    description: Option<&str>,
) -> Result<()> {
    if new_key.is_none() && name.is_none() && description.is_none() {
        return Err(err(
            "nothing to change: pass --key, --name or --description",
        ));
    }
    let new_key = new_key.map(db::check_project_key).transpose()?;
    if let Some(name) = name {
        check_name(name)?;
    }
    let tx = db::write_tx(&ctx.conn)?;
    let id = db::project_id(&tx, key)?;
    let old_key = get(&tx, id)?.key;
    let mut claimed = None;
    if let Some(new_key) = new_key.as_deref().filter(|k| *k != old_key) {
        // The new key stops being an alias (of this or another project); the old one becomes
        // one, so APM-3 keeps finding what is now OPS-3.
        claimed = db::claim_project_key(&tx, new_key, Some(id))?;
        db::unique(
            tx.execute(
                "UPDATE projects SET key = ?1 WHERE id = ?2",
                params![new_key, id],
            ),
            || err(format!("project already exists: {new_key}")),
        )?;
        tx.execute(
            "INSERT INTO project_aliases (key, project_id) VALUES (?1, ?2)",
            params![old_key, id],
        )?;
    }
    tx.execute(
        &format!(
            "UPDATE projects SET name = coalesce(?1, name), description = coalesce(?2, description),
             updated_at = {} WHERE id = ?3",
            db::NOW_SQL
        ),
        params![name, description, id],
    )?;
    tx.commit()?;
    written(ctx, id, "updated", claimed)
}

/// Print a created or re-keyed project. When its key was another project's alias, say so:
/// that project's old ids under this key no longer resolve.
fn written(ctx: &Ctx, id: i64, verb: &str, claimed: Option<String>) -> Result<()> {
    let project = get(&ctx.conn, id)?;
    let mut line = format!("{verb} {}", project.key);
    let mut value = serde_json::to_value(&project)?;
    if let Some(from) = claimed {
        line.push_str(&format!(
            " (key was an alias of {from}; its old ids no longer resolve)"
        ));
        value["claimed_alias_from"] = from.into();
    }
    output::written(ctx.json, &value, &line)
}

fn rm(ctx: &Ctx, key: &str) -> Result<()> {
    let tx = db::write_tx(&ctx.conn)?;
    let id = db::project_id(&tx, key)?;
    let project = get(&tx, id)?;
    // Statuses, milestones, tasks and everything on them go with it (ON DELETE CASCADE).
    tx.execute("DELETE FROM projects WHERE id = ?1", params![id])?;
    tx.commit()?;
    output::written(ctx.json, &project, &format!("deleted {}", project.key))
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

/// Task counts (subtasks included) for every status of the project, zeros included.
fn counts(conn: &Connection, project_id: i64) -> Result<Counts> {
    let mut stmt = conn.prepare(
        "SELECT s.name, count(i.id) FROM statuses s
         LEFT JOIN items i ON i.status_id = s.id
         WHERE s.project_id = ?1
         GROUP BY s.id ORDER BY s.position, s.id",
    )?;
    let counts = stmt
        .query_map(params![project_id], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(Counts(counts))
}
