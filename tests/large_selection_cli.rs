//! Selections whose file descriptors take more than 1 MiB.
use std::{fs, path::Path, process::Command};

fn run(root: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_ripscope"))
        .current_dir(root)
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn selections_over_one_mebibyte_of_paths_build() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let long = "x".repeat(100);
    // About 1.1 KB of descriptor per file on Windows, 0.6 KB elsewhere: well past 1 MiB.
    let count = 2000;
    for index in 0..count {
        fs::write(
            root.join(format!("{long}{index}.c")),
            format!("int v{index};\n"),
        )
        .unwrap();
    }
    let build = run(root, &["-bk", "-f", "idx"]);
    assert!(
        build.status.success(),
        "{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let query = run(root, &["-dL", "-f", "idx", "-1", "v1999"]);
    assert_eq!(
        String::from_utf8(query.stdout).unwrap(),
        format!("{long}1999.c v1999 1 int v1999;\n")
    );
}
