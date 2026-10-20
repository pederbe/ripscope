//! Regenerate the query probe fixtures from the pinned reference cscope (ADR 0001, ADR 0002).
//!
//! Each directory in `tests/fixtures/queries/` holds a few synthetic C files and
//! `queries.txt`, one query per line: `<field><pattern>`, or `C <field><pattern>` to add
//! `-C`. The pinned cscope indexes the directory's files on their own (`cscope -bk
//! <files>`, on Linux with a clean environment and `LC_ALL=C`) and answers each query with
//! `cscope -dL`. The files in the database's order, and each query's exit status, rows
//! and stderr, are written to `expected.json`. The query engine's tests compare
//! ripscope's rows with them. The same queries against an inverted-index build (`-bqk`)
//! are recorded alongside, as evidence for the `-q` semantics question.
//!
//! Rows keep every byte: they are decoded as Latin-1, so each byte is one character.
//!
//! usage: cargo run --example query-probes -- [probe-directory ...]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[path = "support/captured_text.rs"]
mod captured_text;
#[path = "support/cscope_db.rs"]
#[allow(dead_code)]
mod cscope_db;
#[path = "support/json_text.rs"]
#[allow(dead_code)]
mod json_text;
#[path = "support/reference.rs"]
#[allow(dead_code)]
mod reference;

use json_text::{FIXTURE, Json};
use reference::{CSCOPE_SHA256, Reference, quote};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const MARK: &str = "##query-probe## ";

fn main() -> ExitCode {
    let names: BTreeSet<String> = std::env::args().skip(1).collect();
    let probes = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/queries");
    match capture(&probes, &names, &Reference::pinned()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("query probes: {error}");
            ExitCode::FAILURE
        }
    }
}

struct Query {
    field: String,
    pattern: String,
    caseless: bool,
    status: BTreeMap<&'static str, i64>,
}

fn capture(root: &Path, names: &BTreeSet<String>, reference: &Reference) -> Result<()> {
    let mut probes: Vec<PathBuf> = fs::read_dir(root)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<std::io::Result<_>>()?;
    probes.retain(|probe| {
        probe.is_dir()
            && (names.is_empty()
                || names.contains(&*probe.file_name().unwrap_or_default().to_string_lossy()))
    });
    probes.sort();
    for probe in probes {
        let mut sources: Vec<String> = Vec::new();
        for entry in fs::read_dir(&probe)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if entry.path().is_file() && name != "queries.txt" && name != "expected.json" {
                sources.push(name);
            }
        }
        sources.sort();
        let mut queries = Vec::new();
        for line in captured_text::lines(&fs::read_to_string(probe.join("queries.txt"))?) {
            let caseless = line.starts_with("C ");
            let text = if caseless { &line[2..] } else { line };
            let mut chars = text.chars();
            let field = chars.next().ok_or("empty query line")?.to_string();
            queries.push(Query {
                field,
                pattern: chars.as_str().to_owned(),
                caseless,
                status: BTreeMap::new(),
            });
        }
        let clean = "timeout --foreground 60 env -i LC_ALL=C PATH=/usr/bin:/bin HOME=\"$work\"";
        let cscope = reference.command();
        let mut script = vec![
            "set -eo pipefail".to_owned(),
            "work=$(mktemp -d)".to_owned(),
            "trap 'rm -rf \"$work\"' EXIT".to_owned(),
        ];
        for name in &sources {
            script.push(format!(
                "cp {} \"$work/\"{}",
                quote(&reference.path(&probe.join(name))),
                quote(name)
            ));
        }
        let quoted: Vec<String> = sources.iter().map(|name| quote(name)).collect();
        script.push("cd \"$work\"".to_owned());
        script.push(format!("{clean} {cscope} -bk {}", quoted.join(" ")));
        script.push(format!(
            "{clean} {cscope} -bqk -f qscope.out {}",
            quoted.join(" ")
        ));
        script.push(format!("echo '{MARK}database'"));
        script.push("cat cscope.out".to_owned());
        for (number, query) in queries.iter().enumerate() {
            for (suffix, database) in [("", "cscope.out"), ("q", "qscope.out")] {
                let options = format!(
                    "-dL -f {database}{}",
                    if query.caseless { " -C" } else { "" }
                );
                let command = format!(
                    "{clean} {cscope} {options} -{} {}",
                    query.field,
                    quote(&query.pattern)
                );
                script.extend([
                    format!("echo '{MARK}rows{suffix} {number}'"),
                    format!("set +e; {command} > rows 2> err; status=$?; set -e"),
                    "cat rows".to_owned(),
                    format!("echo '{MARK}stderr{suffix} {number}'"),
                    "cat err".to_owned(),
                    format!("echo '{MARK}status{suffix} {number}' $status"),
                ]);
            }
        }
        let output = reference.run(&["bash", "-c", &script.join("\n")], None, 1900.0)?;
        let mut sections: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut current: Option<String> = None;
        for line in cscope_db::latin1(&output.stdout).split('\n') {
            if let Some(marked) = line.strip_prefix(MARK) {
                if marked.starts_with("status") {
                    let parts: Vec<&str> = marked.split(' ').collect();
                    let [kind, number, status] = parts[..] else {
                        return Err(format!("malformed status line: {marked}").into());
                    };
                    let kind = if kind == "status" {
                        "status"
                    } else {
                        "statusq"
                    };
                    queries[number.parse::<usize>()?]
                        .status
                        .insert(kind, status.parse()?);
                    current = None;
                } else {
                    sections.insert(marked.to_owned(), Vec::new());
                    current = Some(marked.to_owned());
                }
            } else if let Some(section) = current.as_ref() {
                sections
                    .get_mut(section)
                    .expect("open section")
                    .push(line.to_owned());
            }
        }
        let section = |name: &str| -> Result<&Vec<String>> {
            sections
                .get(name)
                .ok_or_else(|| format!("missing section {name}").into())
        };
        let database: Vec<u8> = section("database")?
            .join("\n")
            .chars()
            .map(|ch| u8::try_from(u32::from(ch)).expect("Latin-1 text"))
            .collect();
        let files: Vec<String> = cscope_db::parse_cscope_database(&database)?
            .into_iter()
            .map(|(name, _)| name)
            .collect();
        let mut written = Vec::new();
        let mut total_rows = 0;
        for (number, query) in queries.iter().enumerate() {
            let mut entry = Json::object([
                ("field", Json::from(query.field.as_str())),
                ("pattern", Json::from(query.pattern.as_str())),
                ("caseless", Json::from(query.caseless)),
            ]);
            // Statuses are set as the script reports them, before the rows.
            for kind in ["status", "statusq"] {
                if let Some(status) = query.status.get(kind) {
                    entry.set(kind, *status);
                }
            }
            for suffix in ["", "q"] {
                let rows: Vec<String> = section(&format!("rows{suffix} {number}"))?
                    .iter()
                    .filter(|row| !row.is_empty())
                    .cloned()
                    .collect();
                if suffix.is_empty() {
                    total_rows += rows.len();
                }
                entry.set(&format!("rows{suffix}"), rows);
                let stderr = section(&format!("stderr{suffix} {number}"))?.join("\n");
                entry.set(&format!("stderr{suffix}"), captured_text::strip(&stderr));
            }
            written.push(entry);
        }
        let fixture = Json::object([
            (
                "reference",
                Json::object([
                    ("binary_sha256", Json::from(CSCOPE_SHA256)),
                    (
                        "commands",
                        Json::from(vec![
                            "cscope -bk <files>",
                            "cscope -dL [-C] -<field> <pattern>",
                        ]),
                    ),
                    (
                        "inverted_index",
                        Json::from(
                            "rowsq/statusq/stderrq: the same queries on \
                             `cscope -bqk -f qscope.out <files>`",
                        ),
                    ),
                    ("rows", Json::from("each byte one character (Latin-1)")),
                ]),
            ),
            ("files", Json::from(files.clone())),
            ("queries", Json::Array(written)),
        ]);
        fs::write(probe.join("expected.json"), fixture.text(FIXTURE) + "\n")?;
        println!(
            "{}: {} files, {} queries, {total_rows} rows",
            probe.file_name().unwrap_or_default().to_string_lossy(),
            files.len(),
            queries.len()
        );
    }
    Ok(())
}
