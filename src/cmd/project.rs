use clap::Subcommand;

use super::{Ctx, not_implemented};
use crate::error::Result;

#[derive(Subcommand, Debug)]
pub enum ProjectCmd {
    /// Create a project with a short KEY (letters/digits, e.g. APM) used in task ids
    Add {
        key: String,
        name: String,
        #[arg(short, long)]
        description: Option<String>,
    },
    /// List projects
    List,
    /// Show a project with its task counts
    Show { key: String },
    /// Rename or redescribe a project
    Edit {
        key: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(short, long)]
        description: Option<String>,
    },
    /// Delete a project and everything in it
    Rm { key: String },
}

pub fn run(_ctx: &Ctx, _cmd: ProjectCmd) -> Result<()> {
    Err(not_implemented("project"))
}
