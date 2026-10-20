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

fn query(root: &Path, field: &str, pattern: &str) -> Vec<u8> {
    let output = run(
        root,
        &["-dL", "-f", "index", &format!("-{field}"), pattern],
        b"",
    );
    assert!(output.status.success(), "{:?}", output);
    assert!(output.stderr.is_empty());
    let persistent = run(
        root,
        &["-dl", "-f", "index"],
        format!("{field}{pattern}\nq\n").as_bytes(),
    );
    assert!(persistent.status.success(), "{:?}", persistent);
    assert!(persistent.stderr.is_empty());
    let mut expected = format!(
        ">> cscope: {} lines\n",
        output.stdout.iter().filter(|&&b| b == b'\n').count()
    )
    .into_bytes();
    expected.extend_from_slice(&output.stdout);
    expected.extend_from_slice(b">> ");
    if output.stdout.is_empty() {
        expected = b">> Unable to search database\n>> ".to_vec();
    }
    assert_eq!(persistent.stdout, expected);
    output.stdout
}

#[test]
fn literal_table_queries_keep_their_rows_after_source_removal() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let mut text = String::from("const char *table[] = {\n");
    for _ in 0..410 {
        text.push_str(&"\"literal_name\",".repeat(40));
        text.push('\n');
    }
    text.push_str("};\nvoid after(void) { target(); }\n");
    fs::write(root.join("case.c"), &text).unwrap();
    let build = run(root, &["-bk", "-f", "index", "case.c"], b"");
    assert!(build.status.success(), "{:?}", build);
    assert!(build.stdout.is_empty() && build.stderr.is_empty());
    let queries = [
        ("0", "table"),
        ("1", "table"),
        ("2", "after"),
        ("3", "target"),
        ("7", "case.c"),
        ("8", "case.c"),
        ("9", "table"),
        ("0", "literal_name"),
    ];
    let before: Vec<_> = queries
        .iter()
        .map(|(field, pattern)| query(root, field, pattern))
        .collect();
    assert_eq!(before[1], b"case.c table 1 const char *table[] = {\n");
    assert!(
        before[2].ends_with(b"void after(void ) { target(); }\n"),
        "{}",
        String::from_utf8_lossy(&before[2])
    );
    assert!(
        before[3].ends_with(b"void after(void ) { target(); }\n"),
        "{}",
        String::from_utf8_lossy(&before[3])
    );
    assert!(
        before[7].is_empty(),
        "string contents must not become symbols"
    );
    for field in ["4", "6"] {
        let rows = query(root, field, "literal_name");
        assert_eq!(rows.iter().filter(|&&b| b == b'\n').count(), 410);
        assert!(
            String::from_utf8(rows)
                .unwrap()
                .lines()
                .all(|line| line.ends_with(&"\"literal_name\",".repeat(40)))
        );
    }
    fs::remove_file(root.join("case.c")).unwrap();
    for ((field, pattern), expected) in queries.iter().zip(before) {
        assert_eq!(query(root, field, pattern), expected);
    }
}
