use std::path::PathBuf;

use clap::{Parser, Subcommand};

use crate::cmd::{attr, comment, item, milestone, project};

/// agent-pm: a small project tracker for agents and humans.
#[derive(Parser, Debug)]
#[command(name = "apm", version, about, propagate_version = true)]
pub struct Cli {
    /// SQLite file to use (default: $APM_DB, else ~/.agent-pm/apm.db)
    #[arg(long, global = true, value_name = "PATH")]
    pub db: Option<PathBuf>,

    /// Print machine-readable JSON instead of text
    #[arg(long, global = true)]
    pub json: bool,

    /// Who is acting (default: $APM_AGENT, else the OS user)
    #[arg(long = "as", global = true, value_name = "NAME")]
    pub actor: Option<String>,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Manage projects
    #[command(subcommand)]
    Project(project::ProjectCmd),
    /// Create a task (or a subtask with --parent)
    Add(item::AddArgs),
    /// List tasks (open ones unless --all)
    List(item::ListArgs),
    /// Show one task with its subtasks, attributes and comments
    Show(item::KeyArg),
    /// Change a task
    Edit(item::EditArgs),
    /// Set a task's status to doing
    Start(item::KeyArg),
    /// Set a task's status to done
    Done(item::KeyArg),
    /// Set a task's status to blocked
    Block(item::KeyArg),
    /// Delete a task and its subtasks
    Rm(item::KeyArg),
    /// Manage milestones
    #[command(subcommand)]
    Milestone(milestone::MilestoneCmd),
    /// Manage a project's custom attributes
    #[command(subcommand)]
    Attr(attr::AttrCmd),
    /// Add a comment to a task
    Comment(comment::CommentArgs),
    /// Print a compact usage guide for agents
    Guide,
    /// Show the database path and schema version
    Db,
}

/// `--as` > `$APM_AGENT` > `$USER` / `$USERNAME` > "unknown".
pub fn actor(flag: Option<&str>) -> String {
    flag.map(str::to_string)
        .or_else(|| std::env::var("APM_AGENT").ok())
        .or_else(|| std::env::var("USER").ok())
        .or_else(|| std::env::var("USERNAME").ok())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}
