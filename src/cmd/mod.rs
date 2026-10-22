//! One module per command group. Each owns its clap args and its `run`.

pub mod attr;
pub mod claim;
pub mod comment;
pub mod db;
pub mod guide;
pub mod history;
pub mod item;
pub mod link;
pub mod milestone;
pub mod mv;
pub mod port;
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

/// Run a command; returns the exit code for a success (0, or 3 from `next`).
pub fn run(cli: Cli) -> Result<i32> {
    if let Command::Guide = cli.command {
        return guide::run(cli.json).map(|()| 0);
    }
    let path = crate::db::path(cli.db.as_deref())?;
    let ctx = Ctx {
        conn: crate::db::open(&path)?,
        json: cli.json,
        actor: cli::actor(cli.actor.as_deref()),
    };
    let done = match cli.command {
        Command::Project(c) => project::run(&ctx, c),
        Command::Add(a) => item::add(&ctx, a),
        Command::List(a) => item::list(&ctx, a),
        Command::Show(a) => item::show(&ctx, a),
        Command::Edit(a) => item::edit(&ctx, a),
        Command::Start(a) => item::set_status(&ctx, a, Shortcut::Start),
        Command::Done(a) => item::set_status(&ctx, a, Shortcut::Done),
        Command::Block(a) => item::set_status(&ctx, a, Shortcut::Block),
        Command::Rm(a) => item::rm(&ctx, a),
        Command::Move(a) => mv::run(&ctx, a),
        Command::Claim(a) => claim::claim(&ctx, a),
        Command::Release(a) => claim::release(&ctx, a),
        Command::Next(a) => return claim::next(&ctx, a),
        Command::Claims(a) => claim::claims(&ctx, a),
        Command::Milestone(c) => milestone::run(&ctx, c),
        Command::Status(c) => status::run(&ctx, c),
        Command::Attr(c) => attr::run(&ctx, c),
        Command::Comment(a) => comment::run(&ctx, a),
        Command::Link(a) => link::link(&ctx, a),
        Command::Unlink(a) => link::unlink(&ctx, a),
        Command::Tags(a) => tag::run(&ctx, a),
        Command::History(a) => history::run(&ctx, a),
        Command::Export(a) => port::export(&ctx, a),
        Command::Import(a) => port::import(&ctx, a),
        Command::Backup(a) => port::backup(&ctx, &path, a),
        Command::Db => db::run(&ctx, &path),
        Command::Guide => unreachable!("handled before opening the database"),
    };
    done.map(|()| 0)
}
