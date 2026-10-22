//! One module per command group. Each owns its clap args and its `run`.

pub mod attr;
pub mod comment;
pub mod guide;
pub mod item;
pub mod milestone;
pub mod project;

use rusqlite::Connection;

use crate::cli::{self, Cli, Command};
use crate::db;
use crate::error::{Result, err};
use crate::output;

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
    let path = db::path(cli.db.as_deref())?;
    let ctx = Ctx {
        conn: db::open(&path)?,
        json: cli.json,
        actor: cli::actor(cli.actor.as_deref()),
    };
    match cli.command {
        Command::Project(c) => project::run(&ctx, c),
        Command::Add(a) => item::add(&ctx, a),
        Command::List(a) => item::list(&ctx, a),
        Command::Show(a) => item::show(&ctx, a),
        Command::Edit(a) => item::edit(&ctx, a),
        Command::Start(a) => item::set_status(&ctx, a, crate::model::Status::Doing),
        Command::Done(a) => item::set_status(&ctx, a, crate::model::Status::Done),
        Command::Block(a) => item::set_status(&ctx, a, crate::model::Status::Blocked),
        Command::Rm(a) => item::rm(&ctx, a),
        Command::Milestone(c) => milestone::run(&ctx, c),
        Command::Attr(c) => attr::run(&ctx, c),
        Command::Comment(a) => comment::run(&ctx, a),
        Command::Db => {
            let version = db::schema_version(&ctx.conn)?;
            if ctx.json {
                output::json(&serde_json::json!({
                    "path": path.display().to_string(),
                    "schema_version": version,
                }))
            } else {
                println!("{}\nschema v{version}", path.display());
                Ok(())
            }
        }
        Command::Guide => unreachable!("handled before opening the database"),
    }
}

/// Placeholder for commands a slice has not built yet. Remove with the last caller.
pub fn not_implemented(what: &str) -> crate::error::AppError {
    err(format!("not implemented yet: {what}"))
}
