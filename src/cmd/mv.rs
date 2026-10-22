//! `apm move`: a task and its subtasks to another project, under new numbers.
//! Rows keep their ids, so comments, tags, links and history stay attached; the old KEY-N
//! numbers keep resolving through `item_aliases`.

use std::collections::BTreeMap;

use clap::Args;
use rusqlite::{Connection, params};
use serde::{Serialize, Serializer};

use super::item::{self, Item};
use super::{Ctx, attr, milestone, status};
use crate::db::{self, EventKind};
use crate::error::{Result, err};
use crate::output;

#[derive(Args, Debug)]
pub struct MoveArgs {
    /// Task id, e.g. APM-12
    pub key: String,
    /// Target project key
    pub project: String,
}

/// The `--json` output: the moved task, plus what happened to it and its subtasks.
#[derive(Serialize)]
struct Moved {
    #[serde(flatten)]
    item: Item,
    /// Old KEY-N -> new KEY-N: the task first, then its subtasks level by level.
    renumbered: Pairs,
    /// New KEY-N -> attribute values dropped because the target project has no attribute of
    /// that name and type (or a select without that option). Only tasks that lost values.
    dropped: BTreeMap<String, BTreeMap<String, String>>,
}

/// Ordered `old -> new` pairs, as a JSON object.
struct Pairs(Vec<(String, String)>);

impl Serialize for Pairs {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.collect_map(self.0.iter().map(|(k, v)| (k, v)))
    }
}

pub fn run(ctx: &Ctx, args: MoveArgs) -> Result<()> {
    let tx = db::write_tx(&ctx.conn)?;
    let root = db::item(&tx, &args.key)?;
    let target = db::project_id(&tx, &args.project)?;
    if target == root.project_id {
        let item = item::get(&tx, root.id)?;
        return Err(err(format!(
            "{} is already in project {}",
            item.id, item.project
        )));
    }
    let mut renumbered = Vec::new();
    let mut dropped = BTreeMap::new();
    for id in subtree(&tx, root.id)? {
        let before = item::get(&tx, id)?;
        // The old number stays an alias; the task takes the target's next number.
        tx.execute(
            "INSERT INTO item_aliases (project_id, seq, item_id)
             SELECT project_id, seq, id FROM items WHERE id = ?1",
            params![id],
        )?;
        let seq: i64 = tx.query_row(
            "UPDATE projects SET next_seq = next_seq + 1 WHERE id = ?1 RETURNING next_seq - 1",
            params![target],
            |r| r.get(0),
        )?;
        let status_id = match status::find(&tx, target, &before.status)? {
            Some(s) => s,
            None => status::default_id(&tx, target)?,
        };
        let milestone_id = match &before.milestone {
            Some(name) => milestone::lookup(&tx, target, name)?,
            None => None,
        };
        // Subtasks keep their parent (it moves too); the task itself leaves its parent behind.
        tx.execute(
            &format!(
                "UPDATE items SET project_id = ?1, seq = ?2, status_id = ?3, milestone_id = ?4,
                    parent_id = CASE WHEN id = ?6 THEN NULL ELSE parent_id END,
                    updated_at = {}
                 WHERE id = ?5",
                db::NOW_SQL
            ),
            params![target, seq, status_id, milestone_id, id, root.id],
        )?;
        let lost = attr::move_values(&tx, id, target)?;
        let after = item::get(&tx, id)?;
        record(&tx, &ctx.actor, &before, &after, &lost)?;
        if !lost.is_empty() {
            dropped.insert(after.id.clone(), lost);
        }
        renumbered.push((before.id, after.id));
    }
    tx.commit()?;
    let moved = Moved {
        item: item::get(&ctx.conn, root.id)?,
        renumbered: Pairs(renumbered),
        dropped,
    };
    if ctx.json {
        return output::json(&moved);
    }
    let (old, new) = &moved.renumbered.0[0];
    match moved.renumbered.0.len() - 1 {
        0 => println!("moved {old} → {new}"),
        n => println!("moved {old} → {new} ({n} subtask(s))"),
    }
    for (id, values) in &moved.dropped {
        for (name, value) in values {
            println!("dropped {id} {name}={value}");
        }
    }
    Ok(())
}

/// A task and all its subtasks: the task first, then level by level, by number.
fn subtree(conn: &Connection, root: i64) -> Result<Vec<i64>> {
    let mut stmt = conn.prepare(
        "WITH RECURSIVE down(id, depth) AS (
             SELECT ?1, 0
             UNION ALL SELECT i.id, down.depth + 1 FROM items i JOIN down ON i.parent_id = down.id
         )
         SELECT down.id FROM down JOIN items i ON i.id = down.id ORDER BY down.depth, i.seq",
    )?;
    let ids = stmt
        .query_map(params![root], |r| r.get(0))?
        .collect::<rusqlite::Result<Vec<i64>>>()?;
    Ok(ids)
}

/// History for one moved task: `moved`, then the fields the move changed (status mapped by
/// name or defaulted, milestone cleared, the root's parent cut) and the dropped values.
/// A subtask's parent is the same task under a new number: not a change.
fn record(
    conn: &Connection,
    actor: &str,
    before: &Item,
    after: &Item,
    lost: &BTreeMap<String, String>,
) -> Result<()> {
    let id = after.row_id;
    let (old, new) = (Some(before.id.as_str()), Some(after.id.as_str()));
    db::record_event(conn, id, actor, EventKind::Moved, None, old, new)?;
    let mut fields = vec![
        ("status", Some(&before.status), Some(&after.status)),
        (
            "milestone",
            before.milestone.as_ref(),
            after.milestone.as_ref(),
        ),
    ];
    if after.parent.is_none() {
        fields.push(("parent", before.parent.as_ref(), None));
    }
    for (field, old, new) in fields {
        if old != new {
            let (old, new) = (old.map(String::as_str), new.map(String::as_str));
            db::record_event(conn, id, actor, EventKind::Changed, Some(field), old, new)?;
        }
    }
    for (name, value) in lost {
        let old = Some(value.as_str());
        db::record_event(conn, id, actor, EventKind::Attr, Some(name), old, None)?;
    }
    Ok(())
}
