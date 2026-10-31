use std::path::PathBuf;

use clap::Args;
use rusqlite::{Connection, OptionalExtension, ToSql, params};
use serde::Serialize;

use super::Ctx;
use crate::db::{self, EventKind};
use crate::error::{Result, err};
use crate::model::CommentKind;
use crate::{input, output};

#[derive(Args, Debug)]
pub struct CommentArgs {
    /// Task id, e.g. APM-12
    pub key: String,
    /// The comment, Markdown by convention; `-` reads it from stdin (or use --file)
    #[arg(
        value_name = "TEXT|-",
        required_unless_present = "file",
        conflicts_with = "file"
    )]
    pub text: Option<String>,
    /// Read the comment from a file instead
    #[arg(long, value_name = "PATH")]
    pub file: Option<PathBuf>,
    /// What the entry is for (default note); `show` marks every kind but note
    #[arg(long, value_enum)]
    pub kind: Option<CommentKind>,
    /// Answer question #ID on this task, `41` or `#41` as `show` prints it (makes the entry
    /// an answer; the question is no longer open)
    #[arg(
        long,
        value_name = "ID",
        required_if_eq("kind", "answer"),
        value_parser = parse_comment_id
    )]
    pub answer: Option<i64>,
}

/// clap value parser for a comment id: `41`, or `#41` as `show` and `comment` print it.
fn parse_comment_id(s: &str) -> std::result::Result<i64, String> {
    let n = s.strip_prefix('#').unwrap_or(s);
    match n.parse::<i64>() {
        Ok(id) if id > 0 && n.bytes().all(|b| b.is_ascii_digit()) => Ok(id),
        _ => Err(format!("expected a comment id like 41 or #41, got `{s}`")),
    }
}

/// `--answer` with a `--kind` other than answer, as a usage error (checked after parsing:
/// clap has no conflict with one value of an argument).
pub fn kind_conflict(args: &CommentArgs) -> Option<String> {
    match (args.kind, args.answer) {
        (Some(k), Some(_)) if k != CommentKind::Answer => Some(format!(
            "--answer makes the comment an answer, not a {}; drop --kind",
            k.as_str()
        )),
        _ => None,
    }
}

#[derive(Serialize, Debug)]
pub struct Comment {
    pub id: i64,
    /// The task's KEY-N
    pub item: String,
    pub kind: CommentKind,
    /// For an answer: the id of the question it answers (None once that question is gone)
    pub answers: Option<i64>,
    pub author: String,
    pub body: String,
    pub created_at: String,
}

const SELECT: &str = "SELECT c.id, p.key || '-' || i.seq, c.kind, c.answers, c.author, c.body,
            c.created_at
     FROM comments c
     JOIN items i ON i.id = c.item_id
     JOIN projects p ON p.id = i.project_id";

fn row(r: &rusqlite::Row) -> rusqlite::Result<Comment> {
    Ok(Comment {
        id: r.get(0)?,
        item: r.get(1)?,
        kind: r.get(2)?,
        answers: r.get(3)?,
        author: r.get(4)?,
        body: r.get(5)?,
        created_at: r.get(6)?,
    })
}

/// `apm comment KEY-N <text>`: add a log entry (a comment) authored by the actor.
pub fn run(ctx: &Ctx, args: CommentArgs) -> Result<()> {
    // `kind_conflict` already rejected --answer with another --kind.
    let kind = match args.answer {
        Some(_) => CommentKind::Answer,
        None => args.kind.unwrap_or(CommentKind::Note),
    };
    let text =
        input::read_text(args.text, args.file.as_deref(), &input::COMMENT)?.unwrap_or_default();
    if text.trim().is_empty() {
        return Err(err("comment text cannot be empty"));
    }
    // A comment is activity on the task: bump its updated_at in the same transaction.
    let tx = db::write_tx(&ctx.conn)?;
    let item = db::item(&tx, &args.key)?;
    if let Some(q) = args.answer {
        check_question(&tx, q, item.id)?;
    }
    tx.execute(
        "INSERT INTO comments (item_id, author, body, kind, answers) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![item.id, ctx.actor, text, kind.as_str(), args.answer],
    )?;
    let id = tx.last_insert_rowid();
    let field = (kind != CommentKind::Note).then_some(kind.as_str());
    db::record_event(
        &tx,
        item.id,
        &ctx.actor,
        EventKind::Commented,
        field,
        None,
        Some(&text),
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
    let line = match (c.kind, c.answers) {
        (CommentKind::Question, _) => format!("asked on {} (#{})", c.item, c.id),
        (CommentKind::Answer, Some(q)) => format!("answered #{q} on {}", c.item),
        (CommentKind::Handoff, _) => format!("handoff on {}", c.item),
        _ => format!("commented on {}", c.item),
    };
    output::written(ctx.json, &c, &line)
}

/// An answer's target must be a question on the same task.
fn check_question(conn: &Connection, id: i64, item_id: i64) -> Result<()> {
    let (on, kind): (i64, CommentKind) = conn
        .query_row(
            "SELECT item_id, kind FROM comments WHERE id = ?1",
            params![id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?
        .ok_or_else(|| err(format!("no such comment: #{id}")))?;
    if on != item_id {
        return Err(err(format!(
            "comment #{id} is on {}, not {}; answer it there",
            db::current_key(conn, on)?,
            db::current_key(conn, item_id)?
        )));
    }
    let is = match kind {
        CommentKind::Question => return Ok(()),
        CommentKind::Note => "a note",
        CommentKind::Handoff => "a handoff",
        CommentKind::Answer => "an answer",
    };
    Err(err(format!("comment #{id} is not a question (it is {is})")))
}

/// A task's comments, oldest first. Used by `show`.
pub fn for_item(conn: &Connection, item_id: i64) -> Result<Vec<Comment>> {
    let mut stmt = conn.prepare(&format!("{SELECT} WHERE c.item_id = ?1 ORDER BY c.id"))?;
    let rows = stmt.query_map(params![item_id], row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Whether a task has any log entry at all.
pub fn any(conn: &Connection, item_id: i64) -> Result<bool> {
    Ok(conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM comments WHERE item_id = ?1)",
        params![item_id],
        |r| r.get(0),
    )?)
}

/// The questions no comment answers yet on the tasks that `items` selects (SQL returning item
/// ids, with parameters `args`): the oldest `limit`, and how many there are in all.
pub fn open_questions(
    conn: &Connection,
    items: &str,
    args: &[&dyn ToSql],
    limit: usize,
) -> Result<(Vec<Comment>, i64)> {
    let filter = format!(
        "c.item_id IN ({items}) AND c.kind = 'question'
         AND NOT EXISTS (SELECT 1 FROM comments a WHERE a.answers = c.id)"
    );
    let mut stmt = conn.prepare(&format!(
        "{SELECT} WHERE {filter} ORDER BY c.id LIMIT {limit}"
    ))?;
    let rows = stmt
        .query_map(args, row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let total = conn.query_row(
        &format!("SELECT count(*) FROM comments c WHERE {filter}"),
        args,
        |r| r.get(0),
    )?;
    Ok((rows, total))
}

/// A task's newest `n` comments other than those in `skip`, oldest first, and how many such
/// comments it has in all.
pub fn recent(
    conn: &Connection,
    item_id: i64,
    skip: &[i64],
    n: u32,
) -> Result<(Vec<Comment>, i64)> {
    let skip: Vec<String> = skip.iter().map(i64::to_string).collect();
    let filter = format!("c.item_id = ?1 AND c.id NOT IN ({})", skip.join(", "));
    let mut stmt = conn.prepare(&format!(
        "SELECT * FROM ({SELECT} WHERE {filter} ORDER BY c.id DESC LIMIT ?2) ORDER BY 1"
    ))?;
    let rows = stmt
        .query_map(params![item_id, n], row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let total = conn.query_row(
        &format!("SELECT count(*) FROM comments c WHERE {filter}"),
        params![item_id],
        |r| r.get(0),
    )?;
    Ok((rows, total))
}

/// A task's newest comment of a kind (e.g. its latest handoff), if any.
pub fn latest(conn: &Connection, item_id: i64, kind: CommentKind) -> Result<Option<Comment>> {
    Ok(conn
        .query_row(
            &format!("{SELECT} WHERE c.item_id = ?1 AND c.kind = ?2 ORDER BY c.id DESC LIMIT 1"),
            params![item_id, kind.as_str()],
            row,
        )
        .optional()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comment_ids() {
        assert_eq!(parse_comment_id("41"), Ok(41));
        assert_eq!(parse_comment_id("#41"), Ok(41));
        for bad in ["", "#", "##41", "0", "#0", "-3", "+3", "4a", "# 4"] {
            assert!(parse_comment_id(bad).is_err(), "{bad}");
        }
    }

    /// A migrated in-memory database with tasks 1 and 2 (APM-1, APM-2) and comments
    /// `(task, kind, answers)`; comment ids are 1..n in order.
    fn conn(comments: &[(i64, &str, Option<i64>)]) -> Connection {
        let conn = db::open(std::path::Path::new(":memory:")).unwrap();
        conn.execute_batch(
            "INSERT INTO projects (id, key, name) VALUES (1, 'APM', 'apm');
             INSERT INTO statuses (id, project_id, name, category, position)
                 VALUES (1, 1, 'todo', 'todo', 1);
             INSERT INTO items (id, project_id, seq, title, status_id, created_by)
                 VALUES (1, 1, 1, 'one', 1, 'a'), (2, 1, 2, 'two', 1, 'a');",
        )
        .unwrap();
        for (n, (item, kind, answers)) in comments.iter().enumerate() {
            conn.execute(
                "INSERT INTO comments (item_id, author, body, kind, answers)
                 VALUES (?1, 'a', ?2, ?3, ?4)",
                params![item, format!("c{}", n + 1), kind, answers],
            )
            .unwrap();
        }
        conn
    }

    fn ids(comments: &[Comment]) -> Vec<i64> {
        comments.iter().map(|c| c.id).collect()
    }

    /// Open question ids on the given tasks (all of them), and the total.
    fn open(conn: &Connection, tasks: &[i64], limit: usize) -> (Vec<i64>, i64) {
        let list: Vec<String> = tasks.iter().map(i64::to_string).collect();
        let items = format!("SELECT id FROM items WHERE id IN ({})", list.join(", "));
        let (qs, total) = open_questions(conn, &items, &[], limit).unwrap();
        (ids(&qs), total)
    }

    #[test]
    fn open_questions_are_the_unanswered_ones() {
        let conn = conn(&[
            (1, "question", None),  // 1: answered by 3
            (1, "question", None),  // 2: open
            (1, "answer", Some(1)), // 3
            (1, "note", None),      // 4
            (2, "question", None),  // 5: open, on APM-2
            (2, "handoff", None),   // 6
        ]);
        assert_eq!(open(&conn, &[1], 10), (vec![2], 1));
        assert_eq!(open(&conn, &[1, 2], 10), (vec![2, 5], 2));
        assert_eq!(open(&conn, &[1, 2], 1), (vec![2], 2));
        assert_eq!(open(&conn, &[2], 10), (vec![5], 1));
        let (qs, _) = open_questions(&conn, "SELECT ?1", &[&2], 10).unwrap();
        assert_eq!(
            (qs[0].item.as_str(), qs[0].kind),
            ("APM-2", CommentKind::Question)
        );
        // Answering closes it.
        conn.execute(
            "INSERT INTO comments (item_id, author, body, kind, answers)
             VALUES (2, 'b', 'yes', 'answer', 5)",
            [],
        )
        .unwrap();
        assert_eq!(open(&conn, &[1, 2], 10), (vec![2], 1));
    }

    #[test]
    fn recent_is_the_newest_n_but_skipped_ones() {
        let conn = conn(&[
            (1, "note", None),
            (1, "note", None),
            (1, "handoff", None),
            (1, "note", None),
            (2, "note", None),
        ]);
        let r = |skip: &[i64], n| {
            let (c, total) = recent(&conn, 1, skip, n).unwrap();
            (ids(&c), total)
        };
        assert_eq!(r(&[], 2), (vec![3, 4], 4));
        assert_eq!(r(&[3], 2), (vec![2, 4], 3));
        assert_eq!(r(&[3], 0), (vec![], 3));
        assert_eq!(r(&[], 50), (vec![1, 2, 3, 4], 4));
        assert!(any(&conn, 1).unwrap());
        conn.execute("INSERT INTO items (id, project_id, seq, title, status_id, created_by) VALUES (3, 1, 3, 't', 1, 'a')", []).unwrap();
        assert!(!any(&conn, 3).unwrap());
    }

    #[test]
    fn latest_is_the_newest_of_a_kind() {
        let conn = conn(&[
            (1, "handoff", None),
            (1, "note", None),
            (1, "handoff", None),
            (1, "note", None),
            (2, "handoff", None),
        ]);
        let id = |item, kind| latest(&conn, item, kind).unwrap().map(|c| c.id);
        assert_eq!(id(1, CommentKind::Handoff), Some(3));
        assert_eq!(id(1, CommentKind::Note), Some(4));
        assert_eq!(id(2, CommentKind::Handoff), Some(5));
        assert_eq!(id(2, CommentKind::Question), None);
    }
}
