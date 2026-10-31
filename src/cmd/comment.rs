use std::path::PathBuf;

use clap::Args;
use rusqlite::{Connection, OptionalExtension, params, params_from_iter};
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
    /// Answer question #ID on this task (makes the entry an answer; the question is no
    /// longer open)
    #[arg(long, value_name = "ID", required_if_eq("kind", "answer"))]
    pub answer: Option<i64>,
}

#[derive(Serialize, Debug)]
pub struct Comment {
    pub id: i64,
    /// The task's KEY-N
    pub item: String,
    /// note, handoff, question or answer
    pub kind: String,
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
    let kind = match (args.kind, args.answer) {
        (Some(k), Some(_)) if k != CommentKind::Answer => {
            return Err(err(format!(
                "--answer makes the comment an answer, not a {}; drop --kind",
                k.as_str()
            )));
        }
        (_, Some(_)) => CommentKind::Answer,
        (k, None) => k.unwrap_or(CommentKind::Note),
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
    let line = match kind {
        CommentKind::Note => format!("commented on {}", c.item),
        CommentKind::Handoff => format!("handoff on {}", c.item),
        CommentKind::Question => format!("asked on {} (#{})", c.item, c.id),
        CommentKind::Answer => format!(
            "answered #{} on {}",
            args.answer.unwrap_or_default(),
            c.item
        ),
    };
    output::written(ctx.json, &c, &line)
}

/// An answer's target must be a question on the same task.
fn check_question(conn: &Connection, id: i64, item_id: i64) -> Result<()> {
    let (on, kind): (i64, String) = conn
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
    if kind != CommentKind::Question.as_str() {
        let a = if kind.starts_with(['a', 'e', 'i', 'o', 'u']) {
            "an"
        } else {
            "a"
        };
        return Err(err(format!(
            "comment #{id} is not a question (it is {a} {kind})"
        )));
    }
    Ok(())
}

/// A task's comments, oldest first. Used by `show`.
pub fn for_item(conn: &Connection, item_id: i64) -> Result<Vec<Comment>> {
    let mut stmt = conn.prepare(&format!("{SELECT} WHERE c.item_id = ?1 ORDER BY c.id"))?;
    let rows = stmt.query_map(params![item_id], row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// The questions on these tasks that no comment answers yet, oldest first.
#[allow(dead_code)] // `apm context` (S4) lists them
pub fn open_questions(conn: &Connection, item_ids: &[i64]) -> Result<Vec<Comment>> {
    if item_ids.is_empty() {
        return Ok(Vec::new());
    }
    let marks = vec!["?"; item_ids.len()].join(", ");
    let mut stmt = conn.prepare(&format!(
        "{SELECT} WHERE c.item_id IN ({marks}) AND c.kind = 'question'
           AND NOT EXISTS (SELECT 1 FROM comments a WHERE a.answers = c.id)
         ORDER BY c.id"
    ))?;
    let rows = stmt.query_map(params_from_iter(item_ids), row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// A task's newest comment of a kind (e.g. its latest handoff), if any.
#[allow(dead_code)] // `apm context` (S4) shows the latest handoff
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
        assert_eq!(ids(&open_questions(&conn, &[1]).unwrap()), [2]);
        assert_eq!(ids(&open_questions(&conn, &[1, 2]).unwrap()), [2, 5]);
        assert_eq!(ids(&open_questions(&conn, &[2]).unwrap()), [5]);
        assert!(open_questions(&conn, &[]).unwrap().is_empty());
        let q = &open_questions(&conn, &[2]).unwrap()[0];
        assert_eq!((q.item.as_str(), q.kind.as_str()), ("APM-2", "question"));
        // Answering closes it.
        conn.execute(
            "INSERT INTO comments (item_id, author, body, kind, answers)
             VALUES (2, 'b', 'yes', 'answer', 5)",
            [],
        )
        .unwrap();
        assert_eq!(ids(&open_questions(&conn, &[1, 2]).unwrap()), [2]);
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
