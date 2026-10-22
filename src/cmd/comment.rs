use clap::Args;
use rusqlite::{Connection, params};
use serde::Serialize;

use super::{Ctx, not_implemented};
use crate::error::Result;

#[derive(Args, Debug)]
pub struct CommentArgs {
    /// Task id, e.g. APM-12
    pub key: String,
    pub text: String,
}

#[derive(Serialize, Debug)]
pub struct Comment {
    pub id: i64,
    pub author: String,
    pub body: String,
    pub created_at: String,
}

pub fn run(_ctx: &Ctx, _args: CommentArgs) -> Result<()> {
    Err(not_implemented("comment"))
}

/// A task's comments, oldest first. Used by `show`.
pub fn for_item(conn: &Connection, item_id: i64) -> Result<Vec<Comment>> {
    let mut stmt = conn.prepare(
        "SELECT id, author, body, created_at FROM comments WHERE item_id = ?1 ORDER BY id",
    )?;
    let rows = stmt.query_map(params![item_id], |r| {
        Ok(Comment {
            id: r.get(0)?,
            author: r.get(1)?,
            body: r.get(2)?,
            created_at: r.get(3)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}
