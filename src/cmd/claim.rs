//! Claims: "agent X is working on this task, until T". `claim`, `release`, `next`, `claims`.
//! A claim is live while `claim_expires` is in the future and the task is open; any other
//! claim is ignored everywhere.

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

/// SQL condition over `items i` joined with `statuses s`: the task has a live claim. A claim
/// on a closed task is never live, whatever path closed it. Never NULL, so `NOT` works.
pub fn live() -> String {
    let closed: Vec<String> = Category::value_variants()
        .iter()
        .filter(|c| c.is_closed())
        .map(|c| format!("'{}'", c.as_str()))
        .collect();
    format!(
        "({} AND s.category NOT IN ({}))",
        unexpired(),
        closed.join(", ")
    )
}

/// SQL condition over `items i`: the claim has not expired (the task may be closed).
fn unexpired() -> String {
    format!(
        "(i.claim_expires IS NOT NULL AND i.claim_expires > {})",
        db::NOW_SQL
    )
}

/// Why a live claim was dropped: `released` events store it as `field`.
#[derive(Clone, Copy, Debug)]
enum Reason {
    /// `release`.
    Manual,
    /// The task was closed.
    Done,
    /// `claim --steal` by someone else.
    Stolen,
}

impl Reason {
    fn as_str(self) -> &'static str {
        match self {
            Reason::Manual => "manual",
            Reason::Done => "done",
            Reason::Stolen => "stolen",
        }
    }
}

/// A live claim as printed in item JSON (`"claim"`).
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct Claim {
    pub by: String,
    pub at: String,
    pub expires: String,
}

/// `next`'s output: the claimed task (null when nothing is ready), and whether it was the
/// actor's own claim, renewed.
#[derive(Serialize)]
struct Next {
    task: Option<item::Item>,
    renewed: bool,
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
    let (expires, renewed) = take(&tx, &ctx.actor, r.id, args.ttl, args.steal)?;
    tx.commit()?;
    let item = item::get(&ctx.conn, r.id)?;
    let verb = if renewed { "renewed" } else { "claimed" };
    output::written(
        ctx.json,
        &item,
        &format!("{verb} {} until {expires}", item.id),
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
    drop_claim(&tx, &ctx.actor, r.id, &holder, Reason::Manual)?;
    touch(&tx, r.id)?;
    tx.commit()?;
    let item = item::get(&ctx.conn, r.id)?;
    output::written(ctx.json, &item, &format!("released {}", item.id))
}

/// `apm next`: in one write transaction, pick the best ready task, claim it, assign it to the
/// actor and start it if it is in a todo-category status (when the project has an active
/// one). Returns the exit code: 0, or `NOTHING_READY`.
pub fn next(ctx: &Ctx, args: NextArgs) -> Result<i32> {
    let tx = db::write_tx(&ctx.conn)?;
    let Some(r) = pick(&tx, &ctx.actor, &args)? else {
        drop(tx);
        let none = Next {
            task: None,
            renewed: false,
        };
        output::written(ctx.json, &none, "nothing ready")?;
        return Ok(NOTHING_READY);
    };
    let before = item::get(&tx, r.id)?;
    let (expires, renewed) = take(&tx, &ctx.actor, r.id, args.ttl, false)?;
    // `pick` only offers tasks that are unassigned, the actor's, or assigned to the holder
    // of a lapsed claim: those become the actor's.
    if before.assignee.as_deref() != Some(ctx.actor.as_str()) {
        tx.execute(
            "UPDATE items SET assignee = ?1 WHERE id = ?2",
            params![ctx.actor, r.id],
        )?;
        db::record_event(
            &tx,
            r.id,
            &ctx.actor,
            EventKind::Changed,
            Some("assignee"),
            before.assignee.as_deref().filter(|a| !a.is_empty()),
            Some(&ctx.actor),
        )?;
    }
    let start = status::first_in(&tx, r.project_id, Category::Active)?;
    if let Some(start) = start.filter(|_| before.status_category == Category::Todo.as_str()) {
        tx.execute(
            "UPDATE items SET status_id = ?1 WHERE id = ?2",
            params![start, r.id],
        )?;
        let after = item::get(&tx, r.id)?;
        db::record_event(
            &tx,
            r.id,
            &ctx.actor,
            EventKind::Changed,
            Some("status"),
            Some(&before.status),
            Some(&after.status),
        )?;
    }
    tx.commit()?;
    let item = item::get(&ctx.conn, r.id)?;
    let verb = if renewed { "renewed" } else { "claimed" };
    let line = format!("{verb} {} until {expires}: {}", item.id, item.title);
    let next = Next {
        task: Some(item),
        renewed,
    };
    output::written(ctx.json, &next, &line)?;
    Ok(0)
}

/// `apm claims`: tasks with a live claim, soonest expiry first.
pub fn claims(ctx: &Ctx, args: ClaimsArgs) -> Result<()> {
    let conn = &ctx.conn;
    let mut conds = vec![live()];
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

/// Drop the claim of a task that is now in a closed (done/cancelled-category) status. An
/// unexpired claim records `released` with reason `done`; an expired one is cleared silently.
/// Call it in the transaction that changed the status. Returns whether a claim was released;
/// the caller bumps `updated_at` if its change did not already.
pub fn release_if_closed(conn: &Connection, actor: &str, item_id: i64) -> Result<bool> {
    let (category, holder): (String, Option<String>) = conn.query_row(
        &format!(
            "SELECT s.category, CASE WHEN {} THEN i.claimed_by END
             FROM items i JOIN statuses s ON s.id = i.status_id WHERE i.id = ?1",
            unexpired()
        ),
        params![item_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    if !is_closed(&category) {
        return Ok(false);
    }
    match &holder {
        Some(holder) => drop_claim(conn, actor, item_id, holder, Reason::Done)?,
        None => clear(conn, item_id)?,
    }
    Ok(holder.is_some())
}

/// Claim a task for `actor` for `ttl` seconds, recording `claimed` (and `released` with reason
/// `stolen` when taking someone else's live claim). Returns the new expiry and whether it
/// renewed the actor's own live claim.
fn take(
    conn: &Connection,
    actor: &str,
    item_id: i64,
    ttl: i64,
    steal: bool,
) -> Result<(String, bool)> {
    let it = item::get(conn, item_id)?;
    if is_closed(&it.status_category) {
        return Err(err(format!("cannot claim {}: it is {}", it.id, it.status)));
    }
    let renew = it.claim.as_ref().is_some_and(|c| c.by == actor);
    if let Some(c) = it.claim.as_ref().filter(|_| !renew) {
        if !steal {
            return Err(err(format!(
                "{} is claimed by {} until {}; pass --steal to take it",
                it.id, c.by, c.expires
            )));
        }
        drop_claim(conn, actor, item_id, &c.by, Reason::Stolen)?;
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
    // old = the expiry of the claim this one replaces (renewed or stolen).
    let previous = it.claim.as_ref().map(|c| c.expires.as_str());
    db::record_event(
        conn,
        item_id,
        actor,
        EventKind::Claimed,
        None,
        previous,
        Some(&expires),
    )?;
    Ok((expires, renew))
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
            &format!(
                "SELECT i.claimed_by FROM items i JOIN statuses s ON s.id = i.status_id
                 WHERE i.id = ?1 AND {}",
                live()
            ),
            params![item_id],
            |r| r.get(0),
        )
        .optional()?)
}

/// Clear a claim and record `released` with its reason; old = the holder.
fn drop_claim(
    conn: &Connection,
    actor: &str,
    item_id: i64,
    holder: &str,
    reason: Reason,
) -> Result<()> {
    clear(conn, item_id)?;
    db::record_event(
        conn,
        item_id,
        actor,
        EventKind::Released,
        Some(reason.as_str()),
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

/// Bump a task's `updated_at`.
pub fn touch(conn: &Connection, item_id: i64) -> Result<()> {
    conn.execute(
        &format!(
            "UPDATE items SET updated_at = {} WHERE id = ?1",
            db::NOW_SQL
        ),
        params![item_id],
    )?;
    Ok(())
}

/// The best ready task for `actor`: open, no open blocker, not in a status named blocked, no
/// live claim by someone else, and unassigned, assigned to `actor`, or assigned to the holder
/// of a lapsed claim (an agent that took it with `next` and went away). The actor's own
/// claimed task comes first (a restarted agent resumes it); then highest priority, soonest
/// due (none last), id.
fn pick(conn: &Connection, actor: &str, args: &NextArgs) -> Result<Option<db::ItemRef>> {
    let mut vals: Vec<SqlValue> = Vec::new();
    let closed = item::closed_marks(&mut vals);
    let named = item::bind(&mut vals, "blocked".to_string());
    let me = item::bind(&mut vals, actor.to_string());
    let live = live();
    let mut conds = vec![
        format!("s.category NOT IN ({closed})"),
        format!(
            "NOT (s.name = {named} COLLATE NOCASE OR {})",
            link::has_open_blocker(&closed)
        ),
        format!("NOT ({live} AND i.claimed_by != {me})"),
        format!(
            "(i.assignee IS NULL OR i.assignee = '' OR i.assignee = {me}
              OR (i.assignee = i.claimed_by AND NOT {}))",
            unexpired()
        ),
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
        "SELECT i.id, i.project_id FROM items i
         JOIN projects p ON p.id = i.project_id
         JOIN statuses s ON s.id = i.status_id
         LEFT JOIN milestones m ON m.id = i.milestone_id
         WHERE {}
         ORDER BY ({live} AND i.claimed_by = {me}) DESC, {} DESC, i.due IS NULL, i.due,
                  p.key, i.seq
         LIMIT 1",
        conds.join(" AND "),
        item::priority_rank()
    );
    Ok(conn
        .query_row(&sql, rusqlite::params_from_iter(vals.iter()), |r| {
            Ok(db::ItemRef {
                id: r.get(0)?,
                project_id: r.get(1)?,
            })
        })
        .optional()?)
}
