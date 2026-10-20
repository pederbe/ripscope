#[macro_use]
mod profile;
mod analyzer;
mod cli;
mod console;
mod engine;
mod hashing;
mod invocation;
mod project;
mod query;
mod refresh;
mod revision;
mod scanner;
mod sink;
mod store;
mod unit;
mod watch;

use std::io::{IsTerminal, Write};

fn main() -> std::process::ExitCode {
    let stdout = std::io::stdout();
    let stderr = std::io::stderr();
    // A Windows console takes only UTF-8 (console.rs).
    let console = |terminal: bool| cfg!(windows) && terminal;
    let mut out: Box<dyn Write> = if console(stdout.is_terminal()) {
        Box::new(console::ConsoleSafe::new(stdout.lock()))
    } else {
        Box::new(stdout.lock())
    };
    let mut err: Box<dyn Write> = if console(stderr.is_terminal()) {
        Box::new(console::ConsoleSafe::new(stderr.lock()))
    } else {
        Box::new(stderr.lock())
    };
    cli::run(std::env::args_os().skip(1), &mut out, &mut err)
}
