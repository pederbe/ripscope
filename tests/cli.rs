use std::{fs, path::Path, process::Command};

fn run(directory: &Path, arguments: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_ripscope"))
        .current_dir(directory)
        .args(arguments)
        .output()
        .unwrap()
}

fn assert_success(output: &std::process::Output) {
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"");
    assert_eq!(output.stderr, b"");
}

#[test]
fn a_prefix_call_before_its_function_has_no_caller_and_its_row_is_at_file_scope() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = "#if A\n__attribute__((annotate(payload())))\n#endif\nvoid f(int callback(int), int values[bound()]) { int local; target(); }\n";
    fs::write(root.join("case.c"), source).unwrap();
    assert_success(&run(root, &["-bk", "-f", "index", "case.c"]));
    fs::remove_file(root.join("case.c")).unwrap();
    // The call sits before the function's head, at file scope, so under ripscope's own
    // query semantics (ADR 0010, "Ripscope's own query semantics give each name the
    // context it sits in") it has no caller.
    for (field, want) in [
        (
            "-0",
            "case.c <global> 2 __attribute__((annotate(payload())))\n",
        ),
        ("-3", ""),
    ] {
        let result = run(root, &["-dL", "-f", "index", field, "payload"]);
        assert!(result.status.success());
        assert!(result.stderr.is_empty());
        assert_eq!(result.stdout, want.as_bytes(), "{field}");
    }
    let callback = run(root, &["-dL", "-f", "index", "-3", "callback"]);
    assert!(callback.status.success() && callback.stdout.is_empty() && callback.stderr.is_empty());
}

#[test]
fn executable_builds_refreshes_and_queries_persisted_definitions() {
    let directory = tempfile::tempdir().unwrap();
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cli-definitions.c"),
        directory.path().join("definitions.c"),
    )
    .unwrap();

    assert_success(&run(
        directory.path(),
        &["-bk", "-f", "index", "definitions.c"],
    ));
    let initial = run(directory.path(), &["-dL", "-f", "index", "-1", "global"]);
    assert_eq!(
        initial.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&initial.stderr)
    );
    assert_eq!(
        initial.stdout,
        b"definitions.c global 5 int global = 1, spare;\n"
    );
    assert_eq!(initial.stderr, b"");

    fs::write(directory.path().join("definitions.c"), "int refreshed;\n").unwrap();
    assert_success(&run(
        directory.path(),
        &["-b", "-k", "-f", "index", "definitions.c"],
    ));
    let refreshed = run(directory.path(), &["-dL", "-f", "index", "-1", "refreshed"]);
    assert_eq!(
        refreshed.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&refreshed.stderr)
    );
    assert_eq!(
        refreshed.stdout,
        b"definitions.c refreshed 1 int refreshed;\n"
    );
    assert_eq!(refreshed.stderr, b"");

    fs::remove_file(directory.path().join("definitions.c")).unwrap();
    let persisted = run(
        directory.path(),
        &["-dL", "-f", "index", "-C", "-1", "REFRESHED"],
    );
    assert_eq!(
        persisted.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&persisted.stderr)
    );
    assert_eq!(
        persisted.stdout,
        b"definitions.c REFRESHED 1 int refreshed;\n"
    );
    assert_eq!(persisted.stderr, b"");

    let missing = run(directory.path(), &["-dL", "-f", "index", "-1", "Missing"]);
    assert_eq!(missing.status.code(), Some(0));
    assert_eq!(missing.stdout, b"");
    assert_eq!(missing.stderr, b"");
}

/// Each occurrence takes the context it sits in (ADR 0010, "Ripscope's own query semantics"): a match on a type in a
/// function head does not skip the head, so the body's lines take the function.
#[test]
fn executable_queries_persisted_symbol_occurrences_with_their_context() {
    let directory = tempfile::tempdir().unwrap();
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/occurrences.c"),
        directory.path().join("occurrences.c"),
    )
    .unwrap();

    assert_success(&run(
        directory.path(),
        &["-bk", "-f", "index", "occurrences.c"],
    ));
    let refreshed = run(directory.path(), &["-L", "-f", "index", "-0", "Result"]);
    assert_eq!(refreshed.status.code(), Some(0));
    assert_eq!(
        refreshed.stdout,
        b"occurrences.c <global> 5 typedef int Result;\noccurrences.c <global> 8 Result make(Result arg, struct Ops *ops) {\noccurrences.c make 9 Result local = SAME(arg);\n"
    );
    assert_eq!(refreshed.stderr, b"");
    fs::remove_file(directory.path().join("occurrences.c")).unwrap();

    for (pattern, expected) in [
        (
            "Result",
            "occurrences.c <global> 5 typedef int Result;\noccurrences.c <global> 8 Result make(Result arg, struct Ops *ops) {\noccurrences.c make 9 Result local = SAME(arg);\n",
        ),
        (
            "arg",
            "occurrences.c make 8 Result make(Result arg, struct Ops *ops) {\noccurrences.c make 9 Result local = SAME(arg);\n",
        ),
        (
            "SAME",
            "occurrences.c <global> 1 #define SAME(x) ((x) + global)\noccurrences.c make 9 Result local = SAME(arg);\n",
        ),
        (
            "global",
            "occurrences.c <global> 6 int global;\noccurrences.c SAME 1 #define SAME(x) ((x) + global)\n",
        ),
        (
            "member",
            "occurrences.c <global> 7 struct Ops { int member; };\noccurrences.c make 10 return local + ops->member;\n",
        ),
        ("UNKNOWN", "occurrences.c <global> 2 #ifdef UNKNOWN\n"),
        (
            "Result|arg",
            "occurrences.c <global> 5 typedef int Result;\noccurrences.c <global> 8 Result make(Result arg, struct Ops *ops) {\noccurrences.c make 9 Result local = SAME(arg);\n",
        ),
        (
            "Unknown",
            "occurrences.c <global> 12 Unknown build(Unknown value) {\noccurrences.c build 13 Unknown copy = value;\n",
        ),
        (
            "value",
            "occurrences.c build 12 Unknown build(Unknown value) {\noccurrences.c build 13 Unknown copy = value;\n",
        ),
        (
            "leak",
            "occurrences.c <global> 17 int leak;\noccurrences.c one_line 16 int one_line(void ) { int leak; }\n",
        ),
    ] {
        let actual = run(directory.path(), &["-dL", "-f", "index", "-0", pattern]);
        assert_eq!(
            actual.status.code(),
            Some(0),
            "{pattern}: {}",
            String::from_utf8_lossy(&actual.stderr)
        );
        assert_eq!(actual.stdout, expected.as_bytes(), "{pattern}");
        assert_eq!(actual.stderr, b"", "{pattern}");
    }
}

#[test]
fn the_command_line_says_what_it_cannot_do_on_stderr() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for (args, message) in [
        (
            &["-dL", "-f", "index", "-1", "Alpha"][..],
            "no index at index",
        ),
        (
            &["-bdLk", "-f", "index", "case.c"],
            "-b cannot be combined with -L, -l or a query",
        ),
        (
            &["-dL", "-f", "index", "-1", "Alpha", "extra.c"],
            "file names can only be given to an index build (-b)",
        ),
        (&["-dL", "-f", "index", "-5"], "-5 needs a value"),
        (&["-z"], "unsupported option: -z"),
        (&["-L"], "-L needs a query such as -0 NAME"),
    ] {
        let refused = run(root, args);
        assert_eq!(refused.status.code(), Some(1), "{args:?}");
        assert_eq!(refused.stdout, b"", "{args:?}");
        assert_eq!(
            String::from_utf8_lossy(&refused.stderr),
            format!("ripscope: {message}\n"),
            "{args:?}"
        );
    }
    // An index build that finds nothing makes no index, as cscope's.
    let missing_source = run(root, &["-bk", "-f", "index", "missing.c"]);
    assert_eq!(missing_source.status.code(), Some(1));
    assert_eq!(
        missing_source.stderr,
        b"ripscope: cannot find file missing.c\nripscope: no source files found\n"
    );
    assert!(!root.join("index").exists());
    // A file that is not a ripscope index is never touched.
    fs::write(root.join("foreign"), b"not a ripscope index").unwrap();
    fs::write(root.join("case.c"), b"int Alpha;\n").unwrap();
    let refused = run(root, &["-bk", "-f", "foreign", "case.c"]);
    assert_eq!(refused.status.code(), Some(1));
    assert_eq!(
        refused.stderr,
        b"ripscope: foreign is not a ripscope index; it was left unchanged\n"
    );
    assert_eq!(
        fs::read(root.join("foreign")).unwrap(),
        b"not a ripscope index"
    );
    // As cscope, a pattern that cannot be used finds nothing and the query succeeds.
    assert_success(&run(root, &["-bk", "-f", "index", "case.c"]));
    let bad_pattern = run(root, &["-dL", "-f", "index", "-1", "["]);
    assert_eq!(bad_pattern.status.code(), Some(0));
    assert_eq!(bad_pattern.stdout, b"");
    assert_eq!(
        bad_pattern.stderr,
        b"ripscope: invalid query pattern: invalid syntax\n"
    );
    // A problem with the index comes first.
    let no_index = run(root, &["-dL", "-f", "missing", "-1", "["]);
    assert_eq!(no_index.status.code(), Some(1));
    assert_eq!(no_index.stderr, b"ripscope: no index at missing\n");
}

#[cfg(unix)]
#[test]
fn a_message_naming_a_path_with_a_line_break_stays_on_one_line() {
    let directory = tempfile::tempdir().unwrap();
    let output = run(
        directory.path(),
        &["-dL", "-f", "missing\nindex", "-1", "Alpha"],
    );
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(output.stdout, b"");
    assert_eq!(output.stderr, b"ripscope: no index at missing\\x0aindex\n");
}

#[test]
fn written_cli_build_discovers_headers_refreshes_and_queries_without_sources() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = "#include \"dep.h\"\n#if FLAG\nvoid left(void) {\n#else\nvoid right(void) {\n#endif\ntarget();\n}\n";
    fs::write(root.join("case.c"), source).unwrap();
    fs::write(root.join("dep.h"), "void helper(void) { target(); }\n").unwrap();
    assert_success(&run(root, &["-bk", "-f", "index", "case.c"]));
    let initial = run(root, &["-dL", "-f", "index", "-3", "target"]);
    assert!(
        initial.status.success(),
        "{}",
        String::from_utf8_lossy(&initial.stderr)
    );
    // Alternative heads share one body: its call has the last head as its caller (ADR 0010, "Ripscope's own query semantics").
    assert_eq!(
        initial.stdout,
        b"case.c right 7 target();\ndep.h helper 1 void helper(void ) { target(); }\n"
    );
    assert!(initial.stderr.is_empty());
    fs::write(root.join("case.c"), source.replace("target()", "changed()")).unwrap();
    let refreshed = run(root, &["-L", "-f", "index", "-3", "changed"]);
    assert!(
        refreshed.status.success(),
        "{}",
        String::from_utf8_lossy(&refreshed.stderr)
    );
    assert_eq!(refreshed.stdout, b"case.c right 7 changed();\n");
    fs::remove_file(root.join("case.c")).unwrap();
    fs::remove_file(root.join("dep.h")).unwrap();
    let stored = run(root, &["-dL", "-f", "index", "-3", "changed"]);
    assert!(stored.status.success());
    assert_eq!(stored.stdout, refreshed.stdout);
    assert!(stored.stderr.is_empty());
}

#[test]
fn numeric_body_stored_queries_survive_source_removal() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let mut text = String::from("int table[] = {\n");
    for _ in 0..160 {
        text.push_str(&"0x1234,".repeat(256));
        text.push('\n');
    }
    text.push_str("};\nvoid after(void) { target(); }\n");
    fs::write(root.join("case.c"), text).unwrap();
    assert_success(&run(root, &["-bk", "-f", "index", "case.c"]));
    let queries = [
        ("-0", "table"),
        ("-1", "table"),
        ("-2", "after"),
        ("-3", "target"),
        ("-9", "table"),
    ];
    let mut expected = Vec::new();
    for (field, pattern) in queries {
        let output = run(root, &["-dL", "-f", "index", field, pattern]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        if field != "-9" {
            assert!(!output.stdout.is_empty(), "{field}");
        }
        expected.push(output.stdout);
    }
    assert_eq!(expected[1], b"case.c table 1 int table[] = {\n");
    for field in ["-4", "-6"] {
        let output = run(root, &["-dL", "-f", "index", field, "0x1234"]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.stdout.iter().filter(|&&b| b == b'\n').count(), 160);
    }
    fs::remove_file(root.join("case.c")).unwrap();
    for ((field, pattern), expected) in queries.into_iter().zip(expected) {
        let output = run(root, &["-dL", "-f", "index", field, pattern]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.stdout, expected);
    }
}
