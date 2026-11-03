//! Create executable scripts without writable handles in a parallel test harness.
#![forbid(unsafe_code)]

use std::{fs, os::unix::fs::PermissionsExt, path::Path, process::Command};

pub fn write(path: &Path, contents: &str, mode: u32) {
    // Only this selected test runs in the child. It never starts another process
    // while the destination is writable, and exits before the caller uses it.
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "executable_fixture::write_fixture",
            "--nocapture",
        ])
        .env_clear()
        .env("RIPSCOPE_EXECUTABLE_FIXTURE_PATH", path)
        .env("RIPSCOPE_EXECUTABLE_FIXTURE_CONTENTS", contents)
        .env("RIPSCOPE_EXECUTABLE_FIXTURE_MODE", mode.to_string())
        .output()
        .unwrap();
    assert!(output.status.success(), "write {path:?}: {output:?}");
}

#[test]
fn write_fixture() {
    let Some(path) = std::env::var_os("RIPSCOPE_EXECUTABLE_FIXTURE_PATH") else {
        return;
    };
    let contents = std::env::var("RIPSCOPE_EXECUTABLE_FIXTURE_CONTENTS").unwrap();
    let mode = std::env::var("RIPSCOPE_EXECUTABLE_FIXTURE_MODE")
        .unwrap()
        .parse()
        .unwrap();
    fs::write(&path, contents).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
}
