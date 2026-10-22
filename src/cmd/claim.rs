//! Claims: "agent X is working on this task, until T". `claim`, `release`, `next`, `claims`.
//! A claim is live while `claim_expires` is in the future; an expired one is ignored everywhere.

use clap::{Args, ValueEnum};
use rusqlite::types::Value as SqlValue;
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use super::{Ctx, item, link, status, tag};
use crate::db::{self, EventKind};
use crate::error::{Result, err};
use crate::model::{Category, parse_ttl};
use crate::output;

/// Exit code of `next` when no task is ready (errors are 1, usage errors 2).
pub const NOTHING_READY: i32 = 3;

/// SQL condition over `items i`: the task has a live claim. Never NULL, so `NOT` works.
pub const LIVE: &str =
    "(i.claim_expires IS NOT NULL AND i.claim_expires > strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))";

/// A live claim as printed in item JSON (`"claim"`).
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct Claim {
    pub by: String,
    pub at: String,
    pub expires: String,
}

#[derive(Args, Debug)]
pub struct ClaimArgs {
    /// Task id, e.g. APM-12
    pub key: String,
    /// How long the claim lasts: <n>m, <n>h or <n>d, 1m to 7d
    #[arg(long, default_value = "2h", value_parser = parse_ttl)]
    pub ttl: i64,
    /// Take the task even if someone else holds a live claim
    #[arg(long)]
    pub steal: bool,
}

#[derive(Args, Debug)]
pub struct ReleaseArgs {
    /// Task id, e.g. APM-12
    pub key: String,
    /// Release someone else's claim
    #[arg(long)]
    pub force: bool,
}

#[derive(Args, Debug)]
pub struct NextArgs {
    #[arg(short, long)]
    pub project: Option<String>,
    /// Only tasks with this tag (repeatable, all must match)
    #[arg(long = "tag", value_name = "TAG")]
    pub tags: Vec<String>,
    /// Milestone name
    #[arg(short, long)]
    pub milestone: Option<String>,
    /// How long the claim lasts: <n>m, <n>h or <n>d, 1m to 7d
    #[arg(long, default_value = "2h", value_parser = parse_ttl)]
    pub ttl: i64,
}

#[derive(Args, Debug)]
pub struct ClaimsArgs {
    #[arg(short, long)]
    pub project: Option<String>,
    /// Only the actor's own claims
    #[arg(long)]
    pub mine: bool,
}

/// `apm claim KEY-N`: claim a task, or renew your own live claim.
pub fn claim(ctx: &Ctx, args: ClaimArgs) -> Result<()> {
    let tx = db::write_tx(&ctx.conn)?;
    let r = db::item(&tx, &args.key)?;
    let expires = take(&tx, &ctx.actor, r.id, args.ttl, args.steal)?;
    tx.commit()?;
    let item = item::get(&ctx.conn, r.id)?;
    output::written(
        ctx.json,
        &item,
        &format!("claimed {} until {expires}", item.id),
    )
}

/// `apm release KEY-N`: drop your live claim (someone else's with --force).
pub fn release(ctx: &Ctx, args: ReleaseArgs) -> Result<()> {
    let tx = db::write_tx(&ctx.conn)?;
    let r = db::item(&tx, &args.key)?;
    let Some(holder) = live_holder(&tx, r.id)? else {
        let item = item::get(&tx, r.id)?;
        return output::written(ctx.json, &item, &format!("unchanged {}", item.id));
    };
    if holder != ctx.actor && !args.force {
        return Err(err(format!(
            "{} is claimed by {holder}, not {}; pass --force to release it",
            db::current_key(&tx, r.id)?,
            ctx.actor
        )));
    }
    drop_claim(&tx, &ctx.actor, r.id, &holder, "manual")?;
    tx.execute(
        &format!(
            "UPDATE items SET updated_at = {} WHERE id = ?1",
            db::NOW_SQL
        ),
        params![r.id],
    )?;
    tx.commit()?;
    let item = item::get(&ctx.conn, r.id)?;
    output::written(ctx.json, &item, &format!("released {}", item.id))
}

/// `apm next`: in one write transaction, pick the best ready task, claim it, assign it to the
/// actor if unassigned and start it if it is in a todo-category status. Returns the exit
/// code: 0, or `NOTHING_READY`.
pub fn next(ctx: &Ctx, args: NextArgs) -> Result<i32> {
    let tx = db::write_tx(&ctx.conn)?;
    let Some(id) = pick(&tx, &ctx.actor, &args)? else {
        drop(tx);
        if ctx.json {
            output::json(&serde_json::json!({ "task": null }))?;
        } else {
            println!("nothing ready");
        }
        return Ok(NOTHING_READY);
    };
    let before = item::get(&tx, id)?;
    let expires = take(&tx, &ctx.actor, id, args.ttl, false)?;
    if before.assignee.as_deref().unwrap_or_default().is_empty() {
        tx.execute(
            "UPDATE items SET assignee = ?1 WHERE id = ?2",
            params![ctx.actor, id],
        )?;
        db::record_event(
            &tx,
            id,
            &ctx.actor,
            EventKind::Changed,
            Some("assignee"),
            before.assignee.as_deref().filter(|a| !a.is_empty()),
            Some(&ctx.actor),
        )?;
    }
    if before.status_category == Category::Todo.as_str() {
        let project_id = db::project_id(&tx, &before.project)?;
        let start = status::for_shortcut(&tx, project_id, status::Shortcut::Start)?;
        tx.execute(
            "UPDATE items SET status_id = ?1 WHERE id = ?2",
            params![start, id],
        )?;
        let after = item::get(&tx, id)?;
        db::record_event(
            &tx,
            id,
            &ctx.actor,
            EventKind::Changed,
            Some("status"),
            Some(&before.status),
            Some(&after.status),
        )?;
    }
    tx.commit()?;
    let item = item::get(&ctx.conn, id)?;
    let line = format!("claimed {} until {expires}: {}", item.id, item.title);
    output::written(ctx.json, &item, &line)?;
    Ok(0)
}

/// `apm claims`: tasks with a live claim, soonest expiry first.
pub fn claims(ctx: &Ctx, args: ClaimsArgs) -> Result<()> {
    let conn = &ctx.conn;
    let mut conds = vec![LIVE.to_string()];
    let mut vals: Vec<SqlValue> = Vec::new();
    if let Some(key) = &args.project {
        let id = db::project_id(conn, key)?;
        conds.push(format!("i.project_id = {}", item::bind(&mut vals, id)));
    }
    if args.mine {
        let mark = item::bind(&mut vals, ctx.actor.clone());
        conds.push(format!("i.claimed_by = {mark}"));
    }
    let items = item::query_sorted(
        conn,
        &conds.join(" AND "),
        &vals,
        "i.claim_expires, p.key, i.seq",
    )?;
    if ctx.json {
        return output::json(&items);
    }
    let rows: Vec<Vec<String>> = items
        .iter()
        .filter_map(|i| {
            let c = i.claim.as_ref()?;
            Some(vec![
                i.id.clone(),
                c.by.clone(),
                c.expires.clone(),
                i.title.clone(),
            ])
        })
        .collect();
    output::table(&["ID", "BY", "EXPIRES", "TITLE"], &rows);
    Ok(())
}

/// Drop the claim of a task that is now in a closed (done/cancelled-category) status. A live
/// claim records `released` with reason `done`; an expired one is cleared silently. Call it
/// in the transaction that changed the status.
pub fn release_if_closed(conn: &Connection, actor: &str, item_id: i64) -> Result<()> {
    let (category, claimed): (String, bool) = conn.query_row(
        "SELECT s.category, i.claimed_by IS NOT NULL
         FROM items i JOIN statuses s ON s.id = i.status_id WHERE i.id = ?1",
        params![item_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    if !is_closed(&category) || !claimed {
        return Ok(());
    }
    match live_holder(conn, item_id)? {
        Some(holder) => drop_claim(conn, actor, item_id, &holder, "done"),
        None => clear(conn, item_id),
    }
}

/// Claim a task for `actor` for `ttl` seconds, recording `claimed` (and `released` with reason
/// `stolen` when taking someone else's live claim). Returns the new expiry.
fn take(conn: &Connection, actor: &str, item_id: i64, ttl: i64, steal: bool) -> Result<String> {
    let it = item::get(conn, item_id)?;
    if is_closed(&it.status_category) {
        return Err(err(format!("cannot claim {}: it is {}", it.id, it.status)));
    }
    let mut renew = false;
    if let Some(c) = &it.claim {
        if c.by == actor {
            renew = true;
        } else if steal {
            drop_claim(conn, actor, item_id, &c.by, "stolen")?;
        } else {
            return Err(err(format!(
                "{} is claimed by {} until {}; pass --steal to take it",
                it.id, c.by, c.expires
            )));
        }
    }
    // A renewal keeps the original claim time.
    let expires: String = conn.query_row(
        &format!(
            "UPDATE items SET claimed_by = ?1,
                 claimed_at = CASE WHEN ?2 THEN claimed_at ELSE {now} END,
                 claim_expires = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?3 || ' seconds'),
                 updated_at = {now}
             WHERE id = ?4 RETURNING claim_expires",
            now = db::NOW_SQL
        ),
        params![actor, renew, ttl, item_id],
        |r| r.get(0),
    )?;
    db::record_event(
        conn,
        item_id,
        actor,
        EventKind::Claimed,
        None,
        None,
        Some(&expires),
    )?;
    Ok(expires)
}

/// Whether a status category (as stored) is done or cancelled.
fn is_closed(category: &str) -> bool {
    Category::value_variants()
        .iter()
        .any(|c| c.is_closed() && c.as_str() == category)
}

/// The holder of a task's live claim, if any.
fn live_holder(conn: &Connection, item_id: i64) -> Result<Option<String>> {
    Ok(conn
        .query_row(
            &format!("SELECT i.claimed_by FROM items i WHERE i.id = ?1 AND {LIVE}"),
            params![item_id],
            |r| r.get(0),
        )
        .optional()?)
}

/// Clear a live claim and record `released` with a reason (`manual`, `done`, `stolen`).
fn drop_claim(
    conn: &Connection,
    actor: &str,
    item_id: i64,
    holder: &str,
    reason: &str,
) -> Result<()> {
    clear(conn, item_id)?;
    db::record_event(
        conn,
        item_id,
        actor,
        EventKind::Released,
        Some(reason),
        Some(holder),
        None,
    )
}

fn clear(conn: &Connection, item_id: i64) -> Result<()> {
    conn.execute(
        "UPDATE items SET claimed_by = NULL, claimed_at = NULL, claim_expires = NULL
         WHERE id = ?1",
        params![item_id],
    )?;
    Ok(())
}

/// The best ready task for `actor`: open, no open blocker, not in a status named blocked, no
/// live claim by someone else, unassigned or assigned to `actor`; highest priority, then
/// soonest due (none last), then id.
fn pick(conn: &Connection, actor: &str, args: &NextArgs) -> Result<Option<i64>> {
    let mut vals: Vec<SqlValue> = Vec::new();
    let closed = item::closed_marks(&mut vals);
    let named = item::bind(&mut vals, "blocked".to_string());
    let me = item::bind(&mut vals, actor.to_string());
    let mut conds = vec![
        format!("s.category NOT IN ({closed})"),
        format!(
            "NOT (s.name = {named} COLLATE NOCASE OR {})",
            link::has_open_blocker(&closed)
        ),
        format!("NOT ({LIVE} AND i.claimed_by != {me})"),
        format!("(i.assignee IS NULL OR i.assignee = '' OR i.assignee = {me})"),
    ];
    let project_id = args
        .project
        .as_deref()
        .map(|k| db::project_id(conn, k))
        .transpose()?;
    if let Some(id) = project_id {
        conds.push(format!("i.project_id = {}", item::bind(&mut vals, id)));
    }
    for t in &args.tags {
        let mark = item::bind(&mut vals, tag::normalize(t)?);
        conds.push(tag::has_tag(&mark));
    }
    if let Some(name) = &args.milestone {
        conds.push(item::milestone_cond(conn, project_id, name, &mut vals)?);
    }
    let sql = format!(
        "SELECT i.id FROM items i
         JOIN projects p ON p.id = i.project_id
         JOIN statuses s ON s.id = i.status_id
         LEFT JOIN milestones m ON m.id = i.milestone_id
         WHERE {}
         ORDER BY {} DESC, i.due IS NULL, i.due, p.key, i.seq
         LIMIT 1",
        conds.join(" AND "),
        item::priority_rank()
    );
    Ok(conn
        .query_row(&sql, rusqlite::params_from_iter(vals.iter()), |r| r.get(0))
        .optional()?)
}
