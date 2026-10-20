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
fn expanded_selections_refresh_preserved_time_edits_and_literal_resolution() {
    for mode in 0..3 {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        fs::create_dir(root.join("nested")).unwrap();
        fs::create_dir(root.join("first")).unwrap();
        fs::create_dir(root.join("second")).unwrap();
        let name = if mode == 2 { "nested/a.c" } else { "a.c" };
        let path = root.join(name);
        fs::write(&path, "#include <choice.inc>\nint Alpha;\n").unwrap();
        let header = root.join("second/choice.inc");
        fs::write(&header, "int Older;\n").unwrap();
        fs::write(root.join("chosen"), format!("{name}\n")).unwrap();
        let mut args = vec!["-bk", "-f", "index", "-I", "first", "-I", "second"];
        if mode == 0 {
            args.extend(["-i", "chosen"]);
        }
        if mode == 2 {
            args.push("-R");
        }
        success(run(root, &args, b""), b"");
        for (file, bytes) in [
            (&path, "#include <choice.inc>\nint Bravo;\n"),
            (&header, "int Newer;\n"),
        ] {
            let metadata = fs::metadata(file).unwrap();
            assert_eq!(metadata.len(), bytes.len() as u64);
            let time = metadata.modified().unwrap();
            fs::write(file, bytes).unwrap();
            fs::File::options()
                .write(true)
                .open(file)
                .unwrap()
                .set_times(fs::FileTimes::new().set_modified(time))
                .unwrap();
            assert_eq!(fs::metadata(file).unwrap().modified().unwrap(), time);
        }
        success(
            run(root, &["-L", "-f", "index", "-1", "Bravo"], b""),
            format!("{name} Bravo 2 int Bravo;\n").as_bytes(),
        );
        success(
            run(root, &["-dL", "-f", "index", "-1", "Newer"], b""),
            b"second/choice.inc Newer 1 int Newer;\n",
        );
        success(run(root, &["-dL", "-f", "index", "-1", "Older"], b""), b"");
        fs::write(root.join("first/choice.inc"), "int First;\n").unwrap();
        success(
            run(root, &["-L", "-f", "index", "-1", "First"], b""),
            b"first/choice.inc First 1 int First;\n",
        );
        success(run(root, &["-dL", "-f", "index", "-1", "Newer"], b""), b"");
        // A computed include is read, not followed, as cscope reads it.
        fs::write(&path, "#include EXTERNAL_TARGET(x)\nint Computed;\n").unwrap();
        success(
            run(root, &["-L", "-f", "index", "-1", "Computed"], b""),
            format!("{name} Computed 2 int Computed;\n").as_bytes(),
        );
        success(run(root, &["-dL", "-f", "index", "-1", "First"], b""), b"");
    }
}

#[test]
fn defined_linkage_guards_keep_stored_queries_after_source_removal() {
    for guard in ["#if defined __cplusplus", "#if defined(__cplusplus)"] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let text = format!(
            "{guard}\nextern \"C\" {{\n#endif\nvoid f(void) {{ target(); }}\n{guard}\n}}\n#endif\n"
        );
        fs::write(root.join("case.c"), &text).unwrap();
        success(run(root, &["-bk", "-f", "index", "case.c"], b""), b"");
        let expected = b"case.c f 4 void f(void ) { target(); }\n";
        success(run(root, &["-dL", "-f", "index", "-1", "f"], b""), expected);
        fs::remove_file(root.join("case.c")).unwrap();
        success(run(root, &["-dL", "-f", "index", "-1", "f"], b""), expected);
        success(
            run(root, &["-dL", "-f", "index", "-2", "f"], b""),
            b"case.c target 4 void f(void ) { target(); }\n",
        );
        success(
            run(root, &["-dL", "-f", "index", "-3", "target"], b""),
            expected,
        );
    }
}
