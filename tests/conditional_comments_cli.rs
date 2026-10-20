mod intended;

use std::{fs, process::Command};

/// Code after a comment's closing delimiter is outside the closed-tail Correction.
/// Its reference reading stays pinned, with sources deleted before querying.
#[test]
fn conditional_comments_with_closing_line_code_keep_the_reference_reading() {
    let original: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/multiline-conditional-comments.json")).unwrap();
    for (name, fields) in original["cases"].as_object().unwrap() {
        if name != "trailing-code" {
            continue;
        }
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let run = |args: &[&str]| {
            Command::new(env!("CARGO_BIN_EXE_ripscope"))
                .current_dir(root)
                .args(args)
                .output()
                .unwrap()
        };
        let files = fields["0"]["files"].as_object().unwrap();
        for (path, source) in files {
            fs::write(root.join(path), source.as_str().unwrap()).unwrap();
        }
        let build = run(&["-bk", "-f", "index", "case.c"]);
        assert!(
            build.status.success(),
            "{name}: {}",
            String::from_utf8_lossy(&build.stderr)
        );
        for path in files.keys() {
            fs::remove_file(root.join(path)).unwrap();
        }
        for (field, capture) in fields.as_object().unwrap() {
            assert_eq!(capture["build"]["exit_code"], 0, "{name} {field}");
            for profile in capture["profiles"].as_array().unwrap() {
                // A caseless profile's arguments carry its -C.
                for query in profile["queries"].as_array().unwrap() {
                    let mut args: Vec<_> = query["args"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|a| a.as_str().unwrap())
                        .collect();
                    args.extend(["-f", "index"]);
                    let actual = run(&args);
                    assert_eq!(
                        actual.status.code(),
                        query["exit_code"].as_i64().map(|code| code as i32),
                        "{name} {args:?}"
                    );
                    intended::check(
                        "multiline-conditional-comments.json",
                        &format!("{name} {}", args[..args.len() - 2].join(" ")),
                        query["stdout"].as_str().unwrap(),
                        &String::from_utf8_lossy(&actual.stdout),
                    );
                    assert_eq!(
                        String::from_utf8_lossy(&actual.stderr),
                        query["stderr"].as_str().unwrap(),
                        "{name} {args:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn conditional_comment_queries_match_paired_reference_after_source_deletion() {
    let original: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/multiline-conditional-comments.json")).unwrap();
    let paired: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/multiline-conditional-comment-corrections.json"
    ))
    .unwrap();
    for (name, fields) in original["cases"].as_object().unwrap() {
        // Closing-line tokens are outside the approved closed-directive profile.
        if name == "trailing-code" {
            continue;
        }
        #[cfg(target_os = "linux")]
        let directory = tempfile::tempdir_in("/var/tmp").unwrap();
        #[cfg(not(target_os = "linux"))]
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let run = |args: &[&str]| {
            Command::new(env!("CARGO_BIN_EXE_ripscope"))
                .current_dir(root)
                .args(args)
                .output()
                .unwrap()
        };
        let files = fields["0"]["files"].as_object().unwrap();
        for (path, source) in files {
            fs::write(root.join(path), source.as_str().unwrap()).unwrap();
        }
        let build = run(&["-bk", "-f", "index", "case.c"]);
        assert!(
            build.status.success(),
            "{name}: {}",
            String::from_utf8_lossy(&build.stderr)
        );
        if name == "declaration-text" {
            for field in ["-4", "-6"] {
                let result = run(&["-dL", "-f", "index", field, "phantom"]);
                assert!(result.status.success());
                assert_eq!(result.stdout, b"case.c <unknown> 4 int phantom;\n");
            }
        }
        for path in files.keys() {
            fs::remove_file(root.join(path)).unwrap();
        }
        for capture in paired["cases"][name]["fields"]
            .as_object()
            .unwrap()
            .values()
        {
            for profile in capture["profiles"].as_array().unwrap() {
                for query in profile["queries"].as_array().unwrap() {
                    let mut args: Vec<_> = query["args"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|a| a.as_str().unwrap())
                        .collect();
                    args.extend(["-f", "index"]);
                    let actual = run(&args);
                    assert!(actual.status.success(), "{name} {args:?}");
                    assert_eq!(
                        actual.stdout,
                        query["stdout"].as_str().unwrap().as_bytes(),
                        "{name} {args:?}"
                    );
                    assert_eq!(
                        actual.stderr,
                        query["stderr"].as_str().unwrap().as_bytes(),
                        "{name} {args:?}"
                    );
                }
            }
        }
    }
}
