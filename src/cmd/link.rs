//! Links between tasks: `A blocks B`, `A relates B`, `A duplicates B`. Cross-project allowed.

use std::collections::{HashMap, VecDeque};

use clap::{Args, ValueEnum};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use super::Ctx;
use crate::db::{self, EventKind};
use crate::error::{Result, err};
use crate::output;

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum LinkKind {
    Blocks,
    Relates,
    Duplicates,
}

impl LinkKind {
    pub fn as_str(self) -> &'static str {
        match self {
            LinkKind::Blocks => "blocks",
            LinkKind::Relates => "relates",
            LinkKind::Duplicates => "duplicates",
        }
    }

    /// The link as seen from its target: `blocked_by`, `relates`, `duplicated_by`.
    fn reverse(self) -> &'static str {
        match self {
            LinkKind::Blocks => "blocked_by",
            LinkKind::Relates => "relates",
            LinkKind::Duplicates => "duplicated_by",
        }
    }
}

#[derive(Args, Debug)]
pub struct LinkArgs {
    /// Task id, e.g. APM-1
    pub from: String,
    #[arg(value_enum)]
    pub kind: LinkKind,
    /// Task id, e.g. APM-2
    pub to: String,
}

/// A written or removed link: the `link` / `unlink` JSON object.
#[derive(Serialize, Debug)]
pub struct Link {
    pub from: String,
    pub kind: &'static str,
    pub to: String,
}

/// The other end of a link, as `show` lists it.
#[derive(Serialize, Debug)]
pub struct Linked {
    pub id: String,
    pub title: String,
    pub status: String,
}

/// A task's links in both directions. `relates` is symmetric: stored once, shown on both.
#[derive(Serialize, Debug, Default)]
pub struct Links {
    pub blocks: Vec<Linked>,
    pub blocked_by: Vec<Linked>,
    pub relates: Vec<Linked>,
    pub duplicates: Vec<Linked>,
    pub duplicated_by: Vec<Linked>,
}

impl Links {
    /// (label, entries) pairs for text output, in a fixed order.
    pub fn groups(&self) -> [(&'static str, &Vec<Linked>); 5] {
        [
            ("blocks", &self.blocks),
            ("blocked by", &self.blocked_by),
            ("relates", &self.relates),
            ("duplicates", &self.duplicates),
            ("duplicated by", &self.duplicated_by),
        ]
    }
}

/// `apm link A <kind> B`.
pub fn link(ctx: &Ctx, args: LinkArgs) -> Result<()> {
    let tx = db::write_tx(&ctx.conn)?;
    let (from, to) = ends(&tx, &args)?;
    let (from_key, to_key) = (key_of(&tx, from)?, key_of(&tx, to)?);
    let shown = Link {
        from: from_key.clone(),
        kind: args.kind.as_str(),
        to: to_key.clone(),
    };
    if find(&tx, from, args.kind, to)?.is_some() {
        return Err(err(format!(
            "link already exists: {from_key} {} {to_key}",
            shown.kind
        )));
    }
    if args.kind == LinkKind::Blocks {
        check_cycle(&tx, from, to)?;
    }
    let res = tx.execute(
        "INSERT INTO links (from_id, to_id, kind) VALUES (?1, ?2, ?3)",
        params![from, to, args.kind.as_str()],
    );
    db::unique(res, || {
        err(format!(
            "link already exists: {from_key} {} {to_key}",
            shown.kind
        ))
    })?;
    record(
        &tx,
        &ctx.actor,
        EventKind::Linked,
        args.kind,
        (from, &from_key),
        (to, &to_key),
    )?;
    tx.commit()?;
    output::written(
        ctx.json,
        &shown,
        &format!("linked {from_key} {} {to_key}", shown.kind),
    )
}

/// `apm unlink A <kind> B`. For `relates`, either direction matches.
pub fn unlink(ctx: &Ctx, args: LinkArgs) -> Result<()> {
    let tx = db::write_tx(&ctx.conn)?;
    let (from, to) = ends(&tx, &args)?;
    let (from_key, to_key) = (key_of(&tx, from)?, key_of(&tx, to)?);
    let kind = args.kind.as_str();
    let (stored_from, stored_to) = find(&tx, from, args.kind, to)?
        .ok_or_else(|| err(format!("no such link: {from_key} {kind} {to_key}")))?;
    tx.execute(
        "DELETE FROM links WHERE from_id = ?1 AND to_id = ?2 AND kind = ?3",
        params![stored_from, stored_to, kind],
    )?;
    let action = EventKind::Unlinked;
    record(
        &tx,
        &ctx.actor,
        action,
        args.kind,
        (from, &from_key),
        (to, &to_key),
    )?;
    tx.commit()?;
    let shown = Link {
        from: from_key,
        kind,
        to: to_key,
    };
    let line = format!("unlinked {} {kind} {}", shown.from, shown.to);
    output::written(ctx.json, &shown, &line)
}

/// A task's links, each group ordered by project key then number. Used by `show`.
pub fn for_item(conn: &Connection, item_id: i64) -> Result<Links> {
    let mut relates = group(conn, item_id, "from_id", "to_id", "relates")?;
    relates.extend(group(conn, item_id, "to_id", "from_id", "relates")?);
    relates.sort_by_key(|l| key_order(&l.id));
    Ok(Links {
        blocks: group(conn, item_id, "from_id", "to_id", "blocks")?,
        blocked_by: group(conn, item_id, "to_id", "from_id", "blocks")?,
        relates,
        duplicates: group(conn, item_id, "from_id", "to_id", "duplicates")?,
        duplicated_by: group(conn, item_id, "to_id", "from_id", "duplicates")?,
    })
}

/// The other ends of a task's `kind` links where the task is the `this` column.
fn group(
    conn: &Connection,
    item_id: i64,
    this: &str,
    other: &str,
    kind: &str,
) -> Result<Vec<Linked>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT p.key || '-' || i.seq, i.title, s.name
         FROM links l
         JOIN items i ON i.id = l.{other}
         JOIN projects p ON p.id = i.project_id
         JOIN statuses s ON s.id = i.status_id
         WHERE l.{this} = ?1 AND l.kind = ?2
         ORDER BY p.key, i.seq"
    ))?;
    let rows = stmt.query_map(params![item_id, kind], |r| {
        Ok(Linked {
            id: r.get(0)?,
            title: r.get(1)?,
            status: r.get(2)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// SQL condition over `items i`: the task has at least one open task blocking it.
pub const HAS_OPEN_BLOCKER: &str = "EXISTS (
    SELECT 1 FROM links bl
    JOIN items bi ON bi.id = bl.from_id
    JOIN statuses bs ON bs.id = bi.status_id
    WHERE bl.to_id = i.id AND bl.kind = 'blocks'
      AND bs.category NOT IN ('done', 'cancelled'))";

/// Resolve both ends and refuse a self-link.
fn ends(conn: &Connection, args: &LinkArgs) -> Result<(i64, i64)> {
    let from = db::item(conn, &args.from)?.id;
    let to = db::item(conn, &args.to)?.id;
    if from == to {
        return Err(err(format!(
            "a task cannot link to itself: {}",
            key_of(conn, from)?
        )));
    }
    Ok((from, to))
}

/// The stored (from, to) of a link, if it exists. `relates` matches either direction.
fn find(conn: &Connection, from: i64, kind: LinkKind, to: i64) -> Result<Option<(i64, i64)>> {
    let symmetric = kind == LinkKind::Relates;
    Ok(conn
        .query_row(
            "SELECT from_id, to_id FROM links
             WHERE kind = ?3 AND ((from_id = ?1 AND to_id = ?2)
                                  OR (?4 AND from_id = ?2 AND to_id = ?1))",
            params![from, to, kind.as_str(), symmetric],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?)
}

/// `from blocks to` must not close a loop: refuse when `to` already (transitively) blocks
/// `from`, naming the cycle.
fn check_cycle(conn: &Connection, from: i64, to: i64) -> Result<()> {
    let mut stmt =
        conn.prepare("SELECT to_id FROM links WHERE from_id = ?1 AND kind = 'blocks'")?;
    // Breadth-first from `to`; `prev` remembers how each task was reached.
    let mut prev: HashMap<i64, i64> = HashMap::from([(to, from)]);
    let mut queue = VecDeque::from([to]);
    while let Some(id) = queue.pop_front() {
        let next = stmt
            .query_map(params![id], |r| r.get::<_, i64>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for n in next {
            if n == from {
                // from -> to -> ... -> id -> from
                let mut back = vec![id];
                while *back.last().unwrap() != to {
                    back.push(prev[back.last().unwrap()]);
                }
                let mut path = vec![from];
                path.extend(back.into_iter().rev());
                path.push(from);
                let keys = path
                    .iter()
                    .map(|&p| key_of(conn, p))
                    .collect::<Result<Vec<_>>>()?;
                return Err(err(format!("blocks cycle: {}", keys.join(" blocks "))));
            }
            if let std::collections::hash_map::Entry::Vacant(e) = prev.entry(n) {
                e.insert(id);
                queue.push_back(n);
            }
        }
    }
    Ok(())
}

/// One event on each end, and bump both tasks' `updated_at`.
fn record(
    conn: &Connection,
    actor: &str,
    action: EventKind,
    kind: LinkKind,
    (from, from_key): (i64, &str),
    (to, to_key): (i64, &str),
) -> Result<()> {
    let sides = [
        (from, kind.as_str(), to_key),
        (to, kind.reverse(), from_key),
    ];
    for (id, field, other) in sides {
        let (old, new) = match action {
            EventKind::Unlinked => (Some(other), None),
            _ => (None, Some(other)),
        };
        db::record_event(conn, id, actor, action, Some(field), old, new)?;
        conn.execute(
            &format!(
                "UPDATE items SET updated_at = {} WHERE id = ?1",
                db::NOW_SQL
            ),
            params![id],
        )?;
    }
    Ok(())
}

/// A task's current `KEY-N`.
fn key_of(conn: &Connection, id: i64) -> Result<String> {
    Ok(conn.query_row(
        "SELECT p.key || '-' || i.seq FROM items i JOIN projects p ON p.id = i.project_id
         WHERE i.id = ?1",
        params![id],
        |r| r.get(0),
    )?)
}

/// `APM-12` as (`APM`, 12), so `APM-2` sorts before `APM-10`.
fn key_order(key: &str) -> (String, i64) {
    db::parse_item_key(key).unwrap_or_else(|_| (key.to_string(), 0))
}
