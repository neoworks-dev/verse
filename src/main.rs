mod audio;
mod bubble;
mod canvas;
mod config;
mod menu;
mod models;
mod overlay;
mod presets;
mod session;
mod transcribe;
mod vocab;

use std::process::ExitCode;

const USAGE: &str = "\
usage: verse <command>

  start     show the overlay and record until `stop` (bind to key press)
  stop      transcribe what was recorded and type it (bind to key release)
  toggle    start, or stop if already recording
  cancel    discard the current recording
  settings  open the menu: model, language, vocabulary, appearance
            (optionally at a page: models, language, vocabulary, replacements)";

fn main() -> ExitCode {
    let command = std::env::args().nth(1).unwrap_or_else(|| "start".into());
    let result = match command.as_str() {
        "start" => session::start(),
        "stop" => session::signal(libc::SIGUSR1),
        "cancel" => session::signal(libc::SIGUSR2),
        "toggle" => match session::running_pid() {
            Some(_) => session::signal(libc::SIGUSR1),
            None => session::start(),
        },
        "settings" => session::settings(std::env::args().nth(2).as_deref()),
        "-h" | "--help" | "help" => {
            println!("{USAGE}");
            Ok(())
        }
        other => {
            eprintln!("unknown command {other:?}\n\n{USAGE}");
            return ExitCode::FAILURE;
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("verse: {err:#}");
            ExitCode::FAILURE
        }
    }
}
