//! Ripscope's command line: index builds, `--verify`, one-shot queries and sessions over
//! the Store, Refresh, the query engine and the result sink.
//!
//! Warnings go where each reader expects them (docs/spec.md, "Keep going instead of
//! failing"): in the line protocol before the answer, where cscope writes its own
//! messages and xcscope and Vim pass over them (under `--vim-errors` a short fixed notice
//! after the rows has Vim say that something went wrong), and as warning frames in a
//! JSON session.

mod answer;
mod session;

use crate::analyzer::Analyzer;
use crate::engine::Reads;
use crate::invocation::{self, Action, Invocation};
use crate::query::IdentifierCase;
use crate::refresh::{self, Selection, Sources, Trust, Unreadable};
use crate::revision::{self, Revision, Units};
use crate::sink::{WarningStyle, one_line};
use crate::store::{Access, REBUILD, Store, StoreError};
use std::ffi::OsString;
use std::io::{self, BufRead, Write};
use std::path::Path;
use std::process::ExitCode;
use std::time::Duration;

pub(super) fn run(
    args: &[OsString],
    stdin: &mut impl BufRead,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> ExitCode {
    let result = invocation::parse(args).and_then(|invocation| {
        let root =
            std::env::current_dir().map_err(|error| format!("working directory: {error}"))?;
        match &invocation.action {
            Action::Help => stdout
                .write_all(HELP.as_bytes())
                .map_err(|error| format!("output failed: {error}")),
            Action::Version => writeln!(stdout, "ripscope {}", env!("CARGO_PKG_VERSION"))
                .map_err(|error| format!("output failed: {error}")),
            Action::Build => build(&invocation, &root, stderr),
            Action::Query { field, pattern } => {
                one_shot(&invocation, &root, *field, pattern, stdout, stderr)
            }
            Action::Session => session::session(&invocation, &root, stdin, stdout, stderr),
            Action::Verify => verify(&invocation, &root, stdout, stderr),
            Action::Project(path) => project(path, &root, stdout),
        }
    });
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let _ = writeln!(stderr, "ripscope: {}", one_line(&error));
            ExitCode::FAILURE
        }
    }
}

const HELP: &str = "\
ripscope: cscope's searches for C, from an index kept up to date

Usage:
  ripscope -b [-R] [-k] [-u] [-I DIR]... [-f INDEX] [--] [SOURCE...]
  ripscope -b [-R] [-k] [-u] [-I DIR]... [-f INDEX] -i LIST
  ripscope -L [-d] [--auto-refresh | --no-auto-refresh] [-C] [-P PATH] [--json] [-f INDEX] -FIELD PATTERN
  ripscope -l [-d] [--auto-refresh | --no-auto-refresh] [-C] [-P PATH] [--json | --vim-errors] [-f INDEX]
  ripscope --verify [-d] [-f INDEX]
  ripscope --project PATH
  ripscope -h | --help
  ripscope -V | --version

Fields: 0 symbol, 1 definition, 2 functions called by, 3 functions calling, 4 text,
5 text (as cscope's line mode), 6 egrep pattern, 7 file, 8 files including,
9 assignments to.

The index is ripscope.db unless -f names another. -b builds it from the sources given,
or those a list names (-i), or else cscope.files if present, or else the C sources in
this directory (-R: and below); each update applies the same rule again. -L and -l
update the index first unless -d, and a session updates it again before the first query
after its sources change. --auto-refresh updates despite -d and --no-auto-refresh never
does, in either argument order. -u reads every file again at startup and on r. -bu
repairs a damaged index. In a session, send the field and the pattern on one line
(1main), r to update, q to quit. --project reports, as JSON for editor integrations,
the project of a file: its root, index in the cache directory, selection, and whether
that index is published with that selection.
";

/// `--project PATH`, for editor integrations: one JSON object naming the Project's
/// root, its index in the cache directory (the cache directory is made when missing,
/// the index is not), the selection arguments an index build or a session there takes,
/// and whether the index has a published revision built with that selection.
fn project(path: &Path, cwd: &Path, stdout: &mut impl Write) -> Result<(), String> {
    let places = crate::project::Places::from_environment(cwd)?;
    let found = crate::project::find(path, &places)?;
    std::fs::create_dir_all(&places.cache)
        .map_err(|error| format!("cannot make {}: {error}", places.cache.display()))?;
    // A session keeps the rule the index was built with, so an index built before a
    // `cscope.files` was added or removed counts as unpublished, and the editor builds
    // it again with the selection the project calls for now.
    let published = found.index.is_file()
        && Store::open(&found.index, Access::Read)
            .ok()
            .and_then(|store| {
                let snapshot = store.snapshot().ok()?;
                refresh::stored_rule(&snapshot).ok().flatten()
            })
            .is_some_and(|rule| rule.sources == found.sources());
    let report = serde_json::json!({
        "root": crate::project::text(&found.root)?,
        "index": crate::project::text(&found.index)?,
        "selection": found.selection,
        "published": published,
    });
    writeln!(stdout, "{report}").map_err(|error| format!("output failed: {error}"))
}

fn case(invocation: &Invocation) -> IdentifierCase {
    if invocation.caseless {
        IdentifierCase::AsciiInsensitive
    } else {
        IdentifierCase::Sensitive
    }
}

/// The selection rule an index build names (see `Sources::named`), which each refresh
/// applies again.
fn selection(invocation: &Invocation, root: &Path) -> Selection {
    named_selection(
        root,
        Sources::named(
            invocation.list.clone(),
            invocation.files.clone(),
            invocation.recursive,
        ),
        invocation.include_dirs.clone(),
        !invocation.kernel,
    )
}

/// The selection rule of files named under `root`, with `-I` directories and, unless `-k`,
/// the system directory: the directories of `INCLUDEDIRS` and `INCDIR` (else
/// `/usr/include`) are read from the environment here, as cscope reads them, and kept in
/// the rule.
pub(super) fn named_selection(
    root: &Path,
    sources: Sources,
    include_dirs: Vec<Vec<u8>>,
    system_includes: bool,
) -> Selection {
    let environment = |name: &str| std::env::var_os(name);
    Selection {
        root: root.to_owned(),
        sources,
        include_dirs,
        environment_dirs: environment("INCLUDEDIRS")
            .map(|dirs| {
                std::env::split_paths(&dirs)
                    .map(|dir| dir.as_os_str().as_encoded_bytes().to_vec())
                    .collect()
            })
            .unwrap_or_default(),
        system_includes,
        system_dir: environment("INCDIR").map_or_else(
            || b"/usr/include".to_vec(),
            |dir| dir.as_encoded_bytes().to_vec(),
        ),
    }
}

/// cscope's words for a named file it cannot use.
fn cannot_find(path: &[u8]) -> String {
    format!("cannot find file {}", String::from_utf8_lossy(path))
}

/// What a refresh found worth telling: named files it could not find, and files that
/// could not be read.
pub(super) fn warnings(report: &refresh::Report) -> Vec<String> {
    let missing = report.missing.iter().map(|path| cannot_find(path));
    let unreadable = report
        .unreadable
        .iter()
        .map(|(path, error)| format!("cannot read {}: {error}", String::from_utf8_lossy(path)));
    missing.chain(unreadable).collect()
}

fn tell(notes: &[String], stderr: &mut impl Write) {
    for note in notes {
        let _ = writeln!(stderr, "ripscope: {}", one_line(note));
    }
}

/// Why an update failed, after what it found on the way: the named files it could not
/// use, which cscope reports one by one before it gives up.
struct Failure {
    notes: Vec<String>,
    reason: String,
}

impl From<refresh::RefreshError> for Failure {
    fn from(error: refresh::RefreshError) -> Self {
        let notes = match &error {
            refresh::RefreshError::NoSources(report) => warnings(report),
            _ => Vec::new(),
        };
        Self {
            notes,
            reason: error.to_string(),
        }
    }
}

impl Failure {
    /// Tells the notes on standard error, and gives the reason to fail with.
    fn told(self, stderr: &mut impl Write) -> String {
        tell(&self.notes, stderr);
        self.reason
    }
}

/// `-u` trusts nothing the index holds; otherwise unchanged files keep their analysis.
fn trust(invocation: &Invocation) -> Trust {
    if invocation.unconditional {
        Trust::Nothing
    } else {
        Trust::Metadata
    }
}

/// `-b`. An index build that makes the index in a folder OneDrive synchronises says so
/// once (ADR 0005); its later updates and queries do not.
fn build(invocation: &Invocation, root: &Path, stderr: &mut impl Write) -> Result<(), String> {
    let selection = selection(invocation, root);
    let options = refresh::Options {
        unreadable: Unreadable::Drop,
        trust: trust(invocation),
        wait: refresh::WAIT,
        analyzer: Analyzer::PRODUCT,
    };
    let built = open_writable(&invocation.index, Access::Write, &selection)
        .and_then(|store| build_into(store, invocation, &selection, options, stderr));
    let done = match built {
        // An unconditional index build makes afresh an index SQLite can no longer read,
        // once the selection rule finds a file to index.
        Err(refresh::RefreshError::Store(StoreError::Damaged(_))) if invocation.unconditional => {
            refresh::check_sources(&selection)
                .and_then(|()| Ok(Store::replace(&invocation.index)?))
                .and_then(|store| build_into(store, invocation, &selection, options, stderr))
        }
        done => done,
    }
    .map_err(|error| Failure::from(error).told(stderr))?;
    tell(&warnings(&done), stderr);
    Ok(())
}

/// Builds the index `store` opened. When opening it made the index in a folder OneDrive
/// synchronises, that is told first: the index stays even if the build then fails or is
/// stopped, and the next index build opens it as it is.
fn build_into(
    mut store: Store,
    invocation: &Invocation,
    selection: &Selection,
    options: refresh::Options,
    stderr: &mut impl Write,
) -> Result<refresh::Report, refresh::RefreshError> {
    if store.made_in_onedrive() {
        tell(
            &[format!(
                "OneDrive may lock or copy the index {}, and -f can keep it elsewhere",
                invocation.index.display()
            )],
            stderr,
        );
    }
    refresh::build(&mut store, selection, options)
}

/// Opens the index for writing. A new one is made only once `selection` finds a file to
/// index, so an index build or an update that finds none leaves nothing behind, as in
/// cscope.
pub(super) fn open_writable(
    index: &Path,
    access: Access,
    selection: &Selection,
) -> Result<Store, refresh::RefreshError> {
    let inspected = Store::inspect(index)?;
    if inspected.makes_new(access) {
        refresh::check_sources(selection)?;
    }
    profile_span!("store_open");
    Ok(inspected.open(access)?)
}

/// Brings the index up to date unless `-d` was given (which never builds one, as in
/// cscope), telling what it found. A query never fails because another process is
/// updating: it answers from the last published revision, and fails only when there is
/// none yet.
///
/// The index the update opened, if it did, is returned read only, so that the caller
/// reads from it rather than opening the index again.
fn prepare(
    invocation: &Invocation,
    root: &Path,
    stderr: &mut impl Write,
    observer: &mut dyn refresh::Observer,
) -> Result<Option<Store>, String> {
    if !invocation.update {
        return Ok(None);
    }
    let Updated {
        skipped,
        warnings,
        store,
    } = update(invocation, root, trust(invocation), observer)
        .map_err(|failure| failure.told(stderr))?;
    tell(
        &skipped.into_iter().chain(warnings).collect::<Vec<_>>(),
        stderr,
    );
    // One that cannot be made a reader's is let go, and the index opened again.
    Ok(store.and_then(|store| store.into_reader().ok()))
}

/// How bringing the index up to date went.
struct Updated {
    /// Why the index was left as it is, if it was: another process was updating it, or
    /// it is kept where it cannot be updated safely.
    skipped: Option<String>,
    /// What the refresh found worth telling.
    warnings: Vec<String>,
    /// The index as the update left it, open for writing.
    store: Option<Store>,
}

/// Brings the index up to date, or says why it was left as it is. An index that cannot
/// be updated just now is answered from as it is, if it holds a published revision.
fn update(
    invocation: &Invocation,
    root: &Path,
    trust: Trust,
    observer: &mut dyn refresh::Observer,
) -> Result<Updated, Failure> {
    let skipped = |note: String| Updated {
        skipped: Some(note),
        warnings: Vec::new(),
        store: None,
    };
    let nothing_published = || revision::nothing_published(&invocation.index);
    match refresh_stored(invocation, root, trust, Duration::ZERO, observer) {
        Ok((report, store)) => Ok(Updated {
            skipped: None,
            warnings: warnings(&report),
            store: Some(store),
        }),
        // Another process is making the index afresh: nothing published to answer from.
        Err(refresh::RefreshError::Store(StoreError::Busy)) if nothing_published() => {
            Err(Failure {
                notes: Vec::new(),
                reason: format!(
                    "no index at {} yet: another process is building it",
                    invocation.index.display()
                ),
            })
        }
        Err(refresh::RefreshError::Store(StoreError::Busy)) => Ok(skipped(
            "another process is updating the index; answering from the last published revision"
                .into(),
        )),
        Err(refresh::RefreshError::Store(
            error @ (StoreError::Location(..)
            | StoreError::ReadOnly(..)
            | StoreError::Companion(..)),
        )) if invocation.index.exists() => {
            if nothing_published() {
                Err(Failure {
                    notes: vec![error.to_string()],
                    reason: revision::unpublished(&invocation.index),
                })
            } else {
                Ok(skipped(format!(
                    "{error}; answering from the index as it is (-d skips the update)"
                )))
            }
        }
        Err(error) => Err(Failure::from(error)),
    }
}

/// Re-applies the index's stored selection rule (the invocation's for a new index), and
/// gives back the index it opened.
fn refresh_stored(
    invocation: &Invocation,
    root: &Path,
    trust: Trust,
    wait: Duration,
    observer: &mut dyn refresh::Observer,
) -> Result<(refresh::Report, Store), refresh::RefreshError> {
    let invoked = selection(invocation, root);
    let mut store = open_writable(&invocation.index, Access::Update, &invoked)?;
    let options = refresh::Options {
        unreadable: Unreadable::Keep,
        trust,
        wait,
        analyzer: Analyzer::PRODUCT,
    };
    let report = refresh::refresh(&mut store, &invoked, options, observer)?;
    Ok((report, store))
}

/// `--verify`: SQLite's integrity checks; unless `-d`, the index
/// brought up to date with every file read and hashed; then every stored analysis checked
/// against its blocks and the name postings ([`crate::store::Snapshot::check_analyses`]).
fn verify(
    invocation: &Invocation,
    root: &Path,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> Result<(), String> {
    let damaged = |problems: &[String], stderr: &mut dyn Write| {
        for problem in problems {
            for line in problem.lines() {
                let _ = writeln!(stderr, "ripscope: {}", one_line(line));
            }
        }
        format!("the index is damaged; {REBUILD}")
    };
    let problems = Store::open(&invocation.index, Access::Read)
        .and_then(|store| store.verify())
        .map_err(|error| error.to_string())?;
    if !problems.is_empty() {
        return Err(damaged(&problems, stderr));
    }
    if invocation.update {
        let (report, _) = refresh_stored(invocation, root, Trust::Bytes, refresh::WAIT, &mut ())
            .map_err(|error| Failure::from(error).told(stderr))?;
        tell(&warnings(&report), stderr);
    }
    let store = Store::open(&invocation.index, Access::Read).map_err(|e| e.to_string())?;
    let snapshot = store.snapshot().map_err(|e| e.to_string())?;
    if snapshot.revision() == 0 {
        return Err(revision::unpublished(&invocation.index));
    }
    refresh::stored_rule(&snapshot).map_err(|e| e.to_string())?;
    if refresh::same_analyzer(&snapshot, Analyzer::PRODUCT) {
        let problems = snapshot
            .check_analyses()
            .map_err(|error| error.to_string())?;
        if !problems.is_empty() {
            return Err(damaged(&problems, stderr));
        }
    } else {
        // Another version's analyses are not this version's to decode; the next update
        // replaces them.
        let _ = writeln!(
            stderr,
            "ripscope: another version of ripscope built this index; its files are analysed again at its next update"
        );
    }
    let files = snapshot.files().len();
    writeln!(
        stdout,
        "index verified: revision {}, {files} {}",
        snapshot.revision(),
        if files == 1 { "file" } else { "files" }
    )
    .map_err(output_failed)
}

/// `-L`: one query, which reads from the index what it needs: field 7 and text searches
/// the paths alone, the fields that read units the files that may hold what they ask for.
/// Warnings come before the first row, where xcscope, which reads the merged output and
/// takes any line whose third word is a number for a row, skips them.
fn one_shot(
    invocation: &Invocation,
    root: &Path,
    field: u8,
    pattern: &[u8],
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> Result<(), String> {
    #[cfg(feature = "session-profile")]
    let _one_shot = crate::profile::Operation::start("one_shot", true);
    let refreshed = prepare(invocation, root, stderr, &mut ())?;
    let index = match &refreshed {
        Some(store) => revision::Index::Opened(&invocation.index, store),
        None => (&invocation.index).into(),
    };
    let mut units = Units::none();
    // cscope answers an unusable pattern with nothing, and succeeds; a problem with the
    // index is told first.
    let query = match answer::compile(field, pattern, case(invocation), invocation.transport) {
        Ok(query) => query,
        Err(unusable) => {
            Revision::read(index, &Reads::Paths, &mut units)?;
            tell(&[unusable.reason().into_owned()], stderr);
            return Ok(());
        }
    };
    let mut found =
        answer::find(&query, invocation, index, &mut units).map_err(|failed| match failed {
            answer::Failed::Index(reason) => reason,
            answer::Failed::Output(error) => output_failed(error),
        })?;
    found
        .deliver(stdout, stderr, WarningStyle::Stderr, |_, _| Ok(()))
        .and_then(|()| stdout.flush())
        .map_err(output_failed)
}

fn output_failed(error: io::Error) -> String {
    format!("output failed: {error}")
}

#[cfg(test)]
mod tests {
    use super::*;

    enum FailingOutput {
        Write,
        Flush,
    }

    impl Write for FailingOutput {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            match self {
                Self::Write => Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed output")),
                Self::Flush => Ok(bytes.len()),
            }
        }

        fn flush(&mut self) -> io::Result<()> {
            Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed output"))
        }
    }

    #[test]
    fn queries_report_stdout_write_and_flush_failures() {
        let work = tempfile::tempdir().expect("work");
        let source = work.path().join("a.c");
        let index = work.path().join("i.db");
        std::fs::write(&source, "int Alpha;\n").expect("source");
        let built = run(
            &[
                "-bk".into(),
                "-f".into(),
                index.clone().into(),
                source.into(),
            ],
            &mut io::empty(),
            &mut Vec::new(),
            &mut Vec::new(),
        );
        assert_eq!(built, ExitCode::SUCCESS);
        for arguments in [
            vec!["-dL", "-1", "Alpha"],
            vec!["--json", "-dL", "-1", "Alpha"],
            vec!["-dl"],
            vec!["--json", "-dl"],
        ] {
            let mut args: Vec<OsString> = arguments.iter().map(OsString::from).collect();
            args.extend(["-f".into(), index.clone().into()]);
            for mut output in [FailingOutput::Write, FailingOutput::Flush] {
                let mut errors = Vec::new();
                let status = run(&args, &mut &b"1Alpha\nq\n"[..], &mut output, &mut errors);
                assert_eq!(status, ExitCode::FAILURE, "{arguments:?}");
                assert_eq!(errors, b"ripscope: output failed: closed output\n");
            }
        }
    }
}
