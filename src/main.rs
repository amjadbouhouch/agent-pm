mod cli;
mod cmd;
mod db;
mod error;
mod input;
mod model;
mod output;

use clap::error::ErrorKind;

fn main() {
    let cli = match cli::Cli::parse_args() {
        Ok(cli) => cli,
        // --help / --version, and usage errors without --json: clap's own output and exit code.
        Err(e)
            if matches!(e.kind(), ErrorKind::DisplayHelp | ErrorKind::DisplayVersion)
                || !std::env::args().any(|a| a == "--json") =>
        {
            e.exit()
        }
        Err(e) => {
            // Usage error under --json: clap's first line as {"error": ...}, exit code 2.
            let text = e.to_string();
            let first = text.lines().next().unwrap_or("invalid arguments");
            output::print_error(&error::err(first.trim_start_matches("error: ")), true);
            std::process::exit(2);
        }
    };
    let json = cli.json;
    match cmd::run(cli) {
        Ok(0) => {}
        Ok(code) => std::process::exit(code),
        Err(e) => {
            output::print_error(&e, json);
            std::process::exit(1);
        }
    }
}
