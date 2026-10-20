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
fn caller_queries_match_reference_after_source_deletion() {
    compare_fixture(
        "caller-queries.json",
        include_str!("fixtures/caller-queries.json"),
    );
}

#[test]
fn one_shot_callers_refresh_normally_and_stored_queries_do_not() {
    let dir = temp();
    let root = dir.path();
    fs::write(root.join("case.c"), "int old(void) { target(); }\n").unwrap();
    assert!(
        run(root, &["-bk", "-f", "index", "case.c"])
            .status
            .success()
    );
    fs::write(root.join("case.c"), "int fresh(void) { target(); }\n").unwrap();
    let query = |mode| run(root, &[mode, "-f", "index", "-3", "target"]);
    assert!(
        String::from_utf8(query("-dL").stdout)
            .unwrap()
            .contains("case.c old 1")
    );
    let result = query("-L");
    assert!(result.status.success(), "{result:?}");
    assert!(
        String::from_utf8(result.stdout)
            .unwrap()
            .contains("case.c fresh 1")
    );
    fs::remove_file(root.join("case.c")).unwrap();
    assert!(
        String::from_utf8(query("-dL").stdout)
            .unwrap()
            .contains("case.c fresh 1")
    );
    let result = query("-L");
    assert!(!result.status.success());
    assert!(result.stdout.is_empty());
}

#[test]
fn callers_apply_approved_signature_corrections_in_both_transports() {
    let mut fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/caller-attribution.json")).unwrap();
    let target = "attribute.c f 3 return target();\nnested.c f 3 return target();\npointer.c f 3 return target();\nreturn.c f 3 target(); return 0;\nsimple.c f 3 return target();\ntwo.c f 3 return target();\nvla.c f 2 int f(int n, int a[target()]) {\nvla.c f 3 return target();\n";
    for profile in fixture["profiles"].as_array_mut().unwrap() {
        for query in profile["queries"].as_array_mut().unwrap() {
            let pattern = query["args"]
                .as_array()
                .unwrap()
                .last()
                .unwrap()
                .as_str()
                .unwrap();
            query["stdout"] = match pattern {
                "target" => target.into(),
                "callback" => "callback.c f 2 return callback(1);\n".into(),
                ".*" => target
                    .replacen(
                        "nested.c",
                        "callback.c f 2 return callback(1);\nnested.c",
                        1,
                    )
                    .into(),
                _ => "".into(),
            };
        }
    }
    compare_fixture("generated", &fixture.to_string());
}

#[test]
fn callers_keep_annotation_arguments_and_lexical_body_roles() {
    for newline in ["\n", "\r\n"] {
        let dir = temp();
        let root = dir.path();
        let source = "int target(void);\n__attribute__((aligned(target()))) int prefix(void) { return target(); }\nint ZEXPORT adapted(void) __attribute__((aligned(target()))) { return target(); }\nint callback_owner(int callback(int), int a[target()]) {\n int prototype(int); prototype(); callback(1);\n #define LOCAL(x) target(x)\n return target();\n}\nint first(void) { target(); target(); } int second(void) { target(); }\nint raw(void) { target(); TARGET(); }\n".replace('\n', newline);
        fs::write(root.join("case.c"), source).unwrap();
        let build = run(root, &["-bk", "-f", "index", "case.c"]);
        assert!(build.status.success(), "{build:?}");
        fs::remove_file(root.join("case.c")).unwrap();
        for pattern in [
            "__attribute__",
            "aligned",
            "prefix",
            "adapted",
            "callback_owner",
        ] {
            let result = run(root, &["-dL", "-f", "index", "-3", pattern]);
            assert!(result.status.success(), "{result:?}");
            assert!(result.stdout.is_empty(), "{pattern}: {result:?}");
        }
        let result = run(root, &["-dL", "-f", "index", "-3", "target"]);
        assert!(result.status.success(), "{result:?}");
        let output = String::from_utf8(result.stdout).unwrap();
        let contexts: Vec<_> = output
            .lines()
            .map(|line| {
                let words: Vec<_> = line.splitn(4, ' ').collect();
                (words[1], words[2])
            })
            .collect();
        assert_eq!(
            contexts,
            [
                ("prefix", "2"),
                ("adapted", "3"),
                ("callback_owner", "4"),
                ("LOCAL", "6"),
                ("callback_owner", "7"),
                ("first", "9"),
                ("second", "9"),
                ("raw", "10")
            ]
        );
        let result = run(root, &["-dL", "-C", "-f", "index", "-3", "target"]);
        assert_eq!(String::from_utf8(result.stdout).unwrap().lines().count(), 9);
        for pattern in ["prototype", "callback"] {
            let result = run(root, &["-dL", "-f", "index", "-3", pattern]);
            let output = String::from_utf8(result.stdout).unwrap();
            assert!(
                output.starts_with("case.c callback_owner 5 "),
                "{pattern}: {output}"
            );
            assert_eq!(output.lines().count(), 1);
        }
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
            writeln!(input, "3{pattern}").unwrap();
            let recorded = query["stdout"].as_str().unwrap();
            let key = args[3..].join(" ");
            let body = &intended::expected(name, &key, recorded);
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
            assert_eq!(result.stdout, body.as_bytes(), "{args:?}");
            if matches!(*pattern, "[" | "" | " target") {
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
