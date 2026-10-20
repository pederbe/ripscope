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
fn directory_refreshes_membership_at_saved_root_and_preserves_stored_reads() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("sources");
    fs::create_dir(&root).unwrap();
    let index = temp.path().join("index");
    let index = index.to_str().unwrap();
    fs::write(root.join("b.c"), "int Beta;\n").unwrap();
    fs::write(root.join("a.c"), "int Alpha;\n").unwrap();
    fs::create_dir(root.join("nested")).unwrap();
    fs::write(root.join("nested/ignored.c"), "int Ignored;\n").unwrap();
    success(run(&root, &["-bk", "-f", index], b""), b"");
    let query = ["-L", "-f", index, "-1", ".*"];
    success(
        run(temp.path(), &query, b""),
        b"a.c .* 1 int Alpha;\nb.c .* 1 int Beta;\n",
    );
    fs::remove_file(root.join("a.c")).unwrap();
    fs::rename(root.join("b.c"), root.join("renamed.c")).unwrap();
    fs::write(root.join("new.h"), "int New;\n").unwrap();
    success(
        run(temp.path(), &query, b""),
        b"new.h .* 1 int New;\nrenamed.c .* 1 int Beta;\n",
    );
    fs::write(root.join("new.h"), "int Changed;\n").unwrap();
    success(
        run(temp.path(), &["-dl", "-f", index], b"r\n1Changed\nq\n"),
        b">> \n>> cscope: 1 lines\nnew.h Changed 1 int Changed;\n>> ",
    );
    fs::rename(&root, temp.path().join("gone")).unwrap();
    success(
        run(temp.path(), &["-dL", "-f", index, "-1", "Changed"], b""),
        b"new.h Changed 1 int Changed;\n",
    );
    success(
        run(temp.path(), &["-dl", "-f", index], b"1Changed\nq\n"),
        b">> cscope: 1 lines\nnew.h Changed 1 int Changed;\n>> ",
    );
    // With the directory the index was built in gone, nothing can be indexed: the update fails and the
    // index keeps what it had.
    let failed = run(temp.path(), &query, b"");
    assert!(!failed.status.success());
    assert!(failed.stdout.is_empty());
    assert!(String::from_utf8_lossy(&failed.stderr).ends_with("ripscope: no source files found\n"));
}

#[test]
fn directory_uses_cscope_filename_rules_without_recursing() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    // Distinct basenames also exercise case-sensitive suffixes on Windows.
    let included = [
        "a.c",
        "b.h",
        "c.l",
        "d.y",
        "e.C",
        "f.G",
        "g.H",
        "h.L",
        "i.bp",
        "j.qc",
        "k.qh",
        "l.sd",
        "m.cc",
        "n.hh",
        "o.tcc",
        "p.cpp",
        "q.cxx",
        "r.hpp",
        "t.hxx",
        ".c",
        ".hidden.h",
        "s.c",
    ];
    for name in included {
        fs::write(root.join(name), "int Symbol;\n").unwrap();
    }
    for name in [
        "bad.Y", "bad.CC", "bad.rs", "bad.c++", "s.old.c", "S.old.h", "Makefile", "last.",
    ] {
        fs::write(root.join(name), "not valid C").unwrap();
    }
    fs::create_dir(root.join("directory.c")).unwrap();
    fs::write(root.join("directory.c/child.c"), "int Nested;\n").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink("a.c", root.join("linked.c")).unwrap();
    success(run(root, &["-bk", "-f", "index"], b""), b"");
    let mut names = included.to_vec();
    names.sort();
    let expected: String = names
        .iter()
        .map(|n| format!("{n} <unknown> 1 <unknown>\n"))
        .collect();
    success(
        run(root, &["-dL", "-f", "index", "-7", ".*"], b""),
        expected.as_bytes(),
    );
}

#[test]
fn automatic_list_precedence_and_include_directories_are_resolved_each_refresh() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    fs::write(root.join("a.c"), "int Alpha;\n").unwrap();
    fs::write(root.join("b.c"), "int Beta;\n").unwrap();
    success(run(root, &["-bk", "-f", "index"], b""), b"");
    fs::write(root.join("cscope.files"), "b.c b.c\n").unwrap();
    success(
        run(root, &["-L", "-f", "index", "-1", ".*"], b""),
        b"b.c .* 1 int Beta;\nb.c .* 1 int Beta;\n",
    );
    success(run(root, &["-bk", "-f", "explicit", "a.c"], b""), b"");
    success(
        run(root, &["-dL", "-f", "explicit", "-1", ".*"], b""),
        b"a.c .* 1 int Alpha;\n",
    );
    fs::write(root.join("chosen"), "a.c\n").unwrap();
    success(
        run(root, &["-bk", "-f", "named", "-i", "chosen", "b.c"], b""),
        b"",
    );
    success(
        run(root, &["-dL", "-f", "named", "-1", ".*"], b""),
        b"a.c .* 1 int Alpha;\n",
    );
    for (dir, symbol) in [("one", "One"), ("two", "Two")] {
        fs::create_dir(root.join(dir)).unwrap();
        fs::write(root.join(dir).join("choice.h"), format!("int {symbol};\n")).unwrap();
    }
    fs::write(root.join("a.c"), "#include <choice.h>\nint Alpha;\n").unwrap();
    for (dir, symbol) in [("one", "One"), ("two", "Two")] {
        fs::write(root.join("cscope.files"), format!("-I{dir} a.c\n")).unwrap();
        success(
            run(root, &["-L", "-f", "index", "-1", ".*"], b""),
            format!("a.c .* 2 int Alpha;\n{dir}/choice.h .* 1 int {symbol};\n").as_bytes(),
        );
    }
    fs::remove_file(root.join("cscope.files")).unwrap();
    fs::write(root.join("a.c"), "int Back;\n").unwrap();
    success(
        run(root, &["-L", "-f", "index", "-1", ".*"], b""),
        b"a.c .* 1 int Back;\nb.c .* 1 int Beta;\n",
    );
}

#[test]
fn automatic_selection_failures_preserve_the_published_revision() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    fs::write(root.join("a.c"), "int Before;\n").unwrap();
    success(run(root, &["-bk", "-f", "index"], b""), b"");
    fs::write(root.join("a.c"), "int After;\n").unwrap();
    // An empty list names nothing, and an unknown option makes it unreadable: the update
    // fails and the index keeps what it had.
    for (list, reason) in [
        ("", "ripscope: no source files found\n"),
        ("-Z a.c\n", "ripscope: file list cscope.files: "),
    ] {
        fs::write(root.join("cscope.files"), list).unwrap();
        let failed = run(root, &["-L", "-f", "index", "-1", ".*"], b"");
        assert!(!failed.status.success());
        assert!(failed.stdout.is_empty());
        assert!(
            String::from_utf8_lossy(&failed.stderr).starts_with(reason),
            "{failed:?}"
        );
        success(
            run(root, &["-dL", "-f", "index", "-1", "Before"], b""),
            b"a.c Before 1 int Before;\n",
        );
    }
    // A cscope.files that is a directory passes cscope's read check and names nothing.
    fs::remove_file(root.join("cscope.files")).unwrap();
    fs::create_dir(root.join("cscope.files")).unwrap();
    assert!(
        !run(root, &["-L", "-f", "index", "-1", ".*"], b"")
            .status
            .success()
    );
    fs::remove_dir(root.join("cscope.files")).unwrap();
    // As cscope, a listed file that is missing is reported and the rest is indexed.
    fs::write(root.join("cscope.files"), "a.c missing.c\n").unwrap();
    let partial = run(root, &["-L", "-f", "index", "-1", ".*"], b"");
    assert!(partial.status.success(), "{partial:?}");
    assert_eq!(partial.stdout, b"a.c .* 1 int After;\n");
    assert_eq!(
        String::from_utf8_lossy(&partial.stderr),
        "ripscope: cannot find file missing.c\n"
    );
    fs::remove_file(root.join("cscope.files")).unwrap();
    // No source left: a session's `r` fails, and the session answers from the index.
    fs::remove_file(root.join("a.c")).unwrap();
    let failed = run(root, &["-dl", "-f", "index"], b"r\n1After\n");
    assert!(failed.status.success());
    assert_eq!(
        failed.stdout,
        b">> \n>> cscope: 1 lines\na.c After 1 int After;\n>> "
    );
    assert_eq!(
        String::from_utf8_lossy(&failed.stderr),
        "ripscope: no source files found\n"
    );
    success(
        run(root, &["-dL", "-f", "index", "-1", "After"], b""),
        b"a.c After 1 int After;\n",
    );
    // An index build that finds nothing makes no index.
    assert!(!run(root, &["-bk", "-f", "empty"], b"").status.success());
    assert!(!root.join("empty").exists());
}

#[cfg(unix)]
#[test]
fn file_names_are_bytes_and_an_unreadable_source_keeps_its_last_facts() {
    use std::os::unix::{ffi::OsStringExt, fs::PermissionsExt};
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    fs::write(root.join("a.c"), "int Before;\n").unwrap();
    fs::write(
        root.join(std::ffi::OsString::from_vec(b"bad\xff.txt".to_vec())),
        "ignored",
    )
    .unwrap();
    success(run(root, &["-bk", "-f", "index"], b""), b"");
    // A name that is not UTF-8 is a path like any other, printed as it is.
    let bad = root.join(std::ffi::OsString::from_vec(b"bad\xff.c".to_vec()));
    fs::write(&bad, "int Bad;\n").unwrap();
    success(
        run(root, &["-L", "-f", "index", "-1", ".*"], b""),
        b"a.c .* 1 int Before;\nbad\xff.c .* 1 int Bad;\n",
    );
    fs::remove_file(bad).unwrap();
    // A file that cannot be read keeps its last facts, with a warning (docs/spec.md, "Keep
    // going instead of failing").
    let path = root.join("a.c");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o0)).unwrap();
    if fs::File::open(&path).is_err() {
        let kept = run(root, &["-L", "-f", "index", "-1", ".*"], b"");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(kept.status.success(), "{kept:?}");
        assert_eq!(kept.stdout, b"a.c .* 1 int Before;\n");
        assert!(String::from_utf8_lossy(&kept.stderr).starts_with("ripscope: cannot read a.c: "));
    } else {
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    }
    success(
        run(root, &["-dL", "-f", "index", "-1", "Before"], b""),
        b"a.c Before 1 int Before;\n",
    );
}
