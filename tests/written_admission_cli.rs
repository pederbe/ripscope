use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};

fn run(root: &Path, args: &[&str], input: &[u8]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_ripscope"))
        .current_dir(root)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input).unwrap();
    child.wait_with_output().unwrap()
}

// Both transports must return the same stored rows after source removal.
fn query(root: &Path, field: &str, pattern: &str) -> String {
    let output = run(
        root,
        &["-dL", "-f", "index", &format!("-{field}"), pattern],
        b"",
    );
    assert!(output.status.success(), "{:?}", output);
    assert!(output.stderr.is_empty());
    let persistent = run(
        root,
        &["-dl", "-f", "index"],
        format!("{field}{pattern}\nq\n").as_bytes(),
    );
    assert!(persistent.status.success(), "{:?}", persistent);
    assert!(persistent.stderr.is_empty());
    let lines = output.stdout.iter().filter(|&&b| b == b'\n').count();
    let mut expected = format!(">> cscope: {lines} lines\n").into_bytes();
    expected.extend_from_slice(&output.stdout);
    expected.extend_from_slice(b">> ");
    if output.stdout.is_empty() {
        expected = b">> Unable to search database\n>> ".to_vec();
    }
    assert_eq!(persistent.stdout, expected);
    String::from_utf8(output.stdout).unwrap()
}

// Expected rows follow the pinned cscope contracts captured in
// tests/fixtures/written-static-assertions-cscope.json and
// tests/fixtures/written-diagnostic-pragma-cscope.json.
#[test]
fn static_assertions_and_diagnostic_pragmas_build_and_query_after_source_removal() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    fs::write(
        root.join("case.c"),
        concat!(
            "struct S { int a; };\n",
            "static_assert(offsetof(struct S, a) == 0, \"msg\");\n",
            "#pragma GCC diagnostic push\n",
            "#pragma GCC diagnostic ignored \"-Wunused\"\n",
            "void f(void) { g(); }\n",
            "#pragma GCC diagnostic pop\n",
        ),
    )
    .unwrap();
    let build = run(root, &["-bk", "-f", "index", "case.c"], b"");
    assert!(build.status.success(), "{:?}", build);
    assert!(build.stdout.is_empty() && build.stderr.is_empty());
    fs::remove_file(root.join("case.c")).unwrap();
    assert_eq!(
        query(root, "0", "offsetof"),
        "case.c <global> 2 static_assert(offsetof(struct S, a) == 0, \"msg\");\n"
    );
    assert_eq!(query(root, "1", "a"), "case.c a 1 struct S { int a; };\n");
    assert_eq!(query(root, "3", "offsetof"), "");
    assert_eq!(
        query(root, "0", "push"),
        "case.c <global> 3 #pragma GCC diagnostic push\n"
    );
    assert_eq!(query(root, "1", "push"), "");
    assert_eq!(query(root, "3", "g"), "case.c f 5 void f(void ) { g(); }\n");
}

// The reading follows the pinned cscope's, captured in
// tests/fixtures/written-separator-owners-cscope.json; the answers follow ripscope's own
// query semantics (ADR 0010, "Ripscope's own query semantics"): the body's call has the last head as its caller.
#[test]
fn separator_calls_after_alternative_function_starts_query_after_source_removal() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    fs::write(
        root.join("case.c"),
        concat!(
            "#if A\n",
            "void left(void) {\n",
            "#elif CHECK(B)\n",
            "void right(void) {\n",
            "#else\n",
            "void third(void) {\n",
            "#endif\n",
            "after();\n",
            "}\n",
        ),
    )
    .unwrap();
    let build = run(root, &["-bk", "-f", "index", "case.c"], b"");
    assert!(build.status.success(), "{:?}", build);
    assert!(build.stdout.is_empty() && build.stderr.is_empty());
    fs::remove_file(root.join("case.c")).unwrap();
    assert_eq!(query(root, "3", "CHECK"), "case.c left 3 #elif CHECK(B)\n");
    assert_eq!(query(root, "3", "after"), "case.c third 8 after();\n");
    assert_eq!(query(root, "2", "left"), "case.c CHECK 3 #elif CHECK(B)\n");
}

// Approved differences: see
// tests/fixtures/written-declaration-annotations-approved.json.
#[test]
fn annotated_and_prefixed_function_pointers_query_after_source_removal() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    fs::write(
        root.join("case.c"),
        concat!(
            "API void (*Log)(const char *fmt, ...) ATTR PRINTF(3,4);\n",
            "API size_t (*ValueLength)(Key *kp) ATTR;\n",
            "extern monotime (*getMonotonicUs)(void);\n",
            "int after;\n",
        ),
    )
    .unwrap();
    let build = run(root, &["-bk", "-f", "index", "case.c"], b"");
    assert!(build.status.success(), "{:?}", build);
    assert!(build.stdout.is_empty() && build.stderr.is_empty());
    fs::remove_file(root.join("case.c")).unwrap();
    assert_eq!(
        query(root, "1", "Log"),
        "case.c Log 1 API void (*Log)(const char *fmt, ...) ATTR PRINTF(3,4);\n"
    );
    assert_eq!(
        query(root, "1", "ValueLength"),
        "case.c ValueLength 2 API size_t (*ValueLength)(Key *kp) ATTR;\n"
    );
    assert_eq!(query(root, "1", "after"), "case.c after 4 int after;\n");
    for (field, pattern) in [
        ("1", "ATTR"),
        ("1", "PRINTF"),
        ("1", "size_t"),
        ("1", "monotime"),
        ("1", "getMonotonicUs"),
        ("3", "PRINTF"),
    ] {
        assert_eq!(query(root, field, pattern), "", "-{field} {pattern}");
    }
    assert_eq!(
        query(root, "0", "PRINTF"),
        "case.c <global> 1 API void (*Log)(const char *fmt, ...) ATTR PRINTF(3,4);\n"
    );
}

// Expected rows follow the pinned cscope contract captured in
// tests/fixtures/written-conditional-initializers-cscope.json.
#[test]
fn conditional_initializer_entries_query_after_source_removal() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    fs::write(
        root.join("case.c"),
        concat!(
            "struct s v[] = {\n",
            " {1, f},\n",
            "#if X\n",
            " {2, g},\n",
            "#endif\n",
            " {0, 0} };\n",
            "int after;\n",
        ),
    )
    .unwrap();
    let build = run(root, &["-bk", "-f", "index", "case.c"], b"");
    assert!(build.status.success(), "{:?}", build);
    assert!(build.stdout.is_empty() && build.stderr.is_empty());
    fs::remove_file(root.join("case.c")).unwrap();
    assert_eq!(query(root, "1", "v"), "case.c v 1 struct s v[] = {\n");
    assert_eq!(query(root, "0", "g"), "case.c <global> 4 {2, g},\n");
    assert_eq!(query(root, "1", "after"), "case.c after 7 int after;\n");
}
