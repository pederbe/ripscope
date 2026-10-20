//! `ripscope --units-query [--corrections NAMES] -i LIST [-C] [--own-semantics]`: a hidden
//! developer command that answers stored queries with the query engine over Analysis units
//! built in memory. It answers with the sequential semantics, the oracle the compass
//! compares with cscope, unless `--own-semantics` asks for the product's.
//!
//! The listed files are read in the order listed, which the caller makes the database
//! order. Queries come from stdin, one per line as `<field><pattern>`. Each query's rows
//! are printed in the line protocol with raw bytes, then a line `##units-query## <rows>`.
//! A pattern the engine cannot use gives no rows, as in cscope.

use crate::analyzer::Analyzer;
use crate::engine::{Field, Semantics};
use crate::query::{IdentifierCase, StoredQuery};
use crate::refresh;
use crate::revision::Revision;
use crate::unit::AnalysisUnit;
use std::ffi::OsString;
use std::io::{BufRead, Write};
use std::process::ExitCode;

pub(super) fn run(
    args: &[OsString],
    stdin: &mut impl BufRead,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> ExitCode {
    let (analyzer, args) = match super::hidden::analyzer(args) {
        Ok(split) => split,
        Err(error) => {
            let _ = writeln!(stderr, "ripscope: --units-query: {error}");
            return ExitCode::FAILURE;
        }
    };
    let mut list = None;
    let mut case = IdentifierCase::Sensitive;
    let mut semantics = Semantics::Sequential;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        if arg == "-i" {
            list = args.next();
        } else if arg == "-C" {
            case = IdentifierCase::AsciiInsensitive;
        } else if arg == "--own-semantics" {
            semantics = Semantics::Own;
        } else {
            let _ = writeln!(
                stderr,
                "ripscope: --units-query: unexpected {}",
                arg.to_string_lossy()
            );
            return ExitCode::FAILURE;
        }
    }
    let Some(list) = list else {
        let _ = writeln!(stderr, "ripscope: --units-query needs -i LIST");
        return ExitCode::FAILURE;
    };
    let names: Vec<Vec<u8>> = match std::fs::read(list) {
        Ok(bytes) => bytes
            .split(|byte| *byte == b'\n')
            .map(|line| line.strip_suffix(b"\r").unwrap_or(line))
            .filter(|line| !line.is_empty())
            .map(<[u8]>::to_vec)
            .collect(),
        Err(error) => {
            let _ = writeln!(
                stderr,
                "ripscope: cannot read {}: {error}",
                list.to_string_lossy()
            );
            return ExitCode::FAILURE;
        }
    };
    let units = read_units(&names, analyzer, stderr);
    let revision = Revision::in_memory(names.into_iter().zip(units).collect());
    answer_queries(&revision, case, semantics, stdin, stdout, stderr)
}

/// Answers queries from `stdin` over `revision` with `semantics`, one per line as
/// `<field><pattern>`: each query's rows in the line protocol with raw bytes, then
/// `##units-query## <rows>`.
pub(super) fn answer_queries(
    revision: &Revision,
    case: IdentifierCase,
    semantics: Semantics,
    stdin: &mut impl BufRead,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> ExitCode {
    // Queries are bytes, as the product's are.
    let mut line = Vec::new();
    loop {
        line.clear();
        match stdin.read_until(b'\n', &mut line) {
            Ok(0) => return ExitCode::SUCCESS,
            Ok(_) => {}
            Err(error) => {
                let _ = writeln!(stderr, "ripscope: cannot read queries: {error}");
                return ExitCode::FAILURE;
            }
        }
        let end = line
            .iter()
            .rposition(|byte| !matches!(byte, b'\n' | b'\r'))
            .map_or(0, |last| last + 1);
        let query = &line[..end];
        let Some(field) = query.first().copied().and_then(Field::from_digit) else {
            let _ = writeln!(
                stderr,
                "ripscope: --units-query: not a stored query: {}",
                String::from_utf8_lossy(query)
            );
            return ExitCode::FAILURE;
        };
        // As the product answers it; a pattern that cannot be used answers nothing.
        let query = StoredQuery::compile(field, &query[1..], case);
        let mut out = Vec::new();
        let mut rows = 0;
        for row in query
            .iter()
            .flat_map(|query| query.answer(semantics, revision))
        {
            out.extend_from_slice(row.path);
            out.push(b' ');
            out.extend_from_slice(row.context);
            out.extend_from_slice(format!(" {} ", row.line).as_bytes());
            out.extend_from_slice(row.text);
            out.push(b'\n');
            rows += 1;
        }
        out.extend_from_slice(format!("##units-query## {rows}\n").as_bytes());
        if stdout
            .write_all(&out)
            .and_then(|()| stdout.flush())
            .is_err()
        {
            return ExitCode::FAILURE;
        }
    }
}

/// Analyzes every file on all cores; an unreadable file contributes an empty unit.
fn read_units(names: &[Vec<u8>], analyzer: Analyzer, stderr: &mut impl Write) -> Vec<AnalysisUnit> {
    let threads = std::thread::available_parallelism().map_or(1, usize::from);
    let chunk = names.len().div_ceil(threads).max(1);
    let results: Vec<Vec<Result<AnalysisUnit, String>>> = std::thread::scope(|scope| {
        let workers: Vec<_> = names
            .chunks(chunk)
            .map(|names| {
                scope.spawn(move || {
                    names
                        .iter()
                        .map(|name| {
                            let path = String::from_utf8_lossy(name);
                            let bytes = std::fs::read(refresh::path_of(name))
                                .map_err(|error| format!("cannot read {path}: {error}"))?;
                            analyzer
                                .analyse(name, &bytes)
                                .map_err(|error| format!("cannot store {path}: {error}"))
                        })
                        .collect()
                })
            })
            .collect();
        workers
            .into_iter()
            .map(|worker| worker.join().expect("analysis worker"))
            .collect()
    });
    let empty = AnalysisUnit::from_records(&[]).expect("empty unit");
    results
        .into_iter()
        .flatten()
        .map(|result| {
            result.unwrap_or_else(|error| {
                let _ = writeln!(stderr, "ripscope: {error}");
                empty.clone()
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scanner::{self, Corrections, Mode};

    /// A request that is not a stored query stops the run, and a pattern that cannot be
    /// used answers no rows, as cscope answers it; the compass relies on both.
    #[test]
    fn requests_that_are_not_stored_queries_stop_and_unusable_patterns_answer_nothing() {
        let records = scanner::scan_with(b"int Alpha;\n", Mode::C, Corrections::ENABLED);
        let unit = AnalysisUnit::from_records(&records).expect("unit");
        let revision = Revision::in_memory(vec![(b"main.c".to_vec(), unit)]);
        let run = |first: &[u8]| {
            let mut input = first.to_vec();
            input.extend_from_slice(b"\n1Alpha\n");
            let mut stdin = std::io::Cursor::new(input);
            let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
            let status = answer_queries(
                &revision,
                IdentifierCase::Sensitive,
                Semantics::Sequential,
                &mut stdin,
                &mut stdout,
                &mut stderr,
            );
            (status, stdout, stderr)
        };
        for request in [&b"4Alpha"[..], b"5x", b"6[", b"4\0", b"x", b"", b" 1"] {
            let (status, stdout, stderr) = run(request);
            assert_eq!(status, ExitCode::FAILURE, "{request:?}");
            assert!(stdout.is_empty(), "{request:?}");
            assert_eq!(
                stderr,
                format!(
                    "ripscope: --units-query: not a stored query: {}\n",
                    String::from_utf8_lossy(request)
                )
                .into_bytes(),
                "{request:?}"
            );
        }
        for request in [&b"1["[..], b"1A\0lpha", b"8\0", b"1\xff"] {
            let (status, stdout, stderr) = run(request);
            assert_eq!(status, ExitCode::SUCCESS, "{request:?}");
            assert_eq!(
                stdout, b"##units-query## 0\nmain.c Alpha 1 int Alpha;\n##units-query## 1\n",
                "{request:?}"
            );
            assert!(stderr.is_empty(), "{request:?}");
        }
    }

    /// Queries are bytes, as the product's are: one that is not UTF-8 is answered, and
    /// so is the next.
    #[test]
    fn queries_that_are_not_utf8_are_answered() {
        let records = scanner::scan_with(
            b"#include \"caf\xe9.h\"\nint Alpha;\n",
            Mode::C,
            Corrections::ENABLED,
        );
        let unit = AnalysisUnit::from_records(&records).expect("unit");
        let revision = Revision::in_memory(vec![(b"main.c".to_vec(), unit)]);
        let mut stdin = std::io::Cursor::new(&b"8caf\xe9.h\r\n1Alpha\n"[..]);
        let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
        let status = answer_queries(
            &revision,
            IdentifierCase::Sensitive,
            Semantics::Sequential,
            &mut stdin,
            &mut stdout,
            &mut stderr,
        );
        assert_eq!(
            status,
            ExitCode::SUCCESS,
            "{}",
            String::from_utf8_lossy(&stderr)
        );
        assert_eq!(
            stdout,
            b"main.c <global> 1 #include \"caf\xe9.h\"\n##units-query## 1\n\
              main.c Alpha 2 int Alpha;\n##units-query## 1\n"
        );
    }

    /// A listed file name is bytes, as the product reads it: on Unix one that is not UTF-8
    /// names its file.
    #[cfg(unix)]
    #[test]
    fn file_names_that_are_not_utf8_are_read() {
        use std::os::unix::ffi::OsStrExt;
        let directory = tempfile::tempdir().expect("directory");
        let mut name = directory.path().as_os_str().as_bytes().to_vec();
        name.extend_from_slice(b"/caf\xe9.c");
        std::fs::write(std::ffi::OsStr::from_bytes(&name), b"int Alpha;\n").expect("file");
        let mut stderr = Vec::new();
        let units = read_units(&[name], Analyzer::PRODUCT, &mut stderr);
        assert!(stderr.is_empty(), "{}", String::from_utf8_lossy(&stderr));
        let expected = scanner::scan_with(b"int Alpha;\n", Mode::C, Corrections::ENABLED);
        assert!(units == [AnalysisUnit::from_records(&expected).expect("unit")]);
    }

    /// The command answers with the oracle unless `--own-semantics` asks for the product's
    /// semantics: a head after a match on its line opens its function only under the latter.
    #[test]
    fn own_semantics_are_asked_for() {
        let directory = tempfile::tempdir().expect("directory");
        let source = directory.path().join("a.c");
        std::fs::write(&source, "int a; int g(void) {\n\treturn a;\n}\n").expect("source");
        let list = directory.path().join("list");
        std::fs::write(&list, format!("{}\n", source.display())).expect("list");
        let answer = |extra: &[&str]| {
            let mut args = vec![OsString::from("-i"), list.clone().into_os_string()];
            args.extend(extra.iter().map(OsString::from));
            let mut stdin = std::io::Cursor::new(&b"0a\n"[..]);
            let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
            assert_eq!(
                run(&args, &mut stdin, &mut stdout, &mut stderr),
                ExitCode::SUCCESS,
                "{}",
                String::from_utf8_lossy(&stderr)
            );
            let text = String::from_utf8(stdout).expect("text");
            text.lines()
                .filter_map(|row| row.split(' ').nth(1).map(str::to_owned))
                .collect::<Vec<_>>()
        };
        assert_eq!(answer(&[]), ["<global>", "<global>", "2"]);
        assert_eq!(answer(&["--own-semantics"]), ["<global>", "g", "2"]);
    }
}
