//! Tags on tasks: lowercase labels, set with `add --tag` / `edit --tag --untag`, counted by `apm tags`.

use clap::Args;
use rusqlite::{Connection, params};
use serde::Serialize;

use super::Ctx;
use crate::db::{self, EventKind};
use crate::error::{Result, err};
use crate::output;

#[derive(Args, Debug)]
pub struct TagsArgs {
    /// Only tasks of this project
    #[arg(short, long)]
    pub project: Option<String>,
}

#[derive(Serialize, Debug)]
pub struct TagCount {
    pub tag: String,
    pub count: i64,
}

/// `apm tags [-p KEY]`: every tag with its task count, most used first, then by name.
pub fn run(ctx: &Ctx, args: TagsArgs) -> Result<()> {
    let conn = &ctx.conn;
    let project_id = args
        .project
        .as_deref()
        .map(|k| db::project_id(conn, k))
        .transpose()?;
    let mut stmt = conn.prepare(
        "SELECT t.tag, count(*) FROM tags t JOIN items i ON i.id = t.item_id
         WHERE ?1 IS NULL OR i.project_id = ?1
         GROUP BY t.tag ORDER BY count(*) DESC, t.tag",
    )?;
    let rows = stmt.query_map(params![project_id], |r| {
        Ok(TagCount {
            tag: r.get(0)?,
            count: r.get(1)?,
        })
    })?;
    let tags = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    if ctx.json {
        return output::json(&tags);
    }
    let rows: Vec<Vec<String>> = tags
        .iter()
        .map(|t| vec![t.tag.clone(), t.count.to_string()])
        .collect();
    output::table(&["TAG", "COUNT"], &rows);
    Ok(())
}

/// Normalize a tag: trimmed, lowercase, 1-40 of `a-z 0-9 _ : . -`.
pub fn normalize(tag: &str) -> Result<String> {
    let t = tag.trim().to_lowercase();
    let ok = (1..=40).contains(&t.len())
        && t.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_:.-".contains(&b));
    if !ok {
        return Err(err(format!(
            "invalid tag `{tag}`: 1-40 of a-z, 0-9, _ : . -"
        )));
    }
    Ok(t)
}

/// Add and remove tags on a task (each in tag order), recording `tagged` / `untagged` events for real changes
/// only. Adding a tag it has, or removing one it lacks, does nothing. Call inside the
/// mutation's transaction.
pub fn apply(
    conn: &Connection,
    actor: &str,
    item_id: i64,
    add: &[String],
    remove: &[String],
) -> Result<()> {
    let add = normalize_all(add)?;
    let remove = normalize_all(remove)?;
    if let Some(t) = add.iter().find(|t| remove.contains(t)) {
        return Err(err(format!("cannot both tag and untag {t}")));
    }
    for tag in &add {
        let n = conn.execute(
            "INSERT OR IGNORE INTO tags (item_id, tag) VALUES (?1, ?2)",
            params![item_id, tag],
        )?;
        if n > 0 {
            db::record_event(
                conn,
                item_id,
                actor,
                EventKind::Tagged,
                None,
                None,
                Some(tag),
            )?;
        }
    }
    for tag in &remove {
        let n = conn.execute(
            "DELETE FROM tags WHERE item_id = ?1 AND tag = ?2",
            params![item_id, tag],
        )?;
        if n > 0 {
            db::record_event(
                conn,
                item_id,
                actor,
                EventKind::Untagged,
                None,
                Some(tag),
                None,
            )?;
        }
    }
    Ok(())
}

/// Normalized, sorted, without duplicates: events come in tag order.
fn normalize_all(tags: &[String]) -> Result<Vec<String>> {
    let mut out = tags
        .iter()
        .map(|t| normalize(t))
        .collect::<Result<Vec<_>>>()?;
    out.sort();
    out.dedup();
    Ok(out)
}

/// A task's tags, sorted.
pub fn for_item(conn: &Connection, item_id: i64) -> Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT tag FROM tags WHERE item_id = ?1 ORDER BY tag")?;
    let rows = stmt.query_map(params![item_id], |r| r.get(0))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// SQL condition over `items i`: the task has the tag bound at `mark`.
pub fn has_tag(mark: &str) -> String {
    format!("EXISTS (SELECT 1 FROM tags t WHERE t.item_id = i.id AND t.tag = {mark})")
}
