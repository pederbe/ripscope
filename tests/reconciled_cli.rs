use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
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
fn run(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ripscope"))
        .current_dir(root)
        .args(args)
        .output()
        .unwrap()
}
fn success(output: &Output) {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
}
#[test]
fn separate_processes_reopen_retained_definitions_after_source_removal() {
    use sha2::{Digest, Sha256};
    let dir = temp();
    let root = dir.path();
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/cli-combined.json")).unwrap();
    let source = include_bytes!("fixtures/cli-combined.c");
    assert_eq!(
        format!("{:x}", Sha256::digest(source)),
        fixture["source_sha256"]
    );
    fs::write(root.join("case.c"), source).unwrap();
    let built = run(root, &["-bk", "-f", "index ü space", "case.c"]);
    success(&built);
    assert!(built.stdout.is_empty());
    fs::remove_file(root.join("case.c")).unwrap();
    for query in fixture["queries"].as_array().unwrap() {
        let output = run(
            root,
            &[
                "-dL",
                "-f",
                "index ü space",
                "-1",
                query["pattern"].as_str().unwrap(),
            ],
        );
        success(&output);
        assert_eq!(output.stdout, query["stdout"].as_str().unwrap().as_bytes());
    }
}

#[test]
fn normal_query_refreshes_same_size_content_and_a_failed_refresh_keeps_that_revision() {
    use std::{fs::File, fs::FileTimes};
    let dir = temp();
    let root = dir.path();
    let path = root.join("case.c");
    fs::write(&path, "int old;\n").unwrap();
    success(&run(root, &["-bk", "-f", "db", "case.c"]));

    let current = run(root, &["-L", "-f", "db", "-1", "old"]);
    success(&current);
    assert_eq!(current.stdout, b"case.c old 1 int old;\n");

    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    fs::write(&path, "int new;\n").unwrap();
    File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_times(FileTimes::new().set_modified(modified))
        .unwrap();
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    let refreshed = run(root, &["-L", "-f", "db", "-1", "new"]);
    success(&refreshed);
    assert_eq!(refreshed.stdout, b"case.c new 1 int new;\n");

    let explicit = run(root, &["-dL", "-f", "db", "-1", "new"]);
    success(&explicit);
    assert_eq!(explicit.stdout, b"case.c new 1 int new;\n");

    fs::remove_file(&path).unwrap();
    let missing = run(root, &["-L", "-f", "db", "-1", "old"]);
    assert!(!missing.status.success());
    assert!(missing.stdout.is_empty());
    assert_eq!(
        String::from_utf8_lossy(&missing.stderr),
        "ripscope: cannot find file case.c\nripscope: no source files found\n"
    );
    let stored = run(root, &["-dL", "-f", "db", "-1", "new"]);
    success(&stored);
    assert_eq!(stored.stdout, b"case.c new 1 int new;\n");
    // As cscope, `-d` and `-L` may be given apart.
    let apart = run(root, &["-d", "-L", "-f", "db", "-1", "new"]);
    success(&apart);
    assert_eq!(apart.stdout, b"case.c new 1 int new;\n");
}

#[test]
fn normal_refresh_reconciles_literal_include_additions_and_removals() {
    let dir = temp();
    let root = dir.path();
    fs::create_dir(root.join("headers")).unwrap();
    fs::write(root.join("seed.c"), "#include <old.h>\nint seed;\n").unwrap();
    fs::write(root.join("headers/old.h"), "int old;\n").unwrap();
    success(&run(root, &["-bk", "-I", "headers", "-f", "db", "seed.c"]));

    fs::write(root.join("seed.c"), "#include <new.h>\nint seed;\n").unwrap();
    fs::write(root.join("headers/new.h"), "int new;\n").unwrap();
    let added = run(root, &["-L", "-f", "db", "-1", "new"]);
    success(&added);
    assert_eq!(added.stdout, b"headers/new.h new 1 int new;\n");
    let removed = run(root, &["-dL", "-f", "db", "-1", "old"]);
    success(&removed);
    assert!(removed.stdout.is_empty());

    fs::write(root.join("seed.c"), "/* no include */\nint seed;\n").unwrap();
    fs::remove_file(root.join("headers/new.h")).unwrap();
    let refreshed = run(root, &["-L", "-f", "db", "-1", "seed"]);
    success(&refreshed);
    assert_eq!(refreshed.stdout, b"seed.c seed 2 int seed;\n");
    let absent = run(root, &["-dL", "-f", "db", "-1", "new"]);
    success(&absent);
    assert!(absent.stdout.is_empty());
}

#[test]
fn sorted_files_case_regex_macros_and_failed_invocations() {
    let dir = temp();
    let root = dir.path();
    fs::write(root.join("z.c"), "#define MATCH 1\nint value;\n").unwrap();
    fs::write(root.join("a.c"), "int value;\n").unwrap();
    success(&run(root, &["-b", "-k", "-f", "db", "z.c", "a.c"]));
    fs::remove_file(root.join("z.c")).unwrap();
    fs::remove_file(root.join("a.c")).unwrap();
    for pattern in ["value", "^value$"] {
        let out = run(root, &["-d", "-L", "-f", "db", "-1", pattern]);
        success(&out);
        assert_eq!(
            out.stdout,
            b"a.c value 1 int value;\nz.c value 2 int value;\n"
        );
    }
    let out = run(root, &["-CdL", "-f", "db", "-1", "match"]);
    success(&out);
    assert_eq!(out.stdout, b"z.c match 1 #define MATCH 1\n");
    let original = fs::read(root.join("db")).unwrap();
    // In the line protocol field 5 is a text search, as in cscope (the files are gone,
    // so it finds nothing), and a pattern that cannot be used finds nothing.
    for args in [
        vec!["-dL", "-f", "db", "-5", "value"],
        vec!["-dL", "-f", "db", "-1", "["],
    ] {
        let out = run(root, &args);
        assert!(out.status.success(), "{args:?}: {out:?}");
        assert!(out.stdout.is_empty());
        assert!(!out.stderr.is_empty());
    }
    // An index build that finds no source, a session with no index and nothing to index.
    for args in [
        vec!["-b", "-k", "-f", "db", "a.c"],
        vec!["-l"],
        vec!["-bk", "-f", "new", "x.c", "x.c"],
        vec!["-bk", "-f", "new", "bad path.c"],
    ] {
        let out = run(root, &args);
        assert!(!out.status.success(), "{args:?}");
        assert!(out.stdout.is_empty());
        assert!(!out.stderr.is_empty());
    }
    assert_eq!(fs::read(root.join("db")).unwrap(), original);
    assert!(!root.join("new").exists());
}
#[test]
fn retained_include_selection_and_spelling_baseline() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/reference/selection-oracle.json")).unwrap();
    // The "nonliteral" case is enforceable: `#include HEADER` is accepted and not
    // followed.
    for case in fixture["cases"].as_array().unwrap() {
        let dir = temp();
        let root = dir.path();
        for (path, text) in case["files"].as_object().unwrap() {
            let file = root.join(path);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(file, text.as_str().unwrap()).unwrap();
        }
        let args: Vec<_> = case["runs"][0]["args"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        success(&run(root, &args));
        for path in case["files"].as_object().unwrap().keys() {
            fs::remove_file(root.join(path)).unwrap();
        }
        let query = &case["runs"][2];
        let args: Vec<_> = query["args"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        let actual = run(root, &args);
        success(&actual);
        assert_eq!(
            actual.stdout,
            query["stdout"].as_str().unwrap().as_bytes(),
            "{}",
            case["name"]
        );
    }
}

#[test]
fn include_waves_keep_seeds_first_and_survive_source_removal() {
    let dir = temp();
    let root = dir.path();
    fs::write(root.join("empty.c"), "/* zero definitions */\n").unwrap();
    fs::write(
        root.join("z.c"),
        "#include \"a.h\"\n#include \"a.h\"\nint value;\n",
    )
    .unwrap();
    fs::write(
        root.join("a.h"),
        "#include \"b.h\"\n#include \"z.c\"\nint value;\n",
    )
    .unwrap();
    fs::write(root.join("b.h"), "int value;\n").unwrap();
    success(&run(root, &["-bk", "-f", "db", "z.c", "./z.c", "empty.c"]));
    for name in ["empty.c", "z.c", "a.h", "b.h"] {
        fs::remove_file(root.join(name)).unwrap();
    }
    let args = ["-dL", "-f", "db", "-1", "value"];
    let actual = run(root, &args);
    success(&actual);
    assert_eq!(
        actual.stdout,
        b"z.c value 3 int value;\na.h value 3 int value;\nb.h value 1 int value;\n"
    );
}

#[test]
fn decorated_signatures_reopen_with_original_cscope_display() {
    // Named field-1 results from the pinned cscope 15.9 binary, LC_ALL=C.
    for (text, expected) in [
        ("int API f(void) {}\n", "case.c f 1 int API f(void ) {}\n"),
        ("Type API f(void) {}\n", "case.c f 1 Type API f(void ) {}\n"),
        ("void API f(void);\n", ""),
        ("API int f(void) {}\n", "case.c f 1 API int f(void ) {}\n"),
        ("API void f(void);\n", ""),
    ] {
        for crlf in [false, true] {
            let dir = temp();
            let root = dir.path();
            fs::write(
                root.join("case.c"),
                if crlf {
                    text.replace('\n', "\r\n")
                } else {
                    text.to_owned()
                },
            )
            .unwrap();
            success(&run(root, &["-bk", "-f", "db", "case.c"]));
            fs::remove_file(root.join("case.c")).unwrap();
            for (name, stdout) in [("f", expected), ("API", "")] {
                let args = ["-dL", "-f", "db", "-1", name];
                let actual = run(root, &args);
                success(&actual);
                assert_eq!(actual.stdout, stdout.as_bytes());
            }
        }
    }
}

#[test]
fn multi_decoration_arrays_match_cscope_after_reopening() {
    assert_array_oracle(include_str!("fixtures/multi-decoration-arrays.json"));
}

#[test]
fn multidimensional_arrays_match_cscope_after_reopening() {
    assert_array_oracle(include_str!("fixtures/multidimensional-arrays.json"));
}

#[test]
fn decorated_unnamed_pointer_parameters_match_cscope_after_reopening() {
    assert_array_oracle(include_str!(
        "fixtures/decorated-unnamed-pointer-parameters.json"
    ));
}

#[test]
fn decorated_named_pointer_parameters_match_cscope_after_reopening() {
    assert_array_oracle(include_str!(
        "fixtures/decorated-named-pointer-parameters.json"
    ));
}

#[test]
fn decorated_pointer_returns_match_cscope_after_reopening() {
    assert_array_oracle(include_str!("fixtures/decorated-pointer-returns.json"));
}

#[test]
fn decorated_pointer_return_cast_composition_matches_cscope_after_reopening() {
    assert_array_oracle(include_str!(
        "fixtures/decorated-pointer-return-cast-composition.json"
    ));
}

#[test]
fn decorated_primitive_pointer_parameter_matches_cscope_after_reopening() {
    assert_array_oracle(include_str!(
        "fixtures/decorated-primitive-pointer-parameter.json"
    ));
}

#[test]
fn decorated_primitive_pointer_crc32_matches_cscope_after_reopening() {
    assert_array_oracle(include_str!(
        "fixtures/decorated-primitive-pointer-crc32.json"
    ));
}

fn assert_array_oracle(fixture: &str) {
    let oracle: serde_json::Value = serde_json::from_str(fixture).unwrap();
    for case in oracle["cases"].as_array().unwrap() {
        for crlf in [false, true] {
            let dir = temp();
            let root = dir.path();
            let text = case["source"].as_str().unwrap();
            fs::write(
                root.join("case.c"),
                if crlf {
                    text.replace('\n', "\r\n")
                } else {
                    text.to_owned()
                },
            )
            .unwrap();
            let build = run(root, &["-bk", "-f", "db", "case.c"]);
            let expected_build = &case["commands"].as_array().unwrap()[0];
            assert_eq!(expected_build["args"], serde_json::json!(["-bk", "case.c"]));
            assert_eq!(
                build.status.code(),
                expected_build["exit"].as_i64().map(|n| n as i32)
            );
            assert_eq!(
                build.stdout,
                expected_build["stdout"].as_str().unwrap().as_bytes(),
                "{}",
                case["name"]
            );
            assert_eq!(
                build.stderr,
                expected_build["stderr"].as_str().unwrap().as_bytes(),
                "{}",
                case["name"]
            );
            success(&build);
            fs::remove_file(root.join("case.c")).unwrap();
            for query in case["commands"].as_array().unwrap().iter().skip(1) {
                assert_eq!(query["exit"], 0);
                assert_eq!(query["stderr"], "");
                let pattern = query["args"].as_array().unwrap()[2].as_str().unwrap();
                let args = ["-dL", "-f", "db", "-1", pattern];
                let actual = run(root, &args);
                success(&actual);
                assert_eq!(
                    actual.stdout,
                    query["stdout"].as_str().unwrap().as_bytes(),
                    "{}",
                    case["name"]
                );
            }
        }
    }
}

#[test]
fn decorated_data_names_reopen_with_approved_typedef_roles() {
    let oracle: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/decorated-data.json")).unwrap();
    for case in oracle["cases"].as_array().unwrap() {
        // cscope takes `int API (*x)(void);` for the definition of `API`; the
        // `definition-bodies` Correction requires a body (the scanner's probe fixtures show
        // both readings).
        if case["name"] == "function-pointer" {
            continue;
        }
        for crlf in [false, true] {
            let dir = temp();
            let root = dir.path();
            let text = case["source"].as_str().unwrap();
            fs::write(
                root.join("case.c"),
                if crlf {
                    text.replace('\n', "\r\n")
                } else {
                    text.to_owned()
                },
            )
            .unwrap();
            let build = run(root, &["-bk", "-f", "db", "case.c"]);
            success(&build);
            fs::remove_file(root.join("case.c")).unwrap();
            for query in case["queries"].as_array().unwrap() {
                assert_eq!(query["exit"], 0);
                assert_eq!(query["stderr"], "");
                let args = ["-dL", "-f", "db", "-1", query["pattern"].as_str().unwrap()];
                let actual = run(root, &args);
                success(&actual);
                // The approved written typedef reading drops the decoration's
                // definition; the other data forms retain the reference rows.
                let expected = if case["name"] == "typedef" && query["pattern"] == "API" {
                    ""
                } else {
                    query["stdout"].as_str().unwrap()
                };
                assert_eq!(actual.stdout, expected.as_bytes(), "{}", case["name"]);
            }
        }
    }
}

#[test]
fn decorated_typedef_names_reopen_with_approved_roles() {
    let oracle: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/decorated-typedef.json")).unwrap();
    for case in oracle["cases"].as_array().unwrap() {
        for crlf in [false, true] {
            let dir = temp();
            let root = dir.path();
            let text = case["source"].as_str().unwrap();
            fs::write(
                root.join("case.c"),
                if crlf {
                    text.replace('\n', "\r\n")
                } else {
                    text.to_owned()
                },
            )
            .unwrap();
            let build = run(root, &["-bk", "-f", "db", "case.c"]);
            success(&build);
            fs::remove_file(root.join("case.c")).unwrap();
            for query in case["queries"].as_array().unwrap() {
                assert_eq!(query["exit"], 0);
                assert_eq!(query["stderr"], "");
                let args = ["-dL", "-f", "db", "-1", query["pattern"].as_str().unwrap()];
                let actual = run(root, &args);
                success(&actual);
                let expected = if query["pattern"] == "API" && case["name"] != "pointer" {
                    // Keep API's real macro definition, dropping its false
                    // typedef mark. Pointer-after-star remains a separate limit.
                    if case["name"] == "macro-defined" {
                        "case.c API 1 #define API\n"
                    } else {
                        ""
                    }
                } else {
                    query["stdout"].as_str().unwrap()
                };
                assert_eq!(actual.stdout, expected.as_bytes(), "{}", case["name"]);
            }
        }
    }
}

#[test]
fn macro_comment_names_match_cscope_after_reopening() {
    let oracle: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/macro-comments.json")).unwrap();
    for case in oracle["cases"].as_array().unwrap() {
        for crlf in [false, true] {
            let dir = temp();
            let root = dir.path();
            let text = case["source"].as_str().unwrap();
            fs::write(
                root.join("case.c"),
                if crlf {
                    text.replace('\n', "\r\n")
                } else {
                    text.to_owned()
                },
            )
            .unwrap();
            let build = run(root, &["-bk", "-f", "db", "case.c"]);
            success(&build);
            fs::remove_file(root.join("case.c")).unwrap();
            for query in case["queries"].as_array().unwrap() {
                assert_eq!(query["exit"], 0);
                assert_eq!(query["stderr"], "");
                let args = ["-dL", "-f", "db", "-1", query["pattern"].as_str().unwrap()];
                let actual = run(root, &args);
                success(&actual);
                assert_eq!(
                    actual.stdout,
                    query["stdout"].as_str().unwrap().as_bytes(),
                    "{}",
                    case["name"]
                );
            }
        }
    }
}

#[test]
fn decorated_cast_names_match_cscope_after_reopening() {
    let oracle: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/decorated-casts.json")).unwrap();
    for case in oracle["cases"].as_array().unwrap() {
        for crlf in [false, true] {
            let dir = temp();
            let root = dir.path();
            let text = case["source"].as_str().unwrap();
            fs::write(
                root.join("case.c"),
                if crlf {
                    text.replace('\n', "\r\n")
                } else {
                    text.to_owned()
                },
            )
            .unwrap();
            let build = run(root, &["-bk", "-f", "db", "case.c"]);
            success(&build);
            fs::remove_file(root.join("case.c")).unwrap();
            for query in case["queries"].as_array().unwrap() {
                assert_eq!(query["exit"], 0);
                assert_eq!(query["stderr"], "");
                let args = ["-dL", "-f", "db", "-1", query["pattern"].as_str().unwrap()];
                let actual = run(root, &args);
                success(&actual);
                assert_eq!(
                    actual.stdout,
                    query["stdout"].as_str().unwrap().as_bytes(),
                    "{}",
                    case["name"]
                );
            }
        }
    }
}
