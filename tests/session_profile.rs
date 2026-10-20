//! Diagnostic output must never enter the public Session protocol.
use std::fs;
use std::io::Write;
use std::process::{Command, Stdio};

#[test]
fn session_diagnostics_preserve_complete_answers_and_use_a_dedicated_file() {
    let work = tempfile::tempdir().unwrap();
    fs::write(work.path().join("a.c"), "#include \"a.h\"\nint value;\n").unwrap();
    fs::write(work.path().join("a.h"), "int declared;\n").unwrap();
    let binary = env!("CARGO_BIN_EXE_ripscope");
    let built = Command::new(binary)
        .current_dir(work.path())
        .args(["-bk", "-f", "index", "a.c"])
        .output()
        .unwrap();
    assert!(built.status.success());
    for enabled in [false, true] {
        let path = work.path().join("phases.jsonl");
        let mut command = Command::new(binary);
        command
            .current_dir(work.path())
            .args(["-dl", "-f", "index"]);
        command.env_remove("RIPSCOPE_PROFILE");
        if enabled {
            command.env("RIPSCOPE_PROFILE", &path);
        }
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(b"8a.h\n8a.h\nq\n")
            .unwrap();
        let result = child.wait_with_output().unwrap();
        assert!(result.status.success());
        assert!(result.stderr.is_empty());
        assert_eq!(result.stdout, b">> cscope: 1 lines\na.c <global> 1 #include \"a.h\"\n>> cscope: 1 lines\na.c <global> 1 #include \"a.h\"\n>> ");
        if enabled && cfg!(feature = "session-profile") {
            let rows: Vec<serde_json::Value> = fs::read_to_string(&path)
                .unwrap()
                .lines()
                .map(|line| serde_json::from_str(line).unwrap())
                .collect();
            // Includer answers come from recorded include rows and load no blocks.
            for request in [1, 2] {
                let phase = |phase| {
                    rows.iter()
                        .filter(|row| row["request"] == request && row["phase"] == phase)
                        .map(|row| row["calls"].as_u64().unwrap())
                        .sum::<u64>()
                };
                assert_eq!(phase("request"), 1);
                assert_eq!(phase("deliver"), 1);
                assert_eq!(phase("blocks_loaded"), 0);
            }
            assert!(
                rows.iter()
                    .any(|row| row["request"] == 0 && row["phase"] == "startup")
            );
        } else {
            assert!(!path.exists());
        }
    }
}
