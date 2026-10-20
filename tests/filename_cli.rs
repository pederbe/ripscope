use std::{fs, path::Path, process::Command};

fn run(root: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_ripscope"))
        .current_dir(root)
        .args(args)
        .output()
        .unwrap()
}

fn temp() -> tempfile::TempDir {
    #[cfg(target_os = "linux")]
    {
        tempfile::tempdir_in("/var/tmp").unwrap()
    }
    #[cfg(not(target_os = "linux"))]
    {
        tempfile::tempdir().unwrap()
    }
}

#[test]
fn filename_queries_match_reference_after_source_deletion() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/filename-queries.json")).unwrap();
    let dir = temp();
    let root = dir.path();
    for (name, source) in fixture["files"].as_object().unwrap() {
        let path = root.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, source.as_str().unwrap()).unwrap();
    }
    let mut args = vec!["-bk", "-f", "index"];
    args.extend(
        fixture["seeds"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s.as_str().unwrap()),
    );
    let build = run(root, &args);
    assert!(build.status.success(), "{:?}", build);
    for name in fixture["files"].as_object().unwrap().keys() {
        fs::remove_file(root.join(name)).unwrap();
    }
    for profile in fixture["profiles"].as_array().unwrap() {
        for query in profile["queries"].as_array().unwrap() {
            let mut args = vec!["-d", "-f", "index"];
            args.extend(
                query["args"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|a| a.as_str().unwrap()),
            );
            let result = run(root, &args);
            let pattern = args.last().unwrap();
            assert_eq!(
                result.stdout,
                query["stdout"].as_str().unwrap().as_bytes(),
                "{args:?}"
            );
            if matches!(*pattern, "[" | "") {
                // As in cscope, a pattern that cannot be used finds nothing and the query
                // succeeds; ripscope says why on standard error.
                assert!(result.status.success(), "{args:?}: {result:?}");
                assert!(!result.stderr.is_empty());
            } else {
                assert!(result.status.success(), "{args:?}: {result:?}");
                assert!(result.stderr.is_empty());
            }
        }
    }
}

#[test]
fn filename_normal_query_refreshes_membership_but_stored_query_does_not() {
    let dir = temp();
    let root = dir.path();
    fs::write(root.join("case.c"), "int Alpha;\n").unwrap();
    assert!(
        run(root, &["-bk", "-f", "index", "case.c"])
            .status
            .success()
    );
    fs::write(root.join("case.c"), "#include \"added.h\"\nint Alpha;\n").unwrap();
    fs::write(root.join("added.h"), "int Beta;\n").unwrap();
    let stored = run(root, &["-dL", "-f", "index", "-7", "added"]);
    assert!(stored.status.success());
    assert!(stored.stdout.is_empty());
    let fresh = run(root, &["-L", "-f", "index", "-7", "added"]);
    assert!(fresh.status.success(), "{fresh:?}");
    assert_eq!(fresh.stdout, b"added.h <unknown> 1 <unknown>\n");
    fs::remove_file(root.join("case.c")).unwrap();
    let failed = run(root, &["-L", "-f", "index", "-7", "."]);
    assert!(!failed.status.success());
    assert!(failed.stdout.is_empty());
    let stored = run(root, &["-dL", "-f", "index", "-7", "added"]);
    assert!(stored.status.success());
    assert_eq!(stored.stdout, fresh.stdout);
}
