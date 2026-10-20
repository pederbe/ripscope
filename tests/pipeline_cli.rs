//! The command line end to end: index builds, one-shot queries and sessions
//! checked against the reference's answers.

mod intended;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

fn ripscope(directory: &Path, args: &[&str], input: Option<&[u8]>) -> Output {
    ripscope_with(directory, args, input, &[])
}

fn json_frames(output: &[u8]) -> Vec<serde_json::Value> {
    output
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .map(|line| serde_json::from_slice(line).expect("frame"))
        .collect()
}

/// [`ripscope`], with `environment` set.
fn ripscope_with(
    directory: &Path,
    args: &[&str],
    input: Option<&[u8]>,
    environment: &[(&str, &str)],
) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_ripscope"))
        .current_dir(directory)
        .env_remove("INCLUDEDIRS")
        .env_remove("INCDIR")
        .env_remove("RIPSCOPE_BLOCK_SIZE")
        .envs(environment.iter().copied())
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("ripscope");
    let mut stdin = child.stdin.take().expect("stdin");
    if let Some(input) = input {
        // A command that ends before reading its input (a refused index) closes the pipe.
        if let Err(error) = stdin.write_all(input) {
            assert_eq!(
                error.kind(),
                std::io::ErrorKind::BrokenPipe,
                "input: {error}"
            );
        }
    }
    drop(stdin);
    child.wait_with_output().expect("output")
}

/// A directory holding `a.c` (`int Alpha;`) and its index `i.db`, built.
fn indexed() -> tempfile::TempDir {
    let work = tempfile::tempdir().expect("work");
    std::fs::write(work.path().join("a.c"), "int Alpha;\n").expect("source");
    let built = ripscope(work.path(), &["-bk", "-f", "i.db", "a.c"], None);
    assert!(built.status.success(), "{built:?}");
    work
}

fn assert_stored_queries(directory: &Path, queries: &[(&str, &str, &str)]) {
    for caseless in [false, true] {
        let mut commands = String::new();
        let mut expected_session = String::from(">> ");
        for (field, name, expected) in queries {
            let original_name = *name;
            let name = if caseless {
                name.to_uppercase()
            } else {
                (*name).to_owned()
            };
            // Definition rows echo the requested pattern in their context column.
            let expected = if caseless && *field == "1" {
                expected.replacen(&format!(" {original_name} "), &format!(" {name} "), 1)
            } else {
                (*expected).to_owned()
            };
            let field_option = format!("-{field}");
            let mut args = vec!["-dL", "-f", "i.db", &field_option, &name];
            if caseless {
                args.push("-C");
            }
            let answer = ripscope(directory, &args, None);
            assert!(
                answer.status.success() && answer.stderr.is_empty(),
                "{answer:?}"
            );
            assert_eq!(
                String::from_utf8_lossy(&answer.stdout),
                expected,
                "{field}{name}"
            );
            commands.push_str(&format!("{field}{name}\n"));
            if expected.is_empty() {
                expected_session.push_str("Unable to search database\n>> ");
            } else {
                expected_session.push_str(&format!(
                    "cscope: {} lines\n{expected}>> ",
                    expected.lines().count()
                ));
            }
        }
        commands.push_str("q\n");
        let mut args = vec!["-dl", "-f", "i.db"];
        if caseless {
            args.push("-C");
        }
        let session = ripscope(directory, &args, Some(commands.as_bytes()));
        assert!(
            session.status.success() && session.stderr.is_empty(),
            "{session:?}"
        );
        assert_eq!(String::from_utf8_lossy(&session.stdout), expected_session);
    }
}

#[test]
fn subscript_assignment_queries_preserve_the_written_context_after_source_removal() {
    let work = tempfile::tempdir().expect("work");
    let path = work.path().join("arrays.c");
    std::fs::write(&path, "int slots[2] = {0};\nvoid f(void) { slots[0]; } slots[1] = 1;\nvoid g(void) { slots[0]; #define SET slots[1] += 2\n}\n").expect("source");
    let built = ripscope(work.path(), &["-bk", "-f", "i.db", "arrays.c"], None);
    assert!(
        built.status.success() && built.stderr.is_empty(),
        "{built:?}"
    );
    std::fs::remove_file(path).expect("remove source");
    let expected = "arrays.c <global> 1 int slots[2] = {0};\narrays.c <global> 2 void f(void ) { slots[0]; } slots[1] = 1;\narrays.c SET 3 #define g(void ) { slots[0]; #define SET slots[1] += 2\n";
    assert_stored_queries(
        work.path(),
        &[
            ("9", "slots", expected),
            ("9", "s.*", expected),
            ("9", "SET", ""),
        ],
    );
    for caseless in [false, true] {
        let mut args = vec![
            "--json",
            "-dL",
            "-f",
            "i.db",
            "-9",
            if caseless { "SLOTS" } else { "slots" },
        ];
        if caseless {
            args.push("-C");
        }
        let answer = ripscope(work.path(), &args, None);
        assert!(
            answer.status.success() && answer.stderr.is_empty(),
            "{answer:?}"
        );
        let frames = json_frames(&answer.stdout);
        assert_eq!(
            frames,
            vec![
                serde_json::json!(["arrays.c", "<global>", 1, "int slots[2] = {0};"]),
                serde_json::json!([
                    "arrays.c",
                    "<global>",
                    2,
                    "void f(void ) { slots[0]; } slots[1] = 1;"
                ]),
                serde_json::json!([
                    "arrays.c",
                    "SET",
                    3,
                    "#define g(void ) { slots[0]; #define SET slots[1] += 2"
                ]),
            ]
        );
        let mut args = vec!["--json", "-dl", "-f", "i.db"];
        if caseless {
            args.push("-C");
        }
        let session = ripscope(
            work.path(),
            &args,
            Some(if caseless {
                b"9SLOTS\n9SET\nq\n"
            } else {
                b"9slots\n9SET\nq\n"
            }),
        );
        assert!(
            session.status.success() && session.stderr.is_empty(),
            "{session:?}"
        );
        let actual = json_frames(&session.stdout);
        let mut expected = vec![
            serde_json::json!({"type":"ready"}),
            serde_json::json!({"type":"result","count":3}),
        ];
        expected.extend(frames);
        expected.extend([
            serde_json::json!({"type":"ready"}),
            serde_json::json!({"type":"result","count":0}),
            serde_json::json!({"type":"ready"}),
        ]);
        assert_eq!(actual, expected);
    }
}

#[test]
fn named_mutation_queries_keep_later_write_contexts_after_source_removal() {
    let work = tempfile::tempdir().expect("work");
    let path = work.path().join("mutations.c");
    std::fs::write(&path, "unsigned counter = 0;\nvoid f(void) { read(counter); } counter++;\nvoid g(void) { read(counter); #define STEP ++counter\n}\nvoid h(void) {\n counter; sizeof(counter++); ++counter;\n left++; right--;\n object.counter++; pointer->counter++; slots[index]++;\n}\n").expect("source");
    let baseline = ripscope(
        work.path(),
        &["--store-build", "-k", "-f", "baseline.db", "mutations.c"],
        None,
    );
    assert!(baseline.status.success(), "{baseline:?}");
    let built = ripscope(work.path(), &["-bk", "-f", "i.db", "mutations.c"], None);
    assert!(
        built.status.success() && built.stderr.is_empty(),
        "{built:?}"
    );
    std::fs::remove_file(path).expect("remove source");
    let expected = "mutations.c <global> 1 unsigned counter = 0;\nmutations.c <global> 2 void f(void ) { read(counter); } counter++;\nmutations.c STEP 3 #define g(void ) { read(counter); #define STEP ++counter\nmutations.c h 6 counter; sizeof(counter++); ++counter;\n";
    assert_stored_queries(
        work.path(),
        &[
            ("9", "counter", expected),
            ("9", "left", "mutations.c h 7 left++; right--;\n"),
            ("9", "right", "mutations.c h 7 left++; right--;\n"),
            ("9", "^(left|right)$", "mutations.c h 7 left++; right--;\n"),
            ("9", "index", ""),
            ("9", "STEP", ""),
        ],
    );
    let expected_frames = vec![
        serde_json::json!(["mutations.c", "<global>", 1, "unsigned counter = 0;"]),
        serde_json::json!([
            "mutations.c",
            "<global>",
            2,
            "void f(void ) { read(counter); } counter++;"
        ]),
        serde_json::json!([
            "mutations.c",
            "STEP",
            3,
            "#define g(void ) { read(counter); #define STEP ++counter"
        ]),
        serde_json::json!([
            "mutations.c",
            "h",
            6,
            "counter; sizeof(counter++); ++counter;"
        ]),
    ];
    for caseless in [false, true] {
        let name = if caseless { "COUNTER" } else { "counter" };
        let mut args = vec!["--json", "-dL", "-f", "i.db", "-9", name];
        if caseless {
            args.push("-C");
        }
        let answer = ripscope(work.path(), &args, None);
        assert!(
            answer.status.success() && answer.stderr.is_empty(),
            "{answer:?}"
        );
        assert_eq!(json_frames(&answer.stdout), expected_frames);
        let mut args = vec!["--json", "-dl", "-f", "i.db"];
        if caseless {
            args.push("-C");
        }
        let commands = format!("9{name}\n9index\nq\n");
        let session = ripscope(work.path(), &args, Some(commands.as_bytes()));
        assert!(
            session.status.success() && session.stderr.is_empty(),
            "{session:?}"
        );
        let mut expected_session = vec![
            serde_json::json!({"type":"ready"}),
            serde_json::json!({"type":"result","count":4}),
        ];
        expected_session.extend(expected_frames.iter().cloned());
        expected_session.extend([
            serde_json::json!({"type":"ready"}),
            serde_json::json!({"type":"result","count":0}),
            serde_json::json!({"type":"ready"}),
        ]);
        assert_eq!(json_frames(&session.stdout), expected_session);
        for semantics in [false, true] {
            let mut args = vec!["--store-query", "-f", "i.db"];
            if semantics {
                args.push("--own-semantics");
            }
            if caseless {
                args.push("-C");
            }
            let query = format!("9{name}\n");
            let answer = ripscope(work.path(), &args, Some(query.as_bytes()));
            assert!(
                answer.status.success() && answer.stderr.is_empty(),
                "{answer:?}"
            );
            assert_eq!(
                String::from_utf8_lossy(&answer.stdout),
                format!("{expected}##units-query## 4\n")
            );
        }
    }
    let baseline = ripscope(
        work.path(),
        &["--store-query", "-f", "baseline.db", "--own-semantics"],
        Some(b"9counter\n"),
    );
    assert!(
        baseline.status.success() && baseline.stderr.is_empty(),
        "{baseline:?}"
    );
    assert_eq!(
        baseline.stdout,
        b"mutations.c <global> 1 unsigned counter = 0;\n##units-query## 1\n"
    );
}

#[test]
fn opt_in_return_type_wrapper_reading_survives_source_removal() {
    let work = tempfile::tempdir().expect("work");
    let source = work.path().join("wrappers.c");
    std::fs::write(
        &source,
        include_str!("fixtures/scanner/probes/70-return-type-wrappers.c"),
    )
    .expect("source");
    let built = ripscope(
        work.path(),
        &[
            "--store-build",
            "--corrections",
            "enabled",
            "-k",
            "-f",
            "i.db",
            "wrappers.c",
        ],
        None,
    );
    assert!(built.status.success(), "{built:?}");
    assert_stored_queries(
        work.path(),
        &[
            (
                "1",
                "API",
                "wrappers.c API 1 #define API(type) type\n\
                 wrappers.c API 3 API(int ) wrapped(void ) { return helper(); }\n\
                 wrappers.c API 5 int API(int value) { return value; }\n",
            ),
            (
                "1",
                "gnu_real",
                "wrappers.c gnu_real 29 gnu_real(int ) GNU_EMPTY(void ) { return 1; }\n",
            ),
        ],
    );
    let candidate = ripscope(
        work.path(),
        &[
            "--store-build",
            "--corrections",
            "enabled,return-type-wrappers",
            "-k",
            "-f",
            "i.db",
            "wrappers.c",
        ],
        None,
    );
    assert!(candidate.status.success(), "{candidate:?}");
    let queries = [
        (
            "1",
            "API",
            "wrappers.c API 1 #define API(type) type\n\
             wrappers.c API 5 int API(int value) { return value; }\n",
        ),
        (
            "1",
            "wrapped",
            "wrappers.c wrapped 3 API(int ) wrapped(void ) { return helper(); }\n",
        ),
        (
            "2",
            "wrapped",
            "wrappers.c helper 3 API(int ) wrapped(void ) { return helper(); }\n",
        ),
        (
            "3",
            "API",
            "wrappers.c call_api 6 int call_api(void ) { return API(3); }\n",
        ),
        (
            "1",
            "pointer_wrapped",
            "wrappers.c pointer_wrapped 8 RETTYPE(const char *) pointer_wrapped(void ) { return 0; }\n",
        ),
        (
            "1",
            "opaque_pointer_wrapped",
            "wrappers.c opaque_pointer_wrapped 9 RETTYPE(Node *) opaque_pointer_wrapped(int value) { return helper(); }\n",
        ),
        (
            "1",
            "nested_return",
            "wrappers.c nested_return 14 RETTYPE(int ) nested_return(void (*cb)(int )) { cb(1); }\n",
        ),
        (
            "2",
            "nested_return",
            "wrappers.c cb 14 RETTYPE(int ) nested_return(void (*cb)(int )) { cb(1); }\n",
        ),
        (
            "1",
            "retained_inner",
            "wrappers.c retained_inner 19 int WRAP(int retained_inner(int value)) ATTR { return value; }\n",
        ),
        ("1", "after", "wrappers.c after 24 int after;\n"),
        (
            "1",
            "implicit_api",
            "wrappers.c implicit_api 26 implicit_api(void ) EMPTY(void ) { return 1; }\n",
        ),
        (
            "3",
            "implicit_api",
            "wrappers.c implicit_call 27 int implicit_call(void ) { return implicit_api(); }\n",
        ),
        ("1", "gnu_real", ""),
        (
            "3",
            "gnu_real",
            "wrappers.c gnu_call 30 int gnu_call(void ) { return gnu_real(2); }\n",
        ),
    ];
    assert_stored_queries(work.path(), &queries);
    std::fs::remove_file(source).expect("remove source");
    assert_stored_queries(work.path(), &queries);
}

#[test]
fn generated_macro_heads_keep_real_calls_after_source_removal() {
    let work = tempfile::tempdir().expect("work");
    std::fs::write(
        work.path().join("generated.c"),
        include_str!("fixtures/scanner/probes/69-macro-function-heads.c"),
    )
    .expect("source");
    let built = ripscope(work.path(), &["-bk", "-f", "i.db", "generated.c"], None);
    assert!(
        built.status.success() && built.stderr.is_empty(),
        "{built:?}"
    );
    let queries = [
        (
            "2",
            "GENERATED_FUNCTION",
            "generated.c helper 3 static int NAME(void ) { return helper(); }\n",
        ),
        (
            "3",
            "NAME",
            "generated.c INVOKE_PARAMETER 4 #define INVOKE_PARAMETER(NAME) NAME()\n\
             generated.c GENERATED_PROTOTYPE 19 #define GENERATED_PROTOTYPE(NAME) static int NAME(void );\n\
             generated.c INVOKE_BLOCK 20 #define INVOKE_BLOCK(NAME) NAME(1) { helper(); }\n\
             generated.c LOCAL_CALLS 21 #define LOCAL_CALLS(NAME) do { int value = NAME(); helper(); } while (0)\n\
             generated.c ANNOTATED_HEAD 22 #define ANNOTATED_HEAD(NAME) static int NAME(void ) SUFFIX { return helper(); }\n\
             generated.c SPLIT_PREFIX 24 NAME(void ) { return helper(); }\n\
             generated.c UNFINISHED_HEAD 25 #define UNFINISHED_HEAD(NAME) static int NAME(void )\n",
        ),
        (
            "2",
            "LONG_HEAD",
            "generated.c helper 12 int length) { helper(); }\n",
        ),
        (
            "2",
            "COMMENT_HEAD",
            "generated.c helper 13 #define COMMENT_HEAD(NAME) static int NAME (void )  { return helper(); }\n",
        ),
        (
            "2",
            "SPLICE_HEAD",
            "generated.c helper 16 { return helper(); }\n",
        ),
        (
            "2",
            "CALLBACK_HEAD",
            "generated.c callback 18 static int NAME(int (*callback)(void ), int value) { return callback() + helper(); }\n\
             generated.c helper 18 static int NAME(int (*callback)(void ), int value) { return callback() + helper(); }\n",
        ),
        (
            "3",
            "callback",
            "generated.c CALLBACK_HEAD 18 static int NAME(int (*callback)(void ), int value) { return callback() + helper(); }\n",
        ),
        ("1", "NAME", ""),
        (
            "1",
            "ordinary",
            "generated.c ordinary 5 int ordinary(void ) { return helper(); }\n",
        ),
        ("1", "prototype", ""),
        (
            "1",
            "after_macros",
            "generated.c after_macros 27 int after_macros(void ) { return helper(); }\n",
        ),
    ];
    for removed in [false, true] {
        if removed {
            std::fs::remove_file(work.path().join("generated.c")).expect("remove source");
        }
        assert_stored_queries(work.path(), &queries);
    }
}

#[test]
fn atomic_object_reduction_is_defined_after_source_removal() {
    let work = tempfile::tempdir().expect("work");
    std::fs::write(
        work.path().join("atomic.c"),
        include_str!("fixtures/navigation/reductions/atomic.c"),
    )
    .expect("source");
    let built = ripscope(work.path(), &["-bk", "-f", "i.db", "atomic.c"], None);
    assert!(
        built.status.success() && built.stderr.is_empty(),
        "{built:?}"
    );
    let queries = [
        (
            "1",
            "atomic_probe",
            "atomic.c atomic_probe 1 _Atomic(int ) atomic_probe;\n",
        ),
        (
            "1",
            "qualified_probe",
            "atomic.c qualified_probe 2 _Atomic int qualified_probe;\n",
        ),
        (
            "1",
            "after_atomic_probe",
            "atomic.c after_atomic_probe 3 int after_atomic_probe;\n",
        ),
        ("1", "_Atomic", ""),
        (
            "0",
            "atomic_probe",
            "atomic.c <global> 1 _Atomic(int ) atomic_probe;\n",
        ),
    ];
    for removed in [false, true] {
        if removed {
            std::fs::remove_file(work.path().join("atomic.c")).expect("remove source");
        }
        assert_stored_queries(work.path(), &queries);
    }
}

#[test]
fn atomic_types_preserve_declaration_controls_and_physical_lines() {
    let work = tempfile::tempdir().expect("work");
    std::fs::write(
        work.path().join("atomic.c"),
        include_str!("fixtures/scanner/probes/67-atomic-types.c"),
    )
    .expect("source");
    let built = ripscope(work.path(), &["-bk", "-f", "i.db", "atomic.c"], None);
    assert!(
        built.status.success() && built.stderr.is_empty(),
        "{built:?}"
    );
    let queries = [
        (
            "1",
            "plain_atomic",
            "atomic.c plain_atomic 4 _Atomic(int ) plain_atomic;\n",
        ),
        (
            "1",
            "initialized_atomic",
            "atomic.c initialized_atomic 5 _Atomic(unsigned long ) initialized_atomic = 2;\n",
        ),
        (
            "1",
            "named_atomic",
            "atomic.c named_atomic 6 static _Atomic(scalar_type) named_atomic;\n",
        ),
        (
            "1",
            "pointer_atomic",
            "atomic.c pointer_atomic 7 _Atomic(int *) pointer_atomic;\n",
        ),
        (
            "1",
            "aggregate_atomic",
            "atomic.c aggregate_atomic 8 _Atomic(struct payload) aggregate_atomic;\n",
        ),
        (
            "1",
            "enum_atomic",
            "atomic.c enum_atomic 9 _Atomic(enum status) enum_atomic;\n",
        ),
        (
            "1",
            "split_atomic",
            "atomic.c split_atomic 11 split_atomic;\n",
        ),
        (
            "1",
            "multiline_atomic",
            "atomic.c multiline_atomic 14 ) multiline_atomic;\n",
        ),
        (
            "1",
            "qualified_atomic",
            "atomic.c qualified_atomic 15 _Atomic int qualified_atomic;\n",
        ),
        (
            "1",
            "atomic_alias",
            "atomic.c atomic_alias 16 typedef _Atomic(int ) atomic_alias;\n",
        ),
        (
            "1",
            "named_atomic_alias",
            "atomic.c named_atomic_alias 17 typedef _Atomic(scalar_type) named_atomic_alias;\n",
        ),
        (
            "1",
            "ordinary_after_alias",
            "atomic.c ordinary_after_alias 18 int ordinary_after_alias;\n",
        ),
        (
            "1",
            "after_extern_atomic",
            "atomic.c after_extern_atomic 20 int after_extern_atomic;\n",
        ),
        (
            "1",
            "after_factory",
            "atomic.c after_factory 22 int after_factory;\n",
        ),
        (
            "1",
            "function",
            "atomic.c function 23 int function(_Atomic(int ) parameter) {\n",
        ),
        (
            "1",
            "after_function",
            "atomic.c after_function 28 int after_function;\n",
        ),
        (
            "1",
            "member_atomic",
            "atomic.c member_atomic 30 _Atomic(int ) member_atomic;\n",
        ),
        (
            "1",
            "named_member",
            "atomic.c named_member 31 _Atomic(scalar_type) named_member;\n",
        ),
        (
            "1",
            "after_member",
            "atomic.c after_member 32 int after_member;\n",
        ),
        (
            "1",
            "comma_one",
            "atomic.c comma_one 34 _Atomic(int ) comma_one, comma_two = 1;\n",
        ),
        (
            "1",
            "comma_two",
            "atomic.c comma_two 34 _Atomic(int ) comma_one, comma_two = 1;\n",
        ),
        (
            "1",
            "initializer_atomic",
            "atomic.c initializer_atomic 35 _Atomic(int ) initializer_atomic = sizeof(_Atomic(int ));\n",
        ),
        (
            "1",
            "after_initializer",
            "atomic.c after_initializer 36 int after_initializer;\n",
        ),
        (
            "1",
            "scalar_type",
            "atomic.c scalar_type 1 typedef int scalar_type;\n",
        ),
        ("1", "_Atomic", ""),
        ("1", "extern_atomic", ""),
        ("1", "factory", ""),
        ("1", "parameter", ""),
        ("1", "local_atomic", ""),
        ("1", "local_static_atomic", ""),
        (
            "0",
            "plain_atomic",
            "atomic.c <global> 4 _Atomic(int ) plain_atomic;\n",
        ),
        (
            "0",
            "local_atomic",
            "atomic.c function 24 _Atomic(int ) local_atomic;\n",
        ),
    ];
    for removed in [false, true] {
        if removed {
            std::fs::remove_file(work.path().join("atomic.c")).expect("remove source");
        }
        assert_stored_queries(work.path(), &queries);
    }
}

#[test]
fn c11_keyword_reductions_keep_only_written_calls_after_source_removal() {
    for (file, source, queries) in [
        (
            "generic.c",
            include_str!("fixtures/navigation/reductions/generic.c"),
            vec![
                ("3", "_Generic", ""),
                (
                    "2",
                    "generic_probe",
                    "generic.c generic_target 3 return _Generic(1, int : generic_target(), default: 0);\n",
                ),
                (
                    "3",
                    "generic_target",
                    "generic.c generic_probe 3 return _Generic(1, int : generic_target(), default: 0);\ngeneric.c after_generic_probe 5 int after_generic_probe(void ) { return generic_target(); }\n",
                ),
                (
                    "2",
                    "after_generic_probe",
                    "generic.c generic_target 5 int after_generic_probe(void ) { return generic_target(); }\n",
                ),
            ],
        ),
        (
            "alignment.c",
            include_str!("fixtures/navigation/reductions/alignment.c"),
            vec![
                ("3", "_Alignof", ""),
                ("2", "alignment_probe", ""),
                ("2", "alignment_neighbor", ""),
            ],
        ),
    ] {
        let work = tempfile::tempdir().expect("work");
        std::fs::write(work.path().join(file), source).expect("source");
        let built = ripscope(work.path(), &["-bk", "-f", "i.db", file], None);
        assert!(
            built.status.success() && built.stderr.is_empty(),
            "{built:?}"
        );
        for removed in [false, true] {
            if removed {
                std::fs::remove_file(work.path().join(file)).expect("remove source");
            }
            assert_stored_queries(work.path(), &queries);
        }
    }
}

#[test]
fn conditional_defined_operator_keeps_real_functions_after_source_removal() {
    let work = tempfile::tempdir().expect("work");
    std::fs::write(
        work.path().join("defined.c"),
        "int helper(void) { return 1; }\n\
         int conditional(void) {\n\
         #if defined(OPTION)\n\
             return helper();\n\
         #else\n\
             return 0;\n\
         #endif\n\
         }\n\
         int defined(int value) { return value; }\n\
         int call_defined(void) { return defined(2); }\n",
    )
    .expect("source");
    let built = ripscope(work.path(), &["-bk", "-f", "i.db", "defined.c"], None);
    assert!(
        built.status.success() && built.stderr.is_empty(),
        "{built:?}"
    );
    let queries = [
        ("2", "conditional", "defined.c helper 4 return helper();\n"),
        (
            "3",
            "defined",
            "defined.c call_defined 10 int call_defined(void ) { return defined(2); }\n",
        ),
        (
            "1",
            "defined",
            "defined.c defined 9 int defined(int value) { return value; }\n",
        ),
        (
            "2",
            "call_defined",
            "defined.c defined 10 int call_defined(void ) { return defined(2); }\n",
        ),
    ];
    for removed in [false, true] {
        if removed {
            std::fs::remove_file(work.path().join("defined.c")).expect("remove source");
        }
        assert_stored_queries(work.path(), &queries);
    }
}

#[test]
fn conditional_defined_preserves_branches_nested_calls_and_neighboring_uses() {
    let work = tempfile::tempdir().expect("work");
    std::fs::write(
        work.path().join("calls.c"),
        include_str!("fixtures/scanner/probes/68-preprocessor-defined.c"),
    )
    .expect("source");
    let built = ripscope(work.path(), &["-bk", "-f", "i.db", "calls.c"], None);
    assert!(
        built.status.success() && built.stderr.is_empty(),
        "{built:?}"
    );
    let queries = [
        (
            "2",
            "conditional",
            "calls.c CHECK 6 #if defined(OPTION) && CHECK(nested())\ncalls.c nested 6 #if defined(OPTION) && CHECK(nested())\ncalls.c helper 7 return helper();\ncalls.c alternate 9 return alternate(inside());\ncalls.c inside 9 return alternate(inside());\ncalls.c fallback 11 return fallback();\n",
        ),
        (
            "2",
            "continued",
            "calls.c CHECK_SPLIT 16 defined (SECOND) && CHECK_SPLIT(inner_split())\ncalls.c inner_split 16 defined (SECOND) && CHECK_SPLIT(inner_split())\ncalls.c first_arm 17 first_arm();\ncalls.c second_arm 19 second_arm();\ncalls.c defined 21 return defined(3);\n",
        ),
        (
            "2",
            "unspaced",
            "calls.c unspaced_arm 25 unspaced_arm();\ncalls.c comment_arm 27 comment_arm();\ncalls.c defined_neighbor 29 return defined_neighbor();\n",
        ),
        (
            "2",
            "neighboring",
            "calls.c defined_neighbor 32 defined_neighbor();\ncalls.c use 33 use(defined(4));\ncalls.c defined 33 use(defined(4));\ncalls.c defined 34 #pragma retain defined(5)\ncalls.c true_branch 36 true_branch();\ncalls.c false_branch 38 false_branch();\n",
        ),
        (
            "3",
            "defined",
            "calls.c MACRO_DEFINED 1 #define MACRO_DEFINED(x) defined(x)\ncalls.c call_defined 4 int call_defined(void ) { return defined(2); }\ncalls.c continued 21 return defined(3);\ncalls.c neighboring 33 use(defined(4));\ncalls.c neighboring 34 #pragma retain defined(5)\n",
        ),
        (
            "1",
            "defined",
            "calls.c defined 2 int defined(int value) { return value; }\n",
        ),
        (
            "3",
            "inner_split",
            "calls.c continued 16 defined (SECOND) && CHECK_SPLIT(inner_split())\n",
        ),
        (
            "3",
            "nested",
            "calls.c conditional 6 #if defined(OPTION) && CHECK(nested())\n",
        ),
    ];
    for removed in [false, true] {
        if removed {
            std::fs::remove_file(work.path().join("calls.c")).expect("remove source");
        }
        assert_stored_queries(work.path(), &queries);
    }
}

#[test]
fn c11_expression_keywords_preserve_nested_associations_operands_and_neighbors() {
    let work = tempfile::tempdir().expect("work");
    std::fs::write(
        work.path().join("calls.c"),
        include_str!("fixtures/scanner/probes/66-c11-expression-keywords.c"),
    )
    .expect("source");
    let built = ripscope(work.path(), &["-bk", "-f", "i.db", "calls.c"], None);
    assert!(
        built.status.success() && built.stderr.is_empty(),
        "{built:?}"
    );
    let queries = [
        (
            "2",
            "expressions",
            "calls.c before 4 before();\ncalls.c control 5 _Generic(control(),\ncalls.c chosen 6 int : chosen(),\ncalls.c alternate 7 default: _Generic(x, int : alternate(), default: fallback()));\ncalls.c fallback 7 default: _Generic(x, int : alternate(), default: fallback()));\ncalls.c after 8 after();\n",
        ),
        (
            "2",
            "alignment",
            "calls.c use 11 use(_Alignof(int ));\ncalls.c use 12 use(_Alignof(int [bound()]));\ncalls.c bound 12 use(_Alignof(int [bound()]));\n",
        ),
        ("2", "empty", ""),
        (
            "2",
            "conditional",
            "calls.c arm_one 22 _Generic(0, int : arm_one(), default: 0);\ncalls.c arm_two 24 _Alignof(int [arm_two()]);\ncalls.c neighbor 26 neighbor();\n",
        ),
        (
            "2",
            "attributes",
            "calls.c attribute_bound 29 __attribute__((aligned(_Alignof(int [attribute_bound()])))) int object;\ncalls.c attr_one 30 __attribute__((annotate(_Generic(0, int : attr_one(), default: attr_two())))) int other;\ncalls.c attr_two 30 __attribute__((annotate(_Generic(0, int : attr_one(), default: attr_two())))) int other;\n",
        ),
        (
            "2",
            "spellings",
            "calls.c _Generic_helper 33 _Generic_helper(); _Alignof_helper();\ncalls.c _Alignof_helper 33 _Generic_helper(); _Alignof_helper();\ncalls.c gap_call 34 _Generic (0, int : gap_call(), default: 0);\ncalls.c split_bound 36 (int [split_bound()]);\n",
        ),
        ("3", "_Generic", ""),
        ("3", "_Alignof", ""),
        (
            "3",
            "control",
            "calls.c expressions 5 _Generic(control(),\n",
        ),
        ("3", "chosen", "calls.c expressions 6 int : chosen(),\n"),
        (
            "3",
            "alternate",
            "calls.c expressions 7 default: _Generic(x, int : alternate(), default: fallback()));\n",
        ),
        (
            "3",
            "fallback",
            "calls.c expressions 7 default: _Generic(x, int : alternate(), default: fallback()));\n",
        ),
        (
            "3",
            "bound",
            "calls.c alignment 12 use(_Alignof(int [bound()]));\n",
        ),
        (
            "3",
            "arm_one",
            "calls.c conditional 22 _Generic(0, int : arm_one(), default: 0);\n",
        ),
        (
            "3",
            "arm_two",
            "calls.c conditional 24 _Alignof(int [arm_two()]);\n",
        ),
        (
            "3",
            "attribute_bound",
            "calls.c attributes 29 __attribute__((aligned(_Alignof(int [attribute_bound()])))) int object;\n",
        ),
        (
            "3",
            "attr_one",
            "calls.c attributes 30 __attribute__((annotate(_Generic(0, int : attr_one(), default: attr_two())))) int other;\n",
        ),
        (
            "3",
            "attr_two",
            "calls.c attributes 30 __attribute__((annotate(_Generic(0, int : attr_one(), default: attr_two())))) int other;\n",
        ),
        (
            "3",
            "macro_one",
            "calls.c CHOOSE 1 #define CHOOSE(x) _Generic((x), int : macro_one(), default: macro_two())\n",
        ),
        (
            "3",
            "macro_two",
            "calls.c CHOOSE 1 #define CHOOSE(x) _Generic((x), int : macro_one(), default: macro_two())\n",
        ),
    ];
    for removed in [false, true] {
        if removed {
            std::fs::remove_file(work.path().join("calls.c")).expect("remove source");
        }
        assert_stored_queries(work.path(), &queries);
    }
}

#[test]
fn dereferenced_callback_reduction_survives_source_removal() {
    let work = tempfile::tempdir().expect("work");
    std::fs::write(
        work.path().join("dereferenced.c"),
        include_str!("fixtures/navigation/reductions/dereferenced.c"),
    )
    .expect("source");
    let built = ripscope(work.path(), &["-bk", "-f", "i.db", "dereferenced.c"], None);
    assert!(built.status.success(), "{built:?}");
    for removed in [false, true] {
        if removed {
            std::fs::remove_file(work.path().join("dereferenced.c")).expect("remove source");
        }
        for (field, name, expected) in [
            (
                "-2",
                "dereferenced_probe",
                "dereferenced.c pointer_callback 2 int dereferenced_probe(void ) { return (*pointer_callback)(1); }\n",
            ),
            (
                "-3",
                "pointer_callback",
                "dereferenced.c dereferenced_probe 2 int dereferenced_probe(void ) { return (*pointer_callback)(1); }\n\
                 dereferenced.c direct_probe 3 int direct_probe(void ) { return pointer_callback(1); }\n",
            ),
        ] {
            let answer = ripscope(work.path(), &["-dL", "-f", "i.db", field, name], None);
            assert!(
                answer.status.success() && answer.stderr.is_empty(),
                "{answer:?}"
            );
            assert_eq!(answer.stdout, expected.as_bytes(), "removed={removed}");
        }
        let session = ripscope(
            work.path(),
            &["-dl", "-f", "i.db"],
            Some(b"2dereferenced_probe\n3pointer_callback\nq\n"),
        );
        assert!(
            session.status.success() && session.stderr.is_empty(),
            "{session:?}"
        );
        assert_eq!(
            session.stdout,
            b">> cscope: 1 lines\ndereferenced.c pointer_callback 2 int dereferenced_probe(void ) { return (*pointer_callback)(1); }\n\
              >> cscope: 2 lines\ndereferenced.c dereferenced_probe 2 int dereferenced_probe(void ) { return (*pointer_callback)(1); }\n\
              dereferenced.c direct_probe 3 int direct_probe(void ) { return pointer_callback(1); }\n>> "
        );
    }
}

#[test]
fn dereferenced_calls_keep_neighboring_calls_and_declaration_controls() {
    let work = tempfile::tempdir().expect("work");
    std::fs::write(
        work.path().join("calls.c"),
        include_str!("fixtures/scanner/probes/65-dereferenced-calls.c"),
    )
    .expect("source");
    let built = ripscope(work.path(), &["-bk", "-f", "i.db", "calls.c"], None);
    assert!(built.status.success(), "{built:?}");
    std::fs::remove_file(work.path().join("calls.c")).expect("remove source");
    let expected = [
        "calls.c direct_probe 7 direct_probe(); parameter(1); ops->member(2);",
        "calls.c parameter 7 direct_probe(); parameter(1); ops->member(2);",
        "calls.c member 7 direct_probe(); parameter(1); ops->member(2);",
        "calls.c parameter 8 (*parameter)(3); (*parameter)(4); parameter(5);",
        // The existing reading calls the opaque return type; the declarator stays plain.
        "calls.c T 11 T (*typed_local)(Arg);",
        "calls.c direct_probe 14 direct_probe();",
        "calls.c parameter 16 * parameter",
        "calls.c neighbor 19 neighbor((*parameter)(7));",
        "calls.c parameter 19 neighbor((*parameter)(7));",
        "calls.c parameter 20 if (flag) (*parameter)(8);",
        "calls.c parameter 21 else (*parameter)(9);",
    ];
    let answer = ripscope(work.path(), &["-dL", "-f", "i.db", "-2", "caller"], None);
    assert!(
        answer.status.success() && answer.stderr.is_empty(),
        "{answer:?}"
    );
    assert_eq!(
        String::from_utf8_lossy(&answer.stdout),
        format!("{}\n", expected.join("\n"))
    );
    let callers = ripscope(work.path(), &["-dL", "-f", "i.db", "-3", "parameter"], None);
    assert!(
        callers.status.success() && callers.stderr.is_empty(),
        "{callers:?}"
    );
    assert_eq!(
        String::from_utf8_lossy(&callers.stdout),
        "calls.c caller 7 direct_probe(); parameter(1); ops->member(2);\n\
         calls.c caller 8 (*parameter)(3); (*parameter)(4); parameter(5);\n\
         calls.c caller 16 * parameter\n\
         calls.c caller 19 neighbor((*parameter)(7));\n\
         calls.c caller 20 if (flag) (*parameter)(8);\n\
         calls.c caller 21 else (*parameter)(9);\n"
    );
    let controls = ripscope(
        work.path(),
        &[
            "-dL",
            "-f",
            "i.db",
            "-3",
            "local|pointer_return|typed_local|array|other|fake|comment_only",
        ],
        None,
    );
    assert!(
        controls.status.success() && controls.stderr.is_empty(),
        "{controls:?}"
    );
    assert!(controls.stdout.is_empty(), "{controls:?}");
    let lists = ripscope(
        work.path(),
        &["-dL", "-f", "i.db", "-2", "declaration_lists"],
        None,
    );
    assert!(
        lists.status.success() && lists.stderr.is_empty(),
        "{lists:?}"
    );
    assert_eq!(
        String::from_utf8_lossy(&lists.stdout),
        "calls.c T 31 T (*third)(Arg), (*fourth)(Arg);\n\
         calls.c factory 32 int (*initialized)(int ) = (*factory)(1), (*last)(int );\n\
         calls.c expression_callback 33 (void )0, (*expression_callback)(1);\n\
         calls.c factory_result 34 factory_result()(*not_called)(1);\n"
    );
    for (name, expected) in [
        (
            "declaration_controls",
            "calls.c factory 44 int variable = (*factory)((*nested_value)(1)), (*last_value)(int );\n\
            calls.c nested_value 44 int variable = (*factory)((*nested_value)(1)), (*last_value)(int );\n\
            calls.c discarded 46 (void )(*discarded)(1);\n\
            calls.c casted 47 (int )(*casted)(1);\n\
            calls.c controlled 48 if (flag) (void )(*controlled)(1);\n\
            calls.c outer 49 (*outer)((*nested)(2));\n\
            calls.c nested 49 (*outer)((*nested)(2));\n",
        ),
        (
            "conditional_expression",
            "calls.c selected 51 void conditional_expression(void ) { return flag ? (*selected)(1) : (*other_selected)(2); }\n\
            calls.c other_selected 51 void conditional_expression(void ) { return flag ? (*selected)(1) : (*other_selected)(2); }\n",
        ),
        (
            "directives",
            "calls.c arm_one 54 (*arm_one)(1);\n\
            calls.c arm_two 56 (*arm_two)(2);\n\
            calls.c after_conditional 58 (*after_conditional)(3);\n\
            calls.c after_define 60 (*after_define)(4);\n",
        ),
        (
            "expression_controls",
            "calls.c plain_call 69 int scalar = plain_call(1, (*initializer_cb)(2));\n\
            calls.c initializer_cb 69 int scalar = plain_call(1, (*initializer_cb)(2));\n\
            calls.c factory 70 int (*callback)(int ) = (*factory)(1, (*nested_initializer_cb)(2));\n\
            calls.c nested_initializer_cb 70 int (*callback)(int ) = (*factory)(1, (*nested_initializer_cb)(2));\n\
            calls.c nested_bound_cb 71 int nested_array[indices[0] + (*nested_bound_cb)(1)];\n\
            calls.c condition_callback 72 if ((int )(*condition_callback)(1)) (*body_callback)(2);\n\
            calls.c body_callback 72 if ((int )(*condition_callback)(1)) (*body_callback)(2);\n\
            calls.c loop_condition 73 while ((int )(*loop_condition)(1)) (*loop_body)(2);\n\
            calls.c loop_body 73 while ((int )(*loop_condition)(1)) (*loop_body)(2);\n",
        ),
        (
            "designated_initializers",
            "calls.c element_cb 76 int array[2] = { [0] = (*element_cb)(1) }, (*after_array)(int );\n\
            calls.c field_cb 77 struct S object = { .x = (*field_cb)(1), .y = (*other_field_cb)(2) }, (*after_struct)(int );\n\
            calls.c other_field_cb 77 struct S object = { .x = (*field_cb)(1), .y = (*other_field_cb)(2) }, (*after_struct)(int );\n",
        ),
        (
            "aggregate_declarations",
            "calls.c aggregate_cb 81 struct { int x; } initialized = { .x = (*aggregate_cb)(1) }, (*after_anonymous_init)(int );\n\
            calls.c _Alignas 87 _Alignas(16) int aligned, (*after_alignas)(int );\n",
        ),
        (
            "optional_control",
            "calls.c optional_cb 94 (*optional_cb)(1);\n",
        ),
        (
            "optional_else",
            "calls.c optional_cb 101 (*optional_cb)(2);\n",
        ),
        (
            "for_declaration",
            "calls.c for_condition 104 for (int i = 0, (*for_pointer)(int ); i < (*for_condition)(1); i += (*for_step)(1)) {\n\
            calls.c for_step 104 for (int i = 0, (*for_pointer)(int ); i < (*for_condition)(1); i += (*for_step)(1)) {\n\
            calls.c for_body 105 (*for_body)(i);\n",
        ),
        (
            "for_typedef_declaration",
            "calls.c typed_condition 110 for (Count i = 0, (*typedef_pointer)(int ); (*typed_condition)(i); (*typed_increment)(i)) {\n\
            calls.c typed_increment 110 for (Count i = 0, (*typedef_pointer)(int ); (*typed_condition)(i); (*typed_increment)(i)) {\n\
            calls.c typed_body 111 (*typed_body)(i);\n",
        ),
    ] {
        let answer = ripscope(work.path(), &["-dL", "-f", "i.db", "-2", name], None);
        assert!(
            answer.status.success() && answer.stderr.is_empty(),
            "{answer:?}"
        );
        assert_eq!(answer.stdout, expected.as_bytes(), "{name}");
    }
    let callers = ripscope(
        work.path(),
        &["-dL", "-f", "i.db", "-3", "optional_cb"],
        None,
    );
    assert_eq!(
        callers.stdout,
        b"calls.c optional_control 94 (*optional_cb)(1);\ncalls.c optional_else 101 (*optional_cb)(2);\n"
    );
}

#[cfg(windows)]
#[test]
fn windows_index_aliases_check_the_logs_sqlite_opens() {
    for suffix in ["-wal", "-shm"] {
        let work = indexed();
        let sentinel = work.path().join("sentinel");
        std::fs::write(&sentinel, "kept").expect("sentinel");
        std::fs::hard_link(&sentinel, work.path().join(format!("i.db{suffix}")))
            .expect("companion link");
        for mode in ["-dL", "-L"] {
            let refused = ripscope(work.path(), &[mode, "-f", "i.db.", "-1", "Alpha"], None);
            assert!(!refused.status.success(), "{suffix} {mode}: {refused:?}");
            assert_eq!(std::fs::read(&sentinel).expect("sentinel"), b"kept");
        }
    }
}

#[cfg(windows)]
#[test]
fn a_new_windows_index_uses_the_normalized_destination() {
    let work = tempfile::tempdir().expect("work");
    std::fs::write(work.path().join("a.c"), "int Alpha;\n").expect("source");
    let sentinel = work.path().join("sentinel");
    std::fs::write(&sentinel, "kept").expect("sentinel");
    // The index was deleted without its old log. Creating a new one removes the log,
    // without ever replaying or writing through it.
    std::fs::hard_link(&sentinel, work.path().join("i.db-wal")).expect("old log");
    let built = ripscope(work.path(), &["-bk", "-f", "i.db. ", "a.c"], None);
    assert!(built.status.success(), "{built:?}");
    assert!(built.stderr.is_empty(), "{built:?}");
    assert_eq!(std::fs::read(&sentinel).expect("sentinel"), b"kept");
    let stored = ripscope(work.path(), &["-dL", "-f", "i.db", "-1", "Alpha"], None);
    assert!(stored.status.success(), "{stored:?}");
    assert_eq!(stored.stdout, b"a.c Alpha 1 int Alpha;\n", "{stored:?}");
}

/// An index build that makes the index in a folder OneDrive synchronises, as the variables
/// that name OneDrive's folders say, warns that OneDrive may lock or copy it. Later index
/// builds, updates, queries and sessions there say nothing, until an index build makes it
/// afresh (ADR 0005).
#[cfg(windows)]
#[test]
fn an_index_made_in_a_onedrive_folder_is_built_with_one_warning() {
    let home = tempfile::tempdir().expect("home");
    let synced = home.path().join("OneDrive - Example");
    let project = synced.join("Documents").join("project");
    std::fs::create_dir_all(&project).expect("project");
    std::fs::create_dir(home.path().join("elsewhere")).expect("elsewhere");
    let source = project.join("a.c");
    std::fs::write(&source, "int Alpha;\n").expect("source");
    let root = synced.to_str().expect("root");
    let run = |args: &[&str], input: Option<&[u8]>| {
        let output = ripscope_with(&project, args, input, &[("OneDrive", root)]);
        assert!(output.status.success(), "{args:?}: {output:?}");
        output
    };
    let warning = |index: &str| {
        format!(
            "ripscope: OneDrive may lock or copy the index {index}, and -f can keep it elsewhere\n"
        )
    };
    let built = run(&["-bk", "-f", "i.db", "a.c"], None);
    assert_eq!(String::from_utf8_lossy(&built.stderr), warning("i.db"));

    std::fs::write(&source, "int Alpha;\nint Beta;\n").expect("edit");
    let updated = run(&["-L", "-f", "i.db", "-1", "Beta"], None);
    assert_eq!(updated.stdout, b"a.c Beta 2 int Beta;\n", "{updated:?}");
    std::fs::write(&source, "int Alpha;\nint Beta;\nint Gamma;\n").expect("edit");
    let session = run(&["-l", "-f", "i.db"], Some(b"1Gamma\nr\n1Gamma\nq\n"));
    let answer = String::from_utf8_lossy(&session.stdout);
    assert_eq!(
        answer.matches("a.c Gamma 3 int Gamma;\n").count(),
        2,
        "{session:?}"
    );
    for args in [
        &["-bk", "-f", "i.db", "a.c"][..],
        &["-bku", "-f", "i.db", "a.c"],
    ] {
        let again = run(args, None);
        assert!(again.stderr.is_empty(), "{args:?}: {again:?}");
    }
    let queried = run(&["-dL", "-f", "i.db", "-1", "Gamma"], None);
    assert!(queried.stderr.is_empty(), "{queried:?}");
    assert_eq!(queried.stdout, b"a.c Gamma 3 int Gamma;\n");

    // Made afresh over an index SQLite can no longer read, and in an empty file.
    let built = run(&["-bk", "-f", "d.db", "a.c"], None);
    assert_eq!(String::from_utf8_lossy(&built.stderr), warning("d.db"));
    let index = project.join("d.db");
    let mut bytes = std::fs::read(&index).expect("index");
    assert!(bytes.len() > 4 * 4096, "{}", bytes.len());
    bytes[2 * 4096..].fill(0xa5);
    std::fs::write(&index, &bytes).expect("damaged");
    let replaced = run(&["-bku", "-f", "d.db", "a.c"], None);
    assert_eq!(String::from_utf8_lossy(&replaced.stderr), warning("d.db"));
    std::fs::write(project.join("empty.db"), "").expect("empty file");
    let made = run(&["-bk", "-f", "empty.db", "a.c"], None);
    assert_eq!(String::from_utf8_lossy(&made.stderr), warning("empty.db"));

    // Outside OneDrive's folders an index build says nothing.
    let outside = run(&["-bk", "-f", "../../../elsewhere/i.db", "a.c"], None);
    assert!(outside.stderr.is_empty(), "{outside:?}");

    // A build that fails once it has made the index still warns, since the index stays
    // and the next build opens it as it is. Here its only source is locked against
    // reading, which opening it does not notice.
    let locked = project.join("locked.c");
    std::fs::write(&locked, "int Locked;\n").expect("locked source");
    let lock = std::fs::File::open(&locked).expect("open");
    lock.lock().expect("lock");
    let args = ["-bk", "-f", "l.db", "locked.c"];
    let failed = ripscope_with(&project, &args, None, &[("OneDrive", root)]);
    drop(lock);
    assert!(!failed.status.success(), "{failed:?}");
    let told = String::from_utf8_lossy(&failed.stderr);
    assert!(told.starts_with(&warning("l.db")), "{failed:?}");
    assert!(
        told.ends_with("ripscope: no source files found\n"),
        "{failed:?}"
    );
    assert!(project.join("l.db").exists());
}

#[cfg(windows)]
#[test]
fn a_long_index_path_supports_builds_refreshes_and_stored_queries() {
    for minimum_length in [270, 1100] {
        check_long_index_path(minimum_length);
    }
}

#[cfg(windows)]
fn check_long_index_path(minimum_length: usize) {
    let work = tempfile::tempdir().expect("work");
    let mut directory = work.path().to_owned();
    while directory.as_os_str().len() < minimum_length {
        directory.push("nested project directory");
    }
    std::fs::create_dir_all(&directory).expect("index directory");
    let index = directory.join("navigation.db");
    let name = index.to_str().expect("index path");
    assert!(
        !name.starts_with(r"\\?\"),
        "exercise an ordinary Windows path"
    );
    std::fs::write(work.path().join("a.c"), "int Alpha;\n").expect("source");
    let built = ripscope(work.path(), &["-bk", "-f", name, "a.c"], None);
    assert!(built.status.success(), "{built:?}");
    assert!(built.stderr.is_empty(), "{built:?}");

    std::fs::write(work.path().join("a.c"), "int Alpha;\nint Beta;\n").expect("edit");
    let relative = index.strip_prefix(work.path()).expect("relative index");
    let relative = relative.to_str().expect("index path").replace('\\', "/");
    let updated = ripscope(work.path(), &["-L", "-f", &relative, "-1", "Beta"], None);
    assert!(updated.status.success(), "{updated:?}");
    assert_eq!(updated.stdout, b"a.c Beta 2 int Beta;\n", "{updated:?}");
    assert!(updated.stderr.is_empty(), "{updated:?}");

    std::fs::remove_file(work.path().join("a.c")).expect("remove source");
    let stored = ripscope(work.path(), &["-dL", "-f", name, "-1", "Beta"], None);
    assert!(stored.status.success(), "{stored:?}");
    assert_eq!(stored.stdout, b"a.c Beta 2 int Beta;\n", "{stored:?}");
    assert!(stored.stderr.is_empty(), "{stored:?}");
}

#[test]
fn split_pointer_definitions_are_queryable_after_source_removal() {
    let work = tempfile::tempdir().expect("work");
    std::fs::write(
        work.path().join("callbacks.c"),
        "struct ops {\nT (*read)\n(struct buffer *p, int count);\n};\n\
         T (*handler)\n(int value);\n",
    )
    .expect("source");
    let built = ripscope(work.path(), &["-bk", "-f", "i.db", "callbacks.c"], None);
    assert!(built.status.success(), "{built:?}");
    std::fs::remove_file(work.path().join("callbacks.c")).expect("remove source");
    let definitions = ripscope(
        work.path(),
        &["-dL", "-f", "i.db", "-1", "read|handler"],
        None,
    );
    assert!(definitions.status.success(), "{definitions:?}");
    assert_eq!(
        definitions.stdout,
        b"callbacks.c read|handler 2 T (*read)\ncallbacks.c read|handler 5 T (*handler)\n"
    );
    let parameters = ripscope(
        work.path(),
        &["-dL", "-f", "i.db", "-1", "p|count|value"],
        None,
    );
    assert!(parameters.status.success(), "{parameters:?}");
    assert!(parameters.stdout.is_empty(), "{parameters:?}");
}

#[test]
fn decorated_pointer_definitions_are_queryable_after_source_removal() {
    let work = tempfile::tempdir().expect("work");
    std::fs::write(
        work.path().join("callbacks.c"),
        "struct ops {\n\
         HRESULT (STDMETHODCALLTYPE __RPC_FAR *QueryInterface)(IVim *self, REFID id);\n\
         };\nT (CALLBACK *handler)(int value);\n",
    )
    .expect("source");
    let built = ripscope(work.path(), &["-bk", "-f", "i.db", "callbacks.c"], None);
    assert!(built.status.success(), "{built:?}");
    std::fs::remove_file(work.path().join("callbacks.c")).expect("remove source");
    let definitions = ripscope(
        work.path(),
        &["-dL", "-f", "i.db", "-1", "QueryInterface|handler"],
        None,
    );
    assert!(definitions.status.success(), "{definitions:?}");
    assert_eq!(definitions.stdout,
        b"callbacks.c QueryInterface|handler 2 HRESULT (STDMETHODCALLTYPE __RPC_FAR *QueryInterface)(IVim *self, REFID id);\ncallbacks.c QueryInterface|handler 4 T (CALLBACK *handler)(int value);\n");
    let occurrences = ripscope(
        work.path(),
        &[
            "-dL",
            "-f",
            "i.db",
            "-1",
            "CALLBACK|STDMETHODCALLTYPE|__RPC_FAR|self|id|value",
        ],
        None,
    );
    assert!(occurrences.status.success(), "{occurrences:?}");
    assert!(occurrences.stdout.is_empty(), "{occurrences:?}");
}

#[test]
fn typedef_parameter_names_are_not_definitions_after_source_removal() {
    let work = tempfile::tempdir().expect("work");
    std::fs::write(
        work.path().join("types.c"),
        "typedef int callback(int count);\ntypedef void (*handler)(int code);\n\
         int count;\nint code;\n",
    )
    .expect("source");
    let built = ripscope(work.path(), &["-bk", "-f", "i.db", "types.c"], None);
    assert!(built.status.success(), "{built:?}");
    std::fs::remove_file(work.path().join("types.c")).expect("remove source");
    for (name, line) in [("count", 3), ("code", 4)] {
        let answer = ripscope(work.path(), &["-dL", "-f", "i.db", "-1", name], None);
        assert!(answer.status.success(), "{answer:?}");
        assert_eq!(
            answer.stdout,
            format!("types.c {name} {line} int {name};\n").as_bytes()
        );
    }
    let references = ripscope(work.path(), &["-dL", "-f", "i.db", "-0", "count"], None);
    assert!(references.status.success(), "{references:?}");
    assert_eq!(
        references.stdout,
        b"types.c <global> 1 typedef int callback(int count);\ntypes.c <global> 3 int count;\n"
    );
    let types = ripscope(
        work.path(),
        &["-dL", "-f", "i.db", "-1", "callback|handler"],
        None,
    );
    assert!(types.status.success(), "{types:?}");
    assert_eq!(types.stdout,
        b"types.c callback|handler 1 typedef int callback(int count);\ntypes.c callback|handler 2 typedef void (*handler)(int code);\n");
}

#[test]
fn attributed_entry_points_are_queryable_after_source_removal() {
    let work = tempfile::tempdir().expect("work");
    std::fs::write(
        work.path().join("entry.c"),
        include_bytes!("fixtures/scanner/probes/47-attributed-bodies.c"),
    )
    .expect("source");
    let built = ripscope(work.path(), &["-bk", "-f", "i.db", "entry.c"], None);
    assert!(built.status.success(), "{built:?}");
    std::fs::remove_file(work.path().join("entry.c")).expect("remove source");
    for (name, line, text) in [
        ("VimMain", 8, "VimMain"),
        ("main", 10, "main"),
        (
            "DllMain",
            16,
            "__declspec(dllexport) BOOL WINAPI DllMain(HINSTANCE inst, DWORD reason, LPVOID reserved) {",
        ),
    ] {
        let answer = ripscope(work.path(), &["-dL", "-f", "i.db", "-1", name], None);
        assert!(answer.status.success(), "{answer:?}");
        assert_eq!(
            answer.stdout,
            format!("entry.c {name} {line} {text}\n").as_bytes()
        );
    }
    let attribute = ripscope(
        work.path(),
        &["-dL", "-f", "i.db", "-1", "__declspec"],
        None,
    );
    assert!(attribute.status.success(), "{attribute:?}");
    assert!(attribute.stdout.is_empty(), "{attribute:?}");
    let callees = ripscope(work.path(), &["-dL", "-f", "i.db", "-2", "DllMain"], None);
    assert!(callees.status.success(), "{callees:?}");
    assert_eq!(callees.stdout, b"entry.c enter 17 enter(inst);\n");
}

/// Whether this process may create a file in `directory` (root may in any): a test of a
/// directory that cannot be written has nothing to check where it may.
#[cfg(unix)]
fn may_create_in(directory: &Path) -> bool {
    let probe = directory.join("probe");
    let made = std::fs::write(&probe, "").is_ok();
    let _ = std::fs::remove_file(&probe);
    made
}

fn probes() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/queries")
}

/// Latin-1 text from the fixtures, back to bytes.
fn bytes(text: &str) -> Vec<u8> {
    text.chars()
        .map(|c| u8::try_from(u32::from(c)).expect("byte"))
        .collect()
}

#[test]
fn every_reference_query_probe_is_answered_through_the_command_line() {
    probes_answered(&[]);
}

/// The probes again over indexes that store a block for each line (ADR 0011), as
/// `RIPSCOPE_BLOCK_SIZE` asks when an index is made.
#[test]
fn every_reference_query_probe_is_answered_over_blocks_of_a_line() {
    probes_answered(&[("RIPSCOPE_BLOCK_SIZE", "1")]);
}

/// Builds each reference query probe with `environment` set, and checks its answers.
fn probes_answered(environment: &[(&str, &str)]) {
    let mut failures = Vec::new();
    let mut probes: Vec<PathBuf> = std::fs::read_dir(probes())
        .expect("probes")
        .map(|entry| entry.expect("entry").path())
        .filter(|path| path.is_dir())
        .collect();
    probes.sort();
    for probe in probes {
        let expected: serde_json::Value =
            serde_json::from_slice(&std::fs::read(probe.join("expected.json")).expect("fixture"))
                .expect("json");
        let work = tempfile::tempdir().expect("work");
        let mut names = Vec::new();
        for name in expected["files"].as_array().expect("files") {
            let name = name.as_str().expect("name");
            std::fs::copy(probe.join(name), work.path().join(name)).expect("copy");
            names.push(name.to_owned());
        }
        let mut args = vec!["-bk", "-f", "index.db"];
        args.extend(names.iter().map(String::as_str));
        let built = ripscope_with(work.path(), &args, None, environment);
        assert!(built.status.success(), "{built:?}");
        let blocks: i64 = rusqlite::Connection::open(work.path().join("index.db"))
            .and_then(|db| {
                db.query_row(
                    "SELECT (SELECT value FROM meta WHERE key = 'block size')",
                    [],
                    |row| row.get(0),
                )
            })
            .expect("block size");
        let wanted = environment
            .first()
            .map_or(16_384, |(_, size)| size.parse().expect("size"));
        assert_eq!(blocks, wanted);
        for query in expected["queries"].as_array().expect("queries") {
            let field = query["field"].as_str().expect("field");
            let pattern = query["pattern"].as_str().expect("pattern");
            let option = format!("-{field}");
            let mut args = vec!["-dL", "-f", "index.db"];
            if query["caseless"].as_bool().expect("caseless") {
                args.push("-C");
            }
            args.extend([option.as_str(), pattern]);
            let answered = ripscope(work.path(), &args, None);
            let recorded: String = query["rows"]
                .as_array()
                .expect("rows")
                .iter()
                .map(|row| format!("{}\n", row.as_str().expect("row")))
                .collect();
            let key = format!(
                "{} {}{field}{pattern}",
                probe.file_name().expect("name").to_string_lossy(),
                if args.contains(&"-C") { "-C " } else { "" }
            );
            let want = bytes(&intended::expected("queries", &key, &recorded));
            if std::env::var_os("RIPSCOPE_RECORD_INTENDED").is_some() && environment.is_empty() {
                let actual: String = answered.stdout.iter().map(|&b| char::from(b)).collect();
                intended::check("queries", &key, &recorded, &actual);
                continue;
            }
            if !answered.status.success() || answered.stdout != want {
                failures.push(format!(
                    "{}: {} {field}{pattern}\n  want {:?}\n  got  {:?}\n  err  {}",
                    probe.display(),
                    if args.contains(&"-C") { "-C" } else { "" },
                    String::from_utf8_lossy(&want),
                    String::from_utf8_lossy(&answered.stdout),
                    String::from_utf8_lossy(&answered.stderr)
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Each query rule or labelled scanner Correction changes a recorded answer, so the
/// intended answers show every rule they cover.
#[test]
fn every_intended_query_rule_changes_a_recorded_answer() {
    let answers: serde_json::Value = serde_json::from_slice(
        &std::fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/intended-answers.json"),
        )
        .expect("intended answers"),
    )
    .expect("json");
    let mut seen = std::collections::BTreeSet::new();
    for entries in answers["fixtures"].as_object().expect("fixtures").values() {
        for entry in entries.as_object().expect("entries").values() {
            for change in entry["changes"].as_array().expect("changes") {
                if change["change"] == "order" {
                    continue;
                }
                let rules = change["rules"].as_array().expect("rules");
                assert!(!rules.is_empty(), "unexplained change: {change}");
                for rule in rules {
                    seen.extend(rule.as_str().expect("rule").split('+').map(str::to_owned));
                }
            }
        }
    }
    assert_eq!(
        seen.into_iter().collect::<Vec<_>>(),
        [
            "carry",
            "caseless-own-row",
            "consumption",
            "correction:dereferenced-calls",
            "correction:named-mutations",
            "correction:split-calls",
            "correction:subscript-assignments",
            "heads",
            "listing"
        ]
    );
}

#[test]
fn the_index_reads_with_the_enabled_corrections() {
    let work = tempfile::tempdir().expect("work");
    std::fs::write(
        work.path().join("a.c"),
        "struct s x = {
  .a = 1,
  .b = 2,
};
",
    )
    .expect("source");
    // The product's index reads with the enabled Corrections: with initializer-entries
    // a designated entry defines nothing.
    let built = ripscope(work.path(), &["-bk", "-f", "i.db", "a.c"], None);
    assert!(built.status.success(), "{built:?}");
    let approved = ripscope(work.path(), &["-dL", "-f", "i.db", "-1", "b"], None);
    assert!(approved.status.success(), "{approved:?}");
    assert_eq!(String::from_utf8_lossy(&approved.stdout), "");
    // The reference reading, cscope's, which `--store-build` keeps for the compass: the
    // entry is a global definition.
    let reference = Command::new(env!("CARGO_BIN_EXE_ripscope"))
        .current_dir(work.path())
        .args(["--store-build", "-f", "ref.db", "a.c"])
        .output()
        .expect("store build");
    assert!(reference.status.success(), "{reference:?}");
    let answered = ripscope(work.path(), &["-dL", "-f", "ref.db", "-1", "b"], None);
    assert_eq!(
        String::from_utf8_lossy(&answered.stdout),
        "a.c b 3 .b = 2,
"
    );
}

#[test]
fn a_session_answers_in_the_line_protocol() {
    let work = tempfile::tempdir().expect("work");
    std::fs::write(
        work.path().join("a.c"),
        "int Counter;\nvoid f(void) { Counter = 1; }\n",
    )
    .expect("source");
    let built = ripscope(work.path(), &["-bk", "-f", "i.db", "a.c"], None);
    assert!(built.status.success(), "{built:?}");
    let session = ripscope(
        work.path(),
        &["-dl", "-f", "i.db"],
        Some(b"0counter\nc\n0counter\n9Counter\nq\n"),
    );
    assert!(session.status.success(), "{session:?}");
    assert_eq!(
        String::from_utf8_lossy(&session.stdout),
        ">> Unable to search database\n>> >> cscope: 2 lines\n\
         a.c <global> 1 int Counter;\n\
         a.c f 2 void f(void ) { Counter = 1; }\n\
         >> cscope: 1 lines\n\
         a.c f 2 void f(void ) { Counter = 1; }\n>> "
    );
}

#[test]
fn a_query_brings_the_index_up_to_date_unless_told_not_to() {
    let work = tempfile::tempdir().expect("work");
    let source = work.path().join("a.c");
    std::fs::write(&source, "int old;\n").expect("source");
    assert!(
        ripscope(work.path(), &["-bk", "-f", "i.db", "a.c"], None)
            .status
            .success()
    );
    std::fs::write(&source, "int new_name;\n").expect("edit");
    let stale = ripscope(work.path(), &["-dL", "-f", "i.db", "-1", "new_name"], None);
    assert_eq!(stale.stdout, b"");
    let fresh = ripscope(work.path(), &["-L", "-f", "i.db", "-1", "new_name"], None);
    assert_eq!(fresh.stdout, b"a.c new_name 1 int new_name;\n");
    std::fs::remove_file(&source).expect("removed");
    // Stored fields answer after the sources are gone; text searches read live files.
    let stored = ripscope(work.path(), &["-dL", "-f", "i.db", "-0", "new_name"], None);
    assert_eq!(stored.stdout, b"a.c <global> 1 int new_name;\n");
}

#[test]
fn text_searches_read_the_files_as_they_are() {
    let work = tempfile::tempdir().expect("work");
    std::fs::write(work.path().join("a.c"), "int x; /* NOTE: keep */\nint y;\n").expect("source");
    assert!(
        ripscope(work.path(), &["-bk", "-f", "i.db", "a.c"], None)
            .status
            .success()
    );
    let literal = ripscope(work.path(), &["-dL", "-f", "i.db", "-4", "NOTE:"], None);
    assert_eq!(literal.stdout, b"a.c <unknown> 1 int x; /* NOTE: keep */\n");
    let regex = ripscope(work.path(), &["-dL", "-f", "i.db", "-6", "^int y"], None);
    assert_eq!(regex.stdout, b"a.c <unknown> 2 int y;\n");
}

/// `-c`, `-p N` and `-F FILE` change nothing in cscope's line or one-shot output, so they
/// are read and ignored, on the command line and (`-c` and `-p N`) in a file list. `-T` and
/// `-s` would change the answers, so they are refused.
#[test]
fn options_only_cscopes_own_database_and_screen_read_change_no_answer() {
    let work = tempfile::tempdir().expect("work");
    let source = work.path().join("a.c");
    std::fs::write(&source, "int Counter;\nvoid f(void) { Counter = 1; }\n").expect("source");
    std::fs::write(work.path().join("plain.files"), "a.c\n").expect("plain list");
    std::fs::write(work.path().join("quiet.files"), "-c\n-p 3 a.c\n").expect("quiet list");
    let plain = ripscope(
        work.path(),
        &["-bk", "-f", "plain.db", "-i", "plain.files"],
        None,
    );
    assert!(
        plain.status.success() && plain.stderr.is_empty(),
        "{plain:?}"
    );
    let quiet = ["-c", "-p", "3", "-F", "syms"];
    let mut build = vec!["-bk", "-f", "quiet.db", "-i", "quiet.files"];
    build.extend(quiet);
    let built = ripscope(work.path(), &build, None);
    assert!(
        built.status.success() && built.stderr.is_empty(),
        "{built:?}"
    );
    // The quiet list is read again by every update.
    std::fs::write(&source, "int Counter;\nvoid f(void) { Counter = 2; }\n").expect("edit");
    for (field, pattern, expected) in [
        ("-1", "Counter", "a.c Counter 1 int Counter;\n"),
        ("-9", "Counter", "a.c f 2 void f(void ) { Counter = 2; }\n"),
        (
            "-4",
            "= 2",
            "a.c <unknown> 2 void f(void) { Counter = 2; }\n",
        ),
    ] {
        let plain = ripscope(work.path(), &["-L", "-f", "plain.db", field, pattern], None);
        let mut args = vec!["-Lc", "-p3", "-Fsyms", "-f", "quiet.db", field, pattern];
        args.extend(quiet);
        let answer = ripscope(work.path(), &args, None);
        for output in [&plain, &answer] {
            assert!(
                output.status.success() && output.stderr.is_empty(),
                "{output:?}"
            );
            assert_eq!(String::from_utf8_lossy(&output.stdout), expected, "{field}");
        }
    }
    let session = ripscope(
        work.path(),
        &["-dlc", "-p", "3", "-F", "syms", "-f", "quiet.db"],
        Some(b"1Counter\nq\n"),
    );
    assert!(session.status.success(), "{session:?}");
    assert_eq!(
        String::from_utf8_lossy(&session.stdout),
        ">> cscope: 1 lines\na.c Counter 1 int Counter;\n>> "
    );
    for args in [
        &["-bkT", "-f", "t.db", "a.c"][..],
        &["-bk", "-s", ".", "-f", "s.db"],
    ] {
        let refused = ripscope(work.path(), args, None);
        assert!(!refused.status.success(), "{args:?}: {refused:?}");
    }
    std::fs::write(work.path().join("t.files"), "-T\na.c\n").expect("list with -T");
    let listed = ripscope(work.path(), &["-bk", "-f", "t.db", "-i", "t.files"], None);
    assert!(!listed.status.success(), "{listed:?}");
    assert!(!work.path().join("t.db").exists() && !work.path().join("s.db").exists());
}

fn set_modified(path: &Path, time: std::time::SystemTime) {
    std::fs::File::options()
        .write(true)
        .open(path)
        .and_then(|file| file.set_modified(time))
        .expect("modification time");
}

#[test]
fn verify_checks_the_index_and_hashes_every_file() {
    let work = tempfile::tempdir().expect("work");
    let source = work.path().join("a.c");
    let hour_ago = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
    std::fs::write(&source, "int old;\n").expect("source");
    set_modified(&source, hour_ago);
    let built = ripscope(work.path(), &["-bk", "-f", "i.db", "a.c"], None);
    assert!(built.status.success(), "{built:?}");

    // Other bytes with the same size and modification time, which an ordinary update
    // on Windows trusts: `--verify` reads every file.
    std::fs::write(&source, "int new;\n").expect("edit");
    set_modified(&source, hour_ago);
    let verified = ripscope(work.path(), &["--verify", "-f", "i.db"], None);
    assert!(verified.status.success(), "{verified:?}");
    assert_eq!(
        String::from_utf8_lossy(&verified.stdout),
        "index verified: revision 2, 1 file\n"
    );
    assert_eq!(
        ripscope(work.path(), &["-dL", "-f", "i.db", "-1", "new"], None).stdout,
        b"a.c new 1 int new;\n"
    );

    // A damaged analysis is reported; an ordinary index build keeps it, and `-u` replaces it.
    rusqlite::Connection::open(work.path().join("i.db"))
        .and_then(|db| db.execute("UPDATE blocks SET image = x'00'", []))
        .expect("damaged");
    for rebuild in [
        &["-bk", "-f", "i.db", "a.c"][..],
        &["-bku", "-f", "i.db", "a.c"],
    ] {
        let damaged = ripscope(work.path(), &["--verify", "-d", "-f", "i.db"], None);
        assert!(!damaged.status.success(), "{damaged:?}");
        let message = String::from_utf8_lossy(&damaged.stderr);
        assert!(message.contains("a.c"), "{message}");
        assert!(
            message.contains("rebuild it with -bu and the options it was built with"),
            "{message}"
        );
        let rebuilt = ripscope(work.path(), rebuild, None);
        assert!(rebuilt.status.success(), "{rebuilt:?}");
    }
    let verified = ripscope(work.path(), &["--verify", "-d", "-f", "i.db"], None);
    assert!(verified.status.success(), "{verified:?}");
}

#[test]
fn verify_prints_multiline_sqlite_integrity_problems_one_per_line() {
    let work = indexed();
    {
        let db = rusqlite::Connection::open(work.path().join("i.db")).expect("index");
        // Two tables claim the same page, leaving the original files page unused.
        db.execute_batch(
            "PRAGMA writable_schema = ON;
             UPDATE sqlite_schema
             SET rootpage = (SELECT rootpage FROM sqlite_schema WHERE name = 'blocks')
             WHERE name = 'files';",
        )
        .expect("damaged root page");
    }
    let rows = {
        let db = rusqlite::Connection::open(work.path().join("i.db")).expect("index");
        let mut check = db
            .prepare("PRAGMA integrity_check")
            .expect("integrity check");
        check
            .query_map([], |row| row.get::<_, String>(0))
            .expect("integrity rows")
            .collect::<Result<Vec<_>, _>>()
            .expect("integrity problems")
    };
    assert!(rows.iter().any(|row| row.lines().count() >= 3), "{rows:?}");
    for verify in [
        &["--verify", "-f", "i.db"][..],
        &["--verify", "-d", "-f", "i.db"],
    ] {
        let damaged = ripscope(work.path(), verify, None);
        assert_eq!(damaged.status.code(), Some(1), "{damaged:?}");
        assert!(damaged.stdout.is_empty(), "{damaged:?}");
        let message = String::from_utf8_lossy(&damaged.stderr);
        let lines: Vec<_> = message.lines().collect();
        assert_eq!(
            lines.len(),
            rows.iter().map(|row| row.lines().count()).sum::<usize>() + 1
        );
        assert!(
            lines.iter().all(|line| line.starts_with("ripscope: ")),
            "{message}"
        );
        assert!(
            message.starts_with("ripscope: *** in database main ***\n"),
            "{message}"
        );
        assert!(
            lines[1].starts_with("ripscope: 2nd reference to page "),
            "{message}"
        );
        assert!(
            lines[2].starts_with("ripscope: Page ") && lines[2].ends_with(": never used"),
            "{message}"
        );
        assert!(!message.contains("\\x0a"), "{message}");
        assert!(
            message.ends_with(
                "ripscope: the index is damaged; rebuild it with -bu and the options it was built with\n"
            ),
            "{message}"
        );
    }
}

#[test]
fn verify_splits_lf_and_crlf_problem_lines_and_escapes_other_controls() {
    for ending in ["\n", "\r\n"] {
        for terminal_newline in [false, true] {
            let work = indexed();
            let mut name = format!("first{ending}second\t\x1b\x7f\rinside\u{85}{ending}third");
            if terminal_newline {
                name.push_str(ending);
            }
            {
                let db = rusqlite::Connection::open(work.path().join("i.db")).expect("index");
                // SQLite includes the table name in its real integrity problem row.
                db.execute_batch(&format!(
                    "CREATE TABLE \"{name}\" (value INTEGER CHECK (value = 0));
                     PRAGMA ignore_check_constraints = ON;
                     INSERT INTO \"{name}\" VALUES (1);",
                ))
                .expect("violated check constraint");
            }
            {
                let db = rusqlite::Connection::open(work.path().join("i.db")).expect("index");
                let problem: String = db
                    .query_row("PRAGMA integrity_check", [], |row| row.get(0))
                    .expect("integrity problem");
                assert_eq!(problem, format!("CHECK constraint failed in {name}"));
            }
            for verify in [
                &["--verify", "-f", "i.db"][..],
                &["--verify", "-d", "-f", "i.db"],
            ] {
                let damaged = ripscope(work.path(), verify, None);
                assert_eq!(damaged.status.code(), Some(1), "{damaged:?}");
                assert!(damaged.stdout.is_empty(), "{damaged:?}");
                assert_eq!(
                    damaged.stderr,
                    b"ripscope: CHECK constraint failed in first\n\
                      ripscope: second\\x09\\x1b\\x7f\\x0dinside\\x85\n\
                      ripscope: third\n\
                      ripscope: the index is damaged; rebuild it with -bu and the options it was built with\n",
                    "ending {ending:?}, terminal newline {terminal_newline}"
                );
            }
        }
    }
}

/// An index whose block size cannot be read is damaged: `--verify` says so, with or
/// without `-d`, before any update, even one that would analyse every file again (as for
/// an index another version made); an update that must analyse a file fails, and so does
/// a query without `-d`, while one with `-d` answers; and `-bu` records the block size a
/// new index gets.
#[test]
fn an_index_without_its_block_size_is_rebuilt_with_u() {
    let work = tempfile::tempdir().expect("work");
    std::fs::write(work.path().join("a.c"), "int old;\n").expect("source");
    let built = ripscope(work.path(), &["-bk", "-f", "i.db", "a.c"], None);
    assert!(built.status.success(), "{built:?}");
    rusqlite::Connection::open(work.path().join("i.db"))
        .and_then(|db| db.execute("DELETE FROM meta WHERE key = 'block size'", []))
        .expect("damaged");
    let rebuild = "rebuild it with -bu and the options it was built with";
    for verify in [
        &["--verify", "-f", "i.db"][..],
        &["--verify", "-d", "-f", "i.db"],
    ] {
        let damaged = ripscope(work.path(), verify, None);
        assert!(!damaged.status.success(), "{damaged:?}");
        let message = String::from_utf8_lossy(&damaged.stderr);
        assert!(message.contains("block size cannot be read"), "{message}");
        assert!(message.contains(rebuild), "{message}");
    }
    std::fs::write(work.path().join("a.c"), "int newer;\n").expect("edit");
    for failing in [
        &["--verify", "-f", "i.db"][..],
        &["-L", "-f", "i.db", "-1", "newer"],
    ] {
        let failed = ripscope(work.path(), failing, None);
        assert!(!failed.status.success(), "{failed:?}");
        let message = String::from_utf8_lossy(&failed.stderr);
        assert!(message.contains("block size cannot be read"), "{message}");
        assert!(message.contains(rebuild), "{message}");
    }
    assert_eq!(
        ripscope(work.path(), &["-dL", "-f", "i.db", "-1", "old"], None).stdout,
        b"a.c old 1 int old;\n"
    );
    let built = ripscope(work.path(), &["-bk", "-f", "i.db", "a.c"], None);
    assert!(!built.status.success(), "{built:?}");
    let rebuilt = ripscope_with(
        work.path(),
        &["-bku", "-f", "i.db", "a.c"],
        None,
        &[("RIPSCOPE_BLOCK_SIZE", "1")],
    );
    assert!(rebuilt.status.success(), "{rebuilt:?}");
    let size: i64 = rusqlite::Connection::open(work.path().join("i.db"))
        .and_then(|db| {
            db.query_row(
                "SELECT value FROM meta WHERE key = 'block size'",
                [],
                |row| row.get(0),
            )
        })
        .expect("block size");
    assert_eq!(size, 1);
    let verified = ripscope(work.path(), &["--verify", "-f", "i.db"], None);
    assert!(verified.status.success(), "{verified:?}");
    assert_eq!(
        ripscope(work.path(), &["-dL", "-f", "i.db", "-1", "newer"], None).stdout,
        b"a.c newer 1 int newer;\n"
    );
    rusqlite::Connection::open(work.path().join("i.db"))
        .and_then(|db| {
            db.execute_batch(
                "UPDATE meta SET value = x'00' WHERE key = 'analyzer';
                 DELETE FROM meta WHERE key = 'block size';",
            )
        })
        .expect("damaged");
    let damaged = ripscope(work.path(), &["--verify", "-f", "i.db"], None);
    assert!(!damaged.status.success(), "{damaged:?}");
    let message = String::from_utf8_lossy(&damaged.stderr);
    assert!(message.contains("block size cannot be read"), "{message}");
}

/// An index file that cannot be written is answered from as it is, as cscope's `-d` reads
/// such a database: a query or session says so and skips its update, a JSON session's `r`
/// gets an error frame, and an index build fails, naming the file. One that holds nothing
/// published leaves nothing to answer from, and a query fails.
#[test]
fn an_index_that_cannot_be_written_is_answered_as_it_is() {
    let work = indexed();
    std::fs::write(work.path().join("a.c"), "int Alpha;\nint Beta;\n").expect("edit");
    let index = work.path().join("i.db");
    let set_file_readonly = |file: &std::path::Path, readonly| {
        let mut permissions = std::fs::metadata(file).expect("file").permissions();
        permissions.set_readonly(readonly);
        std::fs::set_permissions(file, permissions).expect("permissions");
    };
    let set_readonly = |readonly| set_file_readonly(&index, readonly);
    set_readonly(true);
    // Root may write any file on Unix: then there is nothing to check.
    if std::fs::OpenOptions::new()
        .append(true)
        .open(&index)
        .is_ok()
    {
        set_readonly(false);
        return;
    }
    let refused = "the index i.db cannot be written";
    let skipped = format!("{refused}; answering from the index as it is (-d skips the update)");

    let query = ripscope(work.path(), &["-L", "-f", "i.db", "-1", "Alpha"], None);
    assert!(query.status.success(), "{query:?}");
    assert_eq!(
        String::from_utf8_lossy(&query.stdout),
        "a.c Alpha 1 int Alpha;\n"
    );
    assert_eq!(
        String::from_utf8_lossy(&query.stderr),
        format!("ripscope: {skipped}\n")
    );
    let stored = ripscope(work.path(), &["-dL", "-f", "i.db", "-1", "Beta"], None);
    assert!(stored.status.success(), "{stored:?}");
    assert!(
        stored.stdout.is_empty() && stored.stderr.is_empty(),
        "{stored:?}"
    );
    let session = ripscope(
        work.path(),
        &["--json", "-l", "-f", "i.db"],
        Some(b"r\n1Alpha\nq\n"),
    );
    assert!(session.status.success(), "{session:?}");
    assert_eq!(
        String::from_utf8_lossy(&session.stderr),
        format!("ripscope: {skipped}\n")
    );
    let frames: Vec<serde_json::Value> = session
        .stdout
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .map(|line| serde_json::from_slice(line).expect("frame"))
        .collect();
    assert_eq!(
        frames,
        [
            serde_json::json!({"type": "ready"}),
            serde_json::json!({"type": "error", "message": skipped}),
            serde_json::json!({"type": "ready"}),
            serde_json::json!({"type": "result", "count": 1}),
            serde_json::json!(["a.c", "Alpha", 1, "int Alpha;"]),
            serde_json::json!({"type": "warning", "message": format!("automatic Refresh did not complete ({skipped}); answering from the last published revision, whose freshness is unconfirmed")}),
            serde_json::json!({"type": "ready"}),
        ]
    );
    let build = ripscope(work.path(), &["-bk", "-f", "i.db", "a.c"], None);
    assert!(!build.status.success(), "{build:?}");
    assert_eq!(
        String::from_utf8_lossy(&build.stderr),
        format!("ripscope: {refused}\n")
    );
    // One that has published nothing yet leaves nothing to answer from: the query says
    // why it could not update the index, and that there is no index yet. The readers
    // above left SQLite's companion files read-only too.
    set_readonly(false);
    for log in ["i.db-wal", "i.db-shm"] {
        let log = work.path().join(log);
        if log.exists() {
            set_file_readonly(&log, false);
        }
    }
    rusqlite::Connection::open(&index)
        .expect("index")
        .execute_batch(
            "UPDATE meta SET value = 0 WHERE key = 'revision';
             DELETE FROM files; DELETE FROM units;",
        )
        .expect("nothing published");
    set_readonly(true);
    let unpublished = ripscope(work.path(), &["-L", "-f", "i.db", "-1", "Alpha"], None);
    assert!(!unpublished.status.success(), "{unpublished:?}");
    assert!(unpublished.stdout.is_empty(), "{unpublished:?}");
    assert_eq!(
        String::from_utf8_lossy(&unpublished.stderr),
        format!(
            "ripscope: {refused}\nripscope: no index at i.db yet: nothing is published in it\n"
        )
    );
    set_readonly(false);
}

/// `--verify` reads every file again, but publishes nothing when nothing changed:
/// the revision stays, so open sessions keep what they loaded.
#[test]
fn verify_keeps_the_revision_of_an_unchanged_index() {
    let work = indexed();
    for _ in 0..2 {
        let verified = ripscope(work.path(), &["--verify", "-f", "i.db"], None);
        assert!(verified.status.success(), "{verified:?}");
        assert_eq!(
            String::from_utf8_lossy(&verified.stdout),
            "index verified: revision 1, 1 file\n"
        );
    }
}

/// SQLite gives the files it keeps beside the index the index file's permissions, so a
/// reader of an index file that cannot be written leaves them read-only. Once the file can
/// be written again, the next update makes them writable too, and succeeds.
#[cfg(unix)]
#[test]
fn an_index_written_again_after_a_read_only_spell_updates() {
    use std::os::unix::fs::PermissionsExt;
    let work = indexed();
    let index = work.path().join("i.db");
    let mode = |path: &Path, mode: u32| {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).expect("mode");
    };
    mode(&index, 0o444);
    // Root may write any file: then there is nothing to check.
    if std::fs::OpenOptions::new()
        .append(true)
        .open(&index)
        .is_ok()
    {
        mode(&index, 0o644);
        return;
    }
    let read = ripscope(work.path(), &["-dL", "-f", "i.db", "-1", "Alpha"], None);
    assert!(read.status.success(), "{read:?}");
    mode(&index, 0o644);
    std::fs::write(work.path().join("a.c"), "int Alpha;\nint Beta;\n").expect("edit");
    let query = ripscope(work.path(), &["-L", "-f", "i.db", "-1", "Beta"], None);
    assert!(query.status.success(), "{query:?}");
    assert_eq!(
        String::from_utf8_lossy(&query.stdout),
        "a.c Beta 2 int Beta;\n"
    );
    assert!(query.stderr.is_empty(), "{query:?}");
}

/// Read-only files where SQLite keeps its log and shared memory, as a reader of a
/// read-only index leaves them on Unix, or marked read-only on Windows, are made writable
/// again by the next update, which then succeeds.
#[test]
fn an_update_makes_read_only_companions_writable_again() {
    let work = indexed();
    for companion in ["i.db-wal", "i.db-shm"] {
        let companion = work.path().join(companion);
        std::fs::write(&companion, "").expect("companion");
        let mut permissions = std::fs::metadata(&companion)
            .expect("companion")
            .permissions();
        permissions.set_readonly(true);
        std::fs::set_permissions(&companion, permissions).expect("read-only");
    }
    std::fs::write(work.path().join("a.c"), "int Alpha;\nint Beta;\n").expect("edit");
    let query = ripscope(work.path(), &["-L", "-f", "i.db", "-1", "Beta"], None);
    assert!(query.status.success(), "{query:?}");
    assert_eq!(
        String::from_utf8_lossy(&query.stdout),
        "a.c Beta 2 int Beta;\n"
    );
}

/// What SQLite would write through, planted where it keeps a log file beside the index.
enum Planted {
    SymbolicLink,
    HardLink,
    Directory,
    #[cfg(unix)]
    Fifo,
}

/// Plants `what` as `i.db{suffix}`, naming `elsewhere` for the links; `None` where this
/// process may not make it (a symbolic link on Windows without the privilege).
fn plant(what: &Planted, suffix: &str, work: &Path, elsewhere: &Path) -> Option<()> {
    let companion = work.join(format!("i.db{suffix}"));
    match what {
        Planted::SymbolicLink => file_link(elsewhere, &companion),
        Planted::HardLink => std::fs::hard_link(elsewhere, &companion).ok(),
        Planted::Directory => std::fs::create_dir(&companion).ok(),
        #[cfg(unix)]
        Planted::Fifo => Command::new("mkfifo")
            .arg(&companion)
            .status()
            .ok()
            .filter(|status| status.success())
            .map(|_| ()),
    }
}

/// A link, a file with another name, a directory or a FIFO where SQLite keeps its log
/// files refuses the index, for a query as for an index build, naming the file, and the file a
/// link names keeps its content: SQLite would write through it, emptying it.
#[test]
fn a_log_file_that_is_not_a_plain_file_refuses_the_index() {
    let planted = [
        Planted::SymbolicLink,
        Planted::HardLink,
        Planted::Directory,
        #[cfg(unix)]
        Planted::Fifo,
    ];
    for what in &planted {
        for suffix in ["-wal", "-shm"] {
            let work = indexed();
            let elsewhere = work.path().join("elsewhere");
            std::fs::write(&elsewhere, "kept").expect("elsewhere");
            if plant(what, suffix, work.path(), &elsewhere).is_none() {
                continue;
            }
            for args in [
                &["-dL", "-f", "i.db", "-1", "Alpha"][..],
                &["-L", "-f", "i.db", "-1", "Alpha"],
                &["-bk", "-f", "i.db", "a.c"],
            ] {
                let refused = ripscope(work.path(), args, None);
                assert_eq!(
                    refused.status.code(),
                    Some(1),
                    "{suffix} {args:?}: {refused:?}"
                );
                assert!(refused.stdout.is_empty(), "{suffix} {args:?}: {refused:?}");
                let companion = if cfg!(windows) {
                    std::fs::canonicalize(work.path())
                        .expect("directory")
                        .join(format!("i.db{suffix}"))
                } else {
                    PathBuf::from(format!("i.db{suffix}"))
                };
                assert_eq!(
                    String::from_utf8_lossy(&refused.stderr),
                    format!(
                        "ripscope: {} is not a plain file (a link, a file with another \
                         name, or not a file); SQLite would write through it, so the index is \
                         not opened; remove it\n",
                        companion.display()
                    ),
                    "{suffix} {args:?}"
                );
            }
            assert_eq!(
                std::fs::read(&elsewhere).expect("kept"),
                b"kept",
                "{suffix}"
            );
        }
    }
}

/// A query whose update writes only when files were read again, as after an index build
/// of files changed within the last seconds, leaves no log behind: each connection
/// opened later would read all of it again.
#[test]
fn a_query_that_updates_leaves_no_log() {
    let work = indexed();
    let queried = ripscope(work.path(), &["-L", "-f", "i.db", "-1", "Alpha"], None);
    assert_eq!(queried.stdout, b"a.c Alpha 1 int Alpha;\n", "{queried:?}");
    assert!(queried.stderr.is_empty(), "{queried:?}");
    let log = work.path().join("i.db-wal");
    assert!(
        std::fs::metadata(&log).map_or(true, |log| log.len() == 0),
        "{:?}",
        std::fs::metadata(&log).map(|log| log.len())
    );
}

/// On Windows SQLite never marks its log files read-only, but they may have been: an
/// update makes them writable again, also one whose only attribute is read-only.
#[cfg(windows)]
#[test]
fn a_read_only_log_file_is_made_writable_on_windows() {
    let work = indexed();
    let wal = work.path().join("i.db-wal");
    std::fs::write(&wal, "").expect("log");
    let attrib = Command::new("attrib")
        .args(["-A", "+R"])
        .arg(&wal)
        .status()
        .expect("attrib");
    assert!(attrib.success());
    assert!(
        std::fs::metadata(&wal)
            .expect("log")
            .permissions()
            .readonly()
    );
    std::fs::write(work.path().join("a.c"), "int Alpha;\nint Beta;\n").expect("edit");
    let updated = ripscope(work.path(), &["-L", "-f", "i.db", "-1", "Beta"], None);
    assert_eq!(updated.stdout, b"a.c Beta 2 int Beta;\n", "{updated:?}");
    assert!(updated.stderr.is_empty(), "{updated:?}");
    if wal.exists() {
        assert!(
            !std::fs::metadata(&wal)
                .expect("log")
                .permissions()
                .readonly()
        );
    }
}

/// A new index gets the permissions the umask gives any new file, as cscope's database
/// does, so a group can share it; SQLite alone would make it 0644 under umask 002.
#[cfg(unix)]
#[test]
fn a_new_index_takes_its_permissions_from_the_umask() {
    use std::os::unix::fs::PermissionsExt;
    let work = tempfile::tempdir().expect("work");
    std::fs::write(work.path().join("a.c"), "int Alpha;\n").expect("source");
    for (umask, expected) in [("002", 0o664), ("022", 0o644), ("077", 0o600)] {
        let index = format!("i{umask}.db");
        let script = format!("umask {umask} && exec \"$0\" \"$@\"");
        let built = Command::new("sh")
            .current_dir(work.path())
            .env_remove("INCLUDEDIRS")
            .env_remove("INCDIR")
            .args(["-c", &script, env!("CARGO_BIN_EXE_ripscope")])
            .args(["-bk", "-f", &index, "a.c"])
            .output()
            .expect("sh");
        assert!(built.status.success(), "{built:?}");
        let mode = std::fs::metadata(work.path().join(&index))
            .expect("index")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, expected, "umask {umask}");
    }
}

/// A reader must create SQLite's shared-memory file beside the index: in a directory that
/// cannot be written the query fails, saying so.
#[cfg(unix)]
#[test]
fn an_index_in_a_directory_that_cannot_be_written_says_why() {
    use std::os::unix::fs::PermissionsExt;
    let work = tempfile::tempdir().expect("work");
    let directory = work.path().join("shared");
    std::fs::create_dir(&directory).expect("directory");
    std::fs::write(directory.join("a.c"), "int Alpha;\n").expect("source");
    let built = ripscope(&directory, &["-bk", "-f", "i.db", "a.c"], None);
    assert!(built.status.success(), "{built:?}");
    let mode = |mode: u32| {
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(mode)).expect("mode");
    };
    mode(0o555);
    // Root may write in any directory: then there is nothing to check.
    if may_create_in(&directory) {
        mode(0o755);
        return;
    }
    let query = ripscope(&directory, &["-dL", "-f", "i.db", "-1", "Alpha"], None);
    mode(0o755);
    assert!(!query.status.success(), "{query:?}");
    assert_eq!(
        String::from_utf8_lossy(&query.stderr),
        "ripscope: cannot read the index i.db: SQLite must create i.db-wal and i.db-shm \
         beside it, and that directory cannot be written; copy the index somewhere writable \
         and give it with -f\n"
    );
}

/// `-h` and `-V` print and stop where they appear, reading and writing no index.
#[test]
fn help_and_version_are_printed_and_end_the_command_line() {
    let work = tempfile::tempdir().expect("work");
    for args in [
        &["-h"][..],
        &["--help"],
        &["-h", "-z"],
        &["--vim-errors", "-h"],
    ] {
        let help = ripscope(work.path(), args, None);
        assert!(help.status.success(), "{args:?}: {help:?}");
        let text = String::from_utf8_lossy(&help.stdout);
        assert!(
            text.contains("\nUsage:\n  ripscope -b "),
            "{args:?}: {text}"
        );
        assert!(help.stderr.is_empty(), "{args:?}: {help:?}");
    }
    for args in [&["-V"][..], &["--version"], &["-V", "-h"], &["-dLV", "-f"]] {
        let version = ripscope(work.path(), args, None);
        assert!(version.status.success(), "{args:?}: {version:?}");
        assert_eq!(
            String::from_utf8_lossy(&version.stdout),
            format!("ripscope {}\n", env!("CARGO_PKG_VERSION")),
            "{args:?}"
        );
    }
    assert_eq!(
        std::fs::read_dir(work.path()).expect("listing").count(),
        0,
        "nothing written"
    );
}

#[test]
fn a_rebuild_of_everything_replaces_an_index_sqlite_cannot_read() {
    let work = tempfile::tempdir().expect("work");
    let mut names = Vec::new();
    for n in 0..300 {
        let name = format!("f{n}.c");
        std::fs::write(
            work.path().join(&name),
            format!("int value{n};\nvoid call{n}(void) {{ value{n} = {n}; }}\n"),
        )
        .expect("source");
        names.push(name);
    }
    let mut args = vec!["-bk", "-f", "i.db"];
    args.extend(names.iter().map(String::as_str));
    assert!(ripscope(work.path(), &args, None).status.success());
    let index = work.path().join("i.db");
    let mut bytes = std::fs::read(&index).expect("index");
    assert!(bytes.len() > 4 * 4096, "{}", bytes.len());
    bytes[2 * 4096..].fill(0xa5);
    std::fs::write(&index, &bytes).expect("damaged");
    let damaged = ripscope(work.path(), &["--verify", "-d", "-f", "i.db"], None);
    assert!(!damaged.status.success(), "{damaged:?}");
    assert!(
        String::from_utf8_lossy(&damaged.stderr).contains("rebuild it with -bu and the options"),
        "{damaged:?}"
    );
    args[0] = "-bku";
    let rebuilt = ripscope(work.path(), &args, None);
    assert!(rebuilt.status.success(), "{rebuilt:?}");
    let verified = ripscope(work.path(), &["--verify", "-d", "-f", "i.db"], None);
    assert!(verified.status.success(), "{verified:?}");
    assert_eq!(
        ripscope(work.path(), &["-dL", "-f", "i.db", "-1", "value299"], None).stdout,
        b"f299.c value299 1 int value299;\n"
    );
}

#[test]
fn a_cscope_database_at_the_index_path_is_left_alone() {
    let work = tempfile::tempdir().expect("work");
    std::fs::write(work.path().join("a.c"), "int x;\n").expect("source");
    let database = b"cscope 15 /src              0000000042\n\t@a.c\n".to_vec();
    std::fs::write(work.path().join("cscope.out"), &database).expect("database");
    let refused = ripscope(work.path(), &["-bk", "-f", "cscope.out", "a.c"], None);
    assert!(!refused.status.success());
    assert!(String::from_utf8_lossy(&refused.stderr).contains("not a ripscope index"));
    assert_eq!(
        std::fs::read(work.path().join("cscope.out")).expect("kept"),
        database
    );
}

/// The names in a directory, sorted.
fn names_in(directory: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(directory)
        .expect("directory")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    names
}

/// SQLite reads a name that begins with `file:` as a URI; an index path is a file name.
#[cfg(unix)]
#[test]
fn an_index_named_like_a_uri_is_a_file_of_that_name() {
    let work = tempfile::tempdir().expect("work");
    std::fs::write(work.path().join("a.c"), "int Alpha;\n").expect("source");
    let built = ripscope(work.path(), &["-bk", "-f", "file:i.db", "a.c"], None);
    assert!(built.status.success(), "{built:?}");
    let answered = ripscope(
        work.path(),
        &["-dL", "-f", "file:i.db", "-1", "Alpha"],
        None,
    );
    assert_eq!(
        String::from_utf8_lossy(&answered.stdout),
        "a.c Alpha 1 int Alpha;\n",
        "{answered:?}"
    );
    assert_eq!(
        names_in(work.path()),
        ["a.c", "file:i.db", "file:i.db-shm", "file:i.db-wal"]
    );
}

/// On Windows a `:` in a file name names a stream of another file, so it is refused, and
/// nothing is left behind.
#[cfg(windows)]
#[test]
fn an_index_name_with_a_colon_is_refused_on_windows() {
    let work = tempfile::tempdir().expect("work");
    std::fs::write(work.path().join("a.c"), "int Alpha;\n").expect("source");
    for args in [
        &["-bk", "-f", "file:i.db", "a.c"][..],
        &["-dL", "-f", "file:i.db", "-1", "Alpha"],
    ] {
        let refused = ripscope(work.path(), args, None);
        assert_eq!(refused.status.code(), Some(1), "{args:?}");
        assert_eq!(
            String::from_utf8_lossy(&refused.stderr),
            "ripscope: file:i.db: a Windows file name cannot hold ':'\n",
            "{args:?}"
        );
        assert_eq!(names_in(work.path()), ["a.c"], "{args:?}");
    }
}

/// A symbolic link to a file, or `None` where this process may not make one (Windows
/// without Developer Mode).
fn file_link(target: &Path, link: &Path) -> Option<()> {
    #[cfg(unix)]
    let made = std::os::unix::fs::symlink(target, link);
    #[cfg(windows)]
    let made = std::os::windows::fs::symlink_file(target, link);
    made.ok()
}

/// A symbolic link to a directory, as `file_link`.
fn directory_link(target: &Path, link: &Path) -> Option<()> {
    #[cfg(unix)]
    let made = std::os::unix::fs::symlink(target, link);
    #[cfg(windows)]
    let made = std::os::windows::fs::symlink_dir(target, link);
    made.ok()
}

/// An index named through a symbolic link is its target, as SQLite reads it: after a
/// reader who could not write the index left its log files read-only beside the target,
/// an update through the link makes them writable again and answers.
#[cfg(unix)]
#[test]
fn an_index_reached_through_a_link_is_repaired_beside_its_target() {
    use std::os::unix::fs::PermissionsExt;
    let work = tempfile::tempdir().expect("work");
    std::fs::create_dir(work.path().join("store")).expect("store");
    std::fs::write(work.path().join("a.c"), "int Alpha;\n").expect("source");
    let built = ripscope(work.path(), &["-bk", "-f", "store/i.db", "a.c"], None);
    assert!(built.status.success(), "{built:?}");
    file_link(Path::new("store/i.db"), &work.path().join("link.db")).expect("link");
    let index = work.path().join("store/i.db");
    let mode = |mode| std::fs::set_permissions(&index, std::fs::Permissions::from_mode(mode));
    mode(0o444).expect("read-only");
    let read = ripscope(work.path(), &["-dL", "-f", "link.db", "-1", "Alpha"], None);
    assert_eq!(read.stdout, b"a.c Alpha 1 int Alpha;\n", "{read:?}");
    mode(0o644).expect("writable");
    std::fs::write(work.path().join("a.c"), "int Alpha;\nint Beta;\n").expect("edit");
    let updated = ripscope(work.path(), &["-L", "-f", "link.db", "-1", "Beta"], None);
    assert_eq!(updated.stdout, b"a.c Beta 2 int Beta;\n", "{updated:?}");
    assert!(updated.stderr.is_empty(), "{updated:?}");
}

/// Rebuilding the index through a symbolic link replaces the index it names and keeps the link, as
/// every update through it writes that index.
#[test]
fn a_rebuild_through_a_link_replaces_its_target_and_keeps_the_link() {
    let work = tempfile::tempdir().expect("work");
    let mut names = Vec::new();
    for n in 0..300 {
        let name = format!("f{n}.c");
        std::fs::write(work.path().join(&name), format!("int value{n};\n")).expect("source");
        names.push(name);
    }
    std::fs::create_dir(work.path().join("store")).expect("store");
    let mut args = vec!["-bk", "-f", "store/i.db"];
    args.extend(names.iter().map(String::as_str));
    assert!(ripscope(work.path(), &args, None).status.success());
    let link = work.path().join("link.db");
    let target = Path::new("store").join("i.db");
    if file_link(&target, &link).is_none() {
        return;
    }
    let index = work.path().join("store/i.db");
    let mut bytes = std::fs::read(&index).expect("index");
    assert!(bytes.len() > 4 * 4096, "{}", bytes.len());
    bytes[2 * 4096..].fill(0xa5);
    std::fs::write(&index, &bytes).expect("damaged");
    args[0] = "-bku";
    args[2] = "link.db";
    let rebuilt = ripscope(work.path(), &args, None);
    assert!(rebuilt.status.success(), "{rebuilt:?}");
    assert!(
        std::fs::symlink_metadata(&link)
            .expect("link")
            .file_type()
            .is_symlink(),
        "the link was replaced"
    );
    let beside_link: Vec<String> = names_in(work.path())
        .into_iter()
        .filter(|name| name.starts_with("link.db"))
        .collect();
    assert_eq!(beside_link, ["link.db"]);
    for index in ["store/i.db", "link.db"] {
        let verified = ripscope(work.path(), &["--verify", "-d", "-f", index], None);
        assert!(verified.status.success(), "{index}: {verified:?}");
    }
}

/// A symbolic link that names nothing is refused, naming what it names, and nothing is
/// made there.
#[test]
fn a_link_to_nothing_is_refused() {
    let work = tempfile::tempdir().expect("work");
    std::fs::write(work.path().join("a.c"), "int Alpha;\n").expect("source");
    if file_link(Path::new("missing.db"), &work.path().join("link.db")).is_none() {
        return;
    }
    for args in [
        &["-bk", "-f", "link.db", "a.c"][..],
        &["-dL", "-f", "link.db", "-1", "Alpha"],
    ] {
        let refused = ripscope(work.path(), args, None);
        assert_eq!(refused.status.code(), Some(1), "{args:?}");
        assert_eq!(
            String::from_utf8_lossy(&refused.stderr),
            format!(
                "ripscope: link.db is a symbolic link to {}, which does not exist\n",
                Path::new("missing.db").display()
            ),
            "{args:?}"
        );
        assert_eq!(names_in(work.path()), ["a.c", "link.db"], "{args:?}");
    }
}

/// A relative link is read from where it is, not from the name it was reached by: `..`
/// after a linked directory leads out of the directory the link names. A chain of links
/// is followed to its end.
#[test]
fn a_relative_link_is_read_from_its_own_directory() {
    let work = tempfile::tempdir().expect("work");
    for directory in ["real/sub", "store"] {
        std::fs::create_dir_all(work.path().join(directory)).expect("directory");
    }
    std::fs::write(work.path().join("a.c"), "int Alpha;\n").expect("source");
    let built = ripscope(work.path(), &["-bk", "-f", "store/i.db", "a.c"], None);
    assert!(built.status.success(), "{built:?}");
    let sub = Path::new("real").join("sub");
    if directory_link(&sub, &work.path().join("linked")).is_none() {
        return;
    }
    let up = Path::new("..").join("..").join("store").join("i.db");
    file_link(&up, &work.path().join("real/sub/i.db")).expect("link");
    file_link(
        &Path::new("linked").join("i.db"),
        &work.path().join("chain.db"),
    )
    .expect("chain");
    std::fs::write(work.path().join("a.c"), "int Alpha;\nint Beta;\n").expect("edit");
    let updated = ripscope(
        work.path(),
        &["-L", "-f", "linked/i.db", "-1", "Beta"],
        None,
    );
    assert_eq!(updated.stdout, b"a.c Beta 2 int Beta;\n", "{updated:?}");
    assert!(updated.stderr.is_empty(), "{updated:?}");
    let answered = ripscope(
        work.path(),
        &["-dL", "-f", "store/i.db", "-1", "Beta"],
        None,
    );
    assert_eq!(answered.stdout, b"a.c Beta 2 int Beta;\n", "{answered:?}");
    assert_eq!(names_in(&work.path().join("real/sub")), ["i.db"]);
    let chained = ripscope(work.path(), &["-dL", "-f", "chain.db", "-1", "Beta"], None);
    assert_eq!(chained.stdout, b"a.c Beta 2 int Beta;\n", "{chained:?}");
}

/// A reader through a link must create SQLite's shared-memory file beside the file the
/// link names: where that directory cannot be written, the query says so, naming it.
#[cfg(unix)]
#[test]
fn an_index_reached_through_a_link_in_a_directory_that_cannot_be_written_says_why() {
    use std::os::unix::fs::PermissionsExt;
    let work = tempfile::tempdir().expect("work");
    let store = work.path().join("store");
    std::fs::create_dir(&store).expect("store");
    std::fs::write(work.path().join("a.c"), "int Alpha;\n").expect("source");
    let built = ripscope(work.path(), &["-bk", "-f", "store/i.db", "a.c"], None);
    assert!(built.status.success(), "{built:?}");
    file_link(Path::new("store/i.db"), &work.path().join("link.db")).expect("link");
    let mode = |mode: u32| {
        std::fs::set_permissions(&store, std::fs::Permissions::from_mode(mode)).expect("mode");
    };
    mode(0o555);
    // Root may write in any directory: then there is nothing to check.
    if may_create_in(&store) {
        mode(0o755);
        return;
    }
    let query = ripscope(work.path(), &["-dL", "-f", "link.db", "-1", "Alpha"], None);
    mode(0o755);
    assert!(!query.status.success(), "{query:?}");
    assert_eq!(
        String::from_utf8_lossy(&query.stderr),
        "ripscope: cannot read the index store/i.db: SQLite must create store/i.db-wal and \
         store/i.db-shm beside it, and that directory cannot be written; copy the index \
         somewhere writable and give it with -f\n"
    );
}

/// An index in WAL mode never has a rollback journal, but SQLite reads one it finds beside
/// the index when it opens it, even to read. So a stray `-journal` is removed before the
/// index is opened, for a query as for an index build, and the query answers.
#[test]
fn a_stray_rollback_journal_is_removed_before_the_index_is_opened() {
    let work = indexed();
    std::fs::write(work.path().join("other"), "kept").expect("other");
    for args in [
        &["-dL", "-f", "i.db", "-1", "Alpha"][..],
        &["-L", "-f", "i.db", "-1", "Alpha"],
    ] {
        // A first byte of zero, which SQLite would leave where it is.
        std::fs::write(work.path().join("i.db-journal"), b"\0stray journal").expect("journal");
        let answered = ripscope(work.path(), args, None);
        assert_eq!(
            answered.stdout, b"a.c Alpha 1 int Alpha;\n",
            "{args:?}: {answered:?}"
        );
        assert!(!work.path().join("i.db-journal").exists(), "{args:?}");
    }
    assert_eq!(
        std::fs::read(work.path().join("other")).expect("kept"),
        b"kept"
    );
}

/// A reader must create both of SQLite's log files beside the index: a stale `-shm` left
/// without its `-wal`, in a directory that cannot be written, still gets the message.
#[cfg(unix)]
#[test]
fn a_lone_shared_memory_file_in_a_directory_that_cannot_be_written_says_why() {
    use std::os::unix::fs::PermissionsExt;
    let work = tempfile::tempdir().expect("work");
    let directory = work.path().join("shared");
    std::fs::create_dir(&directory).expect("directory");
    std::fs::write(directory.join("a.c"), "int Alpha;\n").expect("source");
    let built = ripscope(&directory, &["-bk", "-f", "i.db", "a.c"], None);
    assert!(built.status.success(), "{built:?}");
    std::fs::write(directory.join("i.db-shm"), "").expect("stale shared memory");
    let mode = |mode: u32| {
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(mode)).expect("mode");
    };
    mode(0o555);
    // Root may write in any directory: then there is nothing to check.
    if may_create_in(&directory) {
        mode(0o755);
        return;
    }
    let query = ripscope(&directory, &["-dL", "-f", "i.db", "-1", "Alpha"], None);
    mode(0o755);
    assert!(!query.status.success(), "{query:?}");
    assert_eq!(
        String::from_utf8_lossy(&query.stderr),
        "ripscope: cannot read the index i.db: SQLite must create i.db-wal and i.db-shm \
         beside it, and that directory cannot be written; copy the index somewhere writable \
         and give it with -f\n"
    );
}
