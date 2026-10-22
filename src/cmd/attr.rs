//! Custom attributes: per-project typed fields on tasks.

use std::collections::HashSet;

use clap::{Subcommand, ValueEnum};
use rusqlite::Connection;
use serde_json::{Map, Value};

use super::{Ctx, not_implemented};
use crate::error::Result;

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum AttrType {
    Text,
    Number,
    Select,
    Date,
    Checkbox,
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

pub fn run(_ctx: &Ctx, _cmd: AttrCmd) -> Result<()> {
    Err(not_implemented("attr"))
}

/// Validate and store `name=value` pairs on a task. Used by item add/edit.
pub fn set_values(
    _conn: &Connection,
    _project_id: i64,
    _item_id: i64,
    pairs: &[(String, String)],
) -> Result<()> {
    if pairs.is_empty() {
        return Ok(());
    }
    Err(not_implemented("--set"))
}

/// Remove a task's value for an attribute name. Returns false if `name` is not an attribute
/// of the project, so the caller can report an unknown field. Used by item edit `--unset`.
pub fn unset_value(
    _conn: &Connection,
    _project_id: i64,
    _item_id: i64,
    _name: &str,
) -> Result<bool> {
    Ok(false)
}

/// Item ids matching every `name=value` condition, or None when there are no conditions.
/// Used by item list `--where`.
pub fn filter(
    _conn: &Connection,
    _project_id: Option<i64>,
    conds: &[(String, String)],
) -> Result<Option<HashSet<i64>>> {
    if conds.is_empty() {
        return Ok(None);
    }
    Err(not_implemented("--where"))
}

/// A task's custom attribute values, typed for JSON (number, bool, string). Used by show/list.
pub fn values_for_item(_conn: &Connection, _item_id: i64) -> Result<Map<String, Value>> {
    Ok(Map::new())
}
