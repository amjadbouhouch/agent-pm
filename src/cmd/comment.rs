use clap::Args;
use rusqlite::{Connection, params};
use serde::Serialize;

use super::Ctx;
use crate::error::{Result, err};
use crate::{db, output};

#[derive(Args, Debug)]
pub struct CommentArgs {
    /// Task id, e.g. APM-12
    pub key: String,
    pub text: String,
}

#[derive(Serialize, Debug)]
pub struct Comment {
    pub id: i64,
    /// The task's KEY-N
    pub item: String,
    pub author: String,
    pub body: String,
    pub created_at: String,
}

const SELECT: &str = "SELECT c.id, p.key || '-' || i.seq, c.author, c.body, c.created_at
     FROM comments c
     JOIN items i ON i.id = c.item_id
     JOIN projects p ON p.id = i.project_id";

fn row(r: &rusqlite::Row) -> rusqlite::Result<Comment> {
    Ok(Comment {
        id: r.get(0)?,
        item: r.get(1)?,
        author: r.get(2)?,
        body: r.get(3)?,
        created_at: r.get(4)?,
    })
}

/// `apm comment KEY-N <text>`: add a comment authored by the actor.
pub fn run(ctx: &Ctx, args: CommentArgs) -> Result<()> {
    let item = db::item(&ctx.conn, &args.key)?;
    if args.text.trim().is_empty() {
        return Err(err("comment text cannot be empty"));
    }
    // A comment is activity on the task: bump its updated_at in the same transaction.
    let tx = ctx.conn.unchecked_transaction()?;
    tx.execute(
        "INSERT INTO comments (item_id, author, body) VALUES (?1, ?2, ?3)",
        params![item.id, ctx.actor, args.text],
    )?;
    let id = tx.last_insert_rowid();
    db::record_event(
        &tx,
        item.id,
        &ctx.actor,
        "commented",
        None,
        None,
        Some(&args.text),
    )?;
    tx.execute(
        &format!(
            "UPDATE items SET updated_at = {} WHERE id = ?1",
            db::NOW_SQL
        ),
        params![item.id],
    )?;
    tx.commit()?;
    let c = ctx
        .conn
        .query_row(&format!("{SELECT} WHERE c.id = ?1"), params![id], row)?;
    output::written(ctx.json, &c, &format!("commented on {}", c.item))
}

/// A task's comments, oldest first. Used by `show`.
pub fn for_item(conn: &Connection, item_id: i64) -> Result<Vec<Comment>> {
    let mut stmt = conn.prepare(&format!("{SELECT} WHERE c.item_id = ?1 ORDER BY c.id"))?;
    let rows = stmt.query_map(params![item_id], row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}
