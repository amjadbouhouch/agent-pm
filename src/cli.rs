use std::path::PathBuf;

use clap::error::ErrorKind;
use clap::{CommandFactory, Parser, Subcommand};

use crate::cmd::{attr, comment, item, milestone, project, status};

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

impl Cli {
    /// Parse the process arguments, rejecting a flag right after `list --sort` (see
    /// `item::flag_after_sort`) as clap would reject a missing value.
    pub fn parse_args() -> Result<Cli, clap::Error> {
        let args: Vec<String> = std::env::args_os()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        if let Some(flag) = item::flag_after_sort(&args) {
            let msg = format!("--sort needs a value before {flag}");
            return Err(Cli::command().error(ErrorKind::InvalidValue, msg));
        }
        Cli::try_parse_from(std::env::args_os())
    }
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
    /// Move a task to its project's first active status (default: doing)
    Start(item::KeyArg),
    /// Move a task to its project's first done status (default: done)
    Done(item::KeyArg),
    /// Move a task to its project's status named blocked
    Block(item::KeyArg),
    /// Delete a task and its subtasks
    Rm(item::KeyArg),
    /// Manage milestones
    #[command(subcommand)]
    Milestone(milestone::MilestoneCmd),
    /// Manage a project's custom statuses
    #[command(subcommand)]
    Status(status::StatusCmd),
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
    let given = |v: Option<String>| v.filter(|s| !s.trim().is_empty());
    given(flag.map(str::to_string))
        .or_else(|| given(std::env::var("APM_AGENT").ok()))
        .or_else(|| given(std::env::var("USER").ok()))
        .or_else(|| given(std::env::var("USERNAME").ok()))
        .unwrap_or_else(|| "unknown".to_string())
}
