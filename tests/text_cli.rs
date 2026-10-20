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
fn regex_text_preserves_the_private_reference_grammar() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/regex-text-queries.json")).unwrap();
    let dir = temp();
    let root = dir.path();
    for (name, text) in fixture["files"].as_object().unwrap() {
        fs::write(root.join(name), text.as_str().unwrap()).unwrap();
    }
    assert!(
        run(root, &["-bk", "-f", "index", "z.c", "a.c"])
            .status
            .success()
    );
    for (name, text) in fixture["live_files"].as_object().unwrap() {
        fs::write(root.join(name), text.as_str().unwrap()).unwrap();
    }
    for profile in fixture["profiles"].as_array().unwrap() {
        let insensitive = profile["caseless"].as_bool().unwrap();
        for query in profile["queries"].as_array().unwrap() {
            let pattern = query["args"]
                .as_array()
                .unwrap()
                .last()
                .unwrap()
                .as_str()
                .unwrap();
            if insensitive && pattern.bytes().any(|b| b.is_ascii_uppercase()) {
                continue; // Approved symmetric folding is asserted separately.
            }
            let mut args = vec!["-dL", "-f", "index"];
            if insensitive {
                args.push("-C");
            }
            args.extend(["-6", pattern]);
            let output = run(root, &args);
            if matches!(pattern, "" | "[" | "\\" | "a||b" | "()" | "[a-]") {
                // As in cscope, a pattern that cannot be used finds nothing and the query
                // succeeds; ripscope says why on standard error.
                assert!(output.status.success(), "{args:?}: {output:?}");
                assert!(output.stdout.is_empty());
                assert!(!output.stderr.is_empty(), "{args:?}: {output:?}");
                continue;
            }
            assert!(output.status.success(), "{args:?}: {output:?}");
            assert!(output.stderr.is_empty(), "{output:?}");
            let expected = match pattern {
                "^$" | "^^" | "$^" => "a.c <unknown> 18 \n".to_owned(),
                "\t" => "a.c <unknown> 17 \t\n".to_owned(),
                _ => {
                    let mut rows = query["stdout"]
                        .as_str()
                        .unwrap()
                        .replace("20 tailz.c", "20 tail\nz.c");
                    if rows.ends_with("20 tail") {
                        rows.push('\n');
                    }
                    rows
                }
            };
            assert_eq!(output.stdout, expected.as_bytes(), "{args:?}");
        }
    }
    let folded = run(root, &["-dL", "-f", "index", "-C", "-6", "^Alpha$"]);
    assert!(folded.status.success(), "{folded:?}");
    assert_eq!(folded.stdout, b"a.c <unknown> 1 alpha\na.c <unknown> 2 Alpha\na.c <unknown> 3 ALPHA\nz.c <unknown> 1 alpha\n");
}

#[test]
fn regex_text_refresh_and_navigation_share_the_literal_text_boundary() {
    let dir = temp();
    let root = dir.path();
    fs::write(root.join("a.c"), "int Alpha;\n").unwrap();
    assert!(run(root, &["-bk", "-f", "index", "a.c"]).status.success());
    fs::write(root.join("a.c"), "#include \"new.h\"\nint Alpha;\n").unwrap();
    fs::write(root.join("new.h"), "int NewMember;\n").unwrap();
    let stale = run(root, &["-dL", "-f", "index", "-6", "New.*;"]);
    assert!(stale.status.success(), "{stale:?}");
    assert!(stale.stdout.is_empty());
    let fresh = run(root, &["-L", "-f", "index", "-6", "New.*;"]);
    assert!(fresh.status.success(), "{fresh:?}");
    assert_eq!(fresh.stdout, b"new.h <unknown> 1 int NewMember;\n");
    let stored = run(root, &["-dL", "-f", "index", "-6", "New.*;"]);
    assert!(stored.status.success(), "{stored:?}");
    assert_eq!(stored.stdout, fresh.stdout);
    // As cscope, a line holding a NUL byte is found; ripscope shows the byte as `\x00`.
    fs::write(root.join("a.c"), b"Alpha\n").unwrap();
    fs::write(root.join("new.h"), b"Alpha\0\n").unwrap();
    let nul = run(root, &["-dL", "-f", "index", "-6", "Alpha"]);
    assert!(nul.status.success(), "{nul:?}");
    assert_eq!(
        nul.stdout,
        b"a.c <unknown> 1 Alpha\nnew.h <unknown> 1 Alpha\\x00\n"
    );
    fs::write(root.join("a.c"), b"").unwrap();
    fs::write(root.join("new.h"), b"").unwrap();
    let empty = run(root, &["-dL", "-f", "index", "-6", "a*"]);
    assert!(empty.status.success());
    assert!(
        empty.stdout.is_empty(),
        "empty files have no physical lines"
    );
}

#[test]
fn literal_text_uses_live_bytes_and_preserves_reference_rows() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/text-queries.json")).unwrap();
    let ordinary = &fixture["ordinary"];
    let dir = temp();
    let root = dir.path();
    for (name, text) in ordinary["files"].as_object().unwrap() {
        fs::write(root.join(name), text.as_str().unwrap()).unwrap();
    }
    let build = run(root, &["-bk", "-f", "index", "z.c", "b.c"]);
    assert!(build.status.success(), "{build:?}");
    for (name, text) in ordinary["live_files"].as_object().unwrap() {
        fs::write(root.join(name), text.as_str().unwrap()).unwrap();
    }
    for profile in ordinary["profiles"].as_array().unwrap() {
        let insensitive = profile["caseless"].as_bool().unwrap();
        for query in profile["queries"].as_array().unwrap() {
            let pattern = query["args"]
                .as_array()
                .unwrap()
                .last()
                .unwrap()
                .as_str()
                .unwrap();
            // Separately specified corrections below cover asymmetric reference case
            // folding and malformed EOF row framing. Do not rewrite oracle output.
            if pattern.is_empty()
                || pattern == " "
                || pattern == "tail"
                || (insensitive && pattern.bytes().any(|b| b.is_ascii_uppercase()))
            {
                continue;
            }
            let mut args = vec!["-dL", "-f", "index"];
            if insensitive {
                args.push("-C");
            }
            args.extend(["-4", pattern]);
            let output = run(root, &args);
            assert!(output.status.success(), "{args:?}: {output:?}");
            assert!(output.stderr.is_empty(), "{output:?}");
            assert_eq!(
                output.stdout,
                query["stdout"].as_str().unwrap().as_bytes(),
                "{args:?}"
            );
        }
    }
    let corrected = run(root, &["-dL", "-f", "index", "-C", "-4", "Needle"]);
    assert!(corrected.status.success(), "{corrected:?}");
    assert_eq!(corrected.stdout, b"b.c <unknown> 1 // Needle Needle\nb.c <unknown> 2 \t// needle.*[\\^$+?|(){2}\t \nb.c <unknown> 3 // NEEDLE\r\nz.c <unknown> 1 // Needle\nz.c <unknown> 2 // xneedleY\na.h <unknown> 1 // Needle header\n");
    let tail = run(root, &["-dL", "-f", "index", "-4", "tail"]);
    assert!(tail.status.success(), "{tail:?}");
    assert_eq!(tail.stdout, b"b.c <unknown> 5 // tail\n");
}

#[cfg(unix)]
#[test]
fn a_text_search_skips_a_source_it_cannot_read() {
    use std::os::unix::fs::PermissionsExt;
    let dir = temp();
    let root = dir.path();
    fs::write(root.join("a.c"), "int Alpha;\n").unwrap();
    fs::write(root.join("z.c"), "int AlphaZ;\n").unwrap();
    assert!(
        run(root, &["-bk", "-f", "index", "a.c", "z.c"])
            .status
            .success()
    );
    fs::set_permissions(root.join("z.c"), fs::Permissions::from_mode(0o0)).unwrap();
    assert!(
        fs::read(root.join("z.c")).is_err(),
        "permission test requires an unprivileged user"
    );
    // The file is skipped with a warning (docs/spec.md, "Keep going instead of failing").
    let skipped = run(root, &["-dL", "-f", "index", "-4", "Alpha"]);
    assert!(skipped.status.success(), "{skipped:?}");
    assert_eq!(skipped.stdout, b"a.c <unknown> 1 int Alpha;\n");
    assert!(
        String::from_utf8_lossy(&skipped.stderr).starts_with("ripscope: cannot read z.c: "),
        "{skipped:?}"
    );
}

#[test]
fn normal_text_query_refreshes_membership_but_stored_text_does_not() {
    let dir = temp();
    let root = dir.path();
    fs::write(root.join("a.c"), "int Alpha;\n").unwrap();
    assert!(run(root, &["-bk", "-f", "index", "a.c"]).status.success());
    fs::write(root.join("a.c"), "#include \"new.h\"\nint Alpha;\n").unwrap();
    fs::write(root.join("new.h"), "int NewMember;\n").unwrap();
    assert!(
        run(root, &["-dL", "-f", "index", "-4", "NewMember"])
            .stdout
            .is_empty()
    );
    let fresh = run(root, &["-L", "-f", "index", "-4", "NewMember"]);
    assert!(fresh.status.success(), "{fresh:?}");
    assert_eq!(fresh.stdout, b"new.h <unknown> 1 int NewMember;\n");
    let stored = run(root, &["-dL", "-f", "index", "-4", "NewMember"]);
    assert_eq!(stored.stdout, fresh.stdout);
    assert!(stored.status.success());
}

#[cfg(unix)]
#[test]
fn stored_text_follows_the_current_target_of_a_lexical_symlink() {
    use std::os::unix::fs::symlink;
    let dir = temp();
    let root = dir.path();
    fs::write(root.join("first.c"), "int First;\n").unwrap();
    fs::write(root.join("second.c"), "// Current live bytes\n").unwrap();
    symlink("first.c", root.join("alias.c")).unwrap();
    assert!(
        run(root, &["-bk", "-f", "index", "alias.c"])
            .status
            .success()
    );
    fs::remove_file(root.join("alias.c")).unwrap();
    symlink("second.c", root.join("alias.c")).unwrap();
    let result = run(root, &["-dL", "-f", "index", "-4", "Current"]);
    assert!(result.status.success(), "{result:?}");
    assert_eq!(
        result.stdout,
        b"alias.c <unknown> 1 // Current live bytes\n"
    );
}
