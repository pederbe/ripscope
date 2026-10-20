//! The command line: the compass's hidden commands, then ripscope's own (`pipeline`).

mod dump_units;
mod hidden;
mod pipeline;
mod store_commands;
mod units_query;

use std::{ffi::OsString, io::Write, process::ExitCode};

pub(crate) fn run(
    args: impl IntoIterator<Item = OsString>,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> ExitCode {
    let args: Vec<OsString> = args.into_iter().collect();
    // The compass's commands (docs/architecture/quality.md).
    if args.first().is_some_and(|first| first == "--dump-units") {
        return dump_units::run(&args[1..], stdout, stderr);
    }
    if args.first().is_some_and(|first| first == "--dump-store") {
        return dump_units::stored(&args[1..], stdout, stderr);
    }
    if args.first().is_some_and(|first| first == "--units-query") {
        return units_query::run(&args[1..], &mut std::io::stdin().lock(), stdout, stderr);
    }
    if args.first().is_some_and(|first| first == "--store-build") {
        return store_commands::build(&args[1..], stdout, stderr);
    }
    if args.first().is_some_and(|first| first == "--store-query") {
        return store_commands::query(&args[1..], &mut std::io::stdin().lock(), stdout, stderr);
    }
    pipeline::run(&args, &mut std::io::stdin().lock(), stdout, stderr)
}
