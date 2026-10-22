use clap::Subcommand;
use rusqlite::{Connection, OptionalExtension, params};

use super::{Ctx, not_implemented};
use crate::error::{Result, err};

#[derive(Subcommand, Debug)]
pub enum MilestoneCmd {
    /// Create a milestone in a project
    Add {
        name: String,
        #[arg(short, long)]
        project: String,
        #[arg(short, long)]
        description: Option<String>,
        /// Due date, YYYY-MM-DD
        #[arg(long)]
        due: Option<String>,
    },
    /// List a project's milestones with progress (open ones unless --all)
    List {
        #[arg(short, long)]
        project: String,
        #[arg(long)]
        all: bool,
    },
    /// Show a milestone and its tasks
    Show {
        name: String,
        #[arg(short, long)]
        project: String,
    },
    /// Rename, redescribe or re-date a milestone
    Edit {
        name: String,
        #[arg(short, long)]
        project: String,
        /// New name
        #[arg(long)]
        rename: Option<String>,
        #[arg(short, long)]
        description: Option<String>,
        #[arg(long)]
        due: Option<String>,
    },
    /// Mark a milestone closed
    Close {
        name: String,
        #[arg(short, long)]
        project: String,
    },
    /// Mark a closed milestone open again
    Reopen {
        name: String,
        #[arg(short, long)]
        project: String,
    },
    /// Delete a milestone (its tasks stay, without a milestone)
    Rm {
        name: String,
        #[arg(short, long)]
        project: String,
    },
}

pub fn run(_ctx: &Ctx, _cmd: MilestoneCmd) -> Result<()> {
    Err(not_implemented("milestone"))
}

/// Milestone id for a name within a project. Used by item add/edit/list.
pub fn resolve(conn: &Connection, project_id: i64, name: &str) -> Result<i64> {
    conn.query_row(
        "SELECT id FROM milestones WHERE project_id = ?1 AND name = ?2",
        params![project_id, name],
        |r| r.get(0),
    )
    .optional()?
    .ok_or_else(|| err(format!("no such milestone: {name}")))
}
