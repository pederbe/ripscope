//! Selection rules: which files an index covers, re-applied at each refresh as
//! cscope applies them at each index build.

use std::path::Path;
use std::process::{Command, Output, Stdio};

/// ripscope in `directory`, without the environment's include directories.
fn command(directory: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_ripscope"));
    command
        .current_dir(directory)
        .env_remove("INCLUDEDIRS")
        .env_remove("INCDIR");
    command
}

fn run(directory: &Path, args: &[&str]) -> Output {
    command(directory)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .expect("ripscope")
}

fn write(directory: &Path, name: &str, text: &str) {
    std::fs::write(directory.join(name), text).expect("write");
}

/// The files an index answers for, from field 7 (every file, in database order).
fn files(directory: &Path, index: &str) -> String {
    let listed = run(directory, &["-dL", "-f", index, "-7", ".*"]);
    assert!(listed.status.success(), "{listed:?}");
    String::from_utf8_lossy(&listed.stdout)
        .lines()
        .map(|row| row.split(' ').next().unwrap_or_default().to_owned())
        .collect::<Vec<_>>()
        .join(" ")
}

#[test]
fn a_missing_named_file_is_reported_and_no_file_at_all_fails_without_publishing() {
    let work = tempfile::tempdir().expect("work");
    write(work.path(), "a.c", "int Alpha;\n");
    // As cscope: each missing file named, the rest indexed.
    let built = run(work.path(), &["-bk", "-f", "i.db", "a.c", "missing.c"]);
    assert!(built.status.success(), "{built:?}");
    assert!(
        String::from_utf8_lossy(&built.stderr).contains("cannot find file missing.c"),
        "{built:?}"
    );
    assert_eq!(files(work.path(), "i.db"), "a.c");

    // Nothing to index: as cscope, each missing name in the order given, then the
    // failure; as cscope, an index build that finds nothing makes no index.
    let empty = run(
        work.path(),
        &["-bk", "-f", "new.db", "other.c", "missing.c"],
    );
    assert!(!empty.status.success(), "{empty:?}");
    assert_eq!(
        String::from_utf8_lossy(&empty.stderr),
        "ripscope: cannot find file other.c\nripscope: cannot find file missing.c\n\
         ripscope: no source files found\n"
    );
    let left: Vec<String> = std::fs::read_dir(work.path())
        .expect("listing")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .filter(|name| name.starts_with("new.db"))
        .collect();
    assert!(left.is_empty(), "{left:?}");
    // An empty file at the index path stays as it was, and a query that would build a
    // new index makes none either.
    write(work.path(), "empty.db", "");
    assert!(
        !run(work.path(), &["-bk", "-f", "empty.db", "missing.c"])
            .status
            .success()
    );
    assert_eq!(
        std::fs::read(work.path().join("empty.db")).expect("kept"),
        b""
    );
    let nothing = tempfile::tempdir().expect("nothing");
    let query = run(nothing.path(), &["-L", "-f", "new.db", "-1", "Alpha"]);
    assert!(!query.status.success(), "{query:?}");
    assert_eq!(
        String::from_utf8_lossy(&query.stderr),
        "ripscope: no source files found\n"
    );
    assert!(!nothing.path().join("new.db").exists());

    // Every selected file gone: an update fails, and the index keeps what it had.
    std::fs::remove_file(work.path().join("a.c")).expect("removed");
    // The rule names both files, and cscope reports each.
    let expected = "ripscope: cannot find file a.c\nripscope: cannot find file missing.c\n\
                    ripscope: no source files found\n";
    for update in [
        &["-L", "-f", "i.db", "-1", "Alpha"][..],
        &["--verify", "-f", "i.db"],
    ] {
        let failed = run(work.path(), update);
        assert!(!failed.status.success(), "{failed:?}");
        assert_eq!(String::from_utf8_lossy(&failed.stderr), expected);
    }
    let stored = run(work.path(), &["-dL", "-f", "i.db", "-1", "Alpha"]);
    assert_eq!(stored.stdout, b"a.c Alpha 1 int Alpha;\n");
}

#[test]
fn a_session_keeps_its_index_when_a_refresh_finds_no_files() {
    let work = tempfile::tempdir().expect("work");
    write(work.path(), "a.c", "int Alpha;\n");
    assert!(
        run(work.path(), &["-bk", "-f", "i.db", "a.c"])
            .status
            .success()
    );
    std::fs::remove_file(work.path().join("a.c")).expect("removed");
    // Standard output and standard error in one stream, as Vim's connection reads them.
    let (mut reader, writer) = std::io::pipe().expect("pipe");
    let mut child = command(work.path())
        .args(["-dl", "-f", "i.db"])
        .stdin(Stdio::piped())
        .stdout(writer.try_clone().expect("writer"))
        .stderr(writer)
        .spawn()
        .expect("session");
    {
        use std::io::Write;
        let mut input = child.stdin.take().expect("stdin");
        input.write_all(b"r\n1Alpha\nq\n").expect("requests");
    }
    let mut output = Vec::new();
    std::io::Read::read_to_end(&mut reader, &mut output).expect("output");
    assert!(child.wait().expect("status").success());
    // As cscope, the missing files are named before the empty line that answers `r`, and
    // the session answers from the index it has.
    assert_eq!(
        String::from_utf8_lossy(&output),
        ">> ripscope: cannot find file a.c\nripscope: no source files found\n\
         \n>> cscope: 1 lines\na.c Alpha 1 int Alpha;\n>> "
    );
}

#[test]
fn automatic_discovery_is_applied_again_at_each_refresh() {
    let work = tempfile::tempdir().expect("work");
    write(work.path(), "a.c", "int Alpha;\n");
    write(work.path(), "b.c", "int Beta;\n");
    assert!(run(work.path(), &["-bk", "-f", "i.db"]).status.success());
    assert_eq!(files(work.path(), "i.db"), "a.c b.c");
    // A list appears: it names the files from the next refresh on.
    write(work.path(), "cscope.files", "b.c\n");
    assert!(
        run(work.path(), &["-L", "-f", "i.db", "-1", "Beta"])
            .status
            .success()
    );
    assert_eq!(files(work.path(), "i.db"), "b.c");
    // And when it goes, the directory's sources again.
    std::fs::remove_file(work.path().join("cscope.files")).expect("removed");
    assert!(
        run(work.path(), &["-L", "-f", "i.db", "-1", "Beta"])
            .status
            .success()
    );
    assert_eq!(files(work.path(), "i.db"), "a.c b.c");
}

#[test]
fn an_explicit_index_build_changes_the_rule_that_refresh_and_verify_preserve() {
    let work = tempfile::tempdir().expect("work");
    write(work.path(), "a.c", "int Alpha;\n");
    write(work.path(), "b.c", "int Beta;\n");
    assert!(
        run(work.path(), &["-bk", "-f", "i.db", "a.c"])
            .status
            .success()
    );
    let changed = run(work.path(), &["-bk", "-f", "i.db", "b.c"]);
    assert!(changed.status.success(), "{changed:?}");
    assert_eq!(files(work.path(), "i.db"), "b.c");
    // Refresh and verification ignore the invocation's automatic rule once a
    // rule is published, including when every source is hashed again.
    let refreshed = run(work.path(), &["-Lk", "-f", "i.db", "-1", "Beta"]);
    assert!(refreshed.status.success(), "{refreshed:?}");
    assert_eq!(refreshed.stdout, b"b.c Beta 1 int Beta;\n");
    for _ in 0..2 {
        let verified = run(work.path(), &["--verify", "-f", "i.db"]);
        assert!(verified.status.success(), "{verified:?}");
        assert!(verified.stderr.is_empty(), "{verified:?}");
        assert_eq!(verified.stdout, b"index verified: revision 2, 1 file\n");
    }
    assert_eq!(files(work.path(), "i.db"), "b.c");
}

#[test]
fn file_lists_are_read_as_cscope_reads_them() {
    let work = tempfile::tempdir().expect("work");
    write(work.path(), "a.c", "int Alpha;\n");
    write(work.path(), "b.c", "int Beta;\n");
    write(work.path(), "extra.c", "int Extra;\n");
    // A name listed twice is indexed twice, as cscope does; a name on the command line
    // beside `-i` is not read at all.
    write(work.path(), "list.txt", "b.c\na.c\nb.c\n");
    let built = run(
        work.path(),
        &["-bk", "-f", "i.db", "-i", "list.txt", "extra.c"],
    );
    assert!(built.status.success(), "{built:?}");
    assert_eq!(files(work.path(), "i.db"), "a.c b.c b.c");
    assert_eq!(
        run(work.path(), &["-dL", "-f", "i.db", "-1", "Beta"]).stdout,
        b"b.c Beta 1 int Beta;\nb.c Beta 1 int Beta;\n"
    );
    // Names on the command line are read once each.
    assert!(
        run(work.path(), &["-bk", "-f", "j.db", "a.c", "a.c"])
            .status
            .success()
    );
    assert_eq!(files(work.path(), "j.db"), "a.c");
}

/// An index built in a directory whose path is not UTF-8 keeps its selection rule: an update from
/// elsewhere applies the stored rule, and a text search reads below the stored root.
#[cfg(unix)]
#[test]
fn a_build_directory_whose_path_is_not_utf8_keeps_its_rule() {
    use std::os::unix::ffi::OsStrExt;
    let work = tempfile::tempdir().expect("work");
    let root = work.path().join(std::ffi::OsStr::from_bytes(b"caf\xe9"));
    let elsewhere = work.path().join("elsewhere");
    std::fs::create_dir_all(root.join("sub")).expect("directories");
    std::fs::create_dir(&elsewhere).expect("elsewhere");
    write(&root, "top.c", "int Top;\n");
    write(&root, "sub/nested.c", "int Nested;\n");
    // A file the command line's rule would select from there, and the stored one does not.
    write(&elsewhere, "unrelated.c", "int Unrelated;\n");
    let built = run(&root, &["-bRk", "-f", "../elsewhere/i.db"]);
    assert!(built.status.success(), "{built:?}");

    write(&root, "sub/nested.c", "int Nested;\nint Later;\n");
    let query = run(&elsewhere, &["-L", "-f", "i.db", "-1", "Later"]);
    assert!(query.status.success(), "{query:?}");
    assert_eq!(
        String::from_utf8_lossy(&query.stdout),
        "sub/nested.c Later 2 int Later;\n"
    );
    assert_eq!(files(&elsewhere, "i.db"), "sub/nested.c top.c");
    let text = run(&elsewhere, &["-dL", "-f", "i.db", "-4", "Top"]);
    assert!(text.status.success(), "{text:?}");
    assert_eq!(
        String::from_utf8_lossy(&text.stdout),
        "top.c <unknown> 1 int Top;\n"
    );
}

/// A stored selection rule that cannot be read fails the update and the query: applying
/// the command line's rule instead would index other files. Rebuilding the index repairs it.
#[test]
fn a_stored_rule_that_cannot_be_read_is_an_error() {
    let work = tempfile::tempdir().expect("work");
    write(work.path(), "a.c", "int Alpha;\n");
    assert!(run(work.path(), &["-bk", "-f", "i.db"]).status.success());
    rusqlite::Connection::open(work.path().join("i.db"))
        .and_then(|db| db.execute("UPDATE meta SET value = x'00' WHERE key = 'selection'", []))
        .expect("damaged");
    write(work.path(), "b.c", "int Beta;\n");
    for args in [
        &["-L", "-f", "i.db", "-1", "Alpha"][..],
        &["-dL", "-f", "i.db", "-1", "Alpha"],
        &["--verify", "-f", "i.db"],
        &["--verify", "-d", "-f", "i.db"],
    ] {
        let failed = run(work.path(), args);
        assert!(!failed.status.success(), "{failed:?}");
        assert_eq!(
            String::from_utf8_lossy(&failed.stderr),
            "ripscope: the index's selection rule cannot be read; \
             rebuild it with -bu and the options it was built with\n"
        );
    }
    assert!(run(work.path(), &["-bku", "-f", "i.db"]).status.success());
    assert_eq!(files(work.path(), "i.db"), "a.c b.c");
}

/// Patterns are bytes, as cscope's are in the C locale: a Latin-1 byte finds a
/// file name, an include and text holding it, in both transports and in a session.
#[cfg(unix)]
#[test]
fn patterns_that_are_not_utf8_find_what_holds_their_bytes() {
    use std::ffi::{OsStr, OsString};
    use std::os::unix::ffi::OsStrExt;
    let work = tempfile::tempdir().expect("work");
    let bytes = |text: &[u8]| OsStr::from_bytes(text).to_owned();
    std::fs::write(
        work.path().join(bytes(b"caf\xe9.c")),
        b"int Alpha; /* na\xefve */\n",
    )
    .expect("source");
    std::fs::write(work.path().join(bytes(b"caf\xe9.h")), b"int Delta;\n").expect("header");
    std::fs::write(
        work.path().join("main.c"),
        b"#include \"caf\xe9.h\"\nint Gamma;\n",
    )
    .expect("main");
    assert!(run(work.path(), &["-bk", "-f", "i.db"]).status.success());
    let query = |args: &[OsString]| {
        command(work.path())
            .args(args)
            .stdin(Stdio::null())
            .output()
            .expect("ripscope")
    };
    let args = |field: &str, pattern: &[u8]| -> Vec<OsString> {
        ["-dL", "-f", "i.db", field]
            .iter()
            .map(OsString::from)
            .chain([bytes(pattern)])
            .collect()
    };
    let files = query(&args("-7", b"caf\xe9"));
    assert!(files.status.success(), "{files:?}");
    assert_eq!(
        files.stdout,
        b"caf\xe9.c <unknown> 1 <unknown>\ncaf\xe9.h <unknown> 1 <unknown>\n"
    );
    let includers = query(&args("-8", b"caf\xe9.h"));
    assert_eq!(
        includers.stdout,
        b"main.c <global> 1 #include \"caf\xe9.h\"\n"
    );
    let text = query(&args("-4", b"na\xefve"));
    assert_eq!(
        text.stdout,
        b"caf\xe9.c <unknown> 1 int Alpha; /* na\xefve */\n"
    );
    let mut json = args("-7", b"caf\xe9.c");
    json.insert(0, OsString::from("--json"));
    let json = query(&json);
    let row: serde_json::Value = serde_json::from_slice(&json.stdout).expect("row");
    assert_eq!(row[0], serde_json::json!([99, 97, 102, 233, 46, 99]));
    let session = command(work.path())
        .args(["-dl", "-f", "i.db"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            std::io::Write::write_all(&mut child.stdin.take().expect("stdin"), b"7caf\xe9.c\nq\n")?;
            child.wait_with_output()
        })
        .expect("session");
    assert_eq!(
        session.stdout,
        b">> cscope: 1 lines\ncaf\xe9.c <unknown> 1 <unknown>\n>> "
    );
}

#[test]
fn paths_and_their_patterns_keep_every_character() {
    let work = tempfile::tempdir().expect("work");
    write(work.path(), "ü.c", "#include \"ö.h\"\nint Umlaut;\n");
    write(work.path(), "ö.h", "int Header;\n");
    assert!(
        run(work.path(), &["-bk", "-f", "i.db", "ü.c"])
            .status
            .success()
    );
    assert_eq!(
        String::from_utf8_lossy(&run(work.path(), &["-dL", "-f", "i.db", "-7", "ü"]).stdout),
        "ü.c <unknown> 1 <unknown>\n"
    );
    assert_eq!(
        String::from_utf8_lossy(&run(work.path(), &["-dL", "-f", "i.db", "-8", "ö"]).stdout),
        "ü.c <global> 1 #include \"ö.h\"\n"
    );
    #[cfg(windows)]
    {
        // A Windows path given with `\` is kept with `/`, as discovery and includes
        // spell it.
        let absolute = work.path().join("ü.c");
        let name = absolute.to_str().expect("path");
        assert!(
            run(work.path(), &["-bk", "-f", "j.db", name])
                .status
                .success()
        );
        let answer = run(
            work.path(),
            &["--json", "-dL", "-f", "j.db", "-1", "Umlaut"],
        );
        let row: serde_json::Value = serde_json::from_slice(&answer.stdout).expect("row");
        let shown = row[0].as_str().expect("path");
        assert!(!shown.contains('\\'), "{shown}");
        assert_eq!(Path::new(shown), absolute);
    }
}
