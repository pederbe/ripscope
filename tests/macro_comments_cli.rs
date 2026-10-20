mod intended;

use std::{fs, process::Command};

#[test]
fn macro_comment_queries_match_reference_after_source_deletion() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/multiline-macro-comments.json")).unwrap();
    let corrections: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/multiline-macro-comment-corrections.json"
    ))
    .unwrap();
    for (name, fields) in fixture["cases"].as_object().unwrap() {
        // The `macro-line-comments` Correction ends a macro at its line comment, where
        // cscope runs it on through the next line, so in these cases the lines after the
        // macro are not in it; the scanner's probe fixtures show both readings.
        if matches!(
            name.as_str(),
            "line-comment"
                | "line-comment-single"
                | "line-comment-chain"
                | "line-comment-function"
                | "line-comment-function-body"
                | "line-comment-signature"
                | "line-comment-local-body"
                | "line-comment-approved-definition"
                | "line-comment-eof"
        ) {
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
        fs::write(
            root.join("case.c"),
            fields["0"]["files"]["case.c"].as_str().unwrap(),
        )
        .unwrap();
        let build = run(&["-bk", "-f", "index", "case.c"]);
        assert!(
            build.status.success(),
            "{name}: {}",
            String::from_utf8_lossy(&build.stderr)
        );
        fs::remove_file(root.join("case.c")).unwrap();
        for capture in fields.as_object().unwrap().values() {
            for profile in capture["profiles"].as_array().unwrap() {
                for query in profile["queries"].as_array().unwrap() {
                    let mut args: Vec<_> = query["args"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|arg| arg.as_str().unwrap())
                        .collect();
                    args.extend(["-f", "index"]);
                    let actual = run(&args);
                    assert!(actual.status.success(), "{name} {args:?}");
                    let mut expected = query["stdout"].as_str().unwrap();
                    let pattern = query["args"]
                        .as_array()
                        .unwrap()
                        .last()
                        .unwrap()
                        .as_str()
                        .unwrap();
                    let correction =
                        &corrections["cases"][name][profile["caseless"].to_string()][pattern];
                    if capture["field"] == "1" && !correction.is_null() {
                        assert_eq!(correction["cscope_stdout"], expected);
                        expected = correction["ripscope_stdout"].as_str().unwrap();
                    }
                    intended::check(
                        "multiline-macro-comments.json",
                        &format!("{name} {}", args[..args.len() - 2].join(" ")),
                        expected,
                        &String::from_utf8_lossy(&actual.stdout),
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
