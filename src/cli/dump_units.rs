//! `ripscope --dump-units [--corrections NAMES] (-i LIST | FILE...)`: a hidden developer
//! command that prints the scanner's records for each file, one JSON line per file, in the
//! per-file oracle's schema (`examples/support/cscope_db.rs`). Bytes map one-to-one to characters
//! U+0000-U+00FF. `--corrections` reads with the named Corrections (comma-separated, `all`
//! or `enabled`), for measuring them.

use crate::refresh;
use serde_json::json;
use std::ffi::OsString;
use std::io::Write;
use std::process::ExitCode;

pub(super) fn run(args: &[OsString], stdout: &mut impl Write, stderr: &mut impl Write) -> ExitCode {
    let (analyzer, args) = match super::hidden::analyzer(args) {
        Ok(split) => split,
        Err(error) => {
            let _ = writeln!(stderr, "ripscope: --dump-units: {error}");
            return ExitCode::FAILURE;
        }
    };
    let names: Vec<OsString> = match args.as_slice() {
        [flag, list] if flag == "-i" => match std::fs::read(list) {
            Ok(bytes) => bytes
                .split(|byte| *byte == b'\n')
                .map(|line| line.strip_suffix(b"\r").unwrap_or(line))
                .filter(|line| !line.is_empty())
                .map(|line| refresh::path_of(line).into_os_string())
                .collect(),
            Err(error) => {
                let _ = writeln!(
                    stderr,
                    "ripscope: cannot read {}: {error}",
                    list.to_string_lossy()
                );
                return ExitCode::FAILURE;
            }
        },
        files => files.to_vec(),
    };
    for name in names {
        let display = name.to_string_lossy().into_owned();
        let bytes = match std::fs::read(&name) {
            Ok(bytes) => bytes,
            Err(error) => {
                let _ = writeln!(stderr, "ripscope: cannot read {display}: {error}");
                continue;
            }
        };
        let records: Vec<_> = analyzer
            .records(name.as_encoded_bytes(), &bytes)
            .iter()
            .map(|record| {
                encode_line(
                    record.line,
                    &record.text,
                    record.symbols.iter().map(|symbol| {
                        (
                            symbol.at,
                            symbol.at + symbol.len,
                            symbol.mark.byte(),
                            symbol.assigned,
                        )
                    }),
                    record.define_end_after,
                )
            })
            .collect();
        let line = json!({ "file": display, "records": records });
        if writeln!(stdout, "{line}").is_err() {
            return ExitCode::FAILURE;
        }
    }
    ExitCode::SUCCESS
}

fn latin1(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| char::from(*byte)).collect()
}

fn encode_line(
    number: u32,
    text: &[u8],
    names: impl Iterator<Item = (usize, usize, u8, bool)>,
    define_end: bool,
) -> serde_json::Value {
    let mut fragments = Vec::new();
    let mut symbols = Vec::new();
    let mut from = 0;
    for (at, end, mark, assigned) in names {
        fragments.push(latin1(&text[from..at]));
        symbols.push(json!([
            (mark as char).to_string(),
            latin1(&text[at..end]),
            assigned
        ]));
        from = end;
    }
    fragments.push(latin1(&text[from..]));
    json!({"line": number, "fragments": fragments, "symbols": symbols, "define_end": define_end})
}

/// The units stored queries read, from an index. This measurement entry point never
/// opens a source file.
pub(super) fn stored(
    args: &[OsString],
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> ExitCode {
    let [flag, index] = args else {
        let _ = writeln!(stderr, "ripscope: --dump-store needs -f INDEX");
        return ExitCode::FAILURE;
    };
    if flag != "-f" {
        let _ = writeln!(stderr, "ripscope: --dump-store needs -f INDEX");
        return ExitCode::FAILURE;
    }
    let result = (|| -> Result<(), crate::store::StoreError> {
        let store =
            crate::store::Store::open(std::path::Path::new(index), crate::store::Access::Read)?;
        let snapshot = store.snapshot()?;
        for file in snapshot.files() {
            let unit = snapshot.unit(file.unit)?;
            let records: Vec<_> = unit
                .lines()
                .map(|line| {
                    encode_line(
                        line.number,
                        line.text,
                        line.names().map(|name| {
                            let at = name.at as usize;
                            (at, at + name.text.len(), name.mark.byte(), name.assigned)
                        }),
                        line.define_end_after,
                    )
                })
                .collect();
            // The dump schema has always mapped bytes one-to-one to Latin-1.
            let row = json!({"file":latin1(&file.path),"records":records});
            writeln!(stdout, "{row}").map_err(crate::store::StoreError::Io)?;
        }
        Ok(())
    })();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let _ = writeln!(stderr, "ripscope: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    /// A listed file name is bytes, as the product reads it: on Unix one that is not UTF-8
    /// names its file.
    #[test]
    fn listed_file_names_that_are_not_utf8_are_read() {
        use std::os::unix::ffi::OsStrExt;
        let directory = tempfile::tempdir().expect("directory");
        let mut name = directory.path().as_os_str().as_bytes().to_vec();
        name.extend_from_slice(b"/caf\xe9.c");
        std::fs::write(std::ffi::OsStr::from_bytes(&name), b"int Alpha;\n").expect("file");
        let list = directory.path().join("list");
        std::fs::write(&list, [&name[..], b"\n"].concat()).expect("list");
        let args = [OsString::from("-i"), list.into_os_string()];
        let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
        assert_eq!(run(&args, &mut stdout, &mut stderr), ExitCode::SUCCESS);
        assert!(stderr.is_empty(), "{}", String::from_utf8_lossy(&stderr));
        let line: serde_json::Value = serde_json::from_slice(&stdout).expect("one JSON line");
        assert_eq!(line["records"].as_array().map(Vec::len), Some(1), "{line}");
    }
}
