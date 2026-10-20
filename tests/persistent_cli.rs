mod intended;

use std::{
    fs,
    io::{Read, Write},
    path::Path,
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc::{self, Receiver},
    time::Duration,
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

fn build(root: &Path) {
    let output = Command::new(env!("CARGO_BIN_EXE_ripscope"))
        .current_dir(root)
        .args(["-bk", "-f", "index", "case.c"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

struct Session {
    child: Child,
    input: Option<ChildStdin>,
    output: Receiver<u8>,
}

impl Session {
    fn start(root: &Path, mode: &str) -> Self {
        Self::start_with(root, &[mode, "-f", "index"])
    }

    fn start_with(root: &Path, args: &[&str]) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_ripscope"))
            .current_dir(root)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let input = child.stdin.take();
        let mut stdout = child.stdout.take().unwrap();
        let (send, output) = mpsc::sync_channel(1024);
        std::thread::spawn(move || {
            let mut byte = [0];
            while stdout.read(&mut byte).unwrap_or(0) != 0 {
                if send.send(byte[0]).is_err() {
                    break;
                }
            }
        });
        Self {
            child,
            input,
            output,
        }
    }

    fn send(&mut self, bytes: &[u8]) {
        let input = self.input.as_mut().unwrap();
        input.write_all(bytes).unwrap();
        input.flush().unwrap();
    }

    fn expect(&self, expected: &[u8]) {
        let actual: Vec<_> = (0..expected.len())
            .map(|_| {
                self.output
                    .recv_timeout(Duration::from_secs(20))
                    .expect("response or flushed prompt")
            })
            .collect();
        assert_eq!(actual, expected);
    }

    fn json_line(&self) -> serde_json::Value {
        let mut line = Vec::new();
        loop {
            let byte = self
                .output
                .recv_timeout(Duration::from_secs(20))
                .expect("JSON response");
            if byte == b'\n' {
                break;
            }
            line.push(byte);
        }
        serde_json::from_slice(&line).unwrap()
    }

    fn finish(&mut self) {
        self.input.take();
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                assert!(status.success());
                break;
            }
            assert!(std::time::Instant::now() < deadline, "session did not exit");
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(self.output.recv_timeout(Duration::from_secs(1)).is_err());
    }

    fn expect_unconfirmed_refresh(&mut self) {
        let mut diagnostic = String::new();
        self.child
            .stderr
            .take()
            .unwrap()
            .read_to_string(&mut diagnostic)
            .unwrap();
        assert!(
            diagnostic.contains("ripscope: cannot find file case.c\n"),
            "{diagnostic}"
        );
        assert!(diagnostic.contains("automatic Refresh did not complete (no source files found); answering from the last published revision, whose freshness is unconfirmed"), "{diagnostic}");
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn stored_session_reads_committed_wal_and_releases_handles_for_idle_replacement() {
    let dir = temp();
    let root = dir.path();
    fs::write(root.join("case.c"), "int Alpha;\n").unwrap();
    build(root);
    let mut session = Session::start(root, "-dl");
    session.expect(b">> ");
    fs::remove_file(root.join("case.c")).unwrap();
    for _ in 0..2 {
        session.send(b"1Alpha\n");
        session.expect(b"cscope: 1 lines\ncase.c Alpha 1 int Alpha;\n>> ");
        for name in ["index-wal", "index-shm"] {
            assert!(
                root.join(name).is_file(),
                "{name} retained between requests"
            );
        }
    }
    // Keep an old read transaction across another writer's Publication, leaving the
    // new committed revision in the WAL after both connections close.
    let mut keeper = rusqlite::Connection::open(root.join("index")).unwrap();
    keeper
        .set_db_config(
            rusqlite::config::DbConfig::SQLITE_DBCONFIG_NO_CKPT_ON_CLOSE,
            true,
        )
        .unwrap();
    let transaction = keeper.transaction().unwrap();
    transaction
        .query_row("SELECT count(*) FROM files", [], |row| row.get::<_, i64>(0))
        .unwrap();
    fs::write(root.join("case.c"), "int Bravo;\n").unwrap();
    // A normal query's Refresh does not wait for the old reader's checkpoint lock.
    let published = Command::new(env!("CARGO_BIN_EXE_ripscope"))
        .current_dir(root)
        .args(["-L", "-f", "index", "-1", "Bravo"])
        .output()
        .unwrap();
    assert!(
        published.status.success() && published.stderr.is_empty(),
        "{published:?}"
    );
    assert_eq!(published.stdout, b"case.c Bravo 1 int Bravo;\n");
    drop(transaction);
    drop(keeper);
    assert!(fs::metadata(root.join("index-wal")).unwrap().len() > 32);
    fs::remove_file(root.join("case.c")).unwrap();
    session.send(b"1Bravo\n");
    session.expect(b"cscope: 1 lines\ncase.c Bravo 1 int Bravo;\n>> ");
    let verified = Command::new(env!("CARGO_BIN_EXE_ripscope"))
        .current_dir(root)
        .args(["-d", "--verify", "-f", "index"])
        .output()
        .unwrap();
    assert!(verified.status.success(), "{verified:?}");
    // Move the complete Index, including any remaining logs, while the Session is idle.
    // This also fails on Windows if any connection still holds a file open.
    for name in ["index", "index-wal", "index-shm"] {
        if root.join(name).exists() {
            fs::rename(root.join(name), root.join(format!("old-{name}"))).unwrap();
        }
    }
    fs::write(root.join("case.c"), "int Replacement;\n").unwrap();
    build(root);
    session.send(b"1Bravo\n1Replacement\n");
    session.expect(
        b"Unable to search database\n>> cscope: 1 lines\ncase.c Replacement 1 int Replacement;\n>> ",
    );
    session.finish();
    let moved = Command::new(env!("CARGO_BIN_EXE_ripscope"))
        .current_dir(root)
        .args(["-dL", "-f", "old-index", "-1", "Bravo"])
        .output()
        .unwrap();
    assert!(
        moved.status.success() && moved.stderr.is_empty(),
        "{moved:?}"
    );
    assert_eq!(moved.stdout, b"case.c Bravo 1 int Bravo;\n");
}

#[test]
fn stored_occurrence_context_matches_reference_and_isolates_requests() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/occurrence-context.json")).unwrap();
    let dir = temp();
    let root = dir.path();
    let files = fixture["fields"]["0"]["files"].as_object().unwrap();
    for (path, source) in files {
        fs::write(root.join(path), source.as_str().unwrap()).unwrap();
    }
    let mut build = Command::new(env!("CARGO_BIN_EXE_ripscope"));
    build.current_dir(root).args(["-bk", "-f", "index"]);
    for seed in fixture["fields"]["0"]["seeds"].as_array().unwrap() {
        build.arg(seed.as_str().unwrap());
    }
    let built = build.output().unwrap();
    assert!(
        built.status.success(),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );
    for path in files.keys() {
        fs::remove_file(root.join(path)).unwrap();
    }
    let mut session = Session::start(root, "-dl");
    session.expect(b">> ");
    for profile_index in 0..2 {
        if profile_index == 1 {
            session.send(b"c\n");
            session.expect(b">> ");
        }
        for field in ["0", "9", "0"] {
            for query in fixture["fields"][field]["profiles"][profile_index]["queries"]
                .as_array()
                .unwrap()
            {
                let args: Vec<_> = query["args"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|arg| arg.as_str().unwrap())
                    .collect();
                let key = args.join(" ");
                let recorded = query["stdout"].as_str().unwrap();
                let expected = &intended::expected("occurrence-context.json", &key, recorded);
                assert_eq!(query["exit_code"], 0);
                assert_eq!(query["stderr"], "");
                let output = Command::new(env!("CARGO_BIN_EXE_ripscope"))
                    .current_dir(root)
                    .args(["-f", "index"])
                    .args(&args)
                    .output()
                    .unwrap();
                assert!(output.status.success());
                assert!(
                    output.stderr.is_empty(),
                    "{}",
                    String::from_utf8_lossy(&output.stderr)
                );
                intended::check(
                    "occurrence-context.json",
                    &key,
                    recorded,
                    &String::from_utf8_lossy(&output.stdout),
                );
                session.send(format!("{field}{}\n", args.last().unwrap()).as_bytes());
                if expected.is_empty() {
                    session.expect(b"Unable to search database\n>> ");
                } else {
                    session.expect(
                        format!("cscope: {} lines\n{expected}>> ", expected.lines().count())
                            .as_bytes(),
                    );
                }
            }
        }
    }
    session.send(b"q\n");
    session.finish();
}

#[test]
fn rebuild_refreshes_stored_selection_and_preserves_query_lifecycle() {
    for manual in ["-dl", "--no-auto-refresh"] {
        let dir = temp();
        let root = dir.path();
        fs::write(root.join("case.c"), "int Alpha;\n").unwrap();
        build(root);
        let mut session = Session::start_with(root, &["-l", manual, "-f", "index"]);
        session.expect(b">> ");
        session.send(b"r\nc\n");
        session.expect(b"\n>> >> ");
        fs::write(root.join("case.c"), "#include \"added.h\"\nint Beta;\n").unwrap();
        fs::write(root.join("added.h"), "int Gamma;\n").unwrap();
        session.send(b"1alpha\nr\r\n1beta\n7added\n");
        session.expect(b"cscope: 1 lines\ncase.c alpha 1 int Alpha;\n>> \n>> cscope: 1 lines\ncase.c beta 2 int Beta;\n>> cscope: 1 lines\nadded.h <unknown> 1 <unknown>\n>> ");
        fs::write(root.join("case.c"), "int Delta;\n").unwrap();
        session.send(b"1beta\nr\n7added\n1delta\n");
        session.expect(b"cscope: 1 lines\ncase.c beta 2 int Beta;\n>> \n>> Unable to search database\n>> cscope: 1 lines\ncase.c delta 1 int Delta;\n>> ");
        fs::write(root.join("case.c"), "int External;\n").unwrap();
        build(root);
        session.send(b"r\n1external\n");
        session.expect(b"\n>> cscope: 1 lines\ncase.c external 1 int External;\n>> ");
        fs::write(root.join("other.c"), "int Selected;\n").unwrap();
        assert!(
            Command::new(env!("CARGO_BIN_EXE_ripscope"))
                .current_dir(root)
                .args(["-bk", "-f", "index", "other.c"])
                .status()
                .unwrap()
                .success()
        );
        fs::write(root.join("other.c"), "int Changed;\n").unwrap();
        session.send(b"r\n1changed\n7case\n");
        session.expect(b"\n>> cscope: 1 lines\nother.c changed 1 int Changed;\n>> Unable to search database\n>> ");
        session.send(b"r");
        session.input.take();
        session.expect(b"\n>> ");
        session.finish();
    }
}

#[test]
fn rebuild_source_failure_keeps_queries_and_retry_in_the_same_process() {
    for manual in ["-dl", "--no-auto-refresh"] {
        let dir = temp();
        let root = dir.path();
        fs::write(root.join("case.c"), "int Alpha;\n").unwrap();
        build(root);
        let mut session = Session::start_with(root, &["-l", manual, "--vim-errors", "-f", "index"]);
        session.expect(b">> ");
        session.send(b"c\n");
        session.expect(b">> ");
        fs::remove_file(root.join("case.c")).unwrap();
        session.send(b"r\n1alpha\n");
        session.expect(b"\n>> cscope: 1 lines\ncase.c alpha 1 int Alpha;\n>> ");
        fs::write(root.join("case.c"), "int Beta;\n").unwrap();
        session.send(b"r\n1beta\nq\n");
        session.expect(b"\n>> cscope: 1 lines\ncase.c beta 1 int Beta;\n>> ");
        session.finish();
        let mut diagnostic = String::new();
        session
            .child
            .stderr
            .take()
            .unwrap()
            .read_to_string(&mut diagnostic)
            .unwrap();
        // The failed refresh named the file it missed, and the session carried on.
        assert_eq!(
            diagnostic,
            "ripscope: cannot find file case.c\nripscope: no source files found\n"
        );
    }
}

#[test]
fn filename_session_tracks_committed_membership_without_reading_sources() {
    let dir = temp();
    let root = dir.path();
    fs::write(root.join("case.c"), "int Alpha;\n").unwrap();
    build(root);
    fs::write(root.join("case.c"), "#include \"added.h\"\nint Alpha;\n").unwrap();
    fs::write(root.join("added.h"), "int Beta;\n").unwrap();
    let mut session = Session::start(root, "-l");
    session.expect(b">> ");
    fs::remove_file(root.join("case.c")).unwrap();
    fs::remove_file(root.join("added.h")).unwrap();
    session.send(b"7.\n");
    session.expect(
        b"cscope: 2 lines\ncase.c <unknown> 1 <unknown>\nadded.h <unknown> 1 <unknown>\n>> ",
    );
    session.send(b"7ADDED\n7[\n7\n7missing\n");
    for _ in 0..4 {
        session.expect(b"Unable to search database\n>> ");
    }
    session.send(b"c\n7ADDED\n");
    session.expect(b">> cscope: 1 lines\nadded.h <unknown> 1 <unknown>\n>> ");
    fs::write(root.join("case.c"), "#include \"new.h\"\nint Alpha;\n").unwrap();
    fs::write(root.join("new.h"), "int Gamma;\n").unwrap();
    build(root);
    session.send(b"7\\.h$\n");
    session.expect(b"cscope: 1 lines\nnew.h <unknown> 1 <unknown>\n>> ");
    session.finish();
    fs::remove_file(root.join("case.c")).unwrap();
    fs::remove_file(root.join("new.h")).unwrap();
    let mut stored = Session::start(root, "-dl");
    stored.expect(b">> ");
    stored.send(b"7new");
    stored.input.take();
    stored.expect(b"cscope: 1 lines\nnew.h <unknown> 1 <unknown>\n>> ");
    stored.finish();
}

#[test]
fn callees_refresh_saved_edits_and_keep_last_facts_without_sources() {
    let dir = temp();
    let root = dir.path();
    fs::write(root.join("case.c"), "int old(void) { target(); }\n").unwrap();
    build(root);
    fs::write(root.join("case.c"), "int fresh(void) { target(); }\n").unwrap();
    let mut session = Session::start(root, "-l");
    session.expect(b">> ");
    fs::remove_file(root.join("case.c")).unwrap();
    session.send(b"2.*\n");
    session.expect(b"cscope: 1 lines\ncase.c target 1 int fresh(void ) { target(); }\n>> ");
    fs::write(
        root.join("case.c"),
        "#include \"added.h\"\nint newer(void) { target(); }\n",
    )
    .unwrap();
    fs::write(root.join("added.h"), "int header(void) { target(); }\n").unwrap();
    session.send(b"2.*\n");
    session.expect(b"cscope: 2 lines\ncase.c target 2 int newer(void ) { target(); }\nadded.h target 1 int header(void ) { target(); }\n>> ");
    build(root);
    fs::remove_file(root.join("case.c")).unwrap();
    fs::remove_file(root.join("added.h")).unwrap();
    session.send(b"2[\n2missing\n2.*\n");
    session.expect(b"Unable to search database\n>> Unable to search database\n>> cscope: 2 lines\ncase.c target 2 int newer(void ) { target(); }\nadded.h target 1 int header(void ) { target(); }\n>> ");
    session.finish();
    session.expect_unconfirmed_refresh();
}

#[test]
fn assignments_refresh_saved_edits_and_keep_last_facts_without_sources() {
    let dir = temp();
    let root = dir.path();
    fs::write(root.join("case.c"), "int old(void) { target = 1; }\n").unwrap();
    build(root);
    fs::write(root.join("case.c"), "int fresh(void) { target = 1; }\n").unwrap();
    let mut session = Session::start(root, "-l");
    session.expect(b">> ");
    fs::remove_file(root.join("case.c")).unwrap();
    session.send(b"9target\n");
    session.expect(b"cscope: 1 lines\ncase.c fresh 1 int fresh(void ) { target = 1; }\n>> ");
    fs::write(
        root.join("case.c"),
        "#include \"added.h\"\nint newer(void) { target = 1; }\n",
    )
    .unwrap();
    fs::write(root.join("added.h"), "int header(void) { target = 1; }\n").unwrap();
    session.send(b"9target\n");
    session.expect(b"cscope: 2 lines\ncase.c newer 2 int newer(void ) { target = 1; }\nadded.h header 1 int header(void ) { target = 1; }\n>> ");
    build(root);
    fs::remove_file(root.join("case.c")).unwrap();
    fs::remove_file(root.join("added.h")).unwrap();
    session.send(b"9[\n9missing\n9target\n");
    session.expect(b"Unable to search database\n>> Unable to search database\n>> cscope: 2 lines\ncase.c newer 2 int newer(void ) { target = 1; }\nadded.h header 1 int header(void ) { target = 1; }\n>> ");
    session.finish();
    session.expect_unconfirmed_refresh();
}

#[test]
fn callers_refresh_saved_edits_and_keep_last_facts_without_sources() {
    let dir = temp();
    let root = dir.path();
    fs::write(root.join("case.c"), "int old(void) { target(); }\n").unwrap();
    build(root);
    fs::write(root.join("case.c"), "int fresh(void) { target(); }\n").unwrap();
    let mut session = Session::start(root, "-l");
    session.expect(b">> ");
    fs::remove_file(root.join("case.c")).unwrap();
    session.send(b"3target\n");
    session.expect(b"cscope: 1 lines\ncase.c fresh 1 int fresh(void ) { target(); }\n>> ");
    fs::write(
        root.join("case.c"),
        "#include \"added.h\"\nint newer(void) { target(); }\n",
    )
    .unwrap();
    fs::write(root.join("added.h"), "int header(void) { target(); }\n").unwrap();
    session.send(b"3target\n");
    session.expect(b"cscope: 2 lines\ncase.c newer 2 int newer(void ) { target(); }\nadded.h header 1 int header(void ) { target(); }\n>> ");
    build(root);
    fs::remove_file(root.join("case.c")).unwrap();
    fs::remove_file(root.join("added.h")).unwrap();
    session.send(b"3[\n3missing\n3target\n");
    session.expect(b"Unable to search database\n>> Unable to search database\n>> cscope: 2 lines\ncase.c newer 2 int newer(void ) { target(); }\nadded.h header 1 int header(void ) { target(); }\n>> ");
    session.finish();
    session.expect_unconfirmed_refresh();
}

#[test]
fn includes_refresh_saved_edits_and_keep_last_facts_without_sources() {
    let dir = temp();
    let root = dir.path();
    fs::write(root.join("case.c"), "#include <old.h>\n").unwrap();
    build(root);
    fs::write(root.join("case.c"), "#include <new.h>\n").unwrap();
    let mut session = Session::start(root, "-l");
    session.expect(b">> ");
    fs::remove_file(root.join("case.c")).unwrap();
    session.send(b"8^new.h$\n");
    session.expect(b"cscope: 1 lines\ncase.c <global> 1 #include <new.h>\n>> ");
    fs::write(root.join("case.c"), "#include \"added.h\"\n").unwrap();
    fs::write(root.join("added.h"), "#include <nested.h>\n").unwrap();
    session.send(b"8new\n");
    session.expect(b"Unable to search database\n>> ");
    build(root);
    fs::remove_file(root.join("case.c")).unwrap();
    fs::remove_file(root.join("added.h")).unwrap();
    session.send(b"8.\n");
    session.expect(b"cscope: 2 lines\ncase.c <global> 1 #include \"added.h\"\nadded.h <global> 1 #include <nested.h>\n>> ");
    session.finish();
    session.expect_unconfirmed_refresh();
}

#[test]
fn repeated_process_frames_both_fields_and_recovers_from_request_errors() {
    let dir = temp();
    fs::write(dir.path().join("case.c"), "int Alpha;\nint Beta;\n").unwrap();
    build(dir.path());
    let mut session = Session::start(dir.path(), "-dl");
    session.expect(b">> ");
    session.send(b"1Alpha\n");
    session.expect(b"cscope: 1 lines\ncase.c Alpha 1 int Alpha;\n>> ");
    session.send(b"0Alpha|Beta\r\n");
    session
        .expect(b"cscope: 2 lines\ncase.c <global> 1 int Alpha;\ncase.c <global> 2 int Beta;\n>> ");
    for request in [
        b"1Missing\n".as_slice(),
        b"1[\n",
        b"1(Alpha|a b)\n",
        b"1\xff\n",
        b"1A\0lpha\n",
    ] {
        session.send(request);
        session.expect(b"Unable to search database\n>> ");
    }
    // In the line protocol field 5 is a text search, as in cscope.
    session.send(b"5Alpha\n");
    session.expect(b"cscope: 1 lines\ncase.c <unknown> 1 int Alpha;\n>> ");
    // `R` rebuilds the index, as in cscope, answered with an empty line.
    session.send(b"R\n");
    session.expect(b"\n>> ");
    session.send(b"c\n1ALPHA\n");
    session.expect(b">> cscope: 1 lines\ncase.c ALPHA 1 int Alpha;\n>> ");
    session.send(b"q\n");
    session.finish();
}

#[test]
fn persistent_startup_refreshes_once_and_next_query_sees_external_publication() {
    let dir = temp();
    fs::write(dir.path().join("case.c"), "int Alpha;\n").unwrap();
    build(dir.path());
    fs::write(dir.path().join("case.c"), "int Bravo;\n").unwrap();
    let mut session = Session::start(dir.path(), "-l");
    session.expect(b">> ");
    session.send(b"1Bravo\n");
    session.expect(b"cscope: 1 lines\ncase.c Bravo 1 int Bravo;\n>> ");
    fs::remove_file(dir.path().join("case.c")).unwrap();
    session.send(b"0Bravo\n");
    session.expect(b"cscope: 1 lines\ncase.c <global> 1 int Bravo;\n>> ");
    fs::write(dir.path().join("case.c"), "int Delta;\n").unwrap();
    build(dir.path());
    session.send(b"1Delta\n");
    session.expect(b"cscope: 1 lines\ncase.c Delta 1 int Delta;\n>> ");
    session.finish();
    fs::remove_file(dir.path().join("case.c")).unwrap();
    let mut stored = Session::start(dir.path(), "-dl");
    stored.expect(b">> ");
    stored.send(b"1Delta");
    stored.input.take();
    stored.expect(b"cscope: 1 lines\ncase.c Delta 1 int Delta;\n>> ");
    stored.finish();
}

#[test]
fn oversized_input_is_drained_and_stored_session_adopts_updates() {
    let dir = temp();
    fs::write(dir.path().join("case.c"), "int Alpha;\n").unwrap();
    build(dir.path());
    let mut session = Session::start(dir.path(), "-dl");
    session.expect(b">> ");
    for ending in [b"\n".as_slice(), b"\r\n"] {
        let mut request = b"1".to_vec();
        request.extend(std::iter::repeat_n(b'A', 4096));
        request.extend_from_slice(ending);
        session.send(&request);
        session.expect(b"Unable to search database\n>> ");
    }
    let mut request = b"1".to_vec();
    request.extend(std::iter::repeat_n(b'A', 32_000));
    request.extend_from_slice(b"\n1Alpha\n");
    session.send(&request);
    session
        .expect(b"Unable to search database\n>> cscope: 1 lines\ncase.c Alpha 1 int Alpha;\n>> ");
    fs::write(dir.path().join("case.c"), "int Bravo;\n").unwrap();
    session.send(b"1Alpha\n");
    session.expect(b"cscope: 1 lines\ncase.c Alpha 1 int Alpha;\n>> ");
    build(dir.path());
    session.send(b"1Bravo\n");
    session.expect(b"cscope: 1 lines\ncase.c Bravo 1 int Bravo;\n>> ");
    session.send(b"\x04\n");
    session.finish();
}

#[test]
fn failed_startup_and_conflicting_modes_never_emit_readiness() {
    let dir = temp();
    fs::write(dir.path().join("case.c"), "int Alpha;\n").unwrap();
    build(dir.path());
    fs::remove_file(dir.path().join("case.c")).unwrap();
    // A startup update that finds no source, conflicting modes, and no index. (A repeated
    // `-d` is accepted, as cscope accepts it.)
    for args in [
        vec!["-l", "-f", "index"],
        vec!["-dlL", "-f", "index", "-1", "Alpha"],
        vec!["-dl", "-f", "missing"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_ripscope"))
            .current_dir(dir.path())
            .args(args)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert!(!output.stderr.is_empty());
    }
    let mut stored = Session::start(dir.path(), "-dl");
    stored.expect(b">> ");
    stored.send(b"1Alpha\n");
    stored.expect(b"cscope: 1 lines\ncase.c Alpha 1 int Alpha;\n>> ");
    stored.finish();
}

#[test]
fn stalled_consumer_keeps_owned_old_response_without_blocking_publication() {
    for field in ["0", "2", "3", "9"] {
        let dir = temp();
        let source: String = (0..9000)
            .map(|n| {
                if field == "9" {
                    format!("int symbol{n} = 1;\n")
                } else if field == "0" {
                    format!("int symbol{n};\n")
                } else {
                    format!("int f{n}(void) {{ symbol{n}(); }}\n")
                }
            })
            .collect();
        fs::write(dir.path().join("case.c"), source).unwrap();
        build(dir.path());
        let pattern = if field == "2" { "f.*" } else { "symbol.*" };
        let expected = Command::new(env!("CARGO_BIN_EXE_ripscope"))
            .current_dir(dir.path())
            .args(["-dL", "-f", "index", &format!("-{field}"), pattern])
            .output()
            .unwrap();
        assert!(expected.status.success());
        assert!(expected.stdout.len() > 256 * 1024);
        let mut session = Session::start(dir.path(), "-dl");
        session.expect(b">> ");
        session.send(format!("{field}{pattern}\n").as_bytes());
        session.expect(b"cscope: 9000 lines\n");
        // The bounded reader can drain only 1025 bytes while publication runs.
        fs::write(dir.path().join("case.c"), "int Replacement;\n").unwrap();
        build(dir.path());
        session.expect(&expected.stdout);
        session.expect(b">> ");
        session.send(b"1Replacement\n");
        session.expect(b"cscope: 1 lines\ncase.c Replacement 1 int Replacement;\n>> ");
        session.finish();
    }
}

#[test]
fn a_text_search_skips_a_file_it_cannot_read_and_vim_is_told() {
    for (field, vim) in [(4, false), (4, true), (6, false), (6, true)] {
        let dir = temp();
        let root = dir.path();
        fs::write(root.join("case.c"), "#include \"late.h\"\nint Alpha;\n").unwrap();
        fs::write(root.join("late.h"), "int AlphaLate;\n").unwrap();
        build(root);
        let args = if vim {
            vec!["-dl", "-f", "index", "--vim-errors"]
        } else {
            vec!["-dl", "-f", "index"]
        };
        let mut session = Session::start_with(root, &args);
        session.expect(b">> ");
        fs::remove_file(root.join("late.h")).unwrap();
        session.send(format!("{field}Alpha\n").as_bytes());
        // The file that cannot be read is skipped, with a warning on standard error; the
        // rest answers, and Vim is told after the rows.
        session.expect(b"cscope: 1 lines\ncase.c <unknown> 2 int Alpha;\n");
        if vim {
            session.expect(
                b"ripscope: this answer may be incomplete; a one-shot query (-L) shows why\n\
                  Press the RETURN key to continue:",
            );
        } else {
            session.expect(b">> ");
        }
        // A revision published while the answer waits is read by the next query.
        fs::write(root.join("late.h"), "int Repaired;\n").unwrap();
        fs::write(
            root.join("case.c"),
            "#include \"late.h\"\n#include \"new.h\"\nint Alpha;\n",
        )
        .unwrap();
        fs::write(root.join("new.h"), "int RepairedNew;\n").unwrap();
        build(root);
        if vim {
            session.send(b"\n1Alpha\n");
            session.expect(b">> ");
        } else {
            session.send(b"1Alpha\n");
        }
        session.expect(b"cscope: 1 lines\ncase.c Alpha 3 int Alpha;\n>> ");
        session.send(format!("{field}Repaired\n").as_bytes());
        session.expect(b"cscope: 2 lines\nlate.h <unknown> 1 int Repaired;\nnew.h <unknown> 1 int RepairedNew;\n>> ");
        // No analysis is performed on the live invalid syntax in stored mode.
        fs::write(root.join("case.c"), b"raw NEEDLE\r\nneedle").unwrap();
        session.send(format!("c\n{field}Needle\r\n").as_bytes());
        session.expect(
            b">> cscope: 2 lines\ncase.c <unknown> 1 raw NEEDLE\r\ncase.c <unknown> 2 needle\n>> ",
        );
        session.send(b"q\n");
        session.finish();
    }
}

#[test]
fn regex_text_session_handles_boundaries_and_recovers_from_invalid_and_nul_queries() {
    let dir = temp();
    let root = dir.path();
    fs::write(root.join("case.c"), "int Alpha;\n").unwrap();
    build(root);
    let mut session = Session::start(root, "-dl");
    session.expect(b">> ");
    fs::write(root.join("case.c"), b"\xff Alpha\r\n\nAlpha").unwrap();
    for quantifier in ["?", "*"] {
        session.send(format!("6a{}\n1Alpha\n", quantifier.repeat(4095)).as_bytes());
        session.expect(
            b"Unable to search database\n>> cscope: 1 lines\ncase.c Alpha 1 int Alpha;\n>> ",
        );
    }
    session.send(b"6Alpha$\n6Alpha\r$\r\n6^$\n");
    session.expect(b"Unable to search database\n>> cscope: 1 lines\ncase.c <unknown> 1 \xff Alpha\r\n>> cscope: 1 lines\ncase.c <unknown> 2 \n>> ");
    session.send(b"6[\n6\n6Missing\n6Alpha\n");
    session.expect(b"Unable to search database\n>> Unable to search database\n>> Unable to search database\n>> cscope: 2 lines\ncase.c <unknown> 1 \xff Alpha\r\ncase.c <unknown> 3 Alpha\n>> ");
    // As cscope, a line holding a NUL byte is found; ripscope shows the byte as `\x00`.
    fs::write(root.join("case.c"), b"Alpha\n\0\n").unwrap();
    session.send(b"6.\n1Alpha\n");
    session.expect(
        b"cscope: 2 lines\ncase.c <unknown> 1 Alpha\ncase.c <unknown> 2 \\x00\n\
          >> cscope: 1 lines\ncase.c Alpha 1 int Alpha;\n>> ",
    );
    fs::write(root.join("case.c"), b"ALPHA\n").unwrap();
    session.send(b"c\n6^Alpha$\nq\n");
    session.expect(b">> cscope: 1 lines\ncase.c <unknown> 1 ALPHA\n>> ");
    session.finish();
}

/// The body's call has the last of its alternative heads as its caller (ADR 0010, "Ripscope's own query semantics").
#[test]
fn written_selection_survives_changed_source_startup_and_persistent_rebuild() {
    for mode in ["-l", "-dl"] {
        let dir = temp();
        let root = dir.path();
        let source =
            "#if FLAG\nvoid left(void) {\n#else\nvoid right(void) {\n#endif\ntarget();\n}\n";
        fs::write(root.join("case.c"), source).unwrap();
        let built = Command::new(env!("CARGO_BIN_EXE_ripscope"))
            .current_dir(root)
            .args(["-bk", "-f", "index", "case.c"])
            .output()
            .unwrap();
        assert!(
            built.status.success(),
            "{}",
            String::from_utf8_lossy(&built.stderr)
        );
        fs::write(root.join("case.c"), source.replace("target()", "startup()")).unwrap();
        let mut session = Session::start(root, mode);
        session.expect(b">> ");
        if mode == "-l" {
            session.send(b"3startup\n");
            session.expect(b"cscope: 1 lines\ncase.c right 6 startup();\n>> ");
        } else {
            session.send(b"3target\n");
            session.expect(b"cscope: 1 lines\ncase.c right 6 target();\n>> ");
        }
        fs::write(root.join("case.c"), source.replace("target()", "rebuilt()")).unwrap();
        session.send(b"r\n");
        session.expect(b"\n>> ");
        session.send(b"3rebuilt\n");
        session.expect(b"cscope: 1 lines\ncase.c right 6 rebuilt();\n>> ");
        session.finish();
        fs::remove_file(root.join("case.c")).unwrap();
        let stored = Command::new(env!("CARGO_BIN_EXE_ripscope"))
            .current_dir(root)
            .args(["-dL", "-f", "index", "-3", "rebuilt"])
            .output()
            .unwrap();
        assert!(stored.status.success());
        assert_eq!(stored.stdout, b"case.c right 6 rebuilt();\n");
    }
}

#[test]
fn approved_contexts_preserve_query_dependent_skips_after_source_removal() {
    check_approved_queries(
        "context-approved.json",
        include_str!("fixtures/context-approved.json"),
    );
}

#[test]
fn approved_definitions_and_macro_records_survive_source_removal() {
    check_approved_queries(
        "definition-macro-approved.json",
        include_str!("fixtures/definition-macro-approved.json"),
    );
}

#[test]
fn chained_linkage_annotations_keep_stored_declaration_targets() {
    let source = "extern \"C\" {\n\
        DEPRECATED(3.12) API_DATA(int) DebugFlag;\n\
        DEPRECATED(3.12) API_DATA(const char *) FileEncoding;\n}\n";
    let dir = temp();
    let root = dir.path();
    fs::write(root.join("case.c"), source).unwrap();
    build(root);
    fs::remove_file(root.join("case.c")).unwrap();
    for caseless in [false, true] {
        let mut args = vec!["-dl", "-f", "index"];
        if caseless {
            args.push("-C");
        }
        let mut line = Session::start_with(root, &args);
        line.expect(b">> ");
        args.push("--json");
        let mut json = Session::start_with(root, &args);
        assert_eq!(json.json_line()["type"], "ready");
        for (name, number, text) in [
            ("DebugFlag", 2, "DEPRECATED(3.12) API_DATA(int ) DebugFlag;"),
            (
                "FileEncoding",
                3,
                "DEPRECATED(3.12) API_DATA(const char *) FileEncoding;",
            ),
        ] {
            let pattern = if caseless {
                name.to_ascii_uppercase()
            } else {
                name.to_string()
            };
            let mut args = vec!["-dL", "-f", "index", "-1", &pattern];
            if caseless {
                args.push("-C");
            }
            let one = Command::new(env!("CARGO_BIN_EXE_ripscope"))
                .current_dir(root)
                .args(args)
                .output()
                .unwrap();
            assert!(one.status.success());
            assert!(one.stderr.is_empty());
            let row = format!("case.c {pattern} {number} {text}\n");
            assert_eq!(one.stdout, row.as_bytes());
            let request = format!("1{pattern}\n");
            line.send(request.as_bytes());
            line.expect(format!("cscope: 1 lines\n{row}>> ").as_bytes());
            json.send(request.as_bytes());
            let result = json.json_line();
            assert_eq!(result["type"], "result");
            assert_eq!(result["count"], 1);
            assert_eq!(
                json.json_line(),
                serde_json::json!(["case.c", pattern, number, text])
            );
            assert_eq!(json.json_line()["type"], "ready");
        }
        line.finish();
        json.finish();
    }
}

#[test]
fn supported_declarators_query_after_source_removal() {
    for (name, approved, reference) in [
        (
            "written-prefix-invocations-approved.json",
            include_str!("fixtures/written-prefix-invocations-approved.json"),
            include_str!("fixtures/written-prefix-invocations-cscope.json"),
        ),
        (
            "written-calling-conventions-approved.json",
            include_str!("fixtures/written-calling-conventions-approved.json"),
            include_str!("fixtures/written-calling-conventions-cscope.json"),
        ),
        (
            "written-parenthesized-names-approved.json",
            include_str!("fixtures/written-parenthesized-names-approved.json"),
            include_str!("fixtures/written-parenthesized-names-cscope.json"),
        ),
        (
            "written-tagged-function-pointer-typedefs-approved.json",
            include_str!("fixtures/written-tagged-function-pointer-typedefs-approved.json"),
            include_str!("fixtures/written-tagged-function-pointer-typedefs-cscope.json"),
        ),
        (
            "written-declaration-annotations-approved.json",
            include_str!("fixtures/written-declaration-annotations-approved.json"),
            include_str!("fixtures/written-declaration-annotations-cscope.json"),
        ),
        (
            "written-trailing-member-invocations-approved.json",
            include_str!("fixtures/written-trailing-member-invocations-approved.json"),
            include_str!("fixtures/written-trailing-member-invocations-cscope.json"),
        ),
        (
            "written-bare-word-arms-approved.json",
            include_str!("fixtures/written-bare-word-arms-approved.json"),
            include_str!("fixtures/written-bare-word-arms-cscope.json"),
        ),
    ] {
        let approved: serde_json::Value = serde_json::from_str(approved).unwrap();
        let reference: serde_json::Value = serde_json::from_str(reference).unwrap();
        let mut cases = serde_json::Map::new();
        for (case, fields) in approved["cases"].as_object().unwrap() {
            let observed = reference["cases"][case]
                .as_object()
                .unwrap()
                .values()
                .next()
                .unwrap();
            let source = observed["files"]["case.c"].as_str().unwrap();
            let mut queries = Vec::new();
            for caseless in [false, true] {
                for (field, patterns) in fields.as_object().unwrap() {
                    for (pattern, expected) in patterns.as_object().unwrap() {
                        queries.push(serde_json::json!({
                            "caseless": caseless, "field": field, "pattern": pattern,
                            "approved": expected,
                        }));
                    }
                }
            }
            cases.insert(
                case.clone(),
                serde_json::json!({"source": source, "queries": queries}),
            );
        }
        check_approved_queries(name, &serde_json::json!({"cases": cases}).to_string());
    }
}

#[test]
fn malformed_macro_quotes_keep_following_stored_definitions() {
    let source = "#define BAD don't use this\nint AfterBad;\n\
        #define OPEN \"unfinished\nint AfterOpen;\n\
        #define CLOSED 'x'\nint AfterClosed;\n\
        struct S {\n#define SCOPE_BAD don't\nint member;\n};\n\
        int caller(void) {\n#define LOCAL \"unfinished\ntarget();\n}\nint AfterCaller;\n\
        #if ENABLE\n#define CONDITIONAL don't\n#endif\nint AfterConditional;\n";
    check_stored_rows(
        source,
        &[
            (1, "AfterBad", 2, "AfterBad", "int AfterBad;"),
            (1, "AfterOpen", 4, "AfterOpen", "int AfterOpen;"),
            (1, "AfterClosed", 6, "AfterClosed", "int AfterClosed;"),
            (1, "member", 9, "member", "int member;"),
            (3, "target", 13, "caller", "target();"),
            (1, "AfterCaller", 15, "AfterCaller", "int AfterCaller;"),
            (
                1,
                "AfterConditional",
                19,
                "AfterConditional",
                "int AfterConditional;",
            ),
        ],
        &[],
    );
}

#[test]
fn commented_attributes_keep_stored_members_and_expression_calls() {
    let source = "int expression(void) __attribute__ /* gap */ ((annotate(helper()))) { target(); return 0; }\n\
        struct __attribute__ /* gap */ ((aligned(8))) item { int member; };\n\
        __declspec /* gap */ (align(8)) int global;\nint after;\n\
        #define EXPR __attribute__ /* gap */ ((annotate(macro_helper())))\n";
    check_stored_rows(
        source,
        &[
            (
                1,
                "member",
                2,
                "member",
                "struct __attribute__ ((aligned(8))) item { int member; };",
            ),
            (
                1,
                "global",
                3,
                "global",
                "__declspec (align(8)) int global;",
            ),
            (1, "after", 4, "after", "int after;"),
            (
                3,
                "helper",
                1,
                "expression",
                "int expression(void ) __attribute__ ((annotate(helper()))) { target(); return 0; }",
            ),
            (
                3,
                "macro_helper",
                5,
                "EXPR",
                "#define EXPR __attribute__ ((annotate(macro_helper())))",
            ),
        ],
        &[],
    );
}

#[test]
fn escaped_backslash_pairs_keep_macro_continuations() {
    for count in [2, 4] {
        let source = format!(
            "#define TEXT \"slash {}\nncontinued\"\nint after;\n",
            "\\".repeat(count)
        );
        check_stored_rows(
            &source,
            &[(1, "after", 3, "after", "int after;")],
            &["ncontinued"],
        );
        let closed = format!("#define TEXT \"{}\"\nint after;\n", "\\".repeat(count));
        check_stored_rows(&closed, &[(1, "after", 2, "after", "int after;")], &[]);
    }
}

#[test]
fn approved_typedef_decorations_survive_source_removal() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/decorated-typedef.json")).unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        // The pointer-after-star extension was kept separate from this policy.
        if case["name"] == "pointer" {
            continue;
        }
        let mut rows = Vec::new();
        for query in case["queries"].as_array().unwrap() {
            let pattern = query["pattern"].as_str().unwrap();
            if pattern == "API" {
                continue;
            }
            let stdout = query["stdout"].as_str().unwrap();
            if stdout.is_empty() {
                continue;
            }
            let parts: Vec<_> = stdout.trim_end().splitn(4, ' ').collect();
            rows.push((1, pattern, parts[2].parse().unwrap(), pattern, parts[3]));
        }
        let missing = if case["name"] == "macro-defined" {
            rows.push((1, "API", 1, "API", "#define API"));
            &[][..]
        } else {
            &["API"][..]
        };
        check_stored_rows(case["source"].as_str().unwrap(), &rows, missing);
    }
    for (source, missing) in [
        ("typedef unsigned API x;\n", "API"),
        ("typedef T A B x;\n", "A|B"),
    ] {
        check_stored_rows(source, &[(1, "x", 1, "x", source.trim_end())], &[missing]);
    }
}

#[test]
fn typedef_decorations_pin_the_inherited_suffix_ambiguity() {
    // With ATTR defined as empty, the real alias is actual. The approved
    // written-prefix preference chooses ATTR instead.
    check_stored_rows(
        "typedef int actual ATTR;\nactual instance;\n",
        &[(1, "ATTR", 1, "ATTR", "typedef int actual ATTR;")],
        &["actual"],
    );
    // An invocation prefix was outside that preference and keeps both names.
    check_stored_rows(
        "typedef RET_TYPE(int) actual ATTR;\nactual instance;\n",
        &[
            (
                1,
                "actual",
                1,
                "actual",
                "typedef RET_TYPE(int ) actual ATTR;",
            ),
            (1, "ATTR", 1, "ATTR", "typedef RET_TYPE(int ) actual ATTR;"),
        ],
        &[],
    );
}

// Check exact stored rows through each transport after the source is gone. The
// definition field displays the query pattern as context, including under -C.
fn check_stored_rows(
    source: &str,
    queries: &[(u8, &str, u32, &str, &str)],
    missing_definitions: &[&str],
) {
    for ending in ["\n", "\r\n"] {
        let dir = temp();
        let root = dir.path();
        fs::write(root.join("case.c"), source.replace('\n', ending)).unwrap();
        build(root);
        fs::remove_file(root.join("case.c")).unwrap();
        for caseless in [false, true] {
            let mut args = vec!["-dl", "-f", "index"];
            if caseless {
                args.push("-C");
            }
            let mut line = Session::start_with(root, &args);
            line.expect(b">> ");
            args.push("--json");
            let mut json = Session::start_with(root, &args);
            assert_eq!(json.json_line()["type"], "ready");
            let queries = queries
                .iter()
                .map(|&(field, name, number, context, text)| {
                    (field, name, Some((number, context, text)))
                })
                .chain(missing_definitions.iter().map(|&name| (1, name, None)));
            for (field, name, expected) in queries {
                let pattern = if caseless {
                    name.to_ascii_uppercase()
                } else {
                    name.to_string()
                };
                let expected = expected.map(|(number, context, text)| {
                    let context = if field == 1 { &pattern } else { context };
                    (
                        format!("case.c {context} {number} {text}\n"),
                        serde_json::json!(["case.c", context, number, text]),
                    )
                });
                let row = expected.as_ref().map_or("", |(row, _)| row);
                let flag = format!("-{field}");
                let mut args = vec!["-dL", "-f", "index", &flag, &pattern];
                if caseless {
                    args.push("-C");
                }
                let one = Command::new(env!("CARGO_BIN_EXE_ripscope"))
                    .current_dir(root)
                    .args(args)
                    .output()
                    .unwrap();
                assert!(one.status.success());
                assert!(one.stderr.is_empty());
                assert_eq!(one.stdout, row.as_bytes());
                let request = format!("{field}{pattern}\n");
                line.send(request.as_bytes());
                if expected.is_some() {
                    line.expect(format!("cscope: 1 lines\n{row}>> ").as_bytes());
                } else {
                    line.expect(b"Unable to search database\n>> ");
                }
                json.send(request.as_bytes());
                let result = json.json_line();
                assert_eq!(result["type"], "result");
                assert_eq!(result["count"], usize::from(expected.is_some()));
                if let Some((_, row)) = expected {
                    assert_eq!(json.json_line(), row);
                }
                assert_eq!(json.json_line()["type"], "ready");
            }
            line.finish();
            json.finish();
        }
    }
}

#[test]
fn recovered_heads_and_pointer_aliases_survive_source_removal() {
    let source = "DECLARE(X, T)\nDECLARE(Y, U)\n\
        int real(T *x, STACK_OF(Y) *p) {\nuse_it(x);\n}\n\
        static Object *(*get)(Other *o);\n\
        typedef void (APIENTRYP callback)(Number x);\n";
    let mut queries = Vec::new();
    for caseless in [false, true] {
        for (field, pattern, expected) in [
            (
                "1",
                "real",
                "case.c real 3 int real(T *x, STACK_OF(Y) *p) {\n",
            ),
            ("2", "real", "case.c use_it 4 use_it(x);\n"),
            ("3", "use_it", "case.c real 4 use_it(x);\n"),
            (
                "1",
                "get",
                "case.c get 6 static Object *(*get)(Other *o);\n",
            ),
            (
                "1",
                "callback",
                "case.c callback 7 typedef void (APIENTRYP callback)(Number x);\n",
            ),
            ("1", "Object|APIENTRYP|Number|x|o", ""),
        ] {
            queries.push(serde_json::json!({"caseless":caseless,"field":field,
                "pattern":pattern,"approved":expected}));
        }
    }
    check_approved_queries(
        "recovered-heads-and-aliases",
        &serde_json::json!({
            "cases":{"stored":{"source":source,"queries":queries}}
        })
        .to_string(),
    );
}

fn check_approved_queries(fixture_name: &str, data: &str) {
    let fixture: serde_json::Value = serde_json::from_str(data).unwrap();
    for (name, case) in fixture["cases"].as_object().unwrap() {
        let dir = temp();
        let root = dir.path();
        fs::write(root.join("case.c"), case["source"].as_str().unwrap()).unwrap();
        let built = Command::new(env!("CARGO_BIN_EXE_ripscope"))
            .current_dir(root)
            .args(["-bk", "-f", "index", "case.c"])
            .output()
            .unwrap();
        assert!(built.status.success(), "{name}: {:?}", built.stderr);
        assert!(built.stdout.is_empty() && built.stderr.is_empty());
        fs::remove_file(root.join("case.c")).unwrap();
        for caseless in [false, true] {
            let mut args = vec!["-dl", "-f", "index"];
            if caseless {
                args.push("-C");
            }
            let mut session = Session::start_with(root, &args);
            session.expect(b">> ");
            for query in case["queries"].as_array().unwrap() {
                if query["caseless"].as_bool().unwrap() != caseless {
                    continue;
                }
                let field = query["field"].as_str().unwrap();
                let pattern = query["pattern"].as_str().unwrap();
                let approved = query["approved"].as_str().unwrap();
                let key = format!(
                    "{name} {}{field}{pattern}",
                    if caseless { "-C " } else { "" }
                );
                let expected = &intended::expected(fixture_name, &key, approved);
                let flag = format!("-{field}");
                let mut args = vec!["-dL", "-f", "index", &flag, pattern];
                if caseless {
                    args.push("-C");
                }
                let result = Command::new(env!("CARGO_BIN_EXE_ripscope"))
                    .current_dir(root)
                    .args(args)
                    .output()
                    .unwrap();
                assert!(result.status.success(), "{name}: {:?}", result.stderr);
                assert!(result.stderr.is_empty());
                intended::check(
                    fixture_name,
                    &key,
                    approved,
                    &String::from_utf8_lossy(&result.stdout),
                );
                session.send(format!("{field}{pattern}\n").as_bytes());
                if expected.is_empty() {
                    session.expect(b"Unable to search database\n>> ");
                } else {
                    let frame =
                        format!("cscope: {} lines\n{expected}>> ", expected.lines().count());
                    session.expect(frame.as_bytes());
                }
            }
            session.finish();
        }
    }
}

#[test]
fn approved_real_header_rows_survive_source_removal_in_both_transports() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/real-header-approved.json")).unwrap();
    let dir = temp();
    let root = dir.path();
    let files = fixture["files"].as_object().unwrap();
    for (name, source) in files {
        fs::write(root.join(name), source.as_str().unwrap()).unwrap();
    }
    let output = Command::new(env!("CARGO_BIN_EXE_ripscope"))
        .current_dir(root)
        .args(["-bk", "-f", "index", fixture["seed"].as_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.is_empty() && output.stderr.is_empty());
    for name in files.keys() {
        fs::remove_file(root.join(name)).unwrap();
    }
    for caseless in [false, true] {
        let mut args = vec!["-dl", "-f", "index"];
        if caseless {
            args.push("-C");
        }
        let mut session = Session::start_with(root, &args);
        session.expect(b">> ");
        for query in fixture["queries"].as_array().unwrap() {
            if query["caseless"].as_bool().unwrap() != caseless {
                continue;
            }
            let field = query["field"].as_str().unwrap();
            // The approved reading changed four definition readings. Its full symbol
            // capture was made with other query semantics and uncompacted stored text,
            // which the symbol and displayed-text tests cover.
            if field != "1" {
                continue;
            }
            let pattern = query["pattern"].as_str().unwrap();
            let expected = query["approved"].as_str().unwrap();
            assert_ne!(expected, query["reference"].as_str().unwrap());
            let flag = format!("-{field}");
            let mut one_shot = vec!["-dL", "-f", "index", &flag, pattern];
            if caseless {
                one_shot.push("-C");
            }
            let output = Command::new(env!("CARGO_BIN_EXE_ripscope"))
                .current_dir(root)
                .args(one_shot)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(output.stderr.is_empty());
            assert_eq!(
                output.stdout,
                expected.as_bytes(),
                "field {field}, caseless {caseless}"
            );
            session.send(format!("{field}{pattern}\n").as_bytes());
            let framed = format!("cscope: {} lines\n{expected}>> ", expected.lines().count());
            session.expect(framed.as_bytes());
        }
        session.finish();
    }
}

#[test]
fn message_pragma_queries_preserve_stored_records_and_live_text() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/written-message-pragma.json")).unwrap();
    // Each query's `reference` is cscope's answer, which ripscope gives. Its `expected` is
    // a reading that kept literal spacing. Ripscope stores compacted lines and reads
    // live text unchanged.
    for (name, case) in fixture["cases"].as_object().unwrap() {
        let dir = temp();
        let root = dir.path();
        fs::write(root.join("case.c"), case["source"].as_str().unwrap()).unwrap();
        let built = Command::new(env!("CARGO_BIN_EXE_ripscope"))
            .current_dir(root)
            .args(["-bk", "-f", "index", "case.c"])
            .output()
            .unwrap();
        assert!(
            built.status.success(),
            "{name}: {}",
            String::from_utf8_lossy(&built.stderr)
        );
        assert!(built.stdout.is_empty() && built.stderr.is_empty());
        let queries = case["queries"].as_array().unwrap();
        for query in queries
            .iter()
            .filter(|q| matches!(q["field"].as_str(), Some("4" | "6")))
        {
            let flag = format!("-{}", query["field"].as_str().unwrap());
            let output = Command::new(env!("CARGO_BIN_EXE_ripscope"))
                .current_dir(root)
                .args([
                    "-dL",
                    "-f",
                    "index",
                    &flag,
                    query["pattern"].as_str().unwrap(),
                ])
                .output()
                .unwrap();
            assert!(output.status.success());
            assert!(output.stderr.is_empty());
            assert_eq!(
                output.stdout,
                query["reference"].as_str().unwrap().as_bytes(),
                "{name}: {flag}"
            );
        }
        fs::remove_file(root.join("case.c")).unwrap();
        for caseless in [false, true] {
            let mut args = vec!["-dl", "-f", "index"];
            if caseless {
                args.push("-C");
            }
            let mut session = Session::start_with(root, &args);
            session.expect(b">> ");
            for query in queries {
                let field = query["field"].as_str().unwrap();
                let pattern = query["pattern"].as_str().unwrap();
                let flag = format!("-{field}");
                let mut args = vec!["-dL", "-f", "index", &flag, pattern];
                if caseless {
                    args.push("-C");
                }
                let output = Command::new(env!("CARGO_BIN_EXE_ripscope"))
                    .current_dir(root)
                    .args(args)
                    .output()
                    .unwrap();
                if matches!(field, "4" | "6") {
                    // A text search reads the files as they are now: a deleted one is
                    // skipped with a warning (docs/spec.md, "Keep going instead of failing").
                    assert!(output.status.success(), "{name}: live {field}");
                    assert!(output.stdout.is_empty());
                    assert!(
                        String::from_utf8_lossy(&output.stderr)
                            .starts_with("ripscope: cannot read case.c: "),
                        "{name}: live {field}: {output:?}"
                    );
                    continue;
                }
                let key = format!(
                    "{name} {}{field}{pattern}",
                    if caseless { "-C " } else { "" }
                );
                let reference = query["reference"].as_str().unwrap();
                let expected = &intended::expected("written-message-pragma.json", &key, reference);
                assert!(
                    output.status.success(),
                    "{name}: {field}: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
                assert!(output.stderr.is_empty());
                intended::check(
                    "written-message-pragma.json",
                    &key,
                    reference,
                    &String::from_utf8_lossy(&output.stdout),
                );
                session.send(format!("{field}{pattern}\n").as_bytes());
                if expected.is_empty() {
                    session.expect(b"Unable to search database\n>> ");
                } else {
                    session.expect(
                        format!("cscope: {} lines\n{expected}>> ", expected.lines().count())
                            .as_bytes(),
                    );
                }
            }
            session.finish();
        }
    }
}

#[test]
fn assignment_boundary_queries_keep_branch_records_after_source_removal() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/written-assignment-boundary.json")).unwrap();
    check_written_reference_after_source_removal(&fixture);
}

#[test]
fn clipped_literal_assignments_survive_json_cache_refresh_and_source_removal() {
    for newline in ["\n", "\r\n"] {
        let dir = temp();
        let root = dir.path();
        let source = include_str!("fixtures/assignment-window/literal.c").replace('\n', newline);
        fs::write(root.join("case.c"), &source).unwrap();
        build(root);
        let mut session = Session::start_with(root, &["-dl", "--json", "-f", "index"]);
        assert_eq!(session.json_line()["type"], "ready");
        for refreshed in [false, true] {
            if refreshed {
                fs::write(
                    root.join("case.c"),
                    source.replace("counter++;", "counter--;"),
                )
                .unwrap();
                session.send(b"r\n");
                assert_eq!(session.json_line()["type"], "rebuilt");
                assert_eq!(session.json_line()["type"], "ready");
                fs::remove_file(root.join("case.c")).unwrap();
            }
            let mutation = if refreshed {
                "counter--;"
            } else {
                "counter++;"
            };
            let counter = vec![
                serde_json::json!(["case.c", "nearby", 2, "int counter = 0;"]),
                serde_json::json!(["case.c", "nearby", 4, mutation]),
                serde_json::json!(["case.c", "quoted", 8, "int counter = 0;"]),
                serde_json::json!(["case.c", "quoted", 10, mutation]),
                serde_json::json!(["case.c", "clean_neighbor", 15, "int counter = 0;"]),
                serde_json::json!(["case.c", "clean_neighbor", 16, mutation]),
            ];
            let slots = vec![
                serde_json::json!(["case.c", "nearby", 5, "slots[counter] = 1;"]),
                serde_json::json!(["case.c", "quoted", 11, "slots[counter] = 1;"]),
                serde_json::json!(["case.c", "clean_neighbor", 17, "slots[counter] = 1;"]),
            ];
            for (name, expected) in [("counter", counter), ("slots", slots)] {
                session.send(format!("9{name}\n").as_bytes());
                let result = session.json_line();
                assert_eq!(result["type"], "result");
                assert_eq!(result["count"], expected.len());
                for row in &expected {
                    assert_eq!(&session.json_line(), row);
                }
                assert_eq!(session.json_line()["type"], "ready");
                let output = Command::new(env!("CARGO_BIN_EXE_ripscope"))
                    .current_dir(root)
                    .args(["-dL", "--json", "-f", "index", "-9", name])
                    .output()
                    .unwrap();
                assert!(
                    output.status.success() && output.stderr.is_empty(),
                    "{output:?}"
                );
                let rows: Vec<serde_json::Value> = String::from_utf8(output.stdout)
                    .unwrap()
                    .lines()
                    .map(|line| serde_json::from_str(line).unwrap())
                    .collect();
                assert_eq!(rows, expected);
            }
        }
        session.finish();
    }
}

#[test]
fn initializer_fallback_queries_after_source_removal() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/written-initializer-fallback.json")).unwrap();
    check_written_reference_after_source_removal(&fixture);
}

#[test]
fn body_invocation_queries_after_source_removal() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/written-body-invocations.json")).unwrap();
    check_written_reference_after_source_removal(&fixture);
}

#[test]
fn conditional_continuation_queries_after_source_removal() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/written-conditional-continuations.json"
    ))
    .unwrap();
    check_written_reference_after_source_removal(&fixture);
}

#[test]
fn nested_annotation_queries_after_source_removal() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/written-nested-annotations.json")).unwrap();
    check_written_reference_after_source_removal(&fixture);
}

#[test]
fn lexical_body_region_queries_after_source_removal() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/written-body-regions.json")).unwrap();
    check_written_reference_after_source_removal(&fixture);
}

#[test]
fn body_region_lexical_payload_queries_after_source_removal() {
    let raw: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/body-region-lexical-calls.json")).unwrap();
    let mut cases = serde_json::Map::new();
    for (name, fields) in raw.as_object().unwrap() {
        let mut queries = Vec::new();
        for (field, captured) in fields.as_object().unwrap() {
            for query in captured["profiles"][0]["queries"].as_array().unwrap() {
                let mut query = query.clone();
                let pattern = query["args"].as_array().unwrap().last().unwrap().clone();
                query["args"] = serde_json::json!(["-d", "-L", format!("-{field}"), pattern]);
                queries.push(query);
            }
        }
        cases.insert(
            name.clone(),
            serde_json::json!({"source":fields["0"]["files"]["case.c"],"queries":queries}),
        );
    }
    check_written_reference_after_source_removal(&serde_json::json!({"cases":cases}));
}

#[test]
fn optional_if_queries_after_source_removal() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/written-optional-if.json")).unwrap();
    check_written_reference_after_source_removal(&fixture);
}

#[test]
fn optional_or_queries_after_source_removal() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/written-optional-or.json")).unwrap();
    check_written_reference_after_source_removal(&fixture);
}

#[test]
fn adjacent_else_queries_after_source_removal() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/written-adjacent-else.json")).unwrap();
    check_written_reference_after_source_removal(&fixture);
}

#[test]
fn assignment_continuation_queries_after_source_removal() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/written-assignment-continuations.json"
    ))
    .unwrap();
    check_written_reference_after_source_removal(&fixture);
}

fn check_written_reference_after_source_removal(fixture: &serde_json::Value) {
    for (name, case) in fixture["cases"].as_object().unwrap() {
        let dir = temp();
        let root = dir.path();
        fs::write(root.join("case.c"), case["source"].as_str().unwrap()).unwrap();
        let built = Command::new(env!("CARGO_BIN_EXE_ripscope"))
            .current_dir(root)
            .args(["-bk", "-f", "index", "case.c"])
            .output()
            .unwrap();
        assert!(
            built.status.success(),
            "{name}: {}",
            String::from_utf8_lossy(&built.stderr)
        );
        assert!(built.stdout.is_empty() && built.stderr.is_empty());
        fs::remove_file(root.join("case.c")).unwrap();
        for caseless in [false, true] {
            let mut start = vec!["-dl", "-f", "index"];
            if caseless {
                start.push("-C");
            }
            let mut session = Session::start_with(root, &start);
            session.expect(b">> ");
            for query in case["queries"].as_array().unwrap() {
                let args = query["args"].as_array().unwrap();
                let flag = args[2].as_str().unwrap();
                let pattern = args[3].as_str().unwrap();
                let expected = if caseless {
                    query.get("stdout_caseless").unwrap_or(&query["stdout"])
                } else {
                    &query["stdout"]
                }
                .as_str()
                .unwrap();
                let mut args = vec!["-dL", "-f", "index", flag, pattern];
                if caseless {
                    args.push("-C");
                }
                let output = Command::new(env!("CARGO_BIN_EXE_ripscope"))
                    .current_dir(root)
                    .args(args)
                    .output()
                    .unwrap();
                assert!(
                    output.status.success(),
                    "{name}: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
                assert!(output.stderr.is_empty());
                assert_eq!(
                    output.stdout,
                    expected.as_bytes(),
                    "{name}: {flag}, caseless {caseless}"
                );
                session.send(format!("{}{pattern}\n", &flag[1..]).as_bytes());
                if expected.is_empty() {
                    session.expect(b"Unable to search database\n>> ");
                } else {
                    session.expect(
                        format!("cscope: {} lines\n{expected}>> ", expected.lines().count())
                            .as_bytes(),
                    );
                }
            }
            session.finish();
        }
    }
}

#[test]
fn automatic_selection_rechecks_list_presence_within_the_same_session() {
    let temp = temp();
    let root = temp.path();
    fs::write(root.join("a.c"), "int Alpha;\n").unwrap();
    fs::write(root.join("b.c"), "int Beta;\n").unwrap();
    fs::write(root.join("cscope.files"), "a.c\n").unwrap();
    let built = Command::new(env!("CARGO_BIN_EXE_ripscope"))
        .current_dir(root)
        .args(["-bk", "-f", "index"])
        .output()
        .unwrap();
    assert!(
        built.status.success(),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );
    let mut session = Session::start(root, "-l");
    session.expect(b">> ");
    fs::remove_file(root.join("cscope.files")).unwrap();
    session.send(b"r\n1Beta\n");
    session.expect(b"\n>> cscope: 1 lines\nb.c Beta 1 int Beta;\n>> ");
    fs::write(root.join("cscope.files"), "a.c\n").unwrap();
    session.send(b"r\n1Beta\n");
    session.expect(b"\n>> Unable to search database\n>> ");
    session.finish();
}

#[test]
fn native_rebuild_source_failure_keeps_queries_and_retry() {
    for manual in ["-dl", "--no-auto-refresh"] {
        let dir = temp();
        let root = dir.path();
        fs::write(root.join("case.c"), "int Alpha;\n").unwrap();
        build(root);
        let mut session = Session::start_with(root, &["-l", manual, "--json", "-f", "index"]);
        assert_eq!(session.json_line()["type"], "ready");
        fs::remove_file(root.join("case.c")).unwrap();
        session.send(b"r\n1Alpha\n");
        // The refresh fails, as cscope's finds no source, and says which file it missed.
        assert_eq!(
            session.json_line(),
            serde_json::json!({"type": "error", "message": "no source files found"})
        );
        assert_eq!(
            session.json_line(),
            serde_json::json!({"type": "warning", "message": "cannot find file case.c"})
        );
        assert_eq!(session.json_line()["type"], "ready");
        let result = session.json_line();
        assert_eq!(result["type"], "result");
        assert_eq!(result["count"], 1);
        assert_eq!(
            session.json_line(),
            serde_json::json!(["case.c", "Alpha", 1, "int Alpha;"])
        );
        assert_eq!(session.json_line()["type"], "ready");
        fs::write(root.join("case.c"), "int Beta;\n").unwrap();
        session.send(b"r\n1Beta\nq\n");
        assert_eq!(session.json_line()["type"], "rebuilt");
        assert_eq!(session.json_line()["type"], "ready");
        let result = session.json_line();
        assert_eq!(result["count"], 1);
        assert_eq!(
            session.json_line(),
            serde_json::json!(["case.c", "Beta", 1, "int Beta;"])
        );
        assert_eq!(session.json_line()["type"], "ready");
        session.finish();
        let mut diagnostic = String::new();
        session
            .child
            .stderr
            .take()
            .unwrap()
            .read_to_string(&mut diagnostic)
            .unwrap();
        assert!(diagnostic.is_empty());
    }
}

#[test]
fn lexical_declarations_preserve_stored_queries_after_source_removal() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/lexical-declaration-regression.json")).unwrap();
    let mut cases = serde_json::Map::new();
    for (name, case) in fixture["cases"].as_object().unwrap() {
        // The old capture let a macro's line comment swallow the next function
        // head. Macro-line-comments now ends that macro at its newline. Keep the
        // capture, but derive this case from the matching source without the macro.
        let paired = name.strip_suffix("-True").map(|prefix| {
            let paired = &fixture["cases"][format!("{prefix}-False")];
            assert_eq!(
                case["files"]["case.c"].as_str().unwrap(),
                format!(
                    "#define VALUE 1 // comment\n{}",
                    paired["files"]["case.c"].as_str().unwrap()
                )
            );
            paired
        });
        let mut queries = Vec::new();
        for (field, captured) in case["fields"].as_object().unwrap() {
            let answer = |profile: usize| {
                let captured = paired.map_or(captured, |paired| &paired["fields"][field]);
                let text = captured["profiles"][profile]["queries"][0]["stdout"]
                    .as_str()
                    .unwrap();
                if paired.is_none() {
                    return text.to_owned();
                }
                let mut answer = match field.as_str() {
                    "0" => "case.c <global> 1 #define VALUE 1\n".to_owned(),
                    "1" => "case.c .* 1 #define VALUE 1\n".to_owned(),
                    _ => String::new(),
                };
                for row in text.lines() {
                    let parts: Vec<_> = row.splitn(4, ' ').collect();
                    let line = parts[2].parse::<u32>().unwrap() + 1;
                    answer.push_str(&format!("{} {} {line} {}\n", parts[0], parts[1], parts[3]));
                }
                answer
            };
            for query in captured["profiles"][0]["queries"].as_array().unwrap() {
                assert_eq!(query["args"], serde_json::json!([".*"]));
                queries.push(
                    serde_json::json!({"args":["-d","-L",format!("-{field}"),".*"],
                    "exit_code":0,"stderr":"","stdout":answer(0), "stdout_caseless":answer(1)}),
                );
            }
        }
        cases.insert(
            name.clone(),
            serde_json::json!({"source":case["files"]["case.c"],"queries":queries}),
        );
    }
    check_written_reference_after_source_removal(&serde_json::json!({"cases":cases}));
}

#[test]
fn conditional_delimiter_queries_after_source_removal() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/written-conditional-delimiters.json")).unwrap();
    check_written_reference_after_source_removal(&fixture);
}
