//! Hidden developer commands over the Store:
//! - `ripscope --store-build [--corrections NAMES] -f INDEX [-R] [-k] [-I DIR]...
//!   (-i LIST | FILE...)` builds or refreshes the index and prints what it did, then each
//!   indexed path in database order. It reads as the reference does, which the compass
//!   compares with cscope, unless `--corrections` names Corrections (or `enabled`);
//! - `ripscope --store-query -f INDEX [-C] [--own-semantics]` answers queries from stdin
//!   as `--units-query` does, over the index's current revision.

use super::units_query::answer_queries;
use crate::engine::{Reads, Semantics};
use crate::query::IdentifierCase;
use crate::refresh::{self, Sources};
use crate::revision::{Revision, Units};
use crate::store::Access;
use std::ffi::OsString;
use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::process::ExitCode;

fn bytes(value: &OsString) -> Vec<u8> {
    value.as_encoded_bytes().to_vec()
}

pub(super) fn build(
    args: &[OsString],
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> ExitCode {
    let (analyzer, args) = match super::hidden::analyzer(args) {
        Ok(split) => split,
        Err(error) => {
            let _ = writeln!(stderr, "ripscope: --store-build: {error}");
            return ExitCode::FAILURE;
        }
    };
    let mut index = None;
    let mut list = None;
    let mut files = Vec::new();
    let mut include_dirs = Vec::new();
    let mut recursive = false;
    let mut system_includes = true;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("-f") => index = args.next().map(PathBuf::from),
            Some("-i") => list = args.next().map(bytes),
            Some("-I") => include_dirs.extend(args.next().map(bytes)),
            Some("-R") => recursive = true,
            Some("-k") => system_includes = false,
            _ => files.push(bytes(arg)),
        }
    }
    let Some(index) = index else {
        let _ = writeln!(stderr, "ripscope: --store-build needs -f INDEX");
        return ExitCode::FAILURE;
    };
    let root = match std::env::current_dir() {
        Ok(root) => root,
        Err(error) => {
            let _ = writeln!(stderr, "ripscope: {error}");
            return ExitCode::FAILURE;
        }
    };
    let selection = super::pipeline::named_selection(
        &root,
        Sources::named(list, files, recursive),
        include_dirs,
        system_includes,
    );
    let mut store = match super::pipeline::open_writable(&index, Access::Write, &selection) {
        Ok(store) => store,
        Err(error) => {
            if let refresh::RefreshError::NoSources(report) = &error {
                tell(report, stderr);
            }
            let _ = writeln!(stderr, "ripscope: {error}");
            return ExitCode::FAILURE;
        }
    };
    let options = refresh::Options {
        unreadable: refresh::Unreadable::Drop,
        trust: refresh::Trust::Metadata,
        wait: refresh::WAIT,
        analyzer,
    };
    let report = match refresh::build(&mut store, &selection, options) {
        Ok(report) => report,
        Err(error) => {
            if let refresh::RefreshError::NoSources(report) = &error {
                tell(report, stderr);
            }
            let _ = writeln!(stderr, "ripscope: {error}");
            return ExitCode::FAILURE;
        }
    };
    tell(&report, stderr);
    let _ = writeln!(
        stderr,
        "ripscope: revision {}{}: {} files, {} analysed",
        report.revision,
        if report.published { "" } else { " (unchanged)" },
        report.files,
        report.analysed
    );
    let snapshot = match store.snapshot() {
        Ok(snapshot) => snapshot,
        Err(error) => {
            let _ = writeln!(stderr, "ripscope: {error}");
            return ExitCode::FAILURE;
        }
    };
    for file in snapshot.files() {
        let mut line = file.path.clone();
        line.push(b'\n');
        if stdout.write_all(&line).is_err() {
            return ExitCode::FAILURE;
        }
    }
    ExitCode::SUCCESS
}

/// The named files a refresh could not find, and the files it could not read.
fn tell(report: &refresh::Report, stderr: &mut impl Write) {
    for warning in super::pipeline::warnings(report) {
        let _ = writeln!(stderr, "ripscope: {warning}");
    }
}

pub(super) fn query(
    args: &[OsString],
    stdin: &mut impl BufRead,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> ExitCode {
    let mut index = None;
    let mut case = IdentifierCase::Sensitive;
    let mut semantics = Semantics::Sequential;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("-f") => index = args.next().map(PathBuf::from),
            Some("-C") => case = IdentifierCase::AsciiInsensitive,
            Some("--own-semantics") => semantics = Semantics::Own,
            _ => {
                let _ = writeln!(
                    stderr,
                    "ripscope: --store-query: unexpected {}",
                    arg.to_string_lossy()
                );
                return ExitCode::FAILURE;
            }
        }
    }
    let Some(index) = index else {
        let _ = writeln!(stderr, "ripscope: --store-query needs -f INDEX");
        return ExitCode::FAILURE;
    };
    // The compass compares every file, so one whose unit cannot be read stops the run.
    let revision =
        Revision::read(&index, &Reads::Every, &mut Units::none()).and_then(
            |revision| match revision.failure() {
                Some(error) => Err(error),
                None => Ok(revision),
            },
        );
    let revision = match revision {
        Ok(revision) => revision,
        Err(error) => {
            let _ = writeln!(stderr, "ripscope: {error}");
            return ExitCode::FAILURE;
        }
    };
    answer_queries(&revision, case, semantics, stdin, stdout, stderr)
}
