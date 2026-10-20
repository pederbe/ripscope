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
fn recursive_membership_refreshes_at_saved_root_and_keeps_mode() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    fs::create_dir_all(root.join("nested/deep")).unwrap();
    fs::write(root.join("a.c"), "int Alpha;\n").unwrap();
    fs::write(root.join("nested/deep/b.c"), "int Beta;\n").unwrap();
    let index = temp.path().join("index");
    let index = index.to_str().unwrap();
    success(run(&root, &["-bkR", "-f", index], b""), b"");
    let query = ["-L", "-f", index, "-1", ".*"];
    success(
        run(temp.path(), &query, b""),
        b"a.c .* 1 int Alpha;\nnested/deep/b.c .* 1 int Beta;\n",
    );
    fs::remove_file(root.join("a.c")).unwrap();
    fs::rename(root.join("nested/deep/b.c"), root.join("nested/renamed.c")).unwrap();
    fs::write(root.join("nested/new.h"), "int New;\n").unwrap();
    success(
        run(temp.path(), &query, b""),
        b"nested/new.h .* 1 int New;\nnested/renamed.c .* 1 int Beta;\n",
    );
    fs::write(root.join("cscope.files"), "nested/new.h\n").unwrap();
    success(
        run(temp.path(), &query, b""),
        b"nested/new.h .* 1 int New;\n",
    );
    fs::remove_file(root.join("cscope.files")).unwrap();
    fs::write(root.join("nested/renamed.c"), "int Changed;\n").unwrap();
    success(
        run(temp.path(), &["-dl", "-f", index], b"r\n1Changed\nq\n"),
        b">> \n>> cscope: 1 lines\nnested/renamed.c Changed 1 int Changed;\n>> ",
    );
    fs::rename(&root, temp.path().join("gone")).unwrap();
    success(
        run(temp.path(), &["-dL", "-f", index, "-1", "Changed"], b""),
        b"nested/renamed.c Changed 1 int Changed;\n",
    );
    assert!(!run(temp.path(), &query, b"").status.success());
}

#[test]
fn recursive_traversal_preserves_precedence_aliases_and_include_order() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    for dir in ["nested/deep", ".hidden", "dir.c", "first", "second"] {
        fs::create_dir_all(root.join(dir)).unwrap();
    }
    for (name, text) in [
        ("a.c", "int Alpha;\n"),
        ("nested/deep/b.c", "#include <choice.h>\nint Beta;\n"),
        (".hidden/c.c", "int Hidden;\n"),
        ("dir.c/d.c", "int Directory;\n"),
        ("first/choice.h", "int First;\n"),
        ("second/choice.h", "int Second;\n"),
    ] {
        fs::write(root.join(name), text).unwrap();
    }
    fs::write(root.join("nested/cscope.files"), "missing.c\n").unwrap();
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("nested", root.join("linked")).unwrap();
        std::os::unix::fs::symlink("..", root.join("nested/cycle")).unwrap();
        std::os::unix::fs::symlink("a.c", root.join("linked.c")).unwrap();
    }
    success(
        run(
            root,
            &["-bkR", "-I", "first", "-I", "second", "-f", "index"],
            b"",
        ),
        b"",
    );
    success(run(root, &["-dL", "-f", "index", "-7", ".*"], b""), b".hidden/c.c <unknown> 1 <unknown>\na.c <unknown> 1 <unknown>\ndir.c/d.c <unknown> 1 <unknown>\nfirst/choice.h <unknown> 1 <unknown>\nnested/deep/b.c <unknown> 1 <unknown>\nsecond/choice.h <unknown> 1 <unknown>\n");
    fs::write(root.join("cscope.files"), "-Isecond nested/deep/b.c\n").unwrap();
    success(
        run(root, &["-L", "-f", "index", "-1", ".*"], b""),
        b"nested/deep/b.c .* 2 int Beta;\nfirst/choice.h .* 1 int First;\n",
    );
    success(run(root, &["-bkR", "-f", "explicit", "a.c"], b""), b"");
    success(
        run(root, &["-dL", "-f", "explicit", "-1", ".*"], b""),
        b"a.c .* 1 int Alpha;\n",
    );
    fs::write(root.join("chosen"), "a.c\n").unwrap();
    success(
        run(
            root,
            &["-bkR", "-f", "named", "-i", "chosen", "missing.c"],
            b"",
        ),
        b"",
    );
    fs::remove_file(root.join("chosen")).unwrap();
    assert!(
        !run(root, &["-L", "-f", "named", "-1", ".*"], b"")
            .status
            .success()
    );
    // As cscope, -R beside a query is accepted and changes nothing.
    assert!(
        run(root, &["-dLR", "-f", "index", "-1", ".*"], b"")
            .status
            .success()
    );
}

#[cfg(unix)]
#[test]
fn an_unreadable_directory_keeps_its_files_last_facts() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    fs::create_dir(root.join("nested")).unwrap();
    fs::write(root.join("nested/a.c"), "int Before;\n").unwrap();
    fs::write(root.join("b.c"), "int Other;\n").unwrap();
    success(run(root, &["-bkR", "-f", "index"], b""), b"");
    fs::write(root.join("nested/a.c"), "int After;\n").unwrap();
    fs::set_permissions(root.join("nested"), fs::Permissions::from_mode(0o0)).unwrap();
    if fs::read_dir(root.join("nested")).is_err() {
        // The directory's files are treated as files that cannot be read: they keep their
        // last facts, with a warning (docs/spec.md, "Keep going instead of failing").
        let kept = run(root, &["-L", "-f", "index", "-1", ".*"], b"");
        fs::set_permissions(root.join("nested"), fs::Permissions::from_mode(0o700)).unwrap();
        assert!(kept.status.success(), "{kept:?}");
        assert_eq!(
            kept.stdout,
            b"b.c .* 1 int Other;\nnested/a.c .* 1 int Before;\n"
        );
        assert!(
            String::from_utf8_lossy(&kept.stderr).contains("cannot read nested"),
            "{kept:?}"
        );
    }
    fs::set_permissions(root.join("nested"), fs::Permissions::from_mode(0o700)).unwrap();
    success(
        run(root, &["-L", "-f", "index", "-1", ".*"], b""),
        b"b.c .* 1 int Other;\nnested/a.c .* 1 int After;\n",
    );
}
