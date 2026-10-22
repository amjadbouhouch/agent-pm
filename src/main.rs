// Scaffold: shared helpers are not called until S1-S3 land. orch removes this after they merge.
#![allow(dead_code)]

mod cli;
mod cmd;
mod db;
mod error;
mod model;
mod output;

use clap::Parser;

fn main() {
    let cli = cli::Cli::parse();
    let json = cli.json;
    if let Err(e) = cmd::run(cli) {
        output::print_error(&e, json);
        std::process::exit(1);
    }
}
