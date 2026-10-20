mod intended;

use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Stdio},
};

fn run(root: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_ripscope"))
        .current_dir(root)
        .args(args)
        .output()
        .unwrap()
}

fn temp() -> tempfile::TempDir {
    #[cfg(target_os = "linux")]
    {
        tempfile::tempdir_in("/var/tmp").unwrap()
    }
    #[cfg(not(target_os = "linux"))]
    {
        tempfile::tempdir().unwrap()
    }
}

#[test]
fn assignment_queries_match_reference_after_source_deletion() {
    compare_fixture(
        "assignment-queries.json",
        include_str!("fixtures/assignment-queries.json"),
    );
}

#[test]
fn assignment_boundaries_match_reference_after_source_deletion() {
    compare_fixture(
        "assignment-boundaries.json",
        include_str!("fixtures/assignment-boundaries.json"),
    );
}

#[test]
fn assignments_apply_approved_pointer_state_correction_in_both_transports() {
    let baseline: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/assignment-pointer-state.json")).unwrap();
    // Preserve the oracle. Only x gains the approved missing assignments, and queries for
    // a and all names still use the exact reference expectation.
    let corrected = concat!(
        "comment.c <global> 1 int (*x) (void ); int a=(x=1);\n",
        "following.c <global> 1 int (*fp)(void ); int x; int a=(x=1);\n",
        "next.c <global> 2 int x; int a=(x=1);\n",
        "plain.c <global> 1 int (*x); int a=(x=1);\n",
        "pointer.c <global> 1 int (*x)(void ); int a=(x=1);\n",
        "reset.c <global> 3 int a=(x=1);\n",
        "split.c <global> 2 (void ); int a=(x=1);\n",
    );
    for newline in ["\n", "\r\n"] {
        let mut fixture = baseline.clone();
        for source in fixture["files"].as_object_mut().unwrap().values_mut() {
            *source = source.as_str().unwrap().replace('\n', newline).into();
        }
        for profile in fixture["profiles"].as_array_mut().unwrap() {
            for query in profile["queries"].as_array_mut().unwrap() {
                if query["args"].as_array().unwrap().last().unwrap() == "x" {
                    assert_ne!(query["stdout"].as_str().unwrap(), corrected);
                    query["stdout"] = corrected.into();
                }
            }
        }
        compare_fixture("assignment-pointer-state.json", &fixture.to_string());
    }
}

#[test]
fn assignment_correction_preserves_declaration_and_read_boundaries() {
    for (source, expected) in [
        (
            "int (*fp)(void); struct S { int x; }; int a=(x=1);\n",
            "case.c <global> 1 int (*fp)(void ); struct S { int x; }; int a=(x=1);\n",
        ),
        (
            "int (*fp)(void); enum E { x }; int a=(x=1);\n",
            "case.c <global> 1 int (*fp)(void ); enum E { x }; int a=(x=1);\n",
        ),
        (
            "typedef int (*fp)(void); int x; int a=(x=1);\n",
            "case.c <global> 1 typedef int (*fp)(void ); int x; int a=(x=1);\n",
        ),
        ("int (*fp)(void); extern int x; int a=(x=1);\n", ""),
        ("int (*fp)(void); int a=x+(x=1);\n", ""),
        ("int (*fp)(void); int f(void) { int x; x=1; }\n", ""),
        (
            "int (*fp)(void); int f(void) { struct S { int x; }; x=1; }\n",
            "",
        ),
        ("int (*fp)(void); int f(void) { enum E { x }; x=1; }\n", ""),
    ] {
        let profiles: Vec<_> = [false, true].into_iter().map(|caseless| {
            let args = if caseless { vec!["-dL", "-C", "-9", "x"] } else { vec!["-dL", "-9", "x"] };
            serde_json::json!({"caseless": caseless, "queries": [{"args": args, "stdout": expected}]})
        }).collect();
        compare_fixture("generated", &serde_json::json!({"files": {"case.c": source}, "seeds": ["case.c"], "profiles": profiles}).to_string());
    }
}

#[test]
fn assignment_crlf_and_refresh() {
    let dir = temp();
    let root = dir.path();
    fs::write(
        root.join("case.c"),
        "int x = 1;\r\nint f(void) {\r\n x/*gap*/ += 2;\r\n}\r\n",
    )
    .unwrap();
    assert!(
        run(root, &["-bk", "-f", "index", "case.c"])
            .status
            .success()
    );
    let query = |mode| run(root, &[mode, "-f", "index", "-9", "x"]);
    let old = query("-dL");
    assert_eq!(
        old.stdout,
        b"case.c <global> 1 int x = 1;\ncase.c f 3 x += 2;\n"
    );
    fs::write(root.join("case.c"), "int x = 3;\n").unwrap();
    assert_eq!(query("-dL").stdout, old.stdout);
    let fresh = query("-L");
    assert!(fresh.status.success());
    assert_eq!(fresh.stdout, b"case.c <global> 1 int x = 3;\n");
    fs::remove_file(root.join("case.c")).unwrap();
    assert_eq!(query("-dL").stdout, fresh.stdout);
    assert!(!query("-L").status.success());
}

#[test]
fn clipped_assignment_windows_preserve_exact_stored_rows_without_sources() {
    let files = serde_json::json!({
        "clean.c": include_str!("fixtures/assignment-window/clean.c"),
        "comment.c": include_str!("fixtures/assignment-window/comment.c"),
        "literal.c": include_str!("fixtures/assignment-window/literal.c"),
    });
    let expectations = [
        (
            "counter",
            concat!(
                "clean.c clean_writes 3 int counter = 0;\n",
                "clean.c clean_writes 4 counter++;\n",
                "clean.c clean_writes 5 --counter;\n",
                "clean.c clean_writes 6 slots[++counter] = 1;\n",
                "clean.c clean_writes 7 if (slots[0] == 1) counter += 1;\n",
                "comment.c comment_writes 9 int counter = 0;\n",
                "comment.c comment_writes 10 counter++;\n",
                "comment.c comment_writes 11 --counter;\n",
                "comment.c comment_writes 12 slots[++counter] = 1;\n",
                "comment.c comment_writes 13 if (slots[0] == 1) counter += 1;\n",
                "literal.c nearby 2 int counter = 0;\n",
                "literal.c nearby 4 counter++;\n",
                "literal.c quoted 8 int counter = 0;\n",
                "literal.c quoted 10 counter++;\n",
                "literal.c clean_neighbor 15 int counter = 0;\n",
                "literal.c clean_neighbor 16 counter++;\n",
            ),
        ),
        (
            "slots",
            concat!(
                "clean.c clean_writes 6 slots[++counter] = 1;\n",
                "comment.c comment_writes 12 slots[++counter] = 1;\n",
                "literal.c nearby 5 slots[counter] = 1;\n",
                "literal.c quoted 11 slots[counter] = 1;\n",
                "literal.c clean_neighbor 17 slots[counter] = 1;\n",
            ),
        ),
        (
            "clean_table",
            "clean.c <global> 1 const signed char clean_table[2] = {1, 2};\n",
        ),
        (
            "comment_table",
            "comment.c <global> 4 const signed char comment_table[2] = {1, 2};\n",
        ),
    ];
    for newline in ["\n", "\r\n"] {
        let mut files = files.clone();
        for source in files.as_object_mut().unwrap().values_mut() {
            *source = source.as_str().unwrap().replace('\n', newline).into();
        }
        let profiles: Vec<_> = [false, true]
            .into_iter()
            .map(|caseless| {
                let queries: Vec<_> = expectations
                    .iter()
                    .map(|(name, expected)| {
                        let args = if caseless {
                            vec!["-dL", "-C", "-9", name]
                        } else {
                            vec!["-dL", "-9", name]
                        };
                        serde_json::json!({"args": args, "stdout": expected})
                    })
                    .collect();
                serde_json::json!({"caseless": caseless, "queries": queries})
            })
            .collect();
        compare_fixture(
            "clipped assignment windows",
            &serde_json::json!({
                "files": files, "seeds": ["clean.c", "comment.c", "literal.c"], "profiles": profiles
            })
            .to_string(),
        );
    }
}

fn compare_fixture(name: &str, fixture: &str) {
    let fixture: serde_json::Value = serde_json::from_str(fixture).unwrap();
    let dir = temp();
    let root = dir.path();
    for (name, source) in fixture["files"].as_object().unwrap() {
        let path = root.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, source.as_str().unwrap()).unwrap();
    }
    let mut args = vec!["-bk", "-f", "index"];
    args.extend(
        fixture["seeds"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s.as_str().unwrap()),
    );
    let build = run(root, &args);
    assert!(build.status.success(), "{:?}", build);
    for name in fixture["files"].as_object().unwrap().keys() {
        fs::remove_file(root.join(name)).unwrap();
    }
    let mut input = Vec::new();
    let mut expected = b">> ".to_vec();
    for profile in fixture["profiles"].as_array().unwrap() {
        if profile["caseless"].as_bool().unwrap() {
            input.extend_from_slice(b"c\n");
            expected.extend_from_slice(b">> ");
        }
        for query in profile["queries"].as_array().unwrap() {
            let mut args = vec!["-d", "-f", "index"];
            args.extend(
                query["args"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|a| a.as_str().unwrap()),
            );
            let result = run(root, &args);
            let pattern = args.last().unwrap();
            writeln!(input, "9{pattern}").unwrap();
            let recorded = query["stdout"].as_str().unwrap();
            let key = args[3..].join(" ");
            let body = &intended::expected(name, &key, recorded);
            intended::check(
                name,
                &key,
                recorded,
                &String::from_utf8_lossy(&result.stdout),
            );
            if body.is_empty() {
                expected.extend_from_slice(b"Unable to search database\n>> ");
            } else {
                write!(
                    expected,
                    "cscope: {} lines\n{body}>> ",
                    body.lines().count()
                )
                .unwrap();
            }
            if matches!(*pattern, "[" | "" | " x") {
                // As in cscope, a pattern that cannot be used finds nothing and the query
                // succeeds; ripscope says why on standard error.
                assert!(result.status.success(), "{args:?}: {result:?}");
                assert!(!result.stderr.is_empty());
            } else {
                assert!(result.status.success(), "{args:?}: {result:?}");
                assert!(result.stderr.is_empty());
            }
        }
    }
    let mut child = Command::new(env!("CARGO_BIN_EXE_ripscope"))
        .current_dir(root)
        .args(["-dl", "-f", "index"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(&input).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{output:?}");
    // Only the reasons for the patterns that cannot be used.
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .lines()
            .all(|line| line.starts_with("ripscope: invalid query pattern")),
        "{output:?}"
    );
    assert_eq!(output.stdout, expected);
}
