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

fn check(root: &Path, field: &str, pattern: &str, expected: &str) {
    let one = run(root, &["-dL", "-f", "index", field, pattern], b"");
    assert!(one.status.success(), "{:?}", one.stderr);
    assert_eq!(one.stdout, expected.as_bytes(), "{field} {pattern}");
    let request = format!("{}{pattern}\nq\n", &field[1..]);
    let persistent = run(root, &["-dl", "-f", "index"], request.as_bytes());
    assert!(persistent.status.success(), "{:?}", persistent.stderr);
    let framed = if expected.is_empty() {
        ">> Unable to search database\n>> ".to_owned()
    } else {
        format!(
            ">> cscope: {} lines\n{expected}>> ",
            expected.lines().count()
        )
    };
    assert_eq!(persistent.stdout, framed.as_bytes(), "{field} {pattern}");
}

#[test]
fn bitfields_keep_stored_navigation_after_source_deletion() {
    for newline in ["\n", "\r\n"] {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let source = "#include \"dep.h\"\nstruct S {\nunsigned literal:4;\nunsigned symbolic:WIDTH;\n#if FLAG\nunsigned left:2;\n#else\nunsigned right:3;\n#endif\nunsigned spliced:\\\n4;\nunsigned commented /* c */ : /* w */ 4;\n};\n".replace('\n', newline);
        fs::write(root.join("case.c"), source).unwrap();
        fs::write(root.join("dep.h"), "int dependency;\n").unwrap();
        let build = run(root, &["-bk", "-f", "index", "case.c"], b"");
        assert!(
            build.status.success(),
            "{}",
            String::from_utf8_lossy(&build.stderr)
        );
        fs::remove_file(root.join("case.c")).unwrap();
        fs::remove_file(root.join("dep.h")).unwrap();
        for (name, line, text) in [
            ("literal", 3, "unsigned literal:4;"),
            ("symbolic", 4, "unsigned symbolic:WIDTH;"),
            ("left", 6, "unsigned left:2;"),
            ("right", 8, "unsigned right:3;"),
            ("spliced", 10, "unsigned spliced:\\"),
            ("commented", 12, "unsigned commented : 4;"),
        ] {
            check(root, "-1", name, &format!("case.c {name} {line} {text}\n"));
        }
        check(
            root,
            "-0",
            "WIDTH",
            "case.c <global> 4 unsigned symbolic:WIDTH;\n",
        );
        check(root, "-1", "WIDTH", "");
        check(root, "-2", "S", "");
        check(root, "-3", "WIDTH", "");
        check(root, "-9", ".*", "");
        check(
            root,
            "-8",
            "dep.h",
            "case.c <global> 1 #include \"dep.h\"\n",
        );
    }
}

#[test]
fn bitfields_expression_and_padding_roles_survive_source_deletion() {
    for newline in ["\n", "\r\n"] {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let source = "#include \"dep.h\"\nstruct S {\nunsigned :0, bits:WIDTH(T), :2, next;\nunsigned assigned:(x=4);\n#if FLAG\nunsigned left:FLAG?4:2;\n#else\nunsigned :sizeof(T), right:(unsigned)WIDTH;\n#endif\nunsigned spliced:\\\n(WIDTH+1);\nunsigned commented /* c */ : /* w */ (WIDTH+1);\n};\n".replace('\n', newline);
        fs::write(root.join("case.c"), source).unwrap();
        fs::write(root.join("dep.h"), "int dependency;\n").unwrap();
        let build = run(root, &["-bk", "-f", "index", "case.c"], b"");
        assert!(
            build.status.success(),
            "{}",
            String::from_utf8_lossy(&build.stderr)
        );
        fs::remove_file(root.join("case.c")).unwrap();
        fs::remove_file(root.join("dep.h")).unwrap();
        for (name, line, text) in [
            ("bits", 3, "unsigned :0, bits:WIDTH(T), :2, next;"),
            ("next", 3, "unsigned :0, bits:WIDTH(T), :2, next;"),
            ("assigned", 4, "unsigned assigned:(x=4);"),
            ("left", 6, "unsigned left:FLAG?4:2;"),
            ("right", 8, "unsigned :sizeof(T), right:(unsigned )WIDTH;"),
            ("spliced", 10, "unsigned spliced:\\"),
            ("commented", 12, "unsigned commented : (WIDTH+1);"),
        ] {
            check(root, "-1", name, &format!("case.c {name} {line} {text}\n"));
        }
        check(
            root,
            "-0",
            "x",
            "case.c <global> 4 unsigned assigned:(x=4);\n",
        );
        check(
            root,
            "-9",
            "x",
            "case.c <global> 4 unsigned assigned:(x=4);\n",
        );
        for name in ["WIDTH", "T", "x"] {
            check(root, "-1", name, "");
        }
        check(root, "-2", ".*", "");
        check(root, "-3", ".*", "");
        check(
            root,
            "-8",
            "dep.h",
            "case.c <global> 1 #include \"dep.h\"\n",
        );
    }
}
