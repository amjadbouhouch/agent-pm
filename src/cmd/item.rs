//! Tasks and subtasks ("items"), referenced as KEY-N.

use clap::{Args, ValueEnum};
use rusqlite::types::Value as SqlValue;
use rusqlite::{Connection, Transaction, TransactionBehavior, params, params_from_iter};
use serde::Serialize;
use serde_json::{Map, Value};

use super::status::{self, Shortcut};
use super::{Ctx, attr, comment, milestone};
use crate::db;
use crate::error::{Result, err};
use crate::model::{Category, Priority, check_date, parse_pair};
use crate::output;

#[derive(Args, Debug)]
pub struct KeyArg {
    /// Task id, e.g. APM-12
    pub key: String,
}

#[derive(Args, Debug)]
pub struct AddArgs {
    pub title: String,
    /// Project key (optional with --parent: the parent's project is used)
    #[arg(short, long)]
    pub project: Option<String>,
    /// Make this a subtask of KEY-N
    #[arg(long)]
    pub parent: Option<String>,
    /// Milestone name in the same project
    #[arg(short, long)]
    pub milestone: Option<String>,
    #[arg(short, long)]
    pub assignee: Option<String>,
    #[arg(long, value_enum)]
    pub priority: Option<Priority>,
    /// A status of the project (default: its first todo-category status)
    #[arg(long)]
    pub status: Option<String>,
    /// Due date, YYYY-MM-DD
    #[arg(long)]
    pub due: Option<String>,
    /// Longer description
    #[arg(short, long)]
    pub body: Option<String>,
    /// Custom attribute value, name=value (repeatable)
    #[arg(long = "set", value_name = "NAME=VALUE", value_parser = parse_pair)]
    pub set: Vec<(String, String)>,
}

#[derive(Args, Debug)]
pub struct ListArgs {
    #[arg(short, long)]
    pub project: Option<String>,
    /// Only these status names (comma-separated); implies closed ones may show
    #[arg(long, value_delimiter = ',')]
    pub status: Vec<String>,
    #[arg(short, long)]
    pub assignee: Option<String>,
    #[arg(short, long)]
    pub milestone: Option<String>,
    /// Only subtasks of KEY-N
    #[arg(long)]
    pub parent: Option<String>,
    /// Custom attribute filter, name=value (repeatable, all must match)
    #[arg(long = "where", value_name = "NAME=VALUE", value_parser = parse_pair)]
    pub filter: Vec<(String, String)>,
    /// Include done and cancelled tasks
    #[arg(long)]
    pub all: bool,
}

#[derive(Args, Debug)]
pub struct EditArgs {
    pub key: String,
    #[arg(long)]
    pub title: Option<String>,
    #[arg(short, long)]
    pub body: Option<String>,
    /// A status of the task's project
    #[arg(long)]
    pub status: Option<String>,
    #[arg(long, value_enum)]
    pub priority: Option<Priority>,
    #[arg(short, long)]
    pub assignee: Option<String>,
    #[arg(long)]
    pub due: Option<String>,
    #[arg(short, long)]
    pub milestone: Option<String>,
    /// Move under KEY-N (same project)
    #[arg(long)]
    pub parent: Option<String>,
    #[arg(long = "set", value_name = "NAME=VALUE", value_parser = parse_pair)]
    pub set: Vec<(String, String)>,
    /// Clear a field: assignee, due, milestone, parent, body, or a custom attribute name (repeatable)
    #[arg(long, value_name = "FIELD")]
    pub unset: Vec<String>,
}

/// A task as printed: the `--json` item contract.
#[derive(Serialize, Debug)]
pub struct Item {
    /// Row id, for lookups; not part of the output.
    #[serde(skip)]
    pub row_id: i64,
    pub id: String,
    pub project: String,
    pub title: String,
    pub body: String,
    /// The status name.
    pub status: String,
    pub status_category: String,
    pub priority: String,
    pub assignee: Option<String>,
    pub due: Option<String>,
    pub milestone: Option<String>,
    pub parent: Option<String>,
    pub created_by: String,
    pub created_at: String,
    pub updated_at: String,
    pub attributes: Map<String, Value>,
}

/// `show`: the item plus its subtasks and comments.
#[derive(Serialize)]
struct Detail {
    #[serde(flatten)]
    item: Item,
    subtasks: Vec<Item>,
    comments: Vec<comment::Comment>,
}

const SELECT: &str = "
    SELECT i.id, p.key || '-' || i.seq, p.key, i.title, i.body, s.name, s.category, i.priority,
           i.assignee, i.due, m.name, p.key || '-' || par.seq,
           i.created_by, i.created_at, i.updated_at
    FROM items i
    JOIN projects p ON p.id = i.project_id
    JOIN statuses s ON s.id = i.status_id
    LEFT JOIN milestones m ON m.id = i.milestone_id
    LEFT JOIN items par ON par.id = i.parent_id";

pub fn add(ctx: &Ctx, args: AddArgs) -> Result<()> {
    check_title(&args.title)?;
    let tx = write_tx(&ctx.conn)?;
    let parent = args
        .parent
        .as_deref()
        .map(|p| db::item(&tx, p))
        .transpose()?;
    let project_id = match (args.project.as_deref(), parent) {
        (Some(key), Some(parent)) => {
            let id = db::project_id(&tx, key)?;
            if id != parent.project_id {
                return Err(err(format!(
                    "parent {} is not in project {}",
                    canonical(args.parent.as_deref().unwrap_or_default()),
                    key.to_uppercase()
                )));
            }
            id
        }
        (Some(key), None) => db::project_id(&tx, key)?,
        (None, Some(parent)) => parent.project_id,
        (None, None) => {
            return Err(err("a project is required: pass -p KEY or --parent KEY-N"));
        }
    };
    if let Some(due) = &args.due {
        check_date(due)?;
    }
    let milestone_id = args
        .milestone
        .as_deref()
        .map(|m| milestone::resolve(&tx, project_id, m))
        .transpose()?;
    let status_id = match &args.status {
        Some(name) => status::resolve(&tx, project_id, name)?,
        None => status::default_id(&tx, project_id)?,
    };
    let seq: i64 = tx.query_row(
        "UPDATE projects SET next_seq = next_seq + 1 WHERE id = ?1 RETURNING next_seq - 1",
        params![project_id],
        |r| r.get(0),
    )?;
    tx.execute(
        "INSERT INTO items (project_id, seq, parent_id, milestone_id, title, body, status_id,
                            priority, assignee, due, created_by)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            project_id,
            seq,
            parent.map(|p| p.id),
            milestone_id,
            args.title,
            args.body.as_deref().unwrap_or(""),
            status_id,
            args.priority.unwrap_or(Priority::None).as_str(),
            args.assignee,
            args.due,
            ctx.actor,
        ],
    )?;
    let id = tx.last_insert_rowid();
    db::record_event(
        &tx,
        id,
        &ctx.actor,
        "created",
        None,
        None,
        Some(&args.title),
    )?;
    attr::set_values(&tx, project_id, id, &args.set)?;
    tx.commit()?;
    let item = get(&ctx.conn, id)?;
    output::written(ctx.json, &item, &format!("created {}", item.id))
}

pub fn list(ctx: &Ctx, args: ListArgs) -> Result<()> {
    let conn = &ctx.conn;
    let mut conds: Vec<String> = Vec::new();
    let mut vals: Vec<SqlValue> = Vec::new();
    let project_id = args
        .project
        .as_deref()
        .map(|k| db::project_id(conn, k))
        .transpose()?;
    if let Some(id) = project_id {
        conds.push(format!("i.project_id = {}", bind(&mut vals, id)));
    }
    let parent = args
        .parent
        .as_deref()
        .map(|p| db::item(conn, p))
        .transpose()?;
    if let Some(parent) = parent {
        conds.push(format!("i.parent_id = {}", bind(&mut vals, parent.id)));
    }
    let known_project = project_id.or(parent.map(|p| p.project_id));
    if !args.status.is_empty() {
        // With a known project the names must exist there; otherwise match them in any project.
        let mut marks = Vec::new();
        for name in &args.status {
            let mark = match known_project {
                Some(pid) => bind(&mut vals, status::resolve(conn, pid, name)?),
                None => bind(&mut vals, status::any_project(conn, name)?),
            };
            marks.push(mark);
        }
        let column = match known_project {
            Some(_) => "i.status_id",
            None => "s.name COLLATE NOCASE",
        };
        conds.push(format!("{column} IN ({})", marks.join(", ")));
    } else if !args.all {
        let marks: Vec<String> = Category::value_variants()
            .iter()
            .filter(|c| c.is_closed())
            .map(|c| bind(&mut vals, c.as_str().to_string()))
            .collect();
        conds.push(format!("s.category NOT IN ({})", marks.join(", ")));
    }
    if let Some(who) = &args.assignee {
        conds.push(format!("i.assignee = {}", bind(&mut vals, who.clone())));
    }
    if let Some(name) = &args.milestone {
        // With a known project the name must exist there; otherwise match it in any project.
        match known_project {
            Some(pid) => {
                let id = milestone::resolve(conn, pid, name)?;
                conds.push(format!("i.milestone_id = {}", bind(&mut vals, id)));
            }
            None => {
                let known: bool = conn.query_row(
                    "SELECT count(*) > 0 FROM milestones WHERE name = ?1",
                    params![name],
                    |r| r.get(0),
                )?;
                if !known {
                    return Err(err(format!("no such milestone: {name}")));
                }
                conds.push(format!("m.name = {}", bind(&mut vals, name.clone())));
            }
        }
    }
    let keep = attr::filter(conn, project_id, &args.filter)?;
    let cond = if conds.is_empty() {
        "1".to_string()
    } else {
        conds.join(" AND ")
    };
    let mut items = query(conn, &cond, &vals)?;
    if let Some(keep) = keep {
        items.retain(|i| keep.contains(&i.row_id));
    }
    if ctx.json {
        return output::json(&items);
    }
    let dash = |v: &Option<String>| v.clone().unwrap_or_else(|| "-".to_string());
    let rows: Vec<Vec<String>> = items
        .iter()
        .map(|i| {
            let title = match &i.parent {
                Some(p) => format!("{p} > {}", i.title),
                None => i.title.clone(),
            };
            vec![
                i.id.clone(),
                i.status.clone(),
                i.priority.clone(),
                dash(&i.assignee),
                dash(&i.due),
                title,
            ]
        })
        .collect();
    output::table(&["ID", "STATUS", "PRI", "ASSIGNEE", "DUE", "TITLE"], &rows);
    Ok(())
}

pub fn show(ctx: &Ctx, args: KeyArg) -> Result<()> {
    let conn = &ctx.conn;
    let r = db::item(conn, &args.key)?;
    let detail = Detail {
        item: get(conn, r.id)?,
        subtasks: query(conn, "i.parent_id = ?1", &[r.id.into()])?,
        comments: comment::for_item(conn, r.id)?,
    };
    if ctx.json {
        return output::json(&detail);
    }
    let i = &detail.item;
    println!("{}  {}", i.id, i.title);
    println!("status: {}", i.status);
    println!("priority: {}", i.priority);
    let optional = [
        ("assignee", &i.assignee),
        ("due", &i.due),
        ("milestone", &i.milestone),
        ("parent", &i.parent),
    ];
    for (name, value) in optional {
        if let Some(v) = value {
            println!("{name}: {v}");
        }
    }
    println!("created: {} by {}", i.created_at, i.created_by);
    println!("updated: {}", i.updated_at);
    for (name, value) in &i.attributes {
        match value {
            Value::String(s) => println!("attr {name}={s}"),
            v => println!("attr {name}={v}"),
        }
    }
    if !i.body.is_empty() {
        println!("\n{}", i.body);
    }
    if !detail.subtasks.is_empty() {
        println!("\nsubtasks:");
        for s in &detail.subtasks {
            println!("  {}  {}  {}", s.id, s.status, s.title);
        }
    }
    if !detail.comments.is_empty() {
        println!("\ncomments:");
        for c in &detail.comments {
            println!("  {} {}: {}", c.created_at, c.author, c.body);
        }
    }
    Ok(())
}

pub fn edit(ctx: &Ctx, args: EditArgs) -> Result<()> {
    let tx = write_tx(&ctx.conn)?;
    let item = db::item(&tx, &args.key)?;
    let nothing = args.title.is_none()
        && args.body.is_none()
        && args.status.is_none()
        && args.priority.is_none()
        && args.assignee.is_none()
        && args.due.is_none()
        && args.milestone.is_none()
        && args.parent.is_none()
        && args.set.is_empty()
        && args.unset.is_empty();
    if nothing {
        return Err(err(
            "nothing to change: pass a field flag, --set or --unset",
        ));
    }
    if let Some(title) = &args.title {
        check_title(title)?;
    }
    if let Some(due) = &args.due {
        check_date(due)?;
    }
    let before = get(&tx, item.id)?;
    let mut sets: Vec<String> = Vec::new();
    let mut vals: Vec<SqlValue> = Vec::new();
    let mut set = |col: &str, v: SqlValue| {
        let mark = bind(&mut vals, v);
        sets.push(format!("{col} = {mark}"));
    };
    if let Some(v) = &args.title {
        set("title", v.clone().into());
    }
    if let Some(v) = &args.body {
        set("body", v.clone().into());
    }
    if let Some(name) = &args.status {
        set(
            "status_id",
            status::resolve(&tx, item.project_id, name)?.into(),
        );
    }
    if let Some(v) = args.priority {
        set("priority", v.as_str().to_string().into());
    }
    if let Some(v) = &args.assignee {
        set("assignee", v.clone().into());
    }
    if let Some(v) = &args.due {
        set("due", v.clone().into());
    }
    if let Some(name) = &args.milestone {
        set(
            "milestone_id",
            milestone::resolve(&tx, item.project_id, name)?.into(),
        );
    }
    if let Some(key) = &args.parent {
        let parent = db::item(&tx, key)?;
        check_parent(&tx, item, parent, key, &args.key)?;
        set("parent_id", parent.id.into());
    }
    for field in &args.unset {
        let (col, given) = match field.as_str() {
            "assignee" => ("assignee", args.assignee.is_some()),
            "due" => ("due", args.due.is_some()),
            "milestone" => ("milestone_id", args.milestone.is_some()),
            "parent" => ("parent_id", args.parent.is_some()),
            "body" => ("body", args.body.is_some()),
            "title" | "status" | "priority" => {
                return Err(err(format!("cannot unset {field}: it always has a value")));
            }
            name => {
                if args.set.iter().any(|(k, _)| k == name) {
                    return Err(err(format!("cannot both set and unset {name}")));
                }
                if !attr::unset_value(&tx, item.project_id, item.id, name)? {
                    return Err(err(format!("unknown field: {name}")));
                }
                continue;
            }
        };
        if given {
            return Err(err(format!("cannot both set and unset {field}")));
        }
        let cleared = if col == "body" {
            SqlValue::Text(String::new())
        } else {
            SqlValue::Null
        };
        set(col, cleared);
    }
    sets.push(format!("updated_at = {}", db::NOW_SQL));
    let id_mark = bind(&mut vals, item.id);
    tx.execute(
        &format!("UPDATE items SET {} WHERE id = {id_mark}", sets.join(", ")),
        params_from_iter(vals.iter()),
    )?;
    attr::set_values(&tx, item.project_id, item.id, &args.set)?;
    record_changes(&tx, &ctx.actor, &before, &get(&tx, item.id)?)?;
    tx.commit()?;
    let item = get(&ctx.conn, item.id)?;
    output::written(ctx.json, &item, &format!("updated {}", item.id))
}

pub fn set_status(ctx: &Ctx, args: KeyArg, shortcut: Shortcut) -> Result<()> {
    let tx = write_tx(&ctx.conn)?;
    let item = db::item(&tx, &args.key)?;
    let status_id = status::for_shortcut(&tx, item.project_id, shortcut)?;
    let before = get(&tx, item.id)?;
    tx.execute(
        &format!(
            "UPDATE items SET status_id = ?1, updated_at = {} WHERE id = ?2",
            db::NOW_SQL
        ),
        params![status_id, item.id],
    )?;
    record_changes(&tx, &ctx.actor, &before, &get(&tx, item.id)?)?;
    tx.commit()?;
    let item = get(&ctx.conn, item.id)?;
    output::written(
        ctx.json,
        &item,
        &format!("updated {} ({})", item.id, item.status),
    )
}

pub fn rm(ctx: &Ctx, args: KeyArg) -> Result<()> {
    let tx = write_tx(&ctx.conn)?;
    let r = db::item(&tx, &args.key)?;
    let item = get(&tx, r.id)?;
    let subtasks: i64 = tx.query_row(
        "WITH RECURSIVE down(id) AS (
             SELECT ?1 UNION SELECT i.id FROM items i JOIN down ON i.parent_id = down.id
         ) SELECT count(*) - 1 FROM down",
        params![r.id],
        |row| row.get(0),
    )?;
    // Subtasks, values and comments go with it (ON DELETE CASCADE).
    tx.execute("DELETE FROM items WHERE id = ?1", params![r.id])?;
    tx.commit()?;
    let line = match subtasks {
        0 => format!("deleted {}", item.id),
        n => format!("deleted {} and {n} subtask(s)", item.id),
    };
    output::written(ctx.json, &item, &line)
}

/// A transaction that takes the write lock up front, so a read-then-write cannot fail with
/// SQLITE_BUSY_SNAPSHOT when another agent commits in between (busy_timeout does not retry that).
fn write_tx(conn: &Connection) -> Result<Transaction<'_>> {
    Ok(Transaction::new_unchecked(
        conn,
        TransactionBehavior::Immediate,
    )?)
}

/// One item by row id.
pub fn get(conn: &Connection, id: i64) -> Result<Item> {
    query(conn, "i.id = ?1", &[id.into()])?
        .pop()
        .ok_or_else(|| err(format!("no such item id: {id}")))
}

/// Items matching a WHERE condition over `items i` (joined with `projects p`, `statuses s`,
/// `milestones m` and the parent `par`), ordered by project key then number.
pub fn query(conn: &Connection, cond: &str, vals: &[SqlValue]) -> Result<Vec<Item>> {
    let sql = format!("{SELECT} WHERE {cond} ORDER BY p.key, i.seq");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_from_iter(vals.iter()), |r| {
        Ok(Item {
            row_id: r.get(0)?,
            id: r.get(1)?,
            project: r.get(2)?,
            title: r.get(3)?,
            body: r.get(4)?,
            status: r.get(5)?,
            status_category: r.get(6)?,
            priority: r.get(7)?,
            assignee: r.get(8)?,
            due: r.get(9)?,
            milestone: r.get(10)?,
            parent: r.get(11)?,
            created_by: r.get(12)?,
            created_at: r.get(13)?,
            updated_at: r.get(14)?,
            attributes: Map::new(),
        })
    })?;
    let mut items = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    for item in &mut items {
        item.attributes = attr::values_for_item(conn, item.row_id)?;
    }
    Ok(items)
}

/// Record a `changed` event for every built-in field and an `attr` event for every custom
/// attribute that differs between two snapshots of one task.
fn record_changes(conn: &Connection, actor: &str, before: &Item, after: &Item) -> Result<()> {
    let text = |s: &str| (!s.is_empty()).then(|| s.to_string());
    let fields = [
        ("title", text(&before.title), text(&after.title)),
        ("body", text(&before.body), text(&after.body)),
        ("status", text(&before.status), text(&after.status)),
        ("priority", text(&before.priority), text(&after.priority)),
        ("assignee", before.assignee.clone(), after.assignee.clone()),
        ("due", before.due.clone(), after.due.clone()),
        (
            "milestone",
            before.milestone.clone(),
            after.milestone.clone(),
        ),
        ("parent", before.parent.clone(), after.parent.clone()),
    ];
    for (field, old, new) in fields {
        if old != new {
            let (old, new) = (old.as_deref(), new.as_deref());
            db::record_event(conn, after.row_id, actor, "changed", Some(field), old, new)?;
        }
    }
    let value = |item: &Item, name: &str| {
        item.attributes.get(name).map(|v| match v {
            Value::String(s) => s.clone(),
            v => v.to_string(),
        })
    };
    let names = after.attributes.keys().chain(
        before
            .attributes
            .keys()
            .filter(|k| !after.attributes.contains_key(*k)),
    );
    for name in names {
        let (old, new) = (value(before, name), value(after, name));
        if old != new {
            let (old, new) = (old.as_deref(), new.as_deref());
            db::record_event(conn, after.row_id, actor, "attr", Some(name), old, new)?;
        }
    }
    Ok(())
}

/// A new parent must be in the same project and must not be the task or one of its subtasks.
fn check_parent(
    conn: &Connection,
    item: db::ItemRef,
    parent: db::ItemRef,
    parent_key: &str,
    item_key: &str,
) -> Result<()> {
    let (parent_key, item_key) = (canonical(parent_key), canonical(item_key));
    if parent.project_id != item.project_id {
        return Err(err(format!(
            "parent {parent_key} is in another project than {item_key}"
        )));
    }
    let cycle: bool = conn.query_row(
        "WITH RECURSIVE up(id) AS (
             SELECT ?1 UNION SELECT i.parent_id FROM items i JOIN up ON i.id = up.id
             WHERE i.parent_id IS NOT NULL
         ) SELECT count(*) > 0 FROM up WHERE id = ?2",
        params![parent.id, item.id],
        |r| r.get(0),
    )?;
    if cycle {
        return Err(err(format!(
            "parent cycle: {parent_key} is {item_key} or one of its subtasks"
        )));
    }
    Ok(())
}

fn check_title(title: &str) -> Result<()> {
    if title.trim().is_empty() {
        return Err(err("title cannot be empty"));
    }
    Ok(())
}

/// `apm-01` as `APM-1`, for messages (the reference is already validated).
fn canonical(reference: &str) -> String {
    match db::parse_item_key(reference) {
        Ok((key, n)) => db::item_key(&key, n),
        Err(_) => reference.to_string(),
    }
}

/// Add a parameter and return its `?N` placeholder.
fn bind(vals: &mut Vec<SqlValue>, v: impl Into<SqlValue>) -> String {
    vals.push(v.into());
    format!("?{}", vals.len())
}
