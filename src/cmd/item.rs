//! Tasks and subtasks ("items"), referenced as KEY-N.

use clap::Args;

use super::{Ctx, not_implemented};
use crate::error::Result;
use crate::model::{Priority, Status, parse_pair};

#[derive(Args, Debug)]
pub struct KeyArg {
    /// Task id, e.g. APM-12
    pub key: String,
}

#[derive(Args, Debug)]
pub struct AddArgs {
    pub title: String,
    /// Project key (optional with --parent: the parent's project is used)
    #[arg(short, long)]
    pub project: Option<String>,
    /// Make this a subtask of KEY-N
    #[arg(long)]
    pub parent: Option<String>,
    /// Milestone name in the same project
    #[arg(short, long)]
    pub milestone: Option<String>,
    #[arg(short, long)]
    pub assignee: Option<String>,
    #[arg(long, value_enum)]
    pub priority: Option<Priority>,
    #[arg(long, value_enum)]
    pub status: Option<Status>,
    /// Due date, YYYY-MM-DD
    #[arg(long)]
    pub due: Option<String>,
    /// Longer description
    #[arg(short, long)]
    pub body: Option<String>,
    /// Custom attribute value, name=value (repeatable)
    #[arg(long = "set", value_name = "NAME=VALUE", value_parser = parse_pair)]
    pub set: Vec<(String, String)>,
}

#[derive(Args, Debug)]
pub struct ListArgs {
    #[arg(short, long)]
    pub project: Option<String>,
    /// Only these statuses (comma-separated); implies closed ones may show
    #[arg(long, value_enum, value_delimiter = ',')]
    pub status: Vec<Status>,
    #[arg(short, long)]
    pub assignee: Option<String>,
    #[arg(short, long)]
    pub milestone: Option<String>,
    /// Only subtasks of KEY-N
    #[arg(long)]
    pub parent: Option<String>,
    /// Custom attribute filter, name=value (repeatable, all must match)
    #[arg(long = "where", value_name = "NAME=VALUE", value_parser = parse_pair)]
    pub filter: Vec<(String, String)>,
    /// Include done and cancelled tasks
    #[arg(long)]
    pub all: bool,
}

#[derive(Args, Debug)]
pub struct EditArgs {
    pub key: String,
    #[arg(long)]
    pub title: Option<String>,
    #[arg(short, long)]
    pub body: Option<String>,
    #[arg(long, value_enum)]
    pub status: Option<Status>,
    #[arg(long, value_enum)]
    pub priority: Option<Priority>,
    #[arg(short, long)]
    pub assignee: Option<String>,
    #[arg(long)]
    pub due: Option<String>,
    #[arg(short, long)]
    pub milestone: Option<String>,
    /// Move under KEY-N (same project)
    #[arg(long)]
    pub parent: Option<String>,
    #[arg(long = "set", value_name = "NAME=VALUE", value_parser = parse_pair)]
    pub set: Vec<(String, String)>,
    /// Clear a field: assignee, due, milestone, parent, body, or a custom attribute name (repeatable)
    #[arg(long, value_name = "FIELD")]
    pub unset: Vec<String>,
}

pub fn add(_ctx: &Ctx, _args: AddArgs) -> Result<()> {
    Err(not_implemented("add"))
}

pub fn list(_ctx: &Ctx, _args: ListArgs) -> Result<()> {
    Err(not_implemented("list"))
}

pub fn show(_ctx: &Ctx, _args: KeyArg) -> Result<()> {
    Err(not_implemented("show"))
}

pub fn edit(_ctx: &Ctx, _args: EditArgs) -> Result<()> {
    Err(not_implemented("edit"))
}

pub fn set_status(_ctx: &Ctx, _args: KeyArg, _status: Status) -> Result<()> {
    Err(not_implemented("status change"))
}

pub fn rm(_ctx: &Ctx, _args: KeyArg) -> Result<()> {
    Err(not_implemented("rm"))
}
