//! One module per command group. Each owns its clap args and its `run`.

pub mod attr;
pub mod comment;
pub mod db;
pub mod guide;
pub mod history;
pub mod item;
pub mod link;
pub mod milestone;
pub mod project;
pub mod status;
pub mod tag;

use rusqlite::Connection;

use crate::cli::{self, Cli, Command};
use crate::error::Result;
use status::Shortcut;

/// What every command gets: the open database, the output mode and the actor.
pub struct Ctx {
    pub conn: Connection,
    pub json: bool,
    pub actor: String,
}

pub fn run(cli: Cli) -> Result<()> {
    if let Command::Guide = cli.command {
        return guide::run(cli.json);
    }
    let path = crate::db::path(cli.db.as_deref())?;
    let ctx = Ctx {
        conn: crate::db::open(&path)?,
        json: cli.json,
        actor: cli::actor(cli.actor.as_deref()),
    };
    match cli.command {
        Command::Project(c) => project::run(&ctx, c),
        Command::Add(a) => item::add(&ctx, a),
        Command::List(a) => item::list(&ctx, a),
        Command::Show(a) => item::show(&ctx, a),
        Command::Edit(a) => item::edit(&ctx, a),
        Command::Start(a) => item::set_status(&ctx, a, Shortcut::Start),
        Command::Done(a) => item::set_status(&ctx, a, Shortcut::Done),
        Command::Block(a) => item::set_status(&ctx, a, Shortcut::Block),
        Command::Rm(a) => item::rm(&ctx, a),
        Command::Milestone(c) => milestone::run(&ctx, c),
        Command::Status(c) => status::run(&ctx, c),
        Command::Attr(c) => attr::run(&ctx, c),
        Command::Comment(a) => comment::run(&ctx, a),
        Command::Link(a) => link::link(&ctx, a),
        Command::Unlink(a) => link::unlink(&ctx, a),
        Command::Tags(a) => tag::run(&ctx, a),
        Command::History(a) => history::run(&ctx, a),
        Command::Db => db::run(&ctx, &path),
        Command::Guide => unreachable!("handled before opening the database"),
    }
}
