// Do not open a console window on Windows release builds.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::process::ExitCode;

use mizu::app::{App, Options, UserEvent};
use winit::event_loop::EventLoop;

const HELP: &str = "\
mizu — a minimal PDF viewer

USAGE:
    mizu [OPTIONS] [FILE]

OPTIONS:
    -p, --page <N>    Open at page N (overrides the remembered position)
        --stats       Show frame timings in the status line
        --diag        Log what is drawn (for bug reports; same as MIZU_DIAG=1)
    -h, --help        Show this help
    -V, --version     Show the version

Inside the viewer press `:` for commands and see the README for all keys.";

fn parse_args() -> Result<Option<Options>, lexopt::Error> {
    use lexopt::prelude::*;
    let mut opts = Options {
        file: None,
        page: None,
        stats: false,
        diag: false,
    };
    let mut parser = lexopt::Parser::from_env();
    while let Some(arg) = parser.next()? {
        match arg {
            Short('h') | Long("help") => {
                println!("{HELP}");
                return Ok(None);
            }
            Short('V') | Long("version") => {
                println!("mizu {}", env!("CARGO_PKG_VERSION"));
                return Ok(None);
            }
            Short('p') | Long("page") => {
                let n: usize = parser.value()?.parse()?;
                opts.page = Some(n.saturating_sub(1));
            }
            Long("stats") => opts.stats = true,
            Long("diag") => opts.diag = true,
            Value(v) if opts.file.is_none() => opts.file = Some(PathBuf::from(v)),
            _ => return Err(arg.unexpected()),
        }
    }
    Ok(Some(opts))
}

fn main() -> ExitCode {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();

    let opts = match parse_args() {
        Ok(Some(o)) => o,
        Ok(None) => return ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("mizu: {e}\n\n{HELP}");
            return ExitCode::from(2);
        }
    };
    if let Some(f) = &opts.file {
        if let Err(e) = std::fs::File::open(f) {
            eprintln!("mizu: cannot read {}: {e}", f.display());
            return ExitCode::from(1);
        }
    }

    let event_loop = match EventLoop::<UserEvent>::with_user_event().build() {
        Ok(l) => l,
        Err(e) => {
            eprintln!("mizu: cannot start the event loop: {e}");
            return ExitCode::from(1);
        }
    };
    let mut app = App::new(opts, event_loop.create_proxy());
    if let Err(e) = event_loop.run_app(&mut app) {
        eprintln!("mizu: {e}");
        return ExitCode::from(1);
    }
    if let Some(e) = app.exit_error.take() {
        eprintln!("mizu: {e}");
        return ExitCode::from(1);
    }
    ExitCode::SUCCESS
}
