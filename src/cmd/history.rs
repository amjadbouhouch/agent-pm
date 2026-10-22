//! `apm history KEY-N`: a task's events, oldest first.

use clap::Args;
use rusqlite::params;
use serde::Serialize;

use super::Ctx;
use crate::db;
use crate::error::Result;
use crate::output;

#[derive(Args, Debug)]
pub struct HistoryArgs {
    /// Task id, e.g. APM-12
    pub key: String,
    /// Only the N most recent events (still printed oldest first)
    #[arg(long, value_parser = clap::value_parser!(u32).range(1..))]
    pub limit: Option<u32>,
}

#[derive(Serialize, Debug)]
pub struct Event {
    pub at: String,
    pub actor: String,
    pub kind: String,
    pub field: Option<String>,
    pub old: Option<String>,
    pub new: Option<String>,
}

pub fn run(ctx: &Ctx, args: HistoryArgs) -> Result<()> {
    let conn = &ctx.conn;
    let item = db::item(conn, &args.key)?;
    // Newest N by id, then flipped back to oldest first. No limit = -1 (all rows).
    let limit = args.limit.map_or(-1, i64::from);
    let mut stmt = conn.prepare(
        "SELECT at, actor, kind, field, old, new FROM (
             SELECT * FROM events WHERE item_id = ?1 ORDER BY id DESC LIMIT ?2
         ) ORDER BY id",
    )?;
    let rows = stmt.query_map(params![item.id, limit], |r| {
        Ok(Event {
            at: r.get(0)?,
            actor: r.get(1)?,
            kind: r.get(2)?,
            field: r.get(3)?,
            old: r.get(4)?,
            new: r.get(5)?,
        })
    })?;
    let events = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    if ctx.json {
        return output::json(&events);
    }
    for e in &events {
        println!("{}", line(e));
    }
    Ok(())
}

/// `<at> <actor> <kind> <field>: <old> → <new>`. Field changes always show both sides
/// (`-` = empty); other kinds show only the value they carry (`created title`, `tagged bug`).
fn line(e: &Event) -> String {
    let mut s = format!("{} {} {}", e.at, e.actor, e.kind);
    if let Some(f) = &e.field {
        s.push_str(&format!(" {f}:"));
    }
    let dash = |v: &Option<String>| v.clone().unwrap_or_else(|| "-".to_string());
    match (e.kind.as_str(), &e.old, &e.new) {
        ("changed" | "attr", old, new) => s.push_str(&format!(" {} → {}", dash(old), dash(new))),
        (_, Some(old), Some(new)) => s.push_str(&format!(" {old} → {new}")),
        (_, Some(v), None) | (_, None, Some(v)) => s.push_str(&format!(" {v}")),
        (_, None, None) => {}
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(kind: &str, field: Option<&str>, old: Option<&str>, new: Option<&str>) -> Event {
        let s = |v: Option<&str>| v.map(str::to_string);
        Event {
            at: "2026-10-22T10:00:00Z".to_string(),
            actor: "alice".to_string(),
            kind: kind.to_string(),
            field: s(field),
            old: s(old),
            new: s(new),
        }
    }

    #[test]
    fn lines() {
        let at = "2026-10-22T10:00:00Z alice";
        let cases = [
            (ev("created", None, None, Some("t")), "created t"),
            (
                ev("changed", Some("status"), Some("todo"), Some("doing")),
                "changed status: todo → doing",
            ),
            (
                ev("changed", Some("due"), None, Some("2026-12-01")),
                "changed due: - → 2026-12-01",
            ),
            (
                ev("attr", Some("points"), Some("3"), None),
                "attr points: 3 → -",
            ),
            (ev("tagged", None, None, Some("bug")), "tagged bug"),
            (ev("untagged", None, Some("bug"), None), "untagged bug"),
            (
                ev("linked", Some("blocks"), None, Some("APM-2")),
                "linked blocks: APM-2",
            ),
        ];
        for (e, want) in cases {
            assert_eq!(line(&e), format!("{at} {want}"));
        }
    }
}
