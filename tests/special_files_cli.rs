//! Finite readers reject devices and pipes without losing a published revision.

use std::fs;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

fn run(root: &Path, args: &[&str]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_ripscope"))
        .current_dir(root)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("reader blocked on {args:?}");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    child.wait_with_output().unwrap()
}

#[cfg(unix)]
fn fifo(path: &Path) {
    assert!(Command::new("mkfifo").arg(path).status().unwrap().success());
}

#[cfg(unix)]
#[test]
fn file_lists_and_indexes_reject_fifos_without_waiting_for_writers() {
    let work = tempfile::tempdir().unwrap();
    let root = work.path();
    fs::write(root.join("a.c"), "int Before;\n").unwrap();
    assert!(run(root, &["-bk", "-f", "index", "a.c"]).status.success());
    fifo(&root.join("cscope.files"));
    fifo(&root.join("input"));
    fifo(&root.join("pipe-index"));
    for args in [
        vec!["-bk", "-f", "index"],
        vec!["-bk", "-f", "index", "-i", "input"],
        vec!["-dL", "-f", "pipe-index", "-1", "Before"],
    ] {
        let refused = run(root, &args);
        assert!(!refused.status.success(), "{args:?}: {refused:?}");
        assert!(!refused.stderr.is_empty());
    }
    let kept = run(root, &["-dL", "-f", "index", "-1", "Before"]);
    assert!(kept.status.success(), "{kept:?}");
    assert_eq!(kept.stdout, b"a.c Before 1 int Before;\n");
}

#[cfg(unix)]
#[test]
fn live_text_rejects_a_source_replaced_by_a_fifo_or_device_link() {
    use std::os::unix::fs::symlink;
    let work = tempfile::tempdir().unwrap();
    let root = work.path();
    fs::write(root.join("a.c"), "int Before;\n").unwrap();
    assert!(run(root, &["-bk", "-f", "index", "a.c"]).status.success());
    fs::remove_file(root.join("a.c")).unwrap();
    fifo(&root.join("a.c"));
    for device in [false, true] {
        if device {
            fs::remove_file(root.join("a.c")).unwrap();
            symlink("/dev/null", root.join("a.c")).unwrap();
        }
        let refused = run(root, &["-dL", "-f", "index", "-4", "Before"]);
        assert!(refused.status.success(), "{refused:?}");
        assert!(refused.stdout.is_empty());
        assert!(String::from_utf8_lossy(&refused.stderr).contains("not a regular file"));
    }
    fs::write(root.join("input"), "a.c\n").unwrap();
    fs::remove_file(root.join("a.c")).unwrap();
    fs::write(root.join("actual.c"), "int After;\n").unwrap();
    symlink("actual.c", root.join("a.c")).unwrap();
    let ordinary = run(root, &["-dL", "-f", "index", "-4", "After"]);
    assert_eq!(ordinary.stdout, b"a.c <unknown> 1 int After;\n");
    symlink("/dev/null", root.join("cscope.files")).unwrap();
    let refused = run(root, &["-bk", "-f", "index"]);
    assert!(!refused.status.success());
    assert!(String::from_utf8_lossy(&refused.stderr).contains("not a regular file"));
    fs::remove_file(root.join("cscope.files")).unwrap();
    symlink("input", root.join("cscope.files")).unwrap();
    for args in [
        vec!["-bk", "-f", "index"],
        vec!["-bk", "-f", "index", "-i", "cscope.files"],
    ] {
        let ordinary = run(root, &args);
        assert!(ordinary.status.success(), "{ordinary:?}");
        assert!(ordinary.stderr.is_empty());
    }
}

#[cfg(windows)]
#[test]
fn file_lists_reject_windows_devices() {
    let work = tempfile::tempdir().unwrap();
    fs::write(work.path().join("a.c"), "int Before;\n").unwrap();
    let refused = run(work.path(), &["-bk", "-f", "index", "-i", "NUL"]);
    assert!(!refused.status.success());
    assert!(String::from_utf8_lossy(&refused.stderr).contains("not a regular file"));
    assert!(!work.path().join("index").exists());
}
