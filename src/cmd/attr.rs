//! Custom attributes: per-project typed fields on tasks.

use std::collections::{BTreeMap, HashSet};

use clap::{Subcommand, ValueEnum};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Map, Value, json};

use super::{Ctx, project};
use crate::error::{Result, err};
use crate::{db, model, output};

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum AttrType {
    Text,
    Number,
    Select,
    Date,
    Datetime,
    Checkbox,
}

impl AttrType {
    pub fn as_str(self) -> &'static str {
        match self {
            AttrType::Text => "text",
            AttrType::Number => "number",
            AttrType::Select => "select",
            AttrType::Date => "date",
            AttrType::Datetime => "datetime",
            AttrType::Checkbox => "checkbox",
        }
    }

    fn parse(s: &str) -> Result<Self> {
        <Self as ValueEnum>::from_str(s, false).map_err(|_| err(format!("bad attribute type: {s}")))
    }
}

#[derive(Subcommand, Debug)]
pub enum AttrCmd {
    /// Declare a custom attribute on a project's tasks
    Add {
        name: String,
        #[arg(short, long)]
        project: String,
        #[arg(long = "type", value_enum)]
        kind: AttrType,
        /// Allowed values for a select, comma-separated
        #[arg(long, value_delimiter = ',')]
        options: Vec<String>,
    },
    /// Rename a custom attribute (its values follow)
    Edit {
        name: String,
        #[arg(short, long)]
        project: String,
        /// New name
        #[arg(long)]
        rename: Option<String>,
    },
    /// List a project's custom attributes
    List {
        #[arg(short, long)]
        project: String,
    },
    /// Delete a custom attribute and its values
    Rm {
        name: String,
        #[arg(short, long)]
        project: String,
    },
}

/// Task fields an attribute may not shadow (`--set`, `--where` and `--unset` share one namespace).
const BUILT_IN: &[&str] = &[
    "title",
    "body",
    "status",
    "priority",
    "assignee",
    "due",
    "milestone",
    "parent",
    "id",
    "project",
];

/// One attribute row.
struct Attr {
    id: i64,
    name: String,
    kind: AttrType,
    options: Vec<String>,
}

impl Attr {
    fn from_row(r: &rusqlite::Row) -> rusqlite::Result<(i64, String, String, String)> {
        Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
    }

    fn build((id, name, kind, options): (i64, String, String, String)) -> Result<Attr> {
        Ok(Attr {
            id,
            name,
            kind: AttrType::parse(&kind)?,
            options: serde_json::from_str(&options)?,
        })
    }

    fn to_json(&self, project: &str) -> Value {
        json!({
            "name": self.name,
            "project": project,
            "type": self.kind.as_str(),
            "options": self.options,
        })
    }

    /// Validate `value` for this attribute and return its stored form. Normalized, so equal
    /// values compare equal as text (`yes` -> `true`, `2.0` -> `2`). Only text keeps
    /// surrounding whitespace.
    fn normalize(&self, value: &str) -> Result<String> {
        let bad = |why: String| err(format!("invalid value for {}: {why}", self.name));
        if self.kind == AttrType::Text {
            return Ok(value.to_string());
        }
        let value = value.trim();
        match self.kind {
            AttrType::Text => unreachable!("returned above"),
            // Numbers are f64: integers beyond 2^53 lose precision. `-0` is stored as `0`.
            AttrType::Number => match value.parse::<f64>() {
                Ok(0.0) => Ok("0".to_string()),
                Ok(n) if n.is_finite() => Ok(n.to_string()),
                _ => Err(bad(format!("expected a number, got `{value}`"))),
            },
            AttrType::Date => {
                model::check_date(value).map_err(|e| bad(e.0))?;
                Ok(value.to_string())
            }
            // A date or a date-time, as for `due`; date-times are stored in UTC.
            AttrType::Datetime => model::normalize_when(value).map_err(|e| bad(e.0)),
            AttrType::Checkbox => match value.to_lowercase().as_str() {
                "true" | "yes" | "1" => Ok("true".to_string()),
                "false" | "no" | "0" => Ok("false".to_string()),
                _ => Err(bad(format!("expected true/false, got `{value}`"))),
            },
            AttrType::Select => {
                if self.options.iter().any(|o| o == value) {
                    Ok(value.to_string())
                } else {
                    Err(bad(format!(
                        "`{value}` is not one of {}",
                        self.options.join(", ")
                    )))
                }
            }
        }
    }
}

const COLUMNS: &str = "id, name, type, options";

fn project_attrs(conn: &Connection, project_id: i64) -> Result<Vec<Attr>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM attributes WHERE project_id = ?1 ORDER BY id"
    ))?;
    let rows = stmt
        .query_map(params![project_id], Attr::from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.into_iter().map(Attr::build).collect()
}

fn find(conn: &Connection, project_id: i64, name: &str) -> Result<Option<Attr>> {
    conn.query_row(
        &format!("SELECT {COLUMNS} FROM attributes WHERE project_id = ?1 AND name = ?2"),
        params![project_id, name],
        Attr::from_row,
    )
    .optional()?
    .map(Attr::build)
    .transpose()
}

pub fn run(ctx: &Ctx, cmd: AttrCmd) -> Result<()> {
    match cmd {
        AttrCmd::Add {
            name,
            project,
            kind,
            options,
        } => add(ctx, &name, &project, kind, options),
        AttrCmd::Edit {
            name,
            project,
            rename,
        } => edit(ctx, &name, &project, rename.as_deref()),
        AttrCmd::List { project } => list(ctx, &project),
        AttrCmd::Rm { name, project } => rm(ctx, &name, &project),
    }
}

/// A new attribute name, trimmed: not empty, no `=`, not a built-in field.
fn check_name(name: &str) -> Result<&str> {
    let name = name.trim();
    if name.is_empty() {
        return Err(err("attribute name is empty"));
    }
    if name.contains('=') {
        return Err(err(format!("attribute name may not contain `=`: {name}")));
    }
    if BUILT_IN.contains(&name.to_lowercase().as_str()) {
        return Err(err(format!("built-in field: {name}")));
    }
    Ok(name)
}

/// The UNIQUE (project_id, name) error, for `db::unique`.
fn already_exists(name: &str) -> crate::error::AppError {
    err(format!("attribute already exists: {name}"))
}

/// A project's current key (the user may have passed an alias).
fn key(conn: &Connection, project_id: i64) -> Result<String> {
    Ok(project::get(conn, project_id)?.key)
}

fn add(ctx: &Ctx, name: &str, project: &str, kind: AttrType, options: Vec<String>) -> Result<()> {
    let project_id = db::project_id(&ctx.conn, project)?;
    let name = check_name(name)?;
    let options: Vec<String> = options.iter().map(|o| o.trim().to_string()).collect();
    if kind == AttrType::Select {
        if options.is_empty() {
            return Err(err("a select needs --options"));
        }
        if options.iter().any(String::is_empty) {
            return Err(err("empty option in --options"));
        }
        if let Some(dup) = options
            .iter()
            .enumerate()
            .find_map(|(i, o)| options[..i].contains(o).then_some(o))
        {
            return Err(err(format!("duplicate option: {dup}")));
        }
    } else if !options.is_empty() {
        return Err(err("--options is only for select attributes"));
    }
    // The UNIQUE (project_id, name) constraint is the duplicate check, so a concurrent add
    // of the same name gets the same error.
    db::unique(
        ctx.conn.execute(
            "INSERT INTO attributes (project_id, name, type, options) VALUES (?1, ?2, ?3, ?4)",
            params![
                project_id,
                name,
                kind.as_str(),
                serde_json::to_string(&options)?
            ],
        ),
        || already_exists(name),
    )?;
    let attr = Attr {
        id: ctx.conn.last_insert_rowid(),
        name: name.to_string(),
        kind,
        options,
    };
    let line = format!("added attribute {name}");
    output::written(ctx.json, &attr.to_json(&key(&ctx.conn, project_id)?), &line)
}

fn edit(ctx: &Ctx, name: &str, project: &str, rename: Option<&str>) -> Result<()> {
    let Some(rename) = rename else {
        return Err(err("nothing to change: pass --rename"));
    };
    let rename = check_name(rename)?;
    let project_id = db::project_id(&ctx.conn, project)?;
    let attr = find(&ctx.conn, project_id, name)?
        .ok_or_else(|| err(format!("no such attribute: {name}")))?;
    db::unique(
        ctx.conn.execute(
            "UPDATE attributes SET name = ?1 WHERE id = ?2",
            params![rename, attr.id],
        ),
        || already_exists(rename),
    )?;
    let attr = Attr {
        name: rename.to_string(),
        ..attr
    };
    let line = format!("renamed attribute {name} to {rename}");
    output::written(ctx.json, &attr.to_json(&key(&ctx.conn, project_id)?), &line)
}

fn list(ctx: &Ctx, project: &str) -> Result<()> {
    let project_id = db::project_id(&ctx.conn, project)?;
    let attrs = project_attrs(&ctx.conn, project_id)?;
    if ctx.json {
        let key = key(&ctx.conn, project_id)?;
        let all: Vec<Value> = attrs.iter().map(|a| a.to_json(&key)).collect();
        return output::json(&all);
    }
    let rows: Vec<Vec<String>> = attrs
        .iter()
        .map(|a| {
            vec![
                a.name.clone(),
                a.kind.as_str().to_string(),
                a.options.join(","),
            ]
        })
        .collect();
    output::table(&["NAME", "TYPE", "OPTIONS"], &rows);
    Ok(())
}

fn rm(ctx: &Ctx, name: &str, project: &str) -> Result<()> {
    let project_id = db::project_id(&ctx.conn, project)?;
    let attr = find(&ctx.conn, project_id, name)?
        .ok_or_else(|| err(format!("no such attribute: {name}")))?;
    // Values go with it (item_values.attribute_id ON DELETE CASCADE).
    ctx.conn
        .execute("DELETE FROM attributes WHERE id = ?1", params![attr.id])?;
    let line = format!("removed attribute {name}");
    output::written(ctx.json, &attr.to_json(&key(&ctx.conn, project_id)?), &line)
}

/// Validate and store `name=value` pairs on a task. Used by item add/edit.
/// Every pair is validated before anything is written, so a bad pair writes nothing. The writes
/// themselves are atomic only if the caller holds a transaction (pass `&tx`, it derefs to
/// `&Connection`); this function opens none.
pub fn set_values(
    conn: &Connection,
    project_id: i64,
    item_id: i64,
    pairs: &[(String, String)],
) -> Result<()> {
    if pairs.is_empty() {
        return Ok(());
    }
    let attrs = project_attrs(conn, project_id)?;
    let mut rows = Vec::with_capacity(pairs.len());
    for (name, value) in pairs {
        let attr = attrs
            .iter()
            .find(|a| &a.name == name)
            .ok_or_else(|| err(format!("unknown attribute: {name}")))?;
        rows.push((attr.id, attr.normalize(value)?));
    }
    let mut stmt = conn.prepare(
        "INSERT INTO item_values (item_id, attribute_id, value) VALUES (?1, ?2, ?3)
         ON CONFLICT (item_id, attribute_id) DO UPDATE SET value = excluded.value",
    )?;
    for (attribute_id, value) in rows {
        stmt.execute(params![item_id, attribute_id, value])?;
    }
    Ok(())
}

/// Remove a task's value for an attribute name. Returns false if `name` is not an attribute
/// of the project, so the caller can report an unknown field. Used by item edit `--unset`.
pub fn unset_value(conn: &Connection, project_id: i64, item_id: i64, name: &str) -> Result<bool> {
    let Some(attr) = find(conn, project_id, name)? else {
        return Ok(false);
    };
    conn.execute(
        "DELETE FROM item_values WHERE item_id = ?1 AND attribute_id = ?2",
        params![item_id, attr.id],
    )?;
    Ok(true)
}

/// Item ids matching every `name=value` condition, or None when there are no conditions.
/// Used by item list `--where`. With no project, a name matches that attribute in any project.
/// Values compare normalized (`yes` matches `true`, `2` matches `2.0`).
pub fn filter(
    conn: &Connection,
    project_id: Option<i64>,
    conds: &[(String, String)],
) -> Result<Option<HashSet<i64>>> {
    if conds.is_empty() {
        return Ok(None);
    }
    let mut attr_stmt = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM attributes WHERE name = ?1 AND (?2 IS NULL OR project_id = ?2)"
    ))?;
    let mut value_stmt =
        conn.prepare("SELECT item_id FROM item_values WHERE attribute_id = ?1 AND value = ?2")?;
    let mut result: Option<HashSet<i64>> = None;
    for (name, value) in conds {
        let attrs = attr_stmt
            .query_map(params![name, project_id], Attr::from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if attrs.is_empty() {
            return Err(err(format!("unknown attribute: {name}")));
        }
        let mut matched = HashSet::new();
        let mut first_err = None;
        let mut valid = false;
        for attr in attrs.into_iter().map(Attr::build) {
            let attr = attr?;
            // Across projects one name may have different types; skip those the value can't be.
            let normalized = match attr.normalize(value) {
                Ok(v) => v,
                Err(e) => {
                    first_err.get_or_insert(e);
                    continue;
                }
            };
            valid = true;
            let ids = value_stmt.query_map(params![attr.id, normalized], |r| r.get(0))?;
            for id in ids {
                matched.insert(id?);
            }
        }
        if !valid {
            return Err(first_err.unwrap_or_else(|| err("invalid value")));
        }
        result = Some(match result {
            None => matched,
            Some(prev) => prev.intersection(&matched).copied().collect(),
        });
    }
    Ok(result)
}

/// A task's custom attribute values, typed for JSON (number, bool, string). Used by show/list.
pub fn values_for_item(conn: &Connection, item_id: i64) -> Result<Map<String, Value>> {
    let mut stmt = conn.prepare(
        "SELECT a.name, a.type, v.value FROM item_values v
         JOIN attributes a ON a.id = v.attribute_id
         WHERE v.item_id = ?1 ORDER BY a.id",
    )?;
    let rows = stmt.query_map(params![item_id], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
        ))
    })?;
    let mut out = Map::new();
    for row in rows {
        let (name, kind, value) = row?;
        let typed = match AttrType::parse(&kind)? {
            AttrType::Number => typed_number(&value),
            AttrType::Checkbox => Value::Bool(value == "true"),
            _ => Value::String(value),
        };
        out.insert(name, typed);
    }
    Ok(out)
}

/// Re-point a task's values at another project's attributes, for `move`: a value is kept
/// where the target has an attribute of the same name and type that accepts it, and deleted
/// otherwise. Returns the deleted values (name -> stored value).
pub fn move_values(
    conn: &Connection,
    item_id: i64,
    project_id: i64,
) -> Result<BTreeMap<String, String>> {
    let targets = project_attrs(conn, project_id)?;
    let mut stmt = conn.prepare(
        "SELECT v.attribute_id, a.name, a.type, v.value FROM item_values v
         JOIN attributes a ON a.id = v.attribute_id
         WHERE v.item_id = ?1",
    )?;
    let values = stmt
        .query_map(params![item_id], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut dropped = BTreeMap::new();
    for (attribute_id, name, kind, value) in values {
        let target = targets
            .iter()
            .find(|a| a.name == name && a.kind.as_str() == kind)
            .filter(|a| a.normalize(&value).is_ok());
        match target {
            Some(t) => conn.execute(
                "UPDATE item_values SET attribute_id = ?1 WHERE item_id = ?2 AND attribute_id = ?3",
                params![t.id, item_id, attribute_id],
            )?,
            None => {
                dropped.insert(name, value);
                conn.execute(
                    "DELETE FROM item_values WHERE item_id = ?1 AND attribute_id = ?2",
                    params![item_id, attribute_id],
                )?
            }
        };
    }
    Ok(dropped)
}

/// A task's attribute values as stored (normalized text), by attribute name. Used by history.
pub fn stored_values(conn: &Connection, item_id: i64) -> Result<BTreeMap<String, String>> {
    let mut stmt = conn.prepare(
        "SELECT a.name, v.value FROM item_values v
         JOIN attributes a ON a.id = v.attribute_id
         WHERE v.item_id = ?1",
    )?;
    let rows = stmt.query_map(params![item_id], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// A stored number as JSON: integers stay integers (`2`, not `2.0`).
fn typed_number(s: &str) -> Value {
    if let Ok(i) = s.parse::<i64>() {
        return Value::from(i);
    }
    s.parse::<f64>()
        .ok()
        .and_then(serde_json::Number::from_f64)
        .map_or_else(|| Value::String(s.to_string()), Value::Number)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An in-memory database with project P (id 1), items 1 and 2, and one attribute per type.
    fn setup() -> Connection {
        let conn = db::open(std::path::Path::new(":memory:")).unwrap();
        conn.execute_batch(
            r#"
            INSERT INTO projects (id, key, name) VALUES (1, 'P', 'p'), (2, 'Q', 'q');
            INSERT INTO statuses (id, project_id, name, category, position)
                VALUES (1, 1, 'todo', 'todo', 1), (2, 2, 'todo', 'todo', 1);
            INSERT INTO items (id, project_id, seq, title, status_id, created_by)
                VALUES (1, 1, 1, 'a', 1, 't'), (2, 1, 2, 'b', 1, 't'), (3, 2, 1, 'c', 2, 't');
            INSERT INTO attributes (project_id, name, type, options) VALUES
                (1, 'note', 'text', '[]'),
                (1, 'points', 'number', '[]'),
                (1, 'size', 'select', '["s","m","l"]'),
                (1, 'when', 'date', '[]'),
                (1, 'flag', 'checkbox', '[]'),
                (1, 'at', 'datetime', '[]'),
                (2, 'points', 'number', '[]');
            "#,
        )
        .unwrap();
        conn
    }

    fn pairs(p: &[(&str, &str)]) -> Vec<(String, String)> {
        p.iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn ids(v: &[i64]) -> Option<HashSet<i64>> {
        Some(v.iter().copied().collect())
    }

    #[test]
    fn set_and_read_typed_values() {
        let conn = setup();
        set_values(
            &conn,
            1,
            1,
            &pairs(&[
                ("note", "hi there"),
                ("points", "2.0"),
                ("size", "m"),
                ("when", "2026-10-22"),
                ("flag", "Yes"),
            ]),
        )
        .unwrap();
        let v = Value::Object(values_for_item(&conn, 1).unwrap());
        assert_eq!(
            v,
            json!({"note": "hi there", "points": 2, "size": "m", "when": "2026-10-22", "flag": true})
        );
        // Upsert.
        set_values(&conn, 1, 1, &pairs(&[("points", "1.5"), ("flag", "0")])).unwrap();
        let v = values_for_item(&conn, 1).unwrap();
        assert_eq!(v["points"], json!(1.5));
        assert_eq!(v["flag"], json!(false));
    }

    #[test]
    fn set_validates_every_type() {
        let conn = setup();
        let e = |name: &str, value: &str| set_values(&conn, 1, 1, &pairs(&[(name, value)])).err();
        assert_eq!(e("nope", "x").unwrap().0, "unknown attribute: nope");
        assert!(e("points", "abc").unwrap().0.contains("expected a number"));
        assert!(e("points", "inf").is_some());
        assert!(e("points", "NaN").is_some());
        assert!(e("size", "xl").unwrap().0.contains("not one of s, m, l"));
        assert!(e("size", "M").is_some());
        assert!(e("when", "2026-02-30").unwrap().0.contains("invalid date"));
        assert!(
            e("flag", "maybe")
                .unwrap()
                .0
                .contains("expected true/false")
        );
        assert!(e("note", "").is_none());
        assert!(e("when", " 2026-10-22 ").is_none());
        assert!(e("size", " m ").is_none());
        assert!(e("flag", "TRUE").is_none());
        assert!(e("flag", "NO").is_none());
        // Another project's attribute is unknown here.
        assert!(set_values(&conn, 2, 3, &pairs(&[("size", "m")])).is_err());
    }

    #[test]
    fn set_is_all_or_nothing() {
        let conn = setup();
        let err = set_values(&conn, 1, 1, &pairs(&[("note", "x"), ("points", "abc")]));
        assert!(err.is_err());
        assert!(values_for_item(&conn, 1).unwrap().is_empty());
    }

    #[test]
    fn unset() {
        let conn = setup();
        set_values(&conn, 1, 1, &pairs(&[("note", "x"), ("size", "s")])).unwrap();
        assert!(unset_value(&conn, 1, 1, "note").unwrap());
        assert!(!unset_value(&conn, 1, 1, "nope").unwrap());
        // Unsetting an attribute with no value is fine.
        assert!(unset_value(&conn, 1, 1, "points").unwrap());
        assert_eq!(
            Value::Object(values_for_item(&conn, 1).unwrap()),
            json!({"size": "s"})
        );
    }

    #[test]
    fn filter_normalizes_and_intersects() {
        let conn = setup();
        set_values(&conn, 1, 1, &pairs(&[("points", "2"), ("flag", "true")])).unwrap();
        set_values(&conn, 1, 2, &pairs(&[("points", "2.0"), ("flag", "no")])).unwrap();
        set_values(&conn, 2, 3, &pairs(&[("points", "2")])).unwrap();
        assert_eq!(filter(&conn, Some(1), &[]).unwrap(), None);
        assert_eq!(
            filter(&conn, Some(1), &pairs(&[("points", "2.00")])).unwrap(),
            ids(&[1, 2])
        );
        assert_eq!(
            filter(&conn, Some(1), &pairs(&[("flag", "yes")])).unwrap(),
            ids(&[1])
        );
        assert_eq!(
            filter(&conn, Some(1), &pairs(&[("points", "2"), ("flag", "0")])).unwrap(),
            ids(&[2])
        );
        assert_eq!(
            filter(&conn, Some(1), &pairs(&[("points", "3")])).unwrap(),
            ids(&[])
        );
        // Any project.
        assert_eq!(
            filter(&conn, None, &pairs(&[("points", "2")])).unwrap(),
            ids(&[1, 2, 3])
        );
        assert_eq!(
            filter(&conn, None, &pairs(&[("size", "x")])).unwrap_err().0,
            "invalid value for size: `x` is not one of s, m, l"
        );
        assert_eq!(
            filter(&conn, Some(2), &pairs(&[("flag", "yes")]))
                .unwrap_err()
                .0,
            "unknown attribute: flag"
        );
    }

    #[test]
    fn values_are_trimmed_except_text() {
        let conn = setup();
        set_values(
            &conn,
            1,
            1,
            &pairs(&[
                ("note", " x "),
                ("size", " l"),
                ("when", "2026-01-02 "),
                ("flag", " NO "),
                ("points", " -0 "),
            ]),
        )
        .unwrap();
        assert_eq!(
            Value::Object(values_for_item(&conn, 1).unwrap()),
            json!({"note": " x ", "size": "l", "when": "2026-01-02", "flag": false, "points": 0})
        );
        assert_eq!(
            filter(
                &conn,
                Some(1),
                &pairs(&[("points", "0"), ("flag", "False")])
            )
            .unwrap(),
            ids(&[1])
        );
        assert_eq!(
            filter(&conn, Some(1), &pairs(&[("points", "-0.0")])).unwrap(),
            ids(&[1])
        );
    }

    #[test]
    fn datetimes_normalize_to_utc() {
        let conn = setup();
        let e = |name: &str, value: &str| set_values(&conn, 1, 1, &pairs(&[(name, value)])).err();
        assert!(
            e("at", "2026-10-22T09:30")
                .unwrap()
                .0
                .contains("add Z or an offset")
        );
        assert!(
            e("at", "2026-10-22T25:00Z")
                .unwrap()
                .0
                .contains("invalid value for at")
        );
        assert!(
            e("when", "2026-10-22T09:30Z")
                .unwrap()
                .0
                .contains("invalid date")
        );
        set_values(&conn, 1, 1, &pairs(&[("at", " 2026-10-22T09:30+02:00 ")])).unwrap();
        assert_eq!(
            values_for_item(&conn, 1).unwrap()["at"],
            json!("2026-10-22T07:30:00Z")
        );
        set_values(&conn, 1, 2, &pairs(&[("at", "2026-10-22")])).unwrap();
        assert_eq!(
            values_for_item(&conn, 2).unwrap()["at"],
            json!("2026-10-22")
        );
        assert_eq!(
            filter(&conn, Some(1), &pairs(&[("at", "2026-10-22T08:30+01:00")])).unwrap(),
            ids(&[1])
        );
    }

    #[test]
    fn numbers_as_json() {
        assert_eq!(typed_number("3"), json!(3));
        assert_eq!(typed_number("-0.25"), json!(-0.25));
    }
}
