use std::io::IsTerminal;
use std::process::ExitCode;

use clap::FromArgMatches;
use clap::error::ErrorKind;
use venv::cli::{self, Cli};
use venv::prompt::Aborted;
use venv::{Reported, ui};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    // From the shell function, stdout is evaluated: help, version and parse
    // errors must not land there
    let emit = args
        .iter()
        .any(|a| a == "--emit" || a.starts_with("--emit="));
    let matches = match cli::command().try_get_matches_from(&args) {
        Ok(m) => m,
        Err(e) => {
            // Help and version go to stdout as usual, except in --emit mode;
            // usage errors (and bare `venv`) go to stderr
            let info = matches!(e.kind(), ErrorKind::DisplayHelp | ErrorKind::DisplayVersion);
            if info && !emit {
                print!("{}", render(&e, std::io::stdout().is_terminal()));
                return ExitCode::SUCCESS;
            }
            eprint!("{}", render(&e, std::io::stderr().is_terminal()));
            return if info {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(2)
            };
        }
    };
    let cli = match Cli::from_arg_matches(&matches) {
        Ok(cli) => cli,
        Err(e) => {
            eprint!("{}", e.render().ansi());
            return ExitCode::FAILURE;
        }
    };
    match venv::run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) if e.is::<Reported>() => ExitCode::FAILURE,
        Err(e) if e.is::<Aborted>() => {
            ui::warn("已取消");
            ExitCode::from(130)
        }
        Err(e) => {
            ui::fail(format!("{e:#}"));
            ExitCode::FAILURE
        }
    }
}

fn render(e: &clap::Error, color: bool) -> String {
    let styled = e.render();
    let text = if color {
        styled.ansi().to_string()
    } else {
        styled.to_string()
    };
    cli::localize_help(&text)
}
