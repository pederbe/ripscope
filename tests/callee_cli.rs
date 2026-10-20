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
fn callee_queries_match_reference_after_source_deletion() {
    compare_fixture(
        "callee-queries.json",
        include_str!("fixtures/callee-queries.json"),
    );
}

#[test]
fn callee_owner_boundaries_and_approved_signatures_in_both_transports() {
    for newline in ["\n", "\r\n"] {
        let source = "int empty(void) {\n#define LOCAL hidden()\n}\n#define TOP top()\nint other(void) { other_call(); }\n__attribute__((aligned(prefix_call()))) int prefix(void) { body(); }\nint f(int callback(int), int a[bound()]) {\n callback(1); target(); TARGET();\n}\n";
        let local = "case.c hidden 2 #define LOCAL hidden()\n";
        let top = "case.c top 4 #define TOP top()\n";
        let other = "case.c other_call 5 int other(void ) { other_call(); }\n";
        // ADR 0010: each occurrence has its written owner. LOCAL owns its macro
        // body, and an attribute before a function head is at file scope.
        let prefix =
            "case.c body 6 __attribute__((aligned(prefix_call()))) int prefix(void ) { body(); }\n";
        let signature = "case.c bound 7 int f(int callback(int ), int a[bound()]) {\ncase.c callback 8 callback(1); target(); TARGET();\ncase.c target 8 callback(1); target(); TARGET();\ncase.c TARGET 8 callback(1); target(); TARGET();\n";
        let queries = [
            ("empty|LOCAL", local.into()),
            ("LOCAL", local.into()),
            ("LOCAL|other", format!("{local}{other}")),
            ("empty|TOP", top.into()),
            ("prefix", prefix.into()),
            ("f", signature.into()),
            ("callback", String::new()),
            (".*", format!("{local}{top}{other}{prefix}{signature}")),
        ];
        let profiles: Vec<_> = [false, true]
            .into_iter()
            .map(|caseless| {
                let queries: Vec<_> = queries
                    .iter()
                    .map(|(pattern, stdout)| {
                        let mut args = vec!["-dL", "-2", pattern];
                        if caseless {
                            args.insert(1, "-C");
                        }
                        serde_json::json!({"args": args, "stdout": stdout})
                    })
                    .collect();
                serde_json::json!({"caseless": caseless, "queries": queries})
            })
            .collect();
        compare_fixture(
            "generated",
            &serde_json::json!({
                "files": {"case.c": source.replace('\n', newline)},
                "seeds": ["case.c"], "profiles": profiles
            })
            .to_string(),
        );
    }
}

#[test]
fn one_shot_callees_refresh_normally_and_stored_queries_do_not() {
    let dir = temp();
    let root = dir.path();
    fs::write(root.join("case.c"), "int f(void) { old(); }\n").unwrap();
    assert!(
        run(root, &["-bk", "-f", "index", "case.c"])
            .status
            .success()
    );
    fs::write(root.join("case.c"), "int f(void) { fresh(); }\n").unwrap();
    let query = |mode| run(root, &[mode, "-f", "index", "-2", "f"]);
    assert_eq!(
        query("-dL").stdout,
        b"case.c old 1 int f(void ) { old(); }\n"
    );
    let result = query("-L");
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"case.c fresh 1 int f(void ) { fresh(); }\n");
    fs::remove_file(root.join("case.c")).unwrap();
    assert_eq!(query("-dL").stdout, result.stdout);
    let result = query("-L");
    assert!(!result.status.success());
    assert!(result.stdout.is_empty());
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
            writeln!(input, "2{pattern}").unwrap();
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
            if matches!(*pattern, "[" | "" | " after") {
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
