//! Each rule of the own semantics on a small source, beside the oracle's answer where the
//! two differ, and the properties ADR 0010 rests on over the reference query probes.

use crate::engine::{Field, Semantics};
use crate::query::{IdentifierCase, StoredQuery};
use crate::revision::Revision;
use crate::scanner::{self, Corrections, Mode};
use crate::unit::AnalysisUnit;

/// A file named `name` holding `source`, read as the product reads it.
fn file(name: &str, source: &[u8]) -> (Vec<u8>, AnalysisUnit) {
    let records = scanner::scan_with(
        source,
        Mode::for_name(name.as_bytes()),
        Corrections::ENABLED,
    );
    let unit = AnalysisUnit::from_records(&records).expect("unit");
    (name.as_bytes().to_vec(), unit)
}

/// `(file, line, text)` and the context, for each row.
type Answer = Vec<((Vec<u8>, u32, Vec<u8>), Vec<u8>)>;

fn answer(
    semantics: Semantics,
    revision: &Revision,
    field: Field,
    pattern: &[u8],
    case: IdentifierCase,
) -> Answer {
    let Ok(query) = StoredQuery::compile(field, pattern, case) else {
        return Vec::new();
    };
    query
        .answer(semantics, revision)
        .map(|row| {
            (
                (row.path.to_vec(), row.line, row.text.to_vec()),
                row.context.to_vec(),
            )
        })
        .collect()
}

/// Rows as `file context line`, as a query over `sources` gives them.
fn rows(
    semantics: Semantics,
    sources: &[(&str, &str)],
    query: &str,
    case: IdentifierCase,
) -> Vec<String> {
    let revision = Revision::in_memory(
        sources
            .iter()
            .map(|(name, source)| file(name, source.as_bytes()))
            .collect(),
    );
    let field = Field::from_digit(query.as_bytes()[0]).expect("field");
    answer(semantics, &revision, field, &query.as_bytes()[1..], case)
        .into_iter()
        .map(|((file, line, _), context)| {
            format!(
                "{} {} {line}",
                String::from_utf8_lossy(&file),
                String::from_utf8_lossy(&context)
            )
        })
        .collect()
}

fn own(sources: &[(&str, &str)], query: &str) -> Vec<String> {
    rows(Semantics::Own, sources, query, IdentifierCase::Sensitive)
}

fn sequential(sources: &[(&str, &str)], query: &str) -> Vec<String> {
    rows(
        Semantics::Sequential,
        sources,
        query,
        IdentifierCase::Sensitive,
    )
}

/// A match does not consume the rest of its line: a function head after it still opens
/// the function for the lines below.
#[test]
fn a_head_after_a_match_on_its_line_still_opens_its_function() {
    let sources = [("a.c", "int a; int g(void) {\n\treturn a;\n}\n")];
    assert_eq!(own(&sources, "0a"), ["a.c <global> 1", "a.c g 2"]);
    assert_eq!(
        sequential(&sources, "0a"),
        ["a.c <global> 1", "a.c <global> 2"]
    );
    assert_eq!(own(&sources, "9a"), Vec::<String>::new());
}

/// A line gives at most one row, for its first matching name (for field 9, its first
/// matching assigned name).
#[test]
fn a_line_gives_one_row_for_its_first_matching_name() {
    let sources = [("a.c", "int f(void) {\n\tx = y = z;\n}\n")];
    assert_eq!(own(&sources, "0[xyz]"), ["a.c f 2"]);
    assert_eq!(own(&sources, "9[yz]"), ["a.c f 2"]);
    assert_eq!(own(&sources, "0z"), ["a.c f 2"]);
    let unassigned_first = [("a.c", "int f(void) {\n\tg(y); z = 1;\n}\n")];
    assert_eq!(own(&unassigned_first, "9[yz]"), ["a.c f 2"]);
    assert_eq!(own(&unassigned_first, "9z"), ["a.c f 2"]);
    assert_eq!(own(&unassigned_first, "9y"), Vec::<String>::new());
}

/// A one-line macro defined in a body ends at its line's end: the body's names after it
/// take the function again, and names on its line after its name take the macro.
#[test]
fn a_one_line_macro_in_a_body_ends_at_its_line_end() {
    let sources = [("a.c", "void f(void) {\n#define M limit\n\tx;\n}\n")];
    assert_eq!(own(&sources, "0x"), ["a.c f 3"]);
    assert_eq!(own(&sources, "0limit"), ["a.c M 2"]);
}

/// A function's end closes it: a name after the end is at file scope.
#[test]
fn a_function_end_closes_its_function() {
    let sources = [("a.c", "int f(void) {\n}\nint after = g();\n")];
    assert_eq!(own(&sources, "0after"), ["a.c <global> 3"]);
    assert_eq!(own(&sources, "3g"), Vec::<String>::new());
}

/// Nothing carries from one file to the next: not a function left open, not a macro.
#[test]
fn nothing_carries_from_one_file_to_the_next() {
    let open_function = [
        ("a.c", "void open(void) {\n\tgo();\n"),
        ("b.c", "int v = go();\nint go_on;\n"),
    ];
    assert_eq!(own(&open_function, "3go"), ["a.c open 2"]);
    assert_eq!(own(&open_function, "0go_on"), ["b.c <global> 2"]);
    let open_macro = [("a.h", "#define M(x) \\\n\tx"), ("b.c", "int after;\n")];
    assert_eq!(own(&open_macro, "0after"), ["b.c <global> 1"]);
    assert_eq!(sequential(&open_macro, "0after"), ["b.c M 1"]);
}

/// Field 3 attributes a call to the one function it sits in, the latest head: alternative
/// heads under `#if` share one caller (a known difference from cscope's sequential search,
/// which lists every open head).
#[test]
fn a_call_has_one_caller_the_latest_head() {
    let sources = [(
        "a.c",
        "#if A\nvoid left(void) {\n#else\nvoid right(void) {\n#endif\n\tafter();\n}\n",
    )];
    assert_eq!(own(&sources, "3after"), ["a.c right 6"]);
    assert_eq!(
        sequential(&sources, "3after"),
        ["a.c left 6", "a.c right 6"]
    );
    assert_eq!(own(&sources, "2right"), ["a.c after 6"]);
    assert_eq!(own(&sources, "2left"), Vec::<String>::new());
}

/// A call inside a macro defined in a body belongs to the macro, in field 2 and field 3;
/// the body's own calls go on after it.
#[test]
fn a_macro_defined_in_a_body_owns_its_calls() {
    let sources = [(
        "a.c",
        "void outer(void) {\n#define INNER() target()\n\thidden();\n}\n",
    )];
    assert_eq!(own(&sources, "2outer"), ["a.c hidden 3"]);
    assert_eq!(own(&sources, "2INNER"), ["a.c target 2"]);
    assert_eq!(own(&sources, "3target"), ["a.c INNER 2"]);
    assert_eq!(own(&sources, "3hidden"), ["a.c outer 3"]);
    assert_eq!(own(&sources, "2.*"), ["a.c target 2", "a.c hidden 3"]);
}

/// A call outside every function and macro has no caller.
#[test]
fn a_call_outside_functions_and_macros_has_no_caller() {
    let sources = [("a.c", "int v = go();\n")];
    assert_eq!(own(&sources, "3go"), Vec::<String>::new());
}

/// A macro definition's own row takes the function it sits in, with or without `-C` (the
/// oracle keeps cscope's `-C` quirk, which gives the macro's own name).
#[test]
fn a_macro_definitions_own_row_takes_its_function() {
    let sources = [("a.c", "void f(void) {\n#define ONCE 1\n}\n")];
    assert_eq!(own(&sources, "0ONCE"), ["a.c f 2"]);
    let caseless = |semantics| {
        rows(
            semantics,
            &sources,
            "0ONCE",
            IdentifierCase::AsciiInsensitive,
        )
    };
    assert_eq!(caseless(Semantics::Own), ["a.c f 2"]);
    assert_eq!(caseless(Semantics::Sequential), ["a.c ONCE 2"]);
}

/// A reference query probe's files with the product's reading, and its queries.
struct Probe {
    names: Vec<String>,
    files: Vec<(Vec<u8>, AnalysisUnit)>,
    queries: Vec<(Field, Vec<u8>, IdentifierCase)>,
}

/// Every probe, with `.*` for fields 0, 2, 3 and 9 added to its queries.
fn probes() -> Vec<Probe> {
    crate::engine::tests::Probe::all()
        .iter()
        .map(|probe| {
            let mut queries: Vec<_> = probe
                .queries()
                .map(|(field, pattern, case, _)| (field, pattern.as_bytes().to_vec(), case))
                .collect();
            for &field in b"0239" {
                for case in [IdentifierCase::Sensitive, IdentifierCase::AsciiInsensitive] {
                    queries.push((
                        Field::from_digit(field).expect("field"),
                        b".*".to_vec(),
                        case,
                    ));
                }
            }
            Probe {
                names: probe.sources.iter().map(|(name, _)| name.clone()).collect(),
                files: probe
                    .sources
                    .iter()
                    .map(|(name, source)| file(name, source))
                    .collect(),
                queries,
            }
        })
        .collect()
}

impl Probe {
    /// Its files, in the order the reference read them.
    fn revision(&self) -> Revision {
        Revision::in_memory(self.files.clone())
    }
}

/// File locality, which ADR 0010 rests on: a query over several files answers what
/// it answers over each file alone, file-scope rows first.
#[test]
fn each_file_answers_alone() {
    let mut checked = 0;
    for probe in probes() {
        let whole_revision = probe.revision();
        let alone_revisions: Vec<Revision> = probe
            .files
            .iter()
            .map(|file| Revision::in_memory(vec![file.clone()]))
            .collect();
        for (field, pattern, case) in &probe.queries {
            let whole = answer(Semantics::Own, &whole_revision, *field, pattern, *case);
            let alone: Vec<Answer> = alone_revisions
                .iter()
                .map(|revision| answer(Semantics::Own, revision, *field, pattern, *case))
                .collect();
            let global = |row: &(_, Vec<u8>)| row.1 == b"<global>";
            let merged: Answer = alone
                .iter()
                .flat_map(|rows| rows.iter().filter(|row| global(row)))
                .chain(
                    alone
                        .iter()
                        .flat_map(|rows| rows.iter().filter(|row| !global(row))),
                )
                .cloned()
                .collect();
            assert_eq!(
                whole, merged,
                "{:?} {field:?} {pattern:?} {case:?}",
                probe.names
            );
            checked += 1;
        }
    }
    assert!(checked > 100, "{checked}");
}

/// Fields 2 and 3 read calls the same way: `2.*` and `3.*` give the same rows.
#[test]
fn every_call_is_a_callee_and_has_a_caller_alike() {
    let mut calls = 0;
    for probe in probes() {
        let revision = probe.revision();
        for case in [IdentifierCase::Sensitive, IdentifierCase::AsciiInsensitive] {
            let located = |field| -> Vec<_> {
                answer(Semantics::Own, &revision, field, b".*", case)
                    .into_iter()
                    .map(|(located, _)| located)
                    .collect()
            };
            let callees = located(Field::Callees);
            assert_eq!(
                callees,
                located(Field::Callers),
                "{:?} {case:?}",
                probe.names
            );
            calls += callees.len();
        }
    }
    assert!(calls > 20, "{calls}");
}
