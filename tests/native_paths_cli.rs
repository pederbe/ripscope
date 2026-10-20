use serde_json::Value;
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

fn success(output: Output) -> Vec<Value> {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty(), "{:?}", output.stderr);
    output
        .stdout
        .split(|b| *b == b'\n')
        .filter(|line| !line.is_empty())
        .map(|line| serde_json::from_slice(line).unwrap())
        .collect()
}

const SOURCE: &str = "#include \"dep.h\"\nint value = 1;\nint callee(void) { return value; }\nint caller(void) { value = callee(); return value; }\n";
const CASES: &[(&str, &str)] = &[
    ("0", "value"),
    ("1", "caller"),
    ("2", "caller"),
    ("3", "callee"),
    ("4", "value"),
    ("6", "value"),
    ("7", "main"),
    ("8", "dep.h"),
    ("9", "value"),
];

#[test]
fn ordinary_rows_keep_legacy_content_for_every_field() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("main.c"), SOURCE).unwrap();
    fs::write(root.path().join("dep.h"), "int header_symbol;\n").unwrap();
    assert!(
        run(root.path(), &["-bk", "-f", "index", "main.c"], b"")
            .status
            .success()
    );
    for (field, pattern) in CASES {
        let flag = format!("-{field}");
        let legacy = run(root.path(), &["-dL", "-f", "index", &flag, pattern], b"");
        assert!(legacy.status.success());
        let rows = success(run(
            root.path(),
            &["--json", "-dL", "-f", "index", &flag, pattern],
            b"",
        ));
        let mut plain = Vec::new();
        for row in rows {
            writeln!(
                plain,
                "{} {} {} {}",
                row[0].as_str().unwrap(),
                row[1].as_str().unwrap(),
                row[2],
                row[3].as_str().unwrap()
            )
            .unwrap();
        }
        assert_eq!(plain, legacy.stdout, "field {field}");
    }
}

#[test]
fn native_paths_round_trip_every_field_and_transport_with_refresh_and_source_deletion() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project café 漢字");
    fs::create_dir_all(root.join("include space Ω")).unwrap();
    fs::write(root.join("include space Ω/dep.h"), "int header_symbol;\n").unwrap();
    fs::write(root.join("main ü.c"), SOURCE).unwrap();
    fs::write(
        root.join("files ü.txt"),
        "-I \"include space Ω\"\n\"main ü.c\"\n",
    )
    .unwrap();
    let index = root.join("index café");
    let index = index.to_str().unwrap();
    let build = run(&root, &["-bk", "-f", index, "-i", "files ü.txt"], b"");
    assert!(
        build.status.success(),
        "{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let root_arg = root.to_str().unwrap();
    let mut requests = Vec::new();
    let mut expected = Vec::new();
    for (field, pattern) in CASES {
        let flag = format!("-{field}");
        let rows = success(run(
            temp.path(),
            &["--json", "-dL", "-f", index, "-P", root_arg, &flag, pattern],
            b"",
        ));
        assert!(!rows.is_empty(), "field {field}");
        for row in &rows {
            assert_eq!(row.as_array().unwrap().len(), 4);
            let path = root.join(row[0].as_str().unwrap());
            assert_eq!(path, root.join("main ü.c"), "field {field}");
            let line = usize::try_from(row[2].as_u64().unwrap()).unwrap();
            assert!(line > 0);
            let source = fs::read(&path).unwrap();
            assert!(
                source
                    .split(|byte| *byte == b'\n')
                    .nth(line - 1)
                    .is_some_and(|text| !text.is_empty()),
                "{}:{line}",
                path.display()
            );
        }
        expected.push(rows);
        writeln!(requests, "{field}{pattern}").unwrap();
    }
    requests.extend_from_slice(b"7Missing\nq\n");
    let frames = success(run(
        temp.path(),
        &["--json", "-dl", "-f", index, "-P", root_arg],
        &requests,
    ));
    let mut frames = frames.iter();
    assert_eq!(frames.next().unwrap()["type"], "ready");
    for rows in &expected {
        let header = frames.next().unwrap();
        assert_eq!(header["type"], "result");
        assert_eq!(header["count"].as_u64().unwrap() as usize, rows.len());
        for row in rows {
            assert_eq!(frames.next().unwrap(), row);
        }
        assert_eq!(frames.next().unwrap()["type"], "ready");
    }
    assert_eq!(frames.next().unwrap()["count"], 0);
    assert_eq!(frames.next().unwrap()["type"], "ready");
    assert!(frames.next().is_none());

    let unicode = success(run(&root, &["--json", "-dL", "-f", index, "-7", "ü"], b""));
    assert_eq!(unicode[0][0], "main ü.c");
    // As cscope, the line protocol prints a path with blanks and non-ASCII as it is.
    let line = run(&root, &["-dL", "-f", index, "-1", "caller"], b"");
    assert!(line.status.success(), "{line:?}");
    assert!(
        String::from_utf8_lossy(&line.stdout).starts_with("main ü.c caller "),
        "{line:?}"
    );

    fs::write(root.join("main ü.c"), format!("{SOURCE}int Fresh;\n")).unwrap();
    let updated = success(run(
        temp.path(),
        &["--json", "-L", "-f", index, "-1", "Fresh"],
        b"",
    ));
    assert_eq!(updated.len(), 1);
    fs::remove_file(root.join("main ü.c")).unwrap();
    fs::remove_dir_all(root.join("include space Ω")).unwrap();
    fs::remove_file(root.join("files ü.txt")).unwrap();
    for (field, pattern) in CASES.iter().filter(|(f, _)| !["4", "6"].contains(f)) {
        let flag = format!("-{field}");
        let rows = success(run(
            temp.path(),
            &["--json", "-dL", "-f", index, &flag, pattern],
            b"",
        ));
        assert!(!rows.is_empty(), "field {field}");
        let frames = success(run(
            temp.path(),
            &["--json", "-dl", "-f", index],
            format!("{field}{pattern}\nq\n").as_bytes(),
        ));
        assert_eq!(frames[0]["type"], "ready");
        assert_eq!(
            frames[1],
            serde_json::json!({"type": "result", "count": rows.len()})
        );
        assert_eq!(frames[2..2 + rows.len()], rows);
        assert_eq!(frames[2 + rows.len()]["type"], "ready");
        assert_eq!(frames.len(), rows.len() + 3);
    }
}

#[test]
fn json_live_text_preserves_non_utf8_bytes_and_cr_and_recovers_from_missing_sources() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("a.c"), "int value;\n").unwrap();
    assert!(
        run(root.path(), &["-bk", "-f", "index", "a.c"], b"")
            .status
            .success()
    );
    fs::write(root.path().join("a.c"), b"needle\xff\r\nneedle UTF8\r\n").unwrap();
    for field in ["-4", "-6"] {
        let rows = success(run(
            root.path(),
            &["--json", "-dL", "-f", "index", field, "needle"],
            b"",
        ));
        assert_eq!(
            rows[0][3],
            serde_json::json!([110, 101, 101, 100, 108, 101, 255, 13])
        );
        assert_eq!(rows[1][3], "needle UTF8\r");
        let frames = success(run(
            root.path(),
            &["--json", "-dl", "-f", "index"],
            format!("{}needle\nq\n", &field[1..]).as_bytes(),
        ));
        assert_eq!(frames[1], serde_json::json!({"type": "result", "count": 2}));
        assert_eq!(frames[2..4], rows);
        assert_eq!(frames[4]["type"], "ready");
    }
    fs::remove_file(root.path().join("a.c")).unwrap();
    let frames = success(run(
        root.path(),
        &["--json", "-dl", "-f", "index"],
        b"4needle\n1value\nq\n",
    ));
    // The file that cannot be read is skipped, with a warning frame after the result.
    assert_eq!(frames[1], serde_json::json!({"type": "result", "count": 0}));
    assert_eq!(frames[2]["type"], "warning");
    assert!(
        frames[2]["message"]
            .as_str()
            .unwrap()
            .starts_with("cannot read a.c: ")
    );
    assert_eq!(frames[3]["type"], "ready");
    assert_eq!(frames[4]["count"], 1);
}

#[test]
fn json_warnings_accompany_available_rows_and_do_not_leak_into_the_next_answer() {
    let root = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join("good.c"),
        "int good(void) { return helper(); }\n",
    )
    .unwrap();
    fs::write(
        root.path().join("gone.c"),
        "int helper(void) { return 1; }\n",
    )
    .unwrap();
    assert!(
        run(
            root.path(),
            &["-bk", "-f", "index", "good.c", "gone.c"],
            b""
        )
        .status
        .success()
    );
    fs::remove_file(root.path().join("gone.c")).unwrap();
    let output = run(
        root.path(),
        &["--json", "-dL", "-f", "index", "-4", "helper"],
        b"",
    );
    assert!(output.status.success(), "{output:?}");
    let warning = String::from_utf8(output.stderr).unwrap();
    assert!(
        warning.starts_with("ripscope: cannot read gone.c: "),
        "{warning}"
    );
    let rows: Vec<Value> = serde_json::Deserializer::from_slice(&output.stdout)
        .into_iter::<Value>()
        .map(Result::unwrap)
        .collect();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0][0], "good.c");
    let clean = success(run(
        root.path(),
        &["--json", "-dL", "-f", "index", "-1", "good"],
        b"",
    ));
    assert_eq!(clean.len(), 1);
    let frames = success(run(
        root.path(),
        &["--json", "-dl", "-f", "index"],
        b"4helper\n1good\nq\n",
    ));
    assert_eq!(frames[0]["type"], "ready");
    assert_eq!(frames[1], serde_json::json!({"type": "result", "count": 1}));
    assert_eq!(frames[2], rows[0]);
    assert_eq!(frames[3]["type"], "warning");
    assert!(
        frames[3]["message"]
            .as_str()
            .unwrap()
            .starts_with("cannot read gone.c: ")
    );
    assert_eq!(frames[4]["type"], "ready");
    assert_eq!(frames[5], serde_json::json!({"type": "result", "count": 1}));
    assert_eq!(frames[6], clean[0]);
    assert_eq!(frames[7]["type"], "ready");
    assert_eq!(frames.len(), 8);
}

#[test]
fn native_framing_distinguishes_invalid_empty_toggle_and_rebuild() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("a.c"), "int Alpha;\n").unwrap();
    assert!(
        run(root.path(), &["-bk", "-f", "index", "a.c"], b"")
            .status
            .success()
    );
    fs::write(root.path().join("a.c"), "int Alpha;\nint Fresh;\n").unwrap();
    let frames = success(run(
        root.path(),
        &["--json", "-dl", "-f", "index"],
        b"1[\n1Missing\nc\n1alpha\n1Fresh\nr\n1Fresh\nq\n",
    ));
    assert_eq!(frames[1]["type"], "error");
    assert_eq!(frames[3], serde_json::json!({"type":"result","count":0}));
    assert_eq!(frames[6]["count"], 1);
    assert_eq!(frames[9], serde_json::json!({"type":"result","count":0}));
    assert_eq!(frames[11]["type"], "rebuilt");
    assert_eq!(frames[13], serde_json::json!({"type":"result","count":1}));
    assert_eq!(frames[14][0], "a.c");
    assert_eq!(frames[14][2], 2);
    assert_eq!(frames[15]["type"], "ready");
    assert_eq!(frames.len(), 16);
    for args in [
        vec!["--json", "-bk", "-f", "other", "a.c"],
        vec!["--json", "--vim-errors", "-dl", "-f", "index"],
    ] {
        let result = run(root.path(), &args, b"");
        assert!(!result.status.success());
        assert!(result.stdout.is_empty());
    }
}

#[cfg(windows)]
#[test]
fn windows_absolute_drive_paths_and_backslashes_survive_native_receiving() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("drive ü space.c");
    fs::write(&source, "int Alpha;\n").unwrap();
    let index = root.path().join("index ü space");
    assert!(
        run(
            root.path(),
            &[
                "-bk",
                "-f",
                index.to_str().unwrap(),
                source.to_str().unwrap()
            ],
            b""
        )
        .status
        .success()
    );
    let rows = success(run(
        root.path(),
        &[
            "--json",
            "-dL",
            "-f",
            index.to_str().unwrap(),
            "-1",
            "Alpha",
        ],
        b"",
    ));
    assert_eq!(Path::new(rows[0][0].as_str().unwrap()), source);
    assert!(!rows[0][0].as_str().unwrap().contains('\\'));
}

#[cfg(unix)]
#[test]
fn posix_quotes_backslashes_and_control_characters_round_trip() {
    let root = tempfile::tempdir().unwrap();
    let name = "quote\" slash\\ tab\t line\n ü.c";
    fs::write(root.path().join(name), "int Alpha;\n").unwrap();
    assert!(
        run(root.path(), &["-bk", "-f", "index", name], b"")
            .status
            .success()
    );
    let rows = success(run(
        root.path(),
        &["--json", "-dL", "-f", "index", "-1", "Alpha"],
        b"",
    ));
    assert_eq!(rows[0][0], name);
}
