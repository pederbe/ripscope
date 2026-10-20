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

/// Reference rows as ripscope shows them: cscope prints control bytes in a row's text
/// raw, ripscope as `\xHH` (docs/spec.md, "Printing rows").
fn shown(rows: &str) -> String {
    rows.chars()
        .map(|c| {
            if c != '\n' && c.is_ascii_control() {
                format!("\\x{:02x}", u32::from(c))
            } else {
                c.to_string()
            }
        })
        .collect()
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
fn include_queries_match_reference_after_source_deletion() {
    for fixture in [
        include_str!("fixtures/include-queries.json"),
        include_str!("fixtures/include-normalization.json"),
        include_str!("fixtures/include-admitted-boundaries.json"),
    ] {
        compare_fixture(fixture);
    }
}

fn compare_fixture(fixture: &str) {
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
            writeln!(input, "8{pattern}").unwrap();
            let body = shown(query["stdout"].as_str().unwrap());
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
            if matches!(*pattern, "[" | "") {
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

#[test]
fn includes_refresh_but_stored_queries_keep_captured_occurrences() {
    let dir = temp();
    let root = dir.path();
    fs::write(root.join("case.c"), "#include <old.h>\n").unwrap();
    assert!(
        run(root, &["-bk", "-f", "index", "case.c"])
            .status
            .success()
    );
    fs::write(root.join("case.c"), "#include <new.h>\n").unwrap();
    let stored = run(root, &["-dL", "-f", "index", "-8", "."]);
    assert!(stored.status.success());
    assert_eq!(stored.stdout, b"case.c <global> 1 #include <old.h>\n");
    let fresh = run(root, &["-L", "-f", "index", "-8", "."]);
    assert!(fresh.status.success());
    assert_eq!(fresh.stdout, b"case.c <global> 1 #include <new.h>\n");
    fs::remove_file(root.join("case.c")).unwrap();
    let refused = run(root, &["-L", "-f", "index", "-8", "."]);
    assert!(!refused.status.success());
    assert!(refused.stdout.is_empty());
    assert_eq!(
        run(root, &["-dL", "-f", "index", "-8", "."]).stdout,
        fresh.stdout
    );
}

#[test]
fn computed_includes_match_reference() {
    // `#include MACRO` is accepted and not followed, as the reference does, so the index
    // matches the capture without overrides.
    let capture: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/written-computed-includes-cscope.json"
    ))
    .unwrap();
    for (name, fields) in capture["cases"].as_object().unwrap() {
        let dir = temp();
        let text = fields["0"]["files"]["case.c"].as_str().unwrap();
        fs::write(dir.path().join("case.c"), text).unwrap();
        assert!(
            run(dir.path(), &["-bk", "-f", "index", "case.c"])
                .status
                .success(),
            "{name}"
        );
        fs::remove_file(dir.path().join("case.c")).unwrap();
        for run_ in fields.as_object().unwrap().values() {
            for profile in run_["profiles"].as_array().unwrap() {
                for query in profile["queries"].as_array().unwrap() {
                    let mut args = vec!["-f", "index"];
                    args.extend(
                        query["args"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(|a| a.as_str().unwrap()),
                    );
                    let out = run(dir.path(), &args);
                    intended::check(
                        "written-computed-includes-cscope.json",
                        &format!("{name} {}", args[2..].join(" ")),
                        query["stdout"].as_str().unwrap(),
                        &String::from_utf8(out.stdout).unwrap().replace("\r\n", "\n"),
                    );
                }
            }
        }
    }
}
