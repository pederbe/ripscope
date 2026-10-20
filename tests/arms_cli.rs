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

fn query(root: &Path, field: &str, pattern: &str) -> Vec<u8> {
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
    let mut expected = format!(
        ">> cscope: {} lines\n",
        output.stdout.iter().filter(|&&b| b == b'\n').count()
    )
    .into_bytes();
    expected.extend_from_slice(&output.stdout);
    expected.extend_from_slice(b">> ");
    if output.stdout.is_empty() {
        expected = b">> Unable to search database\n>> ".to_vec();
    }
    assert_eq!(persistent.stdout, expected);
    output.stdout
}

#[test]
fn macro_constructor_heads_keep_complete_stored_answers_after_source_removal() {
    let source = include_str!("fixtures/scanner/probes/73-macro-constructor-heads.c")
        .lines()
        .take(17)
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    let queries = [
        ("2", "GENERATED", "case.c helper 6 { return helper(); }\n"),
        (
            "2",
            "GENERATED_OPAQUE",
            "case.c helper 17 { return helper(); }\n",
        ),
        (
            "2",
            "CTOR_SAME",
            "case.c helper 11 #define CTOR_SAME(stat) static int CppConcat(same_, stat)(void ) { return helper(); }\n",
        ),
        (
            "3",
            "CppConcat",
            "case.c ordinary 10 int ordinary(void ) { return CppConcat(helper()); }\n",
        ),
        (
            "3",
            "helper",
            "case.c GENERATED 6 { return helper(); }\ncase.c ordinary 10 int ordinary(void ) { return CppConcat(helper()); }\ncase.c CTOR_SAME 11 #define CTOR_SAME(stat) static int CppConcat(same_, stat)(void ) { return helper(); }\ncase.c GENERATED_OPAQUE 17 { return helper(); }\n",
        ),
        (
            "1",
            "CppConcat",
            "case.c CppConcat 2 #define CppConcat(left, right) left ## right\ncase.c CppConcat 9 int CppConcat(int value) { return value; }\n",
        ),
        ("1", "generated_value", ""),
    ];
    for newline in ["\n", "\r\n"] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        fs::write(root.join("case.c"), source.replace('\n', newline)).unwrap();
        let built = run(root, &["-bk", "-f", "index", "case.c"], b"");
        assert!(
            built.status.success() && built.stderr.is_empty(),
            "{built:?}"
        );
        for removed in [false, true] {
            if removed {
                fs::remove_file(root.join("case.c")).unwrap();
            }
            for (field, pattern, expected) in queries {
                assert_eq!(
                    query(root, field, pattern),
                    expected.as_bytes(),
                    "{field}{pattern} {newline:?} removed={removed}"
                );
            }
        }
    }
}

#[test]
fn spliced_macro_formals_keep_their_body_in_stored_queries() {
    let source = include_str!("fixtures/scanner/probes/73-macro-constructor-heads.c");
    for newline in ["\n", "\r\n"] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        fs::write(root.join("case.c"), source.replace('\n', newline)).unwrap();
        let built = run(root, &["-bk", "-f", "index", "case.c"], b"");
        assert!(
            built.status.success() && built.stderr.is_empty(),
            "{built:?}"
        );
        for removed in [false, true] {
            if removed {
                fs::remove_file(root.join("case.c")).unwrap();
            }
            assert_eq!(query(root, "2", "SPLICED_HEADER"), b"case.c helper 28 (stat) int Ctor(left, right)(PG_FUNCTION_ARGS) { return helper(); }\n");
            assert!(query(root, "3", "Ctor").is_empty());
        }
    }
}

/// Nine alternative heads under `#if` share one body. Under ripscope's own query semantics
/// (ADR 0010, "Ripscope's own query semantics") the body's call has one caller, the last head, and only that head lists it;
/// cscope's sequential search gives all nine. The answers survive the source's removal.
#[test]
fn many_arms_give_the_last_head_after_source_removal() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let mut text = String::new();
    for i in 0..9 {
        text.push_str(if i == 0 {
            "#if FIRST\n"
        } else if i == 8 {
            "#else\n"
        } else {
            "#elif NEXT\n"
        });
        text.push_str(&format!("void f{i}(void) {{\n"));
    }
    text.push_str("#endif\ntarget(); }\n");
    fs::write(root.join("case.c"), &text).unwrap();
    let build = run(root, &["-bk", "-f", "index", "case.c"], b"");
    assert!(build.status.success(), "{build:?}");
    assert!(build.stdout.is_empty() && build.stderr.is_empty());
    let queries = [
        ("0", "target"),
        ("1", "f[0-9]"),
        ("2", "f0"),
        ("3", "target"),
        ("7", "case.c"),
        ("8", "case.c"),
        ("9", "target"),
    ];
    let expected: Vec<_> = queries.iter().map(|(f, p)| query(root, f, p)).collect();
    let callers = String::from_utf8(expected[3].clone()).unwrap();
    assert_eq!(callers, "case.c f8 20 target(); }\n");
    for i in 0..9 {
        assert_eq!(query(root, "2", &format!("f{i}")).is_empty(), i < 8, "f{i}");
    }
    for field in ["4", "6"] {
        let output = query(root, field, "target");
        assert!(
            String::from_utf8(output)
                .unwrap()
                .ends_with("target(); }\n")
        );
    }
    fs::remove_file(root.join("case.c")).unwrap();
    for ((field, pattern), before) in queries.iter().zip(expected) {
        assert_eq!(query(root, field, pattern), before);
    }
}
