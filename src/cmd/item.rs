//! Tasks and subtasks ("items"), referenced as KEY-N.

use clap::{Args, ValueEnum};
use rusqlite::types::Value as SqlValue;
use std::collections::BTreeMap;

use rusqlite::{Connection, params, params_from_iter};
use serde::Serialize;
use serde_json::{Map, Value};

use super::status::{self, Shortcut};
use super::{Ctx, attr, comment, link, milestone, tag};
use crate::db::{self, EventKind};
use crate::error::{Result, err};
use crate::model::{Category, Priority, check_date, parse_pair};
use crate::output;

#[derive(Args, Debug)]
pub struct KeyArg {
    /// Task id, e.g. APM-12
    pub key: String,
}

#[derive(Args, Debug)]
pub struct ShowArgs {
    /// Task id, e.g. APM-12
    pub key: String,
    /// Only the task and all its subtasks, as an indented tree (JSON: nested `subtasks`)
    #[arg(long)]
    pub tree: bool,
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
    /// Tag, lowercase a-z 0-9 _ : . - (repeatable)
    #[arg(long = "tag", value_name = "TAG")]
    pub tags: Vec<String>,
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
    /// Only tasks with this tag (repeatable, all must match)
    #[arg(long = "tag", value_name = "TAG")]
    pub tags: Vec<String>,
    /// Only open tasks with no open blocker, not in a status named blocked
    #[arg(long, conflicts_with = "blocked")]
    pub ready: bool,
    /// Only open tasks with an open blocker or in a status named blocked
    #[arg(long)]
    pub blocked: bool,
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
    /// Add a tag (repeatable)
    #[arg(long = "tag", value_name = "TAG")]
    pub tags: Vec<String>,
    /// Remove a tag (repeatable)
    #[arg(long = "untag", value_name = "TAG")]
    pub untags: Vec<String>,
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
    /// Sorted.
    pub tags: Vec<String>,
}

/// `show`: the item plus its links, subtasks and comments.
#[derive(Serialize)]
struct Detail {
    #[serde(flatten)]
    item: Item,
    links: link::Links,
    subtasks: Vec<Node>,
    comments: Vec<comment::Comment>,
}

/// A subtask in `show`. With `--tree` it carries its own subtasks, recursively; without,
/// it is a plain item.
#[derive(Serialize)]
struct Node {
    #[serde(flatten)]
    item: Item,
    #[serde(skip_serializing_if = "Option::is_none")]
    subtasks: Option<Vec<Node>>,
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
    let tx = db::write_tx(&ctx.conn)?;
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
    let seq = db::next_seq(&tx, project_id)?;
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
        EventKind::Created,
        None,
        None,
        Some(&args.title),
    )?;
    attr::set_values(&tx, project_id, id, &args.set)?;
    tag::apply(&tx, &ctx.actor, id, &args.tags, &[])?;
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
                None => bind(&mut vals, status::check_known_name(conn, name)?),
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
    list_links_and_tags(&args, &mut conds, &mut vals)?;
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
                dash(&i.milestone),
                title,
            ]
        })
        .collect();
    output::table(
        &["ID", "STATUS", "PRI", "ASSIGNEE", "DUE", "MS", "TITLE"],
        &rows,
    );
    Ok(())
}

pub fn show(ctx: &Ctx, args: ShowArgs) -> Result<()> {
    let conn = &ctx.conn;
    let r = db::item(conn, &args.key)?;
    let subtasks = if args.tree {
        subtree(conn, r.id)?
    } else {
        query(conn, "i.parent_id = ?1", &[r.id.into()])?
            .into_iter()
            .map(|item| Node {
                item,
                subtasks: None,
            })
            .collect()
    };
    let detail = Detail {
        item: get(conn, r.id)?,
        links: link::for_item(conn, r.id)?,
        subtasks,
        comments: comment::for_item(conn, r.id)?,
    };
    if ctx.json {
        return output::json(&detail);
    }
    if args.tree {
        let mut rows = Vec::new();
        tree_rows(&detail.item, &detail.subtasks, 0, &mut rows);
        for line in output::aligned(&rows) {
            println!("{line}");
        }
        return Ok(());
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
    if !i.tags.is_empty() {
        println!("tags: {}", i.tags.join(", "));
    }
    if !i.body.is_empty() {
        println!("\n{}", i.body);
    }
    for (label, linked) in detail.links.groups() {
        if !linked.is_empty() {
            println!("\n{label}:");
            let rows: Vec<Vec<String>> = linked
                .iter()
                .map(|l| vec![l.id.clone(), l.status.clone(), l.title.clone()])
                .collect();
            print_indented(&rows);
        }
    }
    if !detail.subtasks.is_empty() {
        println!("\nsubtasks:");
        let rows: Vec<Vec<String>> = detail
            .subtasks
            .iter()
            .map(|s| {
                vec![
                    s.item.id.clone(),
                    s.item.status.clone(),
                    s.item.title.clone(),
                ]
            })
            .collect();
        print_indented(&rows);
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
    let tx = db::write_tx(&ctx.conn)?;
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
        && args.unset.is_empty()
        && args.tags.is_empty()
        && args.untags.is_empty();
    if nothing {
        return Err(err(
            "nothing to change: pass a field flag, --set, --unset, --tag or --untag",
        ));
    }
    if let Some(title) = &args.title {
        check_title(title)?;
    }
    if let Some(due) = &args.due {
        check_date(due)?;
    }
    let before = snapshot(&tx, item.id)?;
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
    record_changes(&tx, &ctx.actor, &before, &snapshot(&tx, item.id)?)?;
    tag::apply(&tx, &ctx.actor, item.id, &args.tags, &args.untags)?;
    tx.commit()?;
    let item = get(&ctx.conn, item.id)?;
    output::written(ctx.json, &item, &format!("updated {}", item.id))
}

pub fn set_status(ctx: &Ctx, args: KeyArg, shortcut: Shortcut) -> Result<()> {
    let tx = db::write_tx(&ctx.conn)?;
    let item = db::item(&tx, &args.key)?;
    let status_id = status::for_shortcut(&tx, item.project_id, shortcut)?;
    let before = snapshot(&tx, item.id)?;
    tx.execute(
        &format!(
            "UPDATE items SET status_id = ?1, updated_at = {} WHERE id = ?2",
            db::NOW_SQL
        ),
        params![status_id, item.id],
    )?;
    record_changes(&tx, &ctx.actor, &before, &snapshot(&tx, item.id)?)?;
    tx.commit()?;
    let item = get(&ctx.conn, item.id)?;
    output::written(
        ctx.json,
        &item,
        &format!("updated {} ({})", item.id, item.status),
    )
}

pub fn rm(ctx: &Ctx, args: KeyArg) -> Result<()> {
    let tx = db::write_tx(&ctx.conn)?;
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

/// Rows printed as aligned columns, indented by two spaces.
fn print_indented(rows: &[Vec<String>]) {
    for line in output::aligned(rows) {
        println!("  {line}");
    }
}

/// A task's subtasks, each with its own subtasks, recursively (by number).
fn subtree(conn: &Connection, id: i64) -> Result<Vec<Node>> {
    query(conn, "i.parent_id = ?1", &[id.into()])?
        .into_iter()
        .map(|item| {
            let subtasks = Some(subtree(conn, item.row_id)?);
            Ok(Node { item, subtasks })
        })
        .collect()
}

/// `show --tree` rows: id (indented two spaces per level), status, title.
fn tree_rows(item: &Item, subtasks: &[Node], depth: usize, rows: &mut Vec<Vec<String>>) {
    rows.push(vec![
        format!("{}{}", "  ".repeat(depth), item.id),
        item.status.clone(),
        item.title.clone(),
    ]);
    for node in subtasks {
        let children = node.subtasks.as_deref().unwrap_or_default();
        tree_rows(&node.item, children, depth + 1, rows);
    }
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
            tags: Vec::new(),
        })
    })?;
    let mut items = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    for item in &mut items {
        item.attributes = attr::values_for_item(conn, item.row_id)?;
        item.tags = tag::for_item(conn, item.row_id)?;
    }
    Ok(items)
}

/// `list --tag`, `--ready`, `--blocked` as conditions over `items i` / `statuses s`.
/// Ready and blocked tasks are open ones, whatever `--all` says. A task is blocked when an
/// open task blocks it or its status is named `blocked` (the `block` shortcut's status).
fn list_links_and_tags(
    args: &ListArgs,
    conds: &mut Vec<String>,
    vals: &mut Vec<SqlValue>,
) -> Result<()> {
    for t in &args.tags {
        let mark = bind(vals, tag::normalize(t)?);
        conds.push(tag::has_tag(&mark));
    }
    if args.ready || args.blocked {
        let closed: Vec<String> = Category::value_variants()
            .iter()
            .filter(|c| c.is_closed())
            .map(|c| bind(vals, c.as_str().to_string()))
            .collect();
        let closed = closed.join(", ");
        conds.push(format!("s.category NOT IN ({closed})"));
        let named = bind(vals, "blocked".to_string());
        let not = if args.ready { "NOT " } else { "" };
        conds.push(format!(
            "{not}(s.name = {named} COLLATE NOCASE OR {})",
            link::has_open_blocker(&closed)
        ));
    }
    Ok(())
}

/// A task as history compares it: its fields and its stored attribute values.
struct Snapshot {
    item: Item,
    attrs: BTreeMap<String, String>,
}

fn snapshot(conn: &Connection, id: i64) -> Result<Snapshot> {
    Ok(Snapshot {
        item: get(conn, id)?,
        attrs: attr::stored_values(conn, id)?,
    })
}

/// Record a `changed` event for every built-in field and an `attr` event for every custom
/// attribute (in name order) that differs between two snapshots of one task.
fn record_changes(
    conn: &Connection,
    actor: &str,
    before: &Snapshot,
    after: &Snapshot,
) -> Result<()> {
    let (b, a) = (&before.item, &after.item);
    let text = |s: &str| (!s.is_empty()).then(|| s.to_string());
    let fields = [
        ("title", text(&b.title), text(&a.title)),
        ("body", text(&b.body), text(&a.body)),
        ("status", text(&b.status), text(&a.status)),
        ("priority", text(&b.priority), text(&a.priority)),
        ("assignee", b.assignee.clone(), a.assignee.clone()),
        ("due", b.due.clone(), a.due.clone()),
        ("milestone", b.milestone.clone(), a.milestone.clone()),
        ("parent", b.parent.clone(), a.parent.clone()),
    ];
    let id = a.row_id;
    for (field, old, new) in fields {
        if old != new {
            let (old, new) = (old.as_deref(), new.as_deref());
            db::record_event(conn, id, actor, EventKind::Changed, Some(field), old, new)?;
        }
    }
    let mut names: Vec<&String> = before.attrs.keys().chain(after.attrs.keys()).collect();
    names.sort();
    names.dedup();
    for name in names {
        let (old, new) = (before.attrs.get(name), after.attrs.get(name));
        if old != new {
            let (old, new) = (old.map(String::as_str), new.map(String::as_str));
            db::record_event(conn, id, actor, EventKind::Attr, Some(name), old, new)?;
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
