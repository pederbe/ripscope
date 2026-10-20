//! The query engine against the reference's query probes (tests/fixtures/queries/): each
//! probe is a few synthetic files indexed and queried by the pinned cscope
//! (examples/query-probes.rs).

use super::{Field, Indexed, Reads, Semantics, Source};
use crate::query::{IdentifierCase, ResultRow, StoredQuery};
use crate::revision::Revision;
use crate::scanner::{self, Corrections, Mode, reference_view};
use crate::unit::{AnalysisUnit, Mark, Split};
use serde_json::Value;
use std::cell::Cell;
use std::collections::BTreeSet;
use std::path::Path;

/// A fixture string: each character is one byte (Latin-1).
fn bytes_of(text: &str) -> Vec<u8> {
    text.chars()
        .map(|c| u8::try_from(u32::from(c)).expect("fixture byte"))
        .collect()
}

/// The query as the product compiles it; cscope answers a pattern it cannot use with no
/// rows.
fn compile(field: Field, pattern: &str, case: IdentifierCase) -> Option<StoredQuery<'_>> {
    StoredQuery::compile(field, pattern.as_bytes(), case).ok()
}

/// A row as cscope's line mode prints it, without the line end.
fn printed(row: &ResultRow<'_>) -> Vec<u8> {
    let mut line = row.path.to_vec();
    line.push(b' ');
    line.extend_from_slice(row.context);
    line.extend_from_slice(format!(" {} ", row.line).as_bytes());
    line.extend_from_slice(&reference_view(row.text));
    line
}

/// The query cscope's `-dL [-C] -N PATTERN` asks: its field, pattern and case.
fn query_of(args: &[Value]) -> (Field, &str, IdentifierCase) {
    let args: Vec<&str> = args
        .iter()
        .map(|arg| arg.as_str().expect("argument"))
        .skip_while(|arg| *arg == "cscope")
        .collect();
    let (options, pattern) = args.split_at(args.len() - 1);
    let case = if options.contains(&"-C") {
        IdentifierCase::AsciiInsensitive
    } else {
        IdentifierCase::Sensitive
    };
    let digit = options
        .iter()
        .find_map(|option| {
            option
                .strip_prefix('-')
                .filter(|rest| rest.len() == 1 && rest.as_bytes()[0].is_ascii_digit())
        })
        .expect("field option")
        .as_bytes()[0];
    (
        Field::from_digit(digit).expect("stored field"),
        pattern[0],
        case,
    )
}

/// cscope's `-dL [-C] -N PATTERN` over `revision`, as it prints it: rows, each with its
/// line end. A pattern cscope cannot use gives no rows.
fn one_shot(revision: &Revision, args: &[Value]) -> Vec<u8> {
    let (field, pattern, case) = query_of(args);
    let Some(query) = compile(field, pattern, case) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for row in query.answer(Semantics::Sequential, revision) {
        out.extend_from_slice(&printed(&row));
        out.push(b'\n');
    }
    out
}

/// A capture's files read with `corrections`, in the order given: the index build's seeds
/// first in database order, then the files they include.
fn analysed(sources: &[(String, Vec<u8>)], corrections: Corrections) -> Revision {
    Revision::in_memory(
        sources
            .iter()
            .map(|(name, bytes)| {
                let records =
                    scanner::scan_with(bytes, Mode::for_name(name.as_bytes()), corrections);
                let unit = AnalysisUnit::from_records(&records).expect("unit");
                (name.as_bytes().to_vec(), unit)
            })
            .collect(),
    )
}

/// A cscope capture: its files, read as the engine reads them, and the queries cscope
/// answered over them.
struct Capture {
    label: String,
    revision: Revision,
    /// Each query's command line and cscope's output.
    queries: Vec<Value>,
}

/// cscope's answers to stored-field queries over small files, each captured with the pinned
/// binary and read with the baseline reading, and the linkage-blocks rows, read with the
/// enabled Corrections.
fn captures() -> Vec<Capture> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut captures = Vec::new();
    for name in CAPTURES {
        let fixture: Value =
            serde_json::from_slice(&std::fs::read(root.join(name)).expect("fixture"))
                .expect("JSON");
        let cases: Vec<(String, &Value)> = match &fixture["cases"] {
            Value::Object(cases) => cases
                .iter()
                .map(|(name, case)| (name.clone(), case))
                .collect(),
            Value::Array(cases) => cases
                .iter()
                .enumerate()
                .map(|(n, case)| (n.to_string(), case))
                .collect(),
            _ => panic!("{name}: cases"),
        };
        for (case_name, case) in cases {
            if let Some(source) = case["source"].as_str() {
                // One file, `case.c`, and its queries.
                let sources = vec![("case.c".to_owned(), bytes_of(source))];
                captures.push(Capture {
                    label: format!("{name} {case_name}"),
                    revision: analysed(&sources, Corrections::default()),
                    queries: case["queries"].as_array().expect("queries").clone(),
                });
                continue;
            }
            // One capture per field: its files, and a profile per case mode.
            for (field, capture) in case.as_object().expect("fields") {
                let seeds: Vec<&str> = capture["seeds"]
                    .as_array()
                    .expect("seeds")
                    .iter()
                    .map(|seed| seed.as_str().expect("seed"))
                    .collect();
                let mut names: Vec<&String> = capture["files"]
                    .as_object()
                    .expect("files")
                    .keys()
                    .collect();
                names.sort_by_key(|name| (!seeds.contains(&name.as_str()), (*name).clone()));
                let sources: Vec<(String, Vec<u8>)> = names
                    .iter()
                    .map(|name| {
                        (
                            (*name).clone(),
                            bytes_of(capture["files"][name.as_str()].as_str().expect("source")),
                        )
                    })
                    .collect();
                captures.push(Capture {
                    label: format!("{name} {case_name} field {field}"),
                    revision: analysed(&sources, Corrections::default()),
                    queries: capture["profiles"]
                        .as_array()
                        .expect("profiles")
                        .iter()
                        .flat_map(|profile| profile["queries"].as_array().expect("queries"))
                        .cloned()
                        .collect(),
                });
            }
        }
    }
    // The linkage-blocks rows, which the Correction gives.
    let linkage: Value = serde_json::from_slice(
        &std::fs::read(root.join("written-linkage-cscope.json")).expect("linkage"),
    )
    .expect("JSON");
    let approved: Value = serde_json::from_slice(
        &std::fs::read(root.join("written-linkage-corrections.json")).expect("approved"),
    )
    .expect("JSON");
    for row in approved.as_array().expect("rows") {
        let case = row["case"].as_str().expect("case");
        let field = row["field"].as_str().expect("field");
        let capture = &linkage["cases"][case][field];
        let sources: Vec<(String, Vec<u8>)> = capture["files"]
            .as_object()
            .expect("files")
            .iter()
            .map(|(name, source)| (name.clone(), bytes_of(source.as_str().expect("source"))))
            .collect();
        let mut args = vec![Value::from("-dL")];
        if row["caseless"].as_bool().expect("caseless") {
            args.push(Value::from("-C"));
        }
        args.push(Value::from(format!("-{field}")));
        args.push(row["pattern"].clone());
        captures.push(Capture {
            label: format!("written-linkage-corrections {case} field {field}"),
            revision: analysed(&sources, Corrections::ENABLED),
            queries: vec![
                serde_json::json!({"args": args, "exit_code": 0, "stdout": row["expected"]}),
            ],
        });
    }
    captures
}

/// The engine over the baseline reading gives every capture, and with the enabled
/// Corrections it gives the linkage-blocks rows.
#[test]
fn engine_matches_the_cscope_captures() {
    let mut failures = Vec::new();
    let mut checked = 0;
    for capture in captures() {
        for query in &capture.queries {
            assert_eq!(query["exit_code"], 0, "{}", capture.label);
            let want = bytes_of(query["stdout"].as_str().expect("stdout"));
            let got = one_shot(&capture.revision, query["args"].as_array().expect("args"));
            if got != want {
                failures.push(format!(
                    "{} {}\n  want {:?}\n  got  {:?}",
                    capture.label,
                    query["args"],
                    String::from_utf8_lossy(&want),
                    String::from_utf8_lossy(&got)
                ));
            }
            checked += 1;
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {checked} differ:\n{}",
        failures.len(),
        failures.join("\n")
    );
    assert!(checked > 18_000, "{checked}");
}

/// A reference query probe (tests/fixtures/queries/): a few synthetic files, in the order
/// the reference read them, and the queries it answered over them.
pub(super) struct Probe {
    pub(super) name: String,
    pub(super) sources: Vec<(String, Vec<u8>)>,
    expected: Value,
}

impl Probe {
    pub(super) fn all() -> Vec<Self> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/queries");
        let mut probes: Vec<_> = std::fs::read_dir(&root)
            .expect("query probes")
            .map(|entry| entry.expect("probe").path())
            .filter(|path| path.is_dir())
            .collect();
        probes.sort();
        assert!(!probes.is_empty());
        probes
            .iter()
            .map(|probe| {
                let expected: Value = serde_json::from_slice(
                    &std::fs::read(probe.join("expected.json")).expect("expected.json"),
                )
                .expect("fixture JSON");
                let sources = expected["files"]
                    .as_array()
                    .expect("files")
                    .iter()
                    .map(|name| {
                        let name = name.as_str().expect("file name");
                        let bytes = std::fs::read(probe.join(name)).expect("probe source");
                        (name.to_owned(), bytes)
                    })
                    .collect();
                Self {
                    name: probe.file_name().unwrap().to_string_lossy().into_owned(),
                    sources,
                    expected,
                }
            })
            .collect()
    }

    /// Its queries: field, pattern and case, and what the reference answered.
    pub(super) fn queries(&self) -> impl Iterator<Item = (Field, &str, IdentifierCase, &Value)> {
        self.expected["queries"]
            .as_array()
            .expect("queries")
            .iter()
            .map(|query| {
                let digit = query["field"].as_str().expect("field").as_bytes()[0];
                let case = if query["caseless"].as_bool().expect("caseless") {
                    IdentifierCase::AsciiInsensitive
                } else {
                    IdentifierCase::Sensitive
                };
                (
                    Field::from_digit(digit).expect("stored field"),
                    query["pattern"].as_str().expect("pattern"),
                    case,
                    query,
                )
            })
    }
}

#[test]
fn engine_matches_every_reference_query_probe() {
    let mut failures = Vec::new();
    for probe in Probe::all() {
        let revision = analysed(&probe.sources, Corrections::default());
        for (field, pattern, case, query) in probe.queries() {
            let label = format!(
                "{}: {}{}{pattern}",
                probe.name,
                if case == IdentifierCase::Sensitive {
                    ""
                } else {
                    "-C "
                },
                query["field"].as_str().expect("field")
            );
            assert_eq!(
                query["status"].as_i64(),
                Some(0),
                "{label}: reference failed"
            );
            let want: Vec<Vec<u8>> = query["rows"]
                .as_array()
                .expect("rows")
                .iter()
                .map(|row| bytes_of(row.as_str().expect("row")))
                .collect();
            let got: Vec<Vec<u8>> = match compile(field, pattern, case) {
                None => Vec::new(),
                Some(query) => query
                    .answer(Semantics::Sequential, &revision)
                    .map(|row| printed(&row))
                    .collect(),
            };
            if got != want {
                let show = |rows: &[Vec<u8>]| {
                    rows.iter()
                        .map(|row| format!("    {}\n", String::from_utf8_lossy(row)))
                        .collect::<String>()
                };
                failures.push(format!(
                    "{label}\n  want:\n{}  got:\n{}",
                    show(&want),
                    show(&got)
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// A source that gives a query only the files whose unit holds what it reads, as name
/// postings do.
struct Exact<'r>(&'r Revision);

impl Source for Exact<'_> {
    fn paths(&self) -> impl Iterator<Item = &[u8]> {
        self.0.paths()
    }

    fn read(&self, reads: &Reads<'_>) -> Vec<Indexed<'_>> {
        let mut files = self.0.files();
        files.retain(|file| reads.holds(file.unit));
        files
    }
}

/// Some of a revision's files: a part of a revision read in parts.
struct Part<'r>(Vec<Indexed<'r>>);

impl Source for Part<'_> {
    fn paths(&self) -> impl Iterator<Item = &[u8]> {
        self.0.iter().map(|file| file.path)
    }

    fn read(&self, _reads: &Reads<'_>) -> Vec<Indexed<'_>> {
        self.0.clone()
    }
}

/// A revision's units in blocks of `size` bytes, as an index keeps them (ADR 0011).
struct Blocks<'r> {
    size: usize,
    files: Vec<(&'r [u8], &'r AnalysisUnit, Split<'r>)>,
    /// Blocks an exact name passed over in a file that holds it.
    passed: Cell<usize>,
    /// Blocks an exact name read that start inside its macro's or function's body.
    inside: Cell<usize>,
}

impl<'r> Blocks<'r> {
    fn new(revision: &'r Revision, size: usize) -> Self {
        let files = revision
            .files()
            .into_iter()
            .map(|file| (file.path, file.unit, file.unit.split(size)))
            .collect();
        Self {
            size,
            files,
            passed: Cell::new(0),
            inside: Cell::new(0),
        }
    }
}

/// The blocks a query reads, as the index gives them: for an exact name, those listed for
/// it with a role its field reads; for anything else, every block of each file it needs.
impl Source for Blocks<'_> {
    fn paths(&self) -> impl Iterator<Item = &[u8]> {
        self.files.iter().map(|(path, _, _)| *path)
    }

    fn read(&self, reads: &Reads<'_>) -> Vec<Indexed<'_>> {
        let exact = match reads {
            Reads::Names { roles, pattern } => {
                pattern.exact_name().map(|name| (name, roles.mask()))
            }
            _ => None,
        };
        let mut read = Vec::new();
        for (path, unit, split) in &self.files {
            if !matches!(reads, Reads::Every) && !reads.holds(unit) {
                continue;
            }
            let listed = exact.map(|(name, mask)| {
                split
                    .postings
                    .iter()
                    .find(|(held, roles, _, _)| *held == name && roles & mask != 0)
                    .map_or(&[][..], |(_, _, blocks, _)| blocks)
            });
            for (number, (block, start)) in (0u32..).zip(&split.blocks) {
                if let (Some(listed), Some((name, _))) = (listed, exact) {
                    if !listed.contains(&number) {
                        self.passed.set(self.passed.get() + 1);
                        continue;
                    }
                    if [start.macro_name, start.function].contains(&Some(name)) {
                        self.inside.set(self.inside.get() + 1);
                    }
                }
                read.push(Indexed {
                    path,
                    unit: block,
                    start: *start,
                    first: number == 0,
                });
            }
        }
        read
    }
}

/// Files and the queries asked of them.
struct Asked {
    label: String,
    revision: Revision,
    queries: Vec<(Field, Vec<u8>, IdentifierCase)>,
}

/// What each query reads, which the index looks up before the engine reads a unit: over
/// only the files that hold it, every query answers as over every file, under both
/// semantics; in fields 0, 1, 8 and 9, where any matching name gives a row whatever its
/// context, each of those files gives a row, so they read no file in vain; and a unit's
/// name postings (for an exact name, or the names any other pattern matches) and include
/// operands (for field 8) find exactly those files. Only the oracle reads every file.
/// Checked on the reference query probes (with the baseline and the product reading) and
/// the captures, each with its own queries, a few regular expressions, and each name its
/// files hold.
#[test]
fn a_query_needs_only_the_files_that_hold_what_it_reads() {
    let mut inputs = Vec::new();
    for probe in Probe::all() {
        for corrections in [Corrections::default(), Corrections::ENABLED] {
            inputs.push(Asked {
                label: probe.name.clone(),
                revision: analysed(&probe.sources, corrections),
                queries: probe
                    .queries()
                    .map(|(field, pattern, case, _)| (field, pattern.as_bytes().to_vec(), case))
                    .collect(),
            });
        }
    }
    for capture in captures() {
        let queries = capture
            .queries
            .iter()
            .map(|query| {
                let (field, pattern, case) = query_of(query["args"].as_array().expect("args"));
                (field, pattern.as_bytes().to_vec(), case)
            })
            .collect();
        inputs.push(Asked {
            label: capture.label,
            revision: capture.revision,
            queries,
        });
    }
    // A name that starts with a dollar sign only where it is plain, where a regular
    // expression does not try it.
    let plain = [(
        "plain.c".to_owned(),
        b"void f(void) { x = $dollar; $set = 1; }\n".to_vec(),
    )];
    inputs.push(Asked {
        label: "plain".into(),
        revision: analysed(&plain, Corrections::ENABLED),
        queries: Vec::new(),
    });
    // Scopes that blocks start inside: a macro over several lines with calls on each, one
    // defined inside a function's body, the body going on after it, and a file that ends
    // inside a function, before another file.
    let scopes = [
        (
            "macros.c".to_owned(),
            b"#define WRAP(x) \\
    first(x); \\
    second(x)
int f(void)
{
    one();
#define INNER(y) \\
    third(y)
    two();
    WRAP(1);
}
"
            .to_vec(),
        ),
        (
            "open.c".to_owned(),
            b"int g(void)
{
    three();
"
            .to_vec(),
        ),
        (
            "after.c".to_owned(),
            b"int h = four();
"
            .to_vec(),
        ),
    ];
    inputs.push(Asked {
        label: "scopes".into(),
        revision: analysed(&scopes, Corrections::ENABLED),
        queries: Vec::new(),
    });
    // A macro defined inside a function's body, whose calls are its own; and an empty file
    // between one that ends inside a function and one that defines it again, which the
    // oracle's field 2 sees as a new file.
    let bodies = [
        (
            "a.c".to_owned(),
            b"void outer(void) {\n#define INNER() target()\n\thidden();\n}\n".to_vec(),
        ),
        ("f1.c".to_owned(), b"void f(void) {\n\tg();\n".to_vec()),
        ("f15.h".to_owned(), b"/* nothing here */\n".to_vec()),
        ("f2.c".to_owned(), b"void f(void) {\n\th();\n}\n".to_vec()),
    ];
    inputs.push(Asked {
        label: "bodies".into(),
        revision: analysed(&bodies, Corrections::ENABLED),
        queries: Vec::new(),
    });
    // Alternative heads, where preprocessor branches open a body twice or open two, and an
    // old-style head, its parameters declared before its body.
    let alternatives = [(
        "alt.c".to_owned(),
        b"#ifdef A\nint f(int a)\n#else\nint f(long a)\n#endif\n{\n\tone(a);\n}\n\
          #if B\nvoid g(void) {\n#else\nvoid h(void) {\n#endif\n\ttwo();\n}\n\
          int k(a, b)\nint a;\nchar *b;\n{\n\treturn three(a, b);\n}\n"
            .to_vec(),
    )];
    for corrections in [Corrections::default(), Corrections::ENABLED] {
        inputs.push(Asked {
            label: "alternatives".into(),
            revision: analysed(&alternatives, corrections),
            queries: Vec::new(),
        });
    }
    // In every field, as they are and with `-C`, regular expressions that also try plain
    // names that do not start with a letter.
    let mut everywhere = Vec::new();
    for case in [IdentifierCase::Sensitive, IdentifierCase::AsciiInsensitive] {
        for digit in *b"0123789" {
            let field = Field::from_digit(digit).expect("stored field");
            for pattern in [&b".*"[..], b"[^a-z].*", b"[$].*", b"."] {
                let query = StoredQuery::compile(field, pattern, case).expect("pattern");
                everywhere.push((pattern, case, query));
            }
        }
    }
    let (mut checked, mut passed, mut inside) = (0, 0, 0);
    let mut check = |label: &str,
                     revision: &Revision,
                     blocked: &[Blocks<'_>],
                     query: &StoredQuery<'_>,
                     asked: &dyn Fn() -> String| {
        // Over blocks, each walked from the scope it starts in, read as the index gives
        // them, a query answers as over whole units; the oracle reads every block, and
        // walks on across a file's blocks.
        let over_blocks = |semantics: Semantics, all: &[ResultRow<'_>]| {
            for blocks in blocked {
                let over_blocks: Vec<ResultRow<'_>> = query.answer(semantics, blocks).collect();
                assert_eq!(
                    over_blocks,
                    all,
                    "{label}: {semantics:?} blocks of {} {}",
                    blocks.size,
                    asked()
                );
            }
        };
        for semantics in [Semantics::Own, Semantics::Sequential] {
            let reads = query.reads(semantics);
            if matches!(reads, Reads::Every) {
                assert_eq!(semantics, Semantics::Sequential, "{label}: {}", asked());
                let all: Vec<ResultRow<'_>> = query.answer(semantics, revision).collect();
                over_blocks(semantics, &all);
                continue;
            }
            // What the index looks up: a unit's name postings, for an exact name its
            // list and for any other pattern every name it matches, and its include
            // operands for field 8.
            let mut folded = Vec::new();
            for file in revision.files() {
                let postings = file.unit.postings();
                let looked_up = match reads {
                    Reads::Names { roles, pattern } => {
                        let mut wanted = roles.wanted(pattern);
                        let scanned = postings
                            .iter()
                            .any(|(held, bits)| bits & wanted.roles(held) != 0);
                        if let Some(name) = pattern.exact_name() {
                            let listed = postings
                                .iter()
                                .any(|(held, bits)| *held == name && bits & roles.mask() != 0);
                            assert_eq!(listed, scanned, "{label}: {}", asked());
                        }
                        Some(scanned)
                    }
                    Reads::Includes { pattern } => Some(
                        file.unit
                            .includes()
                            .iter()
                            .any(|(operand, _)| pattern.matches_name(operand, &mut folded)),
                    ),
                    Reads::Paths | Reads::Every => None,
                };
                if let Some(looked_up) = looked_up {
                    assert_eq!(looked_up, reads.holds(file.unit), "{label}: {}", asked());
                }
            }
            let exact = Exact(revision);
            let all: Vec<ResultRow<'_>> = query.answer(semantics, revision).collect();
            let over_held: Vec<ResultRow<'_>> = query.answer(semantics, &exact).collect();
            assert_eq!(over_held, all, "{label}: {semantics:?} {}", asked());
            if !matches!(reads, Reads::Paths) {
                over_blocks(semantics, &all);
            }
            // Read in parts, a file each, the query answers the same once each part's
            // rows at file scope come before every part's other rows.
            if !matches!(reads, Reads::Paths) {
                let parts: Vec<Part<'_>> = exact
                    .read(&reads)
                    .into_iter()
                    .map(|file| Part(vec![file]))
                    .collect();
                let (mut in_parts, mut rest) = (Vec::new(), Vec::new());
                for part in &parts {
                    for row in query.answer(semantics, part) {
                        if row.at_file_scope {
                            in_parts.push(row);
                        } else {
                            rest.push(row);
                        }
                    }
                }
                in_parts.append(&mut rest);
                assert_eq!(in_parts, all, "{label}: {semantics:?} in parts {}", asked());
            }
            if matches!(
                query.field(),
                Field::Symbols | Field::Definitions | Field::Includers | Field::Assignments
            ) {
                let read: BTreeSet<&[u8]> =
                    exact.read(&reads).iter().map(|file| file.path).collect();
                let answered: BTreeSet<&[u8]> = all.iter().map(|row| row.path).collect();
                assert_eq!(read, answered, "{label}: {semantics:?} {}", asked());
            }
            checked += 1;
        }
    };
    let name_fields = [
        Field::Symbols,
        Field::Definitions,
        Field::Callees,
        Field::Callers,
        Field::Assignments,
    ];
    for Asked {
        label,
        revision,
        mut queries,
    } in inputs
    {
        // Each name its files hold, and each include operand, as the exact names postings
        // look up.
        let (mut names, mut operands) = (BTreeSet::new(), BTreeSet::new());
        for file in revision.files() {
            for line in file.unit.lines() {
                for name in line.names() {
                    match name.mark {
                        Mark::Include => operands.insert(name.text.get(1..).unwrap_or_default()),
                        _ => names.insert(name.text),
                    };
                }
            }
        }
        for name in names {
            for field in name_fields {
                queries.push((field, name.to_vec(), IdentifierCase::Sensitive));
            }
        }
        for operand in operands {
            queries.push((
                Field::Includers,
                operand.to_vec(),
                IdentifierCase::Sensitive,
            ));
        }
        // Blocks of a line each, of a few lines and of many.
        let blocked = [1, 40, 400].map(|size| Blocks::new(&revision, size));
        for (field, pattern, case) in &queries {
            if let Ok(query) = StoredQuery::compile(*field, pattern, *case) {
                let asked = || format!("{field:?} {:?} {case:?}", String::from_utf8_lossy(pattern));
                check(&label, &revision, &blocked, &query, &asked);
            }
        }
        for (pattern, case, query) in &everywhere {
            let asked = || {
                format!(
                    "{:?} {:?} {case:?}",
                    query.field(),
                    String::from_utf8_lossy(pattern)
                )
            };
            check(&label, &revision, &blocked, query, &asked);
        }
        for blocks in &blocked {
            passed += blocks.passed.get();
            inside += blocks.inside.get();
        }
    }
    assert!(checked > 150_000, "{checked}");
    // Exact names passed over blocks, and read blocks that start inside their bodies.
    assert!(passed > 50_000 && inside > 5_000, "{passed} {inside}");
}

/// What each field reads, the table the index's name postings are looked up by: under the
/// own semantics the names in the roles its rows come from, and field 8's include
/// operands; the oracle's fields 0, 2, 3 and 9, whose context carries from file to file,
/// read every file.
#[test]
fn each_field_reads_the_names_its_rows_come_from() {
    let every_mark: BTreeSet<u8> = (0..=u8::MAX)
        .filter(|byte| Mark::from_byte(*byte).is_some())
        .collect();
    let marks = |bytes: &[u8]| -> BTreeSet<u8> { bytes.iter().copied().collect() };
    // Every mark but a function's or macro's end and an include.
    let symbols: BTreeSet<u8> = &every_mark - &marks(b"})~");
    let definitions = marks(b"#$gmsuetc");
    let query = |digit| {
        let field = Field::from_digit(digit).expect("stored field");
        StoredQuery::compile(field, b"a", IdentifierCase::Sensitive).expect("query")
    };
    for (semantics, digit, expected) in [
        (Semantics::Own, b'0', (symbols.clone(), false)),
        (Semantics::Own, b'1', (definitions.clone(), false)),
        (Semantics::Own, b'2', (marks(b"#$"), false)),
        (Semantics::Own, b'3', (marks(b"`"), false)),
        (Semantics::Own, b'9', (symbols, true)),
        (Semantics::Sequential, b'1', (definitions, false)),
    ] {
        let query = query(digit);
        let Reads::Names { roles, .. } = query.reads(semantics) else {
            panic!("{semantics:?} {digit}: names");
        };
        let read = roles.marks.iter().map(|mark| mark.byte()).collect();
        assert_eq!((read, roles.assigned), expected, "{semantics:?} {digit}");
    }
    for semantics in [Semantics::Own, Semantics::Sequential] {
        assert!(matches!(query(b'7').reads(semantics), Reads::Paths));
        assert!(matches!(
            query(b'8').reads(semantics),
            Reads::Includes { .. }
        ));
    }
    for digit in *b"0239" {
        assert!(matches!(
            query(digit).reads(Semantics::Sequential),
            Reads::Every
        ));
    }
}

/// Field 1 over the pattern fixtures captured from the reference (definition requests,
/// regular expressions and their grammar, counted repetition), with the product's limits:
/// the engine's rows over the baseline reading equal cscope's, except where Ripscope
/// corrects cscope (cscope anchors `Alpha|Beta` as `^Alpha|Beta$`, and Ripscope anchors
/// the whole pattern), or a pattern is refused for the reason each case names (some of
/// which cscope answers: differences from cscope).
#[test]
fn definition_patterns_match_cscope_or_are_refused() {
    use crate::query::Refusal;
    use crate::query::pattern::{PatternError, ResourceLimit, UnsupportedPattern};
    let refused = |pattern: &str| -> Option<PatternError> {
        Some(match pattern {
            " Alpha" | "Alpha Beta" | "9Alpha" | "Alpha?" => PatternError::InvalidIdentifier,
            "[" | "[a-b-c]" | "[a-z-0-9]" | "a{3,2}" | "{2}a" => PatternError::InvalidSyntax,
            r"(a)\1" => PatternError::Unsupported(UnsupportedPattern::Backreference),
            "(?i:alpha)" | "(?:alpha)" => {
                PatternError::Unsupported(UnsupportedPattern::SpecialGroup)
            }
            r"Alpha\$" => PatternError::Unsupported(UnsupportedPattern::FinalDollarEscape),
            "(a))|aa" => PatternError::Unsupported(UnsupportedPattern::ClosingParenthesis),
            "aa|(a)\\" => PatternError::Unsupported(UnsupportedPattern::TrailingEscape),
            "a{32768}" | "a{" | "a{bad}" | "a{1,2,3}" | "a{,32768}" | "a{,}" => {
                PatternError::Unsupported(UnsupportedPattern::CountedRepetition)
            }
            "a{2}{2}" | "a{2}+" => {
                PatternError::Unsupported(UnsupportedPattern::RepeatedQuantifier)
            }
            "a{32767}" => PatternError::Resource(ResourceLimit::CompiledBytes),
            _ => return None,
        })
    };
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let corrections: Value = serde_json::from_slice(
        &std::fs::read(root.join("definition-pattern-corrections.json")).expect("corrections"),
    )
    .expect("corrections JSON");
    let (mut matched, mut corrected, mut refusals) = (0, 0, 0);
    let mut failures = Vec::new();
    for name in [
        "definition-requests.json",
        "definition-regex.json",
        "definition-pattern-grammar.json",
        "definition-counts.json",
        "definition-omitted-counts.json",
    ] {
        let fixture: Value =
            serde_json::from_slice(&std::fs::read(root.join(name)).expect("fixture"))
                .expect("fixture JSON");
        assert_eq!(fixture["build"]["exit_code"], 0, "{name}");
        assert_eq!(
            (&fixture["build"]["stdout"], &fixture["build"]["stderr"]),
            (&Value::from(""), &Value::from("")),
            "{name}"
        );
        let source = fixture["source"].as_str().expect("source");
        let unit =
            AnalysisUnit::from_records(&scanner::scan(source.as_bytes(), Mode::C)).expect("unit");
        let revision = Revision::in_memory(vec![(b"case.c".to_vec(), unit)]);
        for profile in fixture["profiles"].as_array().expect("profiles") {
            let caseless = profile["caseless"].as_bool().expect("caseless");
            let case = if caseless {
                IdentifierCase::AsciiInsensitive
            } else {
                IdentifierCase::Sensitive
            };
            for query in profile["queries"].as_array().expect("queries") {
                assert_eq!(query["exit_code"], 0, "{name}");
                assert_eq!(query["stderr"], "", "{name}");
                let args: Vec<&str> = query["args"]
                    .as_array()
                    .expect("args")
                    .iter()
                    .map(|arg| arg.as_str().expect("argument"))
                    .collect();
                let options: &[&str] = if caseless {
                    &["-dL", "-C", "-1"]
                } else {
                    &["-dL", "-1"]
                };
                assert_eq!(&args[..args.len() - 1], options, "{name}");
                let pattern = args[args.len() - 1];
                let stored = StoredQuery::compile(Field::Definitions, pattern.as_bytes(), case);
                if let Some(expected) = refused(pattern) {
                    assert_eq!(
                        format!("{:?}", stored.map(|_| ()).unwrap_err()),
                        format!("{:?}", Refusal::Pattern(expected)),
                        "{name}: {pattern:?}"
                    );
                    refusals += 1;
                    continue;
                }
                let stored =
                    stored.unwrap_or_else(|error| panic!("{name}: {pattern:?}: {error:?}"));
                let got: String = stored
                    .answer(Semantics::Sequential, &revision)
                    .map(|row| format!("{}\n", String::from_utf8_lossy(&printed(&row))))
                    .collect();
                let mut want = query["stdout"].as_str().expect("stdout");
                let correction = corrections["corrections"]
                    .as_array()
                    .expect("corrections")
                    .iter()
                    .find(|item| item["pattern"] == pattern && item["caseless"] == caseless);
                if let Some(correction) = correction {
                    assert_eq!(correction["cscope_stdout"], want, "{pattern:?}");
                    want = correction["ripscope_stdout"].as_str().expect("corrected");
                    corrected += 1;
                }
                if got != want {
                    failures.push(format!(
                        "{name} {case:?} {pattern:?}\n  want: {want:?}\n  got:  {got:?}"
                    ));
                }
                matched += 1;
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    assert_eq!((matched, corrected, refusals), (108, 2, 48));
}

/// The captures `engine_matches_the_cscope_captures` checks.
const CAPTURES: &[&str] = &[
    "continued-literal-records.json",
    "definition-eligibility.json",
    "preprocessor-records.json",
    "record-rendering.json",
    "written-abstract-pointer-cscope.json",
    "written-aggregate-cscope.json",
    "written-arm-ended-aggregates-cscope.json",
    "written-bare-pragmas-cscope.json",
    "written-closed-alternatives-cscope.json",
    "written-comment-continued-directives-cscope.json",
    "written-compiler-pragmas-cscope.json",
    "written-conditional-directive-comments-cscope.json",
    "written-declaration-wrappers-cscope.json",
    "written-diagnostic-braces-cscope.json",
    "written-enumerator-arms-cscope.json",
    "written-expression-braces-cscope.json",
    "written-function-binding-cscope.json",
    "written-git-path-and-enum-returns-cscope.json",
    "written-head-directives-cscope.json",
    "written-interior-annotations-cscope.json",
    "written-invocation-heads-cscope.json",
    "written-issue-148-149-cscope.json",
    "written-linkage-cscope.json",
    "written-local-brace-lists-cscope.json",
    "written-lone-arm-blocks-cscope.json",
    "written-member-invocations-cscope.json",
    "written-msvc-clang-pragmas-cscope.json",
    "written-parenthesized-invocations-cscope.json",
    "written-pattern-heads-cscope.json",
    "written-pointer-decorations-cscope.json",
    "written-pointer-object-decorations-cscope.json",
    "written-qualifier-arms-cscope.json",
    "written-return-decoration-cscope.json",
    "written-statement-invocations-cscope.json",
    "written-type-invocations-cscope.json",
    "written-unmatched-line-invocations-cscope.json",
    "written-unnamed-parameter-cscope.json",
    "written-unspaced-directives-cscope.json",
    "written-v14-identifiers-cscope.json",
    "written-warning-directives-cscope.json",
    "written-warning-pragma-cscope.json",
];
