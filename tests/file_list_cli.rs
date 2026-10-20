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

fn success(output: Output, expected: &[u8]) {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, expected);
    assert!(
        output.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn named_list_refreshes_membership_and_preserves_stored_queries() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    fs::create_dir(root.join("lists")).unwrap();
    fs::write(root.join("a.c"), "int Alpha;\n").unwrap();
    fs::write(root.join("b.c"), "int Beta;\n").unwrap();
    fs::write(root.join("lists/input"), "b.c\t\"a.c\"\r\na.c\n").unwrap();
    let args = [
        "-bk",
        "-f",
        "index",
        "-i",
        "lists/input",
        "ignored-missing.c",
    ];
    success(run(root, &args, b""), b"");
    let query = ["-dL", "-f", "index", "-1", ".*"];
    success(
        run(root, &query, b""),
        b"a.c .* 1 int Alpha;\na.c .* 1 int Alpha;\nb.c .* 1 int Beta;\n",
    );
    // The saved root, rather than the querying process's directory, owns selection.
    fs::rename(root.join("b.c"), root.join("renamed.c")).unwrap();
    fs::write(root.join("new.c"), "int New;\n").unwrap();
    fs::write(root.join("lists/input"), "renamed.c new.c\n").unwrap();
    let index = root.join("index");
    success(
        run(
            &root.join("lists"),
            &["-L", "-f", index.to_str().unwrap(), "-1", ".*"],
            b"",
        ),
        b"new.c .* 1 int New;\nrenamed.c .* 1 int Beta;\n",
    );
    fs::write(root.join("lists/input"), "a.c\n").unwrap();
    fs::write(root.join("a.c"), "int Gamma;\n").unwrap();
    success(
        run(root, &["-dl", "-f", "index"], b"r\n1Gamma\nq\n"),
        b">> \n>> cscope: 1 lines\na.c Gamma 1 int Gamma;\n>> ",
    );
    fs::remove_file(root.join("lists/input")).unwrap();
    fs::remove_file(root.join("a.c")).unwrap();
    success(run(root, &query, b""), b"a.c .* 1 int Gamma;\n");
    success(
        run(root, &["-dl", "-f", "index"], b"1Gamma\nq\n"),
        b">> cscope: 1 lines\na.c Gamma 1 int Gamma;\n>> ",
    );
}

#[test]
fn list_failures_preserve_prior_publication_without_claiming_freshness() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    fs::write(root.join("a.c"), "int Before;\n").unwrap();
    fs::write(root.join("input"), "a.c\n").unwrap();
    success(run(root, &["-bk", "-f", "index", "-iinput"], b""), b"");
    fs::write(root.join("a.c"), "int After;\n").unwrap();
    // A list that names nothing, or cannot be read, fails the update; the index keeps what
    // it had.
    for bytes in [
        b"".as_slice(),
        b"\"open",
        b"-I\na.c\n",
        b"-Z a.c\n",
        b"a.c\0",
        b"\"\"",
        b"\xff",
    ] {
        fs::write(root.join("input"), bytes).unwrap();
        let failed = run(root, &["-L", "-f", "index", "-1", ".*"], b"");
        assert!(!failed.status.success(), "{bytes:?}");
        assert!(failed.stdout.is_empty());
        assert!(!failed.stderr.is_empty());
        success(
            run(root, &["-dL", "-f", "index", "-1", "Before"], b""),
            b"a.c Before 1 int Before;\n",
        );
    }
    fs::remove_file(root.join("input")).unwrap();
    let failed = run(root, &["-dl", "-f", "index"], b"r\n1Before\n");
    assert!(failed.status.success());
    assert_eq!(
        failed.stdout,
        b">> \n>> cscope: 1 lines\na.c Before 1 int Before;\n>> "
    );
    assert!(
        String::from_utf8_lossy(&failed.stderr)
            .starts_with("ripscope: cannot read the file list input: "),
        "{failed:?}"
    );
    success(
        run(root, &["-dL", "-f", "index", "-1", "Before"], b""),
        b"a.c Before 1 int Before;\n",
    );
    fs::create_dir(root.join("input")).unwrap();
    assert!(
        !run(root, &["-L", "-f", "index", "-1", ".*"], b"")
            .status
            .success()
    );
    success(
        run(root, &["-dL", "-f", "index", "-1", "Before"], b""),
        b"a.c Before 1 int Before;\n",
    );
    // As cscope, a listed file that is missing is reported and the rest is indexed.
    fs::remove_dir(root.join("input")).unwrap();
    fs::write(root.join("input"), "a.c missing.c\n").unwrap();
    let partial = run(root, &["-L", "-f", "index", "-1", ".*"], b"");
    assert!(partial.status.success(), "{partial:?}");
    assert_eq!(partial.stdout, b"a.c .* 1 int After;\n");
    assert_eq!(
        String::from_utf8_lossy(&partial.stderr),
        "ripscope: cannot find file missing.c\n"
    );
}

#[test]
fn list_include_options_are_replaced_on_refresh_and_cli_directories_win() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    for (dir, name) in [("cli", "CommandLine"), ("one", "One"), ("two", "Two")] {
        fs::create_dir(root.join(dir)).unwrap();
        fs::write(root.join(dir).join("choice.h"), format!("int {name};\n")).unwrap();
    }
    fs::write(root.join("a.c"), "#include <choice.h>\nint Alpha;\n").unwrap();
    fs::write(root.join("input"), "-k -I one a.c a.c one/choice.h\n").unwrap();
    success(run(root, &["-bk", "-f", "index", "-i", "input"], b""), b"");
    success(
        run(root, &["-dL", "-f", "index", "-1", ".*"], b""),
        b"a.c .* 2 int Alpha;\na.c .* 2 int Alpha;\none/choice.h .* 1 int One;\n",
    );
    for (list, body) in [
        ("-Itwo a.c\n", "two/choice.h .* 1 int Two;\n"),
        ("-Ione a.c\n", "one/choice.h .* 1 int One;\n"),
    ] {
        fs::write(root.join("input"), list).unwrap();
        success(
            run(root, &["-L", "-f", "index", "-1", ".*"], b""),
            format!("a.c .* 2 int Alpha;\n{body}").as_bytes(),
        );
    }
    success(
        run(
            root,
            &["-bk", "-f", "index", "-i", "input", "-I", "cli"],
            b"",
        ),
        b"",
    );
    success(
        run(root, &["-L", "-f", "index", "-1", "CommandLine"], b""),
        b"cli/choice.h CommandLine 1 int CommandLine;\n",
    );
}

#[test]
fn unsupported_list_invocations_do_not_publish() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    for args in [
        vec!["-bk", "-f", "index", "-i", "-"],
        vec!["-bk", "-f", "index", "-i", ""],
        vec!["-bk", "-f", "index", "-i", "missing"],
        vec!["-bk", "-f", "index", "-i", "a", "-i", "b"],
        vec!["-dL", "-f", "index", "-i", "input", "-1", "Alpha"],
    ] {
        let output = run(root, &args, b"");
        assert!(!output.status.success(), "{args:?}");
        assert!(output.stdout.is_empty());
        assert!(!output.stderr.is_empty());
        assert!(!root.join("index").exists());
    }
}
