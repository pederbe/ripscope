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

#[test]
fn complete_include_comments_preserve_discovery_and_stored_navigation() {
    for newline in ["\n", "\r\n"] {
        for tail in [
            " /* é ∈ 🦀 */\n",
            " /* é\n∈ 🦀 */\n",
            " /* first second */\n",
            " /* first\nsecond */\n",
            " /* first\nsecond */ /* third\nfourth */\n",
            " /* first\n#include \"ghost.h\"\n#endif\nlast */\n",
            " /* first\\\nsecond\nlast */\n",
        ] {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path();
            let source = format!("#include \"dep.h\"{tail}int next;\n").replace('\n', newline);
            let line = source.lines().count();
            fs::write(root.join("case.c"), &source).unwrap();
            fs::write(root.join("dep.h"), "int dependency;\n").unwrap();
            let build = run(root, &["-bk", "-f", "index", "case.c"], b"");
            assert!(
                build.status.success(),
                "{tail:?}: {}",
                String::from_utf8_lossy(&build.stderr)
            );
            fs::remove_file(root.join("case.c")).unwrap();
            fs::remove_file(root.join("dep.h")).unwrap();
            for (field, pattern, expected) in [
                (
                    "-8",
                    "dep.h",
                    "case.c <global> 1 #include \"dep.h\"\n".to_owned(),
                ),
                ("-1", "next", format!("case.c next {line} int next;\n")),
                (
                    "-1",
                    "dependency",
                    "dep.h dependency 1 int dependency;\n".to_owned(),
                ),
            ] {
                let one = run(root, &["-dL", "-f", "index", field, pattern], b"");
                assert!(one.status.success(), "{:?}", one.stderr);
                assert_eq!(one.stdout, expected.as_bytes(), "{tail:?}");
                let request = format!("{}{pattern}\nq\n", &field[1..]);
                let persistent = run(root, &["-dl", "-f", "index"], request.as_bytes());
                assert!(persistent.status.success());
                assert_eq!(
                    persistent.stdout,
                    format!(">> cscope: 1 lines\n{expected}>> ").as_bytes()
                );
            }
        }
    }
}

#[test]
fn include_comments_keep_both_conditional_arms() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::write(root.join("case.c"), "#if FLAG\n#include \"left.h\" /* first\nend */\n#else\n#include \"right.h\" /* second\nend */\n#endif\nint next;\n").unwrap();
    fs::write(root.join("left.h"), "int left;\n").unwrap();
    fs::write(root.join("right.h"), "int right;\n").unwrap();
    let build = run(root, &["-bk", "-f", "index", "case.c"], b"");
    assert!(
        build.status.success(),
        "{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let rows = run(root, &["-dL", "-f", "index", "-8", ".*"], b"");
    assert!(rows.status.success());
    assert_eq!(
        rows.stdout,
        b"case.c <global> 2 #include \"left.h\"\ncase.c <global> 5 #include \"right.h\"\n"
    );
    for symbol in ["left", "right", "next"] {
        let rows = run(root, &["-dL", "-f", "index", "-1", symbol], b"");
        assert!(rows.status.success() && !rows.stdout.is_empty());
    }
}

#[test]
fn utf8_comments_keep_source_deleted_rows_in_both_transports_and_formats() {
    let text = "#include \"dep.h\" /* needle ∈ */\nint value; /* é */\nvoid f(void) { /* 🦀 */ target(); value=1; }\n#if A\nint left; /* ∈ */\n#else\nint right; // é\n#endif\n";
    check_utf8_source_deleted_rows(text);
}

#[test]
fn utf8_literals_keep_source_deleted_rows_in_both_transports_and_formats() {
    let text = "#include \"dep.h\"\nint value;\nvoid f(void) { target(\"\u{e9} \u{2208} \u{1f980} needle\"); value=1; }\n";
    check_utf8_source_deleted_rows(text);
}

fn check_utf8_source_deleted_rows(text: &str) {
    let mut versions = Vec::new();
    for source in [text.to_owned(), text.replace(['é', '∈', '🦀'], "x")] {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("case.c"), &source).unwrap();
        fs::write(root.join("dep.h"), "int dependency;\n").unwrap();
        let built = run(root, &["-bk", "-f", "index", "case.c"], b"");
        assert!(built.status.success(), "{:?}", built.stderr);
        for field in ["-4", "-6"] {
            let live = run(root, &["-dL", "-f", "index", field, "needle"], b"");
            assert!(live.status.success());
            assert!(
                String::from_utf8(live.stdout)
                    .unwrap()
                    .contains(source.lines().find(|line| line.contains("needle")).unwrap())
            );
        }
        fs::remove_file(root.join("case.c")).unwrap();
        fs::remove_file(root.join("dep.h")).unwrap();
        let mut outputs = Vec::new();
        for json in [false, true] {
            for caseless in [false, true] {
                let mut prefix = vec!["-f", "index"];
                if json {
                    prefix.push("--json");
                }
                if caseless {
                    prefix.push("-C");
                }
                for field in ["-0", "-1", "-2", "-3", "-8", "-9"] {
                    let mut args = prefix.clone();
                    args.extend(["-dL", field, ".*"]);
                    let output = run(root, &args, b"");
                    assert!(output.status.success(), "{:?}", output.stderr);
                    assert!(!output.stdout.is_empty());
                    outputs.push(
                        String::from_utf8(output.stdout)
                            .unwrap()
                            .replace(['\u{e9}', '\u{2208}', '\u{1f980}'], "x")
                            .into_bytes(),
                    );
                }
                prefix.push("-dl");
                let output = run(root, &prefix, b"0.*\n1.*\n2.*\n3.*\n8.*\n9.*\nq\n");
                assert!(output.status.success(), "{:?}", output.stderr);
                outputs.push(
                    String::from_utf8(output.stdout)
                        .unwrap()
                        .replace(['\u{e9}', '\u{2208}', '\u{1f980}'], "x")
                        .into_bytes(),
                );
            }
        }
        versions.push(outputs);
    }
    assert_eq!(versions[0], versions[1]);
}
