//! Sessions and editor invocations: what Vim, Neovim and xcscope
//! send, and the answers they parse. Standard output and standard error share one pipe
//! here, as Vim's connection gives them.

use std::io::{Read, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Output, Stdio};
use std::sync::RwLock;
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

/// Tests write files and start processes in turn. On Unix a child starts with a copy of
/// every file its parent has open and keeps it until it runs its program, so a file
/// another test wrote meanwhile is closed for the last time only then. A session takes
/// that late close for a change made after its Refresh began, and refreshes again.
static STARTING: RwLock<()> = RwLock::new(());

/// [`std::fs::write`], while no child process is starting.
fn write(path: impl AsRef<Path>, contents: impl AsRef<[u8]>) -> std::io::Result<()> {
    let _writing = STARTING.write().unwrap();
    std::fs::write(path, contents)
}

/// Starts child processes while no test is writing a file.
trait Start {
    /// [`Command::spawn`].
    fn start(&mut self) -> std::io::Result<Child>;

    /// [`Command::output`]: no input, and both streams captured.
    fn captured(&mut self) -> std::io::Result<Output>;
}

impl Start for Command {
    fn start(&mut self) -> std::io::Result<Child> {
        let _starting = STARTING.read().unwrap();
        self.spawn()
    }

    fn captured(&mut self) -> std::io::Result<Output> {
        self.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .start()?
            .wait_with_output()
    }
}

fn command(directory: &Path, args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_ripscope"));
    command
        .current_dir(directory)
        .env_remove("INCLUDEDIRS")
        .env_remove("INCDIR")
        .args(args);
    command
}

fn run(directory: &Path, args: &[&str]) -> Output {
    command(directory, args).captured().expect("ripscope")
}

fn verified_revision(directory: &Path) -> Vec<u8> {
    let verified = run(directory, &["--verify", "-d", "-f", "i.db"]);
    assert!(verified.status.success(), "{verified:?}");
    assert!(verified.stderr.is_empty(), "{verified:?}");
    assert!(
        verified.stdout.starts_with(b"index verified: revision "),
        "{verified:?}"
    );
    verified.stdout
}

fn index_metadata(directory: &Path) -> (i64, Vec<u8>, Vec<u8>) {
    let connection = rusqlite::Connection::open(directory.join("i.db")).unwrap();
    let revision = connection
        .query_row("SELECT value FROM meta WHERE key='revision'", [], |row| {
            row.get(0)
        })
        .unwrap();
    let identity = connection
        .query_row("SELECT value FROM meta WHERE key='identity'", [], |row| {
            row.get(0)
        })
        .unwrap();
    let analyzer = connection
        .query_row("SELECT value FROM meta WHERE key='analyzer'", [], |row| {
            row.get(0)
        })
        .unwrap();
    (revision, identity, analyzer)
}

/// A one-shot run with standard output and standard error in one stream.
fn merged(directory: &Path, args: &[&str]) -> (bool, Vec<u8>) {
    let (mut reader, writer) = std::io::pipe().expect("pipe");
    let mut child = command(directory, args)
        .stdin(Stdio::null())
        .stdout(writer.try_clone().expect("writer"))
        .stderr(writer)
        .start()
        .expect("ripscope");
    let mut output = Vec::new();
    reader.read_to_end(&mut output).expect("output");
    (child.wait().expect("status").success(), output)
}

fn build(directory: &Path, sources: &[&str]) {
    let mut args = vec!["-bk", "-f", "i.db"];
    args.extend(sources);
    let built = run(directory, &args);
    assert!(built.status.success(), "{built:?}");
}

struct Session {
    child: Child,
    input: Option<ChildStdin>,
    output: Receiver<u8>,
}

impl Session {
    fn start(directory: &Path, args: &[&str]) -> Self {
        Self::spawn(command(directory, args))
    }

    fn spawn(mut command: Command) -> Self {
        let (mut reader, writer) = std::io::pipe().expect("pipe");
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(writer.try_clone().expect("writer"))
            .stderr(writer)
            .start()
            .expect("ripscope");
        let input = child.stdin.take();
        let (send, output) = mpsc::sync_channel(1 << 16);
        std::thread::spawn(move || {
            let mut byte = [0];
            while reader.read(&mut byte).unwrap_or(0) != 0 {
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
        let input = self.input.as_mut().expect("input");
        input.write_all(bytes).expect("send");
        input.flush().expect("flush");
    }

    fn expect(&self, expected: &[u8]) {
        let mut actual: Vec<u8> = (0..expected.len())
            .map(|_| {
                self.output
                    .recv_timeout(Duration::from_secs(20))
                    .expect("more output")
            })
            .collect();
        if actual != expected && actual.last() != Some(&b'\n') {
            // Read the rest of an unexpected line too, so a note says why.
            while let Ok(byte) = self.output.recv_timeout(Duration::from_secs(1)) {
                actual.push(byte);
                if byte == b'\n' {
                    break;
                }
            }
        }
        assert_eq!(
            String::from_utf8_lossy(&actual),
            String::from_utf8_lossy(expected)
        );
    }

    fn expect_line(&self, check: impl FnOnce(&str) -> bool) {
        let mut line = Vec::new();
        loop {
            let byte = self
                .output
                .recv_timeout(Duration::from_secs(20))
                .expect("more output");
            line.push(byte);
            if byte == b'\n' {
                break;
            }
        }
        let line = String::from_utf8_lossy(&line).into_owned();
        assert!(check(&line), "unexpected line {line:?}");
    }

    fn query(&mut self, json: bool, request: &str, rows: &[(&str, &str, u32, &str)]) {
        self.send(format!("{request}\n").as_bytes());
        if json {
            self.expect(format!("{{\"type\":\"result\",\"count\":{}}}\n", rows.len()).as_bytes());
            for &(path, context, line, text) in rows {
                self.expect(
                    format!("{}\n", serde_json::json!([path, context, line, text])).as_bytes(),
                );
            }
        } else if rows.is_empty() {
            self.expect(b"Unable to search database\n");
        } else {
            self.expect(format!("cscope: {} lines\n", rows.len()).as_bytes());
            for &(path, context, line, text) in rows {
                self.expect(format!("{path} {context} {line} {text}\n").as_bytes());
            }
        }
        self.expect(if json {
            b"{\"type\":\"ready\"}\n"
        } else {
            b">> "
        });
    }

    fn finish(mut self) {
        self.input.take();
        let status = self.child.wait().expect("status");
        assert!(status.success());
        assert!(self.output.recv_timeout(Duration::from_secs(1)).is_err());
    }
}

#[test]
fn updating_sessions_refresh_saved_definitions_callers_and_assignments_before_stored_queries() {
    for json in [false, true] {
        let work = tempfile::tempdir().unwrap();
        write(work.path().join("a.c"), "int Alpha;\n").unwrap();
        build(work.path(), &["a.c"]);
        let mut args = vec!["-l", "-f", "i.db"];
        if json {
            args.push("--json");
        }
        let mut session = Session::start(work.path(), &args);
        session.expect(if json {
            b"{\"type\":\"ready\"}\n"
        } else {
            b">> "
        });
        session.query(json, "1Alpha", &[("a.c", "Alpha", 1, "int Alpha;")]);
        write(
            work.path().join("a.c"),
            "int Beta;\nvoid changed(void) { Beta = target(); }\n",
        )
        .unwrap();
        session.query(json, "1Beta", &[("a.c", "Beta", 1, "int Beta;")]);
        session.query(json, "1Alpha", &[]);
        session.query(
            json,
            "3target",
            &[(
                "a.c",
                "changed",
                2,
                "void changed(void ) { Beta = target(); }",
            )],
        );
        session.query(
            json,
            "9Beta",
            &[(
                "a.c",
                "changed",
                2,
                "void changed(void ) { Beta = target(); }",
            )],
        );
        session.send(b"q\n");
        session.finish();
    }
}

#[test]
fn explicit_automatic_refresh_controls_override_d_in_either_order() {
    for args in [
        vec!["--auto-refresh", "-dl"],
        vec!["-dl", "--auto-refresh"],
        vec!["--no-auto-refresh", "-l"],
        vec!["-l", "--no-auto-refresh"],
    ] {
        let automatic = args.contains(&"--auto-refresh");
        let work = tempfile::tempdir().unwrap();
        write(work.path().join("a.c"), "int Alpha;\n").unwrap();
        build(work.path(), &["a.c"]);
        write(work.path().join("a.c"), "int Beta;\n").unwrap();
        let mut args = args;
        args.extend(["-f", "i.db"]);
        let mut session = Session::start(work.path(), &args);
        session.expect(b">> ");
        let (name, text) = if automatic {
            ("Beta", "int Beta;")
        } else {
            ("Alpha", "int Alpha;")
        };
        session.query(false, &format!("1{name}"), &[("a.c", name, 1, text)]);
        write(work.path().join("a.c"), "int Gamma;\n").unwrap();
        if automatic {
            session.query(false, "1Gamma", &[("a.c", "Gamma", 1, "int Gamma;")]);
        } else {
            session.query(false, "1Gamma", &[]);
            session.send(b"r\n");
            session.expect(b"\n>> ");
            session.query(false, "1Gamma", &[("a.c", "Gamma", 1, "int Gamma;")]);
        }
        session.send(b"q\n");
        session.finish();
    }
    let work = tempfile::tempdir().unwrap();
    for args in [
        &["--auto-refresh", "--no-auto-refresh", "-l"][..],
        &["--auto-refresh", "-b"],
        &["--no-auto-refresh", "--verify"],
    ] {
        let refused = run(work.path(), args);
        assert!(!refused.status.success(), "{args:?}: {refused:?}");
        assert!(refused.stdout.is_empty(), "{args:?}: {refused:?}");
    }
}

#[test]
fn one_shot_automatic_refresh_override_updates_despite_host_d() {
    for options in [["--auto-refresh", "-dL"], ["-dL", "--auto-refresh"]] {
        let work = tempfile::tempdir().unwrap();
        write(work.path().join("a.c"), "int Alpha;\n").unwrap();
        build(work.path(), &["a.c"]);
        write(work.path().join("a.c"), "int Beta;\n").unwrap();
        let mut args = options.to_vec();
        args.extend(["-f", "i.db", "-1", "Beta"]);
        let updated = run(work.path(), &args);
        assert!(updated.status.success(), "{updated:?}");
        assert_eq!(updated.stdout, b"a.c Beta 1 int Beta;\n");
        assert!(updated.stderr.is_empty(), "{updated:?}");
    }
}

#[test]
fn automatic_refresh_reapplies_discovery_after_add_delete_rename_and_replacement() {
    for json in [false, true] {
        let work = tempfile::tempdir().unwrap();
        write(work.path().join("a.c"), "int Alpha;\n").unwrap();
        build(work.path(), &[]);
        let mut args = vec!["-l", "-f", "i.db"];
        if json {
            args.push("--json");
        }
        let mut session = Session::start(work.path(), &args);
        session.expect(if json {
            b"{\"type\":\"ready\"}\n"
        } else {
            b">> "
        });
        write(work.path().join("b.c"), "int Beta;\n").unwrap();
        session.query(
            json,
            "7.*",
            &[
                ("a.c", "<unknown>", 1, "<unknown>"),
                ("b.c", "<unknown>", 1, "<unknown>"),
            ],
        );
        std::fs::rename(work.path().join("b.c"), work.path().join("renamed.c")).unwrap();
        std::fs::remove_file(work.path().join("a.c")).unwrap();
        session.query(json, "7.*", &[("renamed.c", "<unknown>", 1, "<unknown>")]);
        write(work.path().join("replacement.tmp"), "int Gamma;\n").unwrap();
        std::fs::rename(
            work.path().join("replacement.tmp"),
            work.path().join("renamed.c"),
        )
        .unwrap();
        session.query(json, "1Gamma", &[("renamed.c", "Gamma", 1, "int Gamma;")]);
        session.query(json, "1Beta", &[]);
        let before = verified_revision(work.path());
        session.query(json, "1Gamma", &[("renamed.c", "Gamma", 1, "int Gamma;")]);
        let after = verified_revision(work.path());
        assert_eq!(before, after, "unchanged requests preserve revision");
        session.send(b"r\n");
        session.expect(if json {
            b"{\"type\":\"rebuilt\"}\n{\"type\":\"ready\"}\n"
        } else {
            b"\n>> "
        });
        assert_eq!(
            after,
            verified_revision(work.path()),
            "explicit Refresh of the same source state agrees"
        );
        session.send(b"q\n");
        session.finish();
    }
}

#[test]
fn automatic_refresh_reloads_file_lists_and_external_headers_and_missing_targets() {
    for json in [false, true] {
        let work = tempfile::tempdir().unwrap();
        let project = work.path().join("project");
        let headers = work.path().join("headers");
        std::fs::create_dir(&project).unwrap();
        std::fs::create_dir(&headers).unwrap();
        write(project.join("a.c"), "#include <late.h>\nint Alpha;\n").unwrap();
        write(project.join("b.c"), "int Beta;\n").unwrap();
        write(project.join("files"), "a.c\n").unwrap();
        let built = run(
            &project,
            &["-bk", "-I", "../headers", "-f", "i.db", "-i", "files"],
        );
        assert!(built.status.success(), "{built:?}");
        let mut args = vec!["-l", "-f", "i.db"];
        if json {
            args.push("--json");
        }
        let mut session = Session::start(&project, &args);
        session.expect(if json {
            b"{\"type\":\"ready\"}\n"
        } else {
            b">> "
        });
        session.query(json, "1Header", &[]);
        write(headers.join("late.h"), "int Header;\n").unwrap();
        session.query(
            json,
            "1Header",
            &[("../headers/late.h", "Header", 1, "int Header;")],
        );
        write(headers.join("late.h"), "int Changed;\n").unwrap();
        session.query(
            json,
            "1Changed",
            &[("../headers/late.h", "Changed", 1, "int Changed;")],
        );
        write(project.join("files"), "b.c\n").unwrap();
        session.query(json, "7.*", &[("b.c", "<unknown>", 1, "<unknown>")]);
        session.send(b"q\n");
        session.finish();
    }
    let work = tempfile::tempdir().unwrap();
    write(work.path().join("a.c"), "int Alpha;\n").unwrap();
    build(work.path(), &["a.c"]);
    let mut session = Session::start(work.path(), &["-l", "-f", "i.db"]);
    session.expect(b">> ");
    write(work.path().join("b.c"), "int Beta;\n").unwrap();
    session.query(false, "7.*", &[("a.c", "<unknown>", 1, "<unknown>")]);
    session.send(b"q\n");
    session.finish();
}

#[test]
fn automatic_refresh_failure_and_concurrent_writer_keep_framed_answers_and_retry() {
    for (json, vim) in [(false, false), (false, true), (true, false)] {
        let work = tempfile::tempdir().unwrap();
        write(work.path().join("a.c"), "int Alpha;\n").unwrap();
        write(work.path().join("files"), "a.c\n").unwrap();
        build(work.path(), &["-i", "files"]);
        let mut args = vec!["-l", "-f", "i.db"];
        if json {
            args.push("--json");
        }
        if vim {
            args.push("--vim-errors");
        }
        let mut session = Session::start(work.path(), &args);
        session.expect(if json {
            b"{\"type\":\"ready\"}\n"
        } else {
            b">> "
        });
        std::fs::remove_file(work.path().join("files")).unwrap();
        for failure in [true, false] {
            let writer = rusqlite::Connection::open(work.path().join("i.db")).unwrap();
            if !failure {
                writer.execute_batch("BEGIN IMMEDIATE").unwrap();
            }
            session.send(b"1Alpha\n");
            if !json {
                session.expect_line(|line| {
                    line.contains("automatic Refresh did not complete")
                        && line.contains("last published revision")
                        && line.contains("freshness is unconfirmed")
                });
                session.expect(b"cscope: 1 lines\na.c Alpha 1 int Alpha;\n");
                if vim {
                    session.expect(b"ripscope: this answer may be incomplete; a one-shot query (-L) shows why\nPress the RETURN key to continue:");
                    session.send(b"\n");
                }
                session.expect(b">> ");
            } else {
                session.expect(
                    b"{\"type\":\"result\",\"count\":1}\n[\"a.c\",\"Alpha\",1,\"int Alpha;\"]\n",
                );
                session.expect_line(|line| {
                    let frame: serde_json::Value = serde_json::from_str(line).unwrap();
                    frame["type"] == "warning"
                        && frame["message"].as_str().is_some_and(|message| {
                            message.contains("automatic Refresh did not complete")
                                && message.contains("last published revision")
                                && message.contains("freshness is unconfirmed")
                        })
                });
                session.expect(b"{\"type\":\"ready\"}\n");
            }
            write(work.path().join("a.c"), "int Beta;\n").unwrap();
            write(work.path().join("files"), "a.c\n").unwrap();
            drop(writer);
        }
        session.query(json, "1Beta", &[("a.c", "Beta", 1, "int Beta;")]);
        session.send(b"q\n");
        session.finish();
    }
}

#[test]
fn text_requests_read_saved_bytes_in_the_files_selected_now() {
    let work = tempfile::tempdir().unwrap();
    write(work.path().join("a.c"), "int Alpha;\n").unwrap();
    build(work.path(), &[]);
    let mut session = Session::start(work.path(), &["-l", "-f", "i.db"]);
    session.expect(b">> ");
    write(work.path().join("a.c"), "int Beta;\n").unwrap();
    write(work.path().join("b.c"), "int Beta;\n").unwrap();
    session.query(
        false,
        "4Beta",
        &[
            ("a.c", "<unknown>", 1, "int Beta;"),
            ("b.c", "<unknown>", 1, "int Beta;"),
        ],
    );
    session.query(
        false,
        "1Beta",
        &[
            ("a.c", "Beta", 1, "int Beta;"),
            ("b.c", "Beta", 1, "int Beta;"),
        ],
    );
    session.send(b"q\n");
    session.finish();
}

/// Holds the index's write lock, as another process's update does: a session that tried
/// to refresh meanwhile would say that it could not.
fn hold_write_lock(directory: &Path) -> rusqlite::Connection {
    let writer = rusqlite::Connection::open(directory.join("i.db")).unwrap();
    writer.execute_batch("BEGIN IMMEDIATE").unwrap();
    writer
}

/// Answers `request` while another process holds the write lock, which a Refresh would
/// wait for: the session must not refresh.
fn settled(
    session: &mut Session,
    directory: &Path,
    request: &str,
    rows: &[(&str, &str, u32, &str)],
) {
    let writer = hold_write_lock(directory);
    session.query(false, request, rows);
    drop(writer);
}

/// Includes of eight headers no directory holds: a directory with that many names missing
/// in it is checked by its times, where no watch reaches.
const ABSENT: &str = "#include <absent1.h>\n#include <absent2.h>\n#include <absent3.h>\n\
#include <absent4.h>\n#include <absent5.h>\n#include <absent6.h>\n#include <absent7.h>\n\
#include <absent8.h>\n";

/// [`settled`] for an answer from headers outside the project, which Windows polls: one
/// changed less than two seconds before a check is read once more after it.
fn settled_outside(
    session: &mut Session,
    directory: &Path,
    request: &str,
    rows: &[(&str, &str, u32, &str)],
) {
    if cfg!(windows) {
        std::thread::sleep(Duration::from_millis(2100));
        session.query(false, request, rows);
    }
    settled(session, directory, request, rows);
}

#[test]
fn unchanged_sources_and_editor_files_answer_without_refreshing() {
    for json in [false, true] {
        let work = tempfile::tempdir().unwrap();
        std::fs::create_dir(work.path().join("src")).unwrap();
        write(work.path().join("src/a.c"), "int Alpha;\n").unwrap();
        let built = run(work.path(), &["-bkR", "-f", "i.db"]);
        assert!(built.status.success(), "{built:?}");
        let mut args = vec!["-l", "-f", "i.db"];
        if json {
            args.push("--json");
        }
        let mut session = Session::start(work.path(), &args);
        session.expect(if json {
            b"{\"type\":\"ready\"}\n"
        } else {
            b">> "
        });
        let alpha = [("src/a.c", "Alpha", 1, "int Alpha;")];
        session.query(json, "1Alpha", &alpha);
        let writer = hold_write_lock(work.path());
        session.query(json, "1Alpha", &alpha);
        // What editors, builds and the watch itself leave beside sources.
        for noise in [
            "src/.a.c.swp",
            "src/a.c~",
            "src/4913",
            "src/#a.c#",
            "src/a.o",
            "notes.txt",
        ] {
            write(work.path().join(noise), "int Noise;\n").unwrap();
        }
        std::fs::remove_file(work.path().join("src/4913")).unwrap();
        session.query(json, "1Alpha", &alpha);
        drop(writer);
        write(work.path().join("src/a.c"), "int Beta;\n").unwrap();
        session.query(json, "1Beta", &[("src/a.c", "Beta", 1, "int Beta;")]);
        session.send(b"q\n");
        session.finish();
    }
}

#[test]
fn a_directory_moved_into_a_discovered_tree_is_watched_from_then_on() {
    let work = tempfile::tempdir().unwrap();
    let project = work.path().join("project");
    std::fs::create_dir_all(project.join("src")).unwrap();
    write(project.join("src/a.c"), "int Alpha;\n").unwrap();
    std::fs::create_dir_all(work.path().join("incoming/deeper")).unwrap();
    write(work.path().join("incoming/deeper/new.c"), "int New;\n").unwrap();
    let built = run(&project, &["-bkR", "-f", "i.db"]);
    assert!(built.status.success(), "{built:?}");
    let mut session = Session::start(&project, &["-l", "-f", "i.db"]);
    session.expect(b">> ");
    session.query(false, "1New", &[]);
    std::fs::rename(work.path().join("incoming"), project.join("src/lib")).unwrap();
    session.query(
        false,
        "1New",
        &[("src/lib/deeper/new.c", "New", 1, "int New;")],
    );
    write(project.join("src/lib/deeper/new.c"), "int Newer;\n").unwrap();
    session.query(
        false,
        "1Newer",
        &[("src/lib/deeper/new.c", "Newer", 1, "int Newer;")],
    );
    session.send(b"q\n");
    session.finish();
}

#[test]
fn another_processs_new_selection_rule_is_watched_after_its_revision() {
    let work = tempfile::tempdir().unwrap();
    std::fs::create_dir(work.path().join("sub")).unwrap();
    write(work.path().join("a.c"), "int Alpha;\n").unwrap();
    write(work.path().join("sub/b.c"), "int Beta;\n").unwrap();
    build(work.path(), &[]);
    let mut session = Session::start(work.path(), &["-l", "-f", "i.db"]);
    session.expect(b">> ");
    session.query(false, "1Beta", &[]);
    let rebuilt = run(work.path(), &["-bkR", "-f", "i.db"]);
    assert!(rebuilt.status.success(), "{rebuilt:?}");
    session.query(false, "1Beta", &[("sub/b.c", "Beta", 1, "int Beta;")]);
    write(work.path().join("sub/b.c"), "int Gamma;\n").unwrap();
    session.query(false, "1Gamma", &[("sub/b.c", "Gamma", 1, "int Gamma;")]);
    session.send(b"q\n");
    session.finish();
}

#[test]
fn files_another_process_indexed_are_watched_once_a_refresh_meets_them() {
    let work = tempfile::tempdir().unwrap();
    write(work.path().join("a.c"), "int Alpha;\n").unwrap();
    write(work.path().join("b.c"), "int Beta;\n").unwrap();
    write(work.path().join("files"), "a.c\n").unwrap();
    build(work.path(), &["-i", "files"]);
    let mut session = Session::start(work.path(), &["-l", "-f", "i.db"]);
    session.expect(b">> ");
    session.query(false, "1Beta", &[]);
    // Another process takes in the new list before the session's next query does.
    write(work.path().join("files"), "a.c\nb.c\n").unwrap();
    let other = run(work.path(), &["-L", "-f", "i.db", "-1", "Beta"]);
    assert_eq!(other.stdout, b"b.c Beta 1 int Beta;\n", "{other:?}");
    session.query(false, "1Beta", &[("b.c", "Beta", 1, "int Beta;")]);
    write(work.path().join("b.c"), "int Gamma;\n").unwrap();
    let gamma = [("b.c", "Gamma", 1, "int Gamma;")];
    session.query(false, "1Gamma", &gamma);
    settled(&mut session, work.path(), "1Gamma", &gamma);
    session.send(b"q\n");
    session.finish();
}

#[test]
fn a_listed_file_made_after_the_session_started_is_indexed() {
    let work = tempfile::tempdir().unwrap();
    std::fs::create_dir(work.path().join("src")).unwrap();
    write(work.path().join("src/a.c"), "int Alpha;\n").unwrap();
    write(
        work.path().join("files"),
        "src/a.c\nsrc/late.c\ngen/later.c\n",
    )
    .unwrap();
    let built = run(work.path(), &["-bk", "-f", "i.db", "-i", "files"]);
    assert!(built.status.success(), "{built:?}");
    let mut session = Session::start(work.path(), &["-l", "-f", "i.db"]);
    session.expect(b"ripscope: cannot find file src/late.c\n");
    session.expect(b"ripscope: cannot find file gen/later.c\n>> ");
    session.query(false, "1Alpha", &[("src/a.c", "Alpha", 1, "int Alpha;")]);
    write(work.path().join("src/late.c"), "int Late;\n").unwrap();
    session.send(b"1Late\n");
    session.expect(b"ripscope: cannot find file gen/later.c\n");
    session.expect(b"cscope: 1 lines\nsrc/late.c Late 1 int Late;\n>> ");
    std::fs::create_dir(work.path().join("gen")).unwrap();
    write(work.path().join("gen/later.c"), "int Later;\n").unwrap();
    let later = [("gen/later.c", "Later", 1, "int Later;")];
    session.query(false, "1Later", &later);
    // Its directory was watched only after the Refresh read it, so it is read once more.
    session.query(false, "1Later", &later);
    settled(&mut session, work.path(), "1Later", &later);
    session.send(b"q\n");
    session.finish();
}

#[test]
fn a_header_directory_made_after_the_session_started_is_watched() {
    let work = tempfile::tempdir().unwrap();
    let project = work.path().join("project");
    std::fs::create_dir(&project).unwrap();
    write(project.join("a.c"), "#include <late.h>\nint Alpha;\n").unwrap();
    let built = run(
        &project,
        &["-bk", "-I", "../headers/gen", "-f", "i.db", "a.c"],
    );
    assert!(built.status.success(), "{built:?}");
    let mut session = Session::start(&project, &["-l", "-f", "i.db"]);
    session.expect(b">> ");
    session.query(false, "1Header", &[]);
    std::fs::create_dir_all(work.path().join("headers/gen")).unwrap();
    session.query(false, "1Header", &[]);
    write(work.path().join("headers/gen/late.h"), "int Header;\n").unwrap();
    let header = [("../headers/gen/late.h", "Header", 1, "int Header;")];
    session.query(false, "1Header", &header);
    settled_outside(&mut session, &project, "1Header", &header);
    session.send(b"q\n");
    session.finish();
}

#[test]
fn a_header_directory_removed_and_made_again_is_watched_again() {
    let work = tempfile::tempdir().unwrap();
    let project = work.path().join("project");
    let generated = work.path().join("generated");
    std::fs::create_dir(&project).unwrap();
    std::fs::create_dir(&generated).unwrap();
    write(project.join("a.c"), "#include <gen.h>\nint Alpha;\n").unwrap();
    write(generated.join("gen.h"), "int Old;\n").unwrap();
    let built = run(
        &project,
        &["-bk", "-I", "../generated", "-f", "i.db", "a.c"],
    );
    assert!(built.status.success(), "{built:?}");
    let mut session = Session::start(&project, &["-l", "-f", "i.db"]);
    session.expect(b">> ");
    let old = [("../generated/gen.h", "Old", 1, "int Old;")];
    session.query(false, "1Old", &old);
    // A clean build makes the directory again with the same names in it, between queries.
    std::fs::remove_dir_all(&generated).unwrap();
    std::fs::create_dir(&generated).unwrap();
    write(generated.join("gen.h"), "int Old;\n").unwrap();
    session.query(false, "1Old", &old);
    write(generated.join("gen.h"), "int New;\n").unwrap();
    let new = [("../generated/gen.h", "New", 1, "int New;")];
    session.query(false, "1New", &new);
    settled_outside(&mut session, &project, "1New", &new);
    session.send(b"q\n");
    session.finish();
}

#[test]
fn headers_in_a_directory_swapped_for_another_are_watched_where_they_are_now() {
    let work = tempfile::tempdir().unwrap();
    let project = work.path().join("project");
    std::fs::create_dir(&project).unwrap();
    std::fs::create_dir_all(work.path().join("sdk/include")).unwrap();
    std::fs::create_dir_all(work.path().join("next/include")).unwrap();
    write(project.join("a.c"), "#include <sdk.h>\nint Alpha;\n").unwrap();
    write(work.path().join("sdk/include/sdk.h"), "int One;\n").unwrap();
    write(work.path().join("next/include/sdk.h"), "int Two;\n").unwrap();
    let built = run(
        &project,
        &["-bk", "-I", "../sdk/include", "-f", "i.db", "a.c"],
    );
    assert!(built.status.success(), "{built:?}");
    let mut session = Session::start(&project, &["-l", "-f", "i.db"]);
    session.expect(b">> ");
    session.query(
        false,
        "1One",
        &[("../sdk/include/sdk.h", "One", 1, "int One;")],
    );
    std::fs::rename(work.path().join("sdk"), work.path().join("old")).unwrap();
    std::fs::rename(work.path().join("next"), work.path().join("sdk")).unwrap();
    session.query(
        false,
        "1Two",
        &[("../sdk/include/sdk.h", "Two", 1, "int Two;")],
    );
    write(work.path().join("sdk/include/sdk.h"), "int Three;\n").unwrap();
    let three = [("../sdk/include/sdk.h", "Three", 1, "int Three;")];
    session.query(false, "1Three", &three);
    settled_outside(&mut session, &project, "1Three", &three);
    session.send(b"q\n");
    session.finish();
}

#[test]
fn headers_made_removed_renamed_or_edited_outside_the_project_are_read_by_the_next_query() {
    let work = tempfile::tempdir().unwrap();
    let project = work.path().join("project");
    let headers = work.path().join("headers");
    let elsewhere = work.path().join("elsewhere");
    for directory in [&project, &headers, &elsewhere] {
        std::fs::create_dir(directory).unwrap();
    }
    let source = format!("#include <late.h>\n{ABSENT}int Alpha;\n");
    write(project.join("a.c"), source).unwrap();
    write(headers.join("draft.h"), "int Draft;\n").unwrap();
    write(elsewhere.join("late.h"), "int Moved;\n").unwrap();
    let built = run(&project, &["-bk", "-I", "../headers", "-f", "i.db", "a.c"]);
    assert!(built.status.success(), "{built:?}");
    let mut session = Session::start(&project, &["-l", "-f", "i.db"]);
    session.expect(b">> ");
    // Each change is made once the session has settled, so that it is read because the
    // session noticed it, not because the change before it was recent.
    settled_outside(&mut session, &project, "1Draft", &[]);
    let late = "../headers/late.h";
    std::fs::rename(headers.join("draft.h"), headers.join("late.h")).unwrap();
    let draft = [(late, "Draft", 1, "int Draft;")];
    session.query(false, "1Draft", &draft);
    settled_outside(&mut session, &project, "1Draft", &draft);
    write(headers.join("late.h"), "int Edited;\n").unwrap();
    let edited = [(late, "Edited", 1, "int Edited;")];
    session.query(false, "1Edited", &edited);
    settled_outside(&mut session, &project, "1Edited", &edited);
    std::fs::remove_file(headers.join("late.h")).unwrap();
    session.query(false, "1Edited", &[]);
    settled_outside(&mut session, &project, "1Edited", &[]);
    write(headers.join("late.h"), "int Made;\n").unwrap();
    let made = [(late, "Made", 1, "int Made;")];
    session.query(false, "1Made", &made);
    settled_outside(&mut session, &project, "1Made", &made);
    // An editor's save: the new text goes to another file renamed over the header.
    write(headers.join("late.h.new"), "int Saved;\n").unwrap();
    std::fs::rename(headers.join("late.h.new"), headers.join("late.h")).unwrap();
    let saved = [(late, "Saved", 1, "int Saved;")];
    session.query(false, "1Saved", &saved);
    settled_outside(&mut session, &project, "1Saved", &saved);
    std::fs::rename(headers.join("late.h"), elsewhere.join("saved.h")).unwrap();
    session.query(false, "1Saved", &[]);
    settled_outside(&mut session, &project, "1Saved", &[]);
    std::fs::rename(elsewhere.join("late.h"), headers.join("late.h")).unwrap();
    let moved = [(late, "Moved", 1, "int Moved;")];
    session.query(false, "1Moved", &moved);
    settled_outside(&mut session, &project, "1Moved", &moved);
    session.send(b"q\n");
    session.finish();
}

#[cfg(unix)]
#[test]
fn files_through_a_directory_link_are_watched_where_it_leads_now() {
    let work = tempfile::tempdir().unwrap();
    let project = work.path().join("project");
    std::fs::create_dir_all(project.join("arch/one")).unwrap();
    std::fs::create_dir_all(project.join("arch/two")).unwrap();
    write(project.join("arch/one/arch.h"), "int One;\n").unwrap();
    write(project.join("arch/two/arch.h"), "int Two;\n").unwrap();
    std::os::unix::fs::symlink("arch/one", project.join("asm")).unwrap();
    build(&project, &["asm/arch.h"]);
    let mut session = Session::start(&project, &["-l", "-f", "i.db"]);
    session.expect(b">> ");
    session.query(false, "1One", &[("asm/arch.h", "One", 1, "int One;")]);
    // `ln -sfn` replaces the link through a rename.
    std::os::unix::fs::symlink("arch/two", project.join("asm.new")).unwrap();
    std::fs::rename(project.join("asm.new"), project.join("asm")).unwrap();
    session.query(false, "1Two", &[("asm/arch.h", "Two", 1, "int Two;")]);
    write(project.join("arch/two/arch.h"), "int Three;\n").unwrap();
    let three = [("asm/arch.h", "Three", 1, "int Three;")];
    session.query(false, "1Three", &three);
    settled(&mut session, &project, "1Three", &three);
    session.send(b"q\n");
    session.finish();
}

#[cfg(unix)]
#[test]
fn a_header_reached_through_a_link_is_watched_where_it_lives() {
    let work = tempfile::tempdir().unwrap();
    let project = work.path().join("project");
    let generated = work.path().join("generated");
    std::fs::create_dir(&project).unwrap();
    std::fs::create_dir(&generated).unwrap();
    write(project.join("a.c"), "#include \"gen.h\"\nint Alpha;\n").unwrap();
    write(generated.join("gen.h"), "int Old;\n").unwrap();
    std::os::unix::fs::symlink("../generated/gen.h", project.join("gen.h")).unwrap();
    write(project.join("files"), "a.c\ngen.h\n").unwrap();
    let built = run(&project, &["-bk", "-f", "i.db", "-i", "files"]);
    assert!(built.status.success(), "{built:?}");
    let mut session = Session::start(&project, &["-l", "-f", "i.db"]);
    session.expect(b">> ");
    session.query(false, "1Old", &[("gen.h", "Old", 1, "int Old;")]);
    write(generated.join("gen.h"), "int Generated;\n").unwrap();
    session.query(
        false,
        "1Generated",
        &[("gen.h", "Generated", 1, "int Generated;")],
    );
    session.send(b"q\n");
    session.finish();
}

/// Windows reports a change under a junction by the path the junction leads to, which a
/// listed name does not spell, even where it leads inside the project. The names under
/// one are checked before each query instead.
#[cfg(windows)]
#[test]
fn listed_files_through_a_junction_inside_the_project_are_checked_before_each_query() {
    let work = tempfile::tempdir().unwrap();
    let project = work.path().join("project");
    std::fs::create_dir_all(project.join("arch/one")).unwrap();
    write(project.join("arch/one/arch.h"), "int One;\n").unwrap();
    let linked = Command::new("cmd")
        .args(["/C", "mklink", "/J", "asm", r"arch\one"])
        .current_dir(&project)
        .captured()
        .unwrap();
    assert!(linked.status.success(), "{linked:?}");
    write(project.join("files"), "asm/arch.h\n").unwrap();
    let built = run(&project, &["-bk", "-f", "i.db", "-i", "files"]);
    assert!(built.status.success(), "{built:?}");
    // Seen more than two seconds after it was written, the header is checked by what it
    // was then.
    std::thread::sleep(Duration::from_millis(2100));
    let mut session = Session::start(&project, &["-l", "-f", "i.db"]);
    session.expect(b">> ");
    let one = [("asm/arch.h", "One", 1, "int One;")];
    settled(&mut session, &project, "1One", &one);
    write(project.join("arch/one/arch.h"), "int Two;\n").unwrap();
    session.query(false, "1Two", &[("asm/arch.h", "Two", 1, "int Two;")]);
    session.send(b"q\n");
    session.finish();
}

#[cfg(unix)]
#[test]
fn headers_where_no_watch_reaches_are_checked_before_each_query() {
    use std::os::unix::fs::PermissionsExt;
    let work = tempfile::tempdir().unwrap();
    let project = work.path().join("project");
    let headers = work.path().join("headers");
    std::fs::create_dir(&project).unwrap();
    std::fs::create_dir(&headers).unwrap();
    let source = format!("#include <late.h>\n#include <later.h>\n{ABSENT}int Alpha;\n");
    write(project.join("a.c"), source).unwrap();
    write(headers.join("late.h"), "int Header;\n").unwrap();
    let built = run(&project, &["-bk", "-I", "../headers", "-f", "i.db", "a.c"]);
    assert!(built.status.success(), "{built:?}");
    // A directory that can be searched but not listed cannot be watched.
    std::fs::set_permissions(&headers, std::fs::Permissions::from_mode(0o311)).unwrap();
    if std::fs::read_dir(&headers).is_ok() {
        std::fs::set_permissions(&headers, std::fs::Permissions::from_mode(0o755)).unwrap();
        eprintln!("skipped: permissions do not apply to this user");
        return;
    }
    // Seen more than two seconds after it was written, the header is checked by what it
    // was then.
    std::thread::sleep(Duration::from_millis(2100));
    let mut session = Session::start(&project, &["-l", "-f", "i.db"]);
    session.expect(b">> ");
    let header = [("../headers/late.h", "Header", 1, "int Header;")];
    session.query(false, "1Header", &header);
    write(headers.join("late.h"), "int Changed;\n").unwrap();
    let changed = [("../headers/late.h", "Changed", 1, "int Changed;")];
    session.query(false, "1Changed", &changed);
    // Read within two seconds of its change, the header is read once more after that.
    std::thread::sleep(Duration::from_millis(2100));
    session.query(false, "1Changed", &changed);
    settled(&mut session, &project, "1Changed", &changed);
    // A header made there moves its directory's times.
    write(headers.join("later.h"), "int Later;\n").unwrap();
    let later = [("../headers/later.h", "Later", 1, "int Later;")];
    session.query(false, "1Later", &later);
    session.send(b"q\n");
    session.finish();
    std::fs::set_permissions(&headers, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// A session run where inotify allows only `watches` watches, or no queue at all, in a
/// user namespace of its own; `None` where one cannot be made.
#[cfg(target_os = "linux")]
fn limited(directory: &Path, args: &[&str], watches: Option<u32>) -> Option<Session> {
    let limit = match watches {
        Some(watches) => format!("echo {watches} > /proc/sys/user/max_inotify_watches"),
        None => "echo 0 > /proc/sys/user/max_inotify_instances".into(),
    };
    let probe = Command::new("unshare")
        .args(["-r", "sh", "-c", &limit])
        .captured();
    if !probe.is_ok_and(|probe| probe.status.success()) {
        return None;
    }
    let mut command = Command::new("unshare");
    command
        .current_dir(directory)
        .env_remove("INCLUDEDIRS")
        .env_remove("INCDIR")
        .args(["-r", "sh", "-c", &format!("{limit} && exec \"$0\" \"$@\"")])
        .arg(env!("CARGO_BIN_EXE_ripscope"))
        .args(args);
    Some(Session::spawn(command))
}

#[cfg(target_os = "linux")]
#[test]
fn sessions_inotify_cannot_watch_refresh_before_every_query() {
    for watches in [Some(2), None] {
        let work = tempfile::tempdir().unwrap();
        for directory in ["one", "two", "three"] {
            std::fs::create_dir(work.path().join(directory)).unwrap();
            let source = format!("int {directory}_value;\n");
            write(work.path().join(directory).join("a.c"), source).unwrap();
        }
        build(work.path(), &["one/a.c", "two/a.c", "three/a.c"]);
        let Some(mut session) = limited(work.path(), &["-l", "-f", "i.db"], watches) else {
            eprintln!("skipped: no user namespace to limit inotify in");
            return;
        };
        let reason = match watches {
            Some(_) => "the inotify watch limit is reached",
            None => "inotify: ",
        };
        session.expect(b">> ");
        session.send(b"1one_value\n");
        session.expect_line(|line| {
            line.starts_with("ripscope: cannot watch the sources for changes (")
                && line.contains(reason)
        });
        session.expect(b"cscope: 1 lines\none/a.c one_value 1 int one_value;\n>> ");
        write(work.path().join("three/a.c"), "int three_changed;\n").unwrap();
        session.query(
            false,
            "1three_changed",
            &[("three/a.c", "three_changed", 1, "int three_changed;")],
        );
        session.send(b"q\n");
        session.finish();
    }
}

#[cfg(target_os = "linux")]
#[test]
fn events_lost_from_a_full_queue_leave_the_next_query_to_refresh() {
    let queued: usize = std::fs::read_to_string("/proc/sys/fs/inotify/max_queued_events")
        .ok()
        .and_then(|limit| limit.trim().parse().ok())
        .unwrap_or(16384);
    if queued > 200_000 {
        eprintln!("skipped: the inotify queue holds {queued} events");
        return;
    }
    let work = tempfile::tempdir().unwrap();
    write(work.path().join("a.c"), "int Alpha;\n").unwrap();
    build(work.path(), &[]);
    let mut session = Session::start(work.path(), &["-l", "-f", "i.db"]);
    session.expect(b">> ");
    session.query(false, "1Alpha", &[("a.c", "Alpha", 1, "int Alpha;")]);
    // Two events a file, none of them about sources, until the queue overflows; the
    // edit's own events are then lost.
    for at in 0..queued / 2 + 64 {
        std::fs::File::create(work.path().join(format!("noise{at}.o"))).unwrap();
    }
    write(work.path().join("a.c"), "int Beta;\n").unwrap();
    session.query(false, "1Beta", &[("a.c", "Beta", 1, "int Beta;")]);
    write(work.path().join("a.c"), "int Gamma;\n").unwrap();
    session.query(false, "1Gamma", &[("a.c", "Gamma", 1, "int Gamma;")]);
    session.send(b"q\n");
    session.finish();
}

#[cfg(unix)]
#[test]
fn an_edit_made_while_automatic_refresh_waits_for_its_list_is_read() {
    use std::os::unix::fs::OpenOptionsExt;
    use std::time::Instant;
    let work = tempfile::tempdir().unwrap();
    write(work.path().join("a.c"), "int Alpha;\n").unwrap();
    write(work.path().join("files"), "a.c\n").unwrap();
    build(work.path(), &["-i", "files"]);
    let mut session = Session::start(work.path(), &["-l", "-f", "i.db"]);
    session.expect(b">> ");
    std::fs::remove_file(work.path().join("files")).unwrap();
    assert!(
        Command::new("mkfifo")
            .arg(work.path().join("files"))
            .captured()
            .unwrap()
            .status
            .success()
    );
    session.send(b"1Beta\n");
    // The real Selection rule's FIFO holds Refresh after it began and before source
    // capture. A nonblocking writer connects only once Refresh is reading that list.
    let start = Instant::now();
    let mut list = loop {
        match std::fs::OpenOptions::new()
            .write(true)
            .custom_flags(rustix::fs::OFlags::NONBLOCK.bits() as i32)
            .open(work.path().join("files"))
        {
            Ok(list) => break list,
            Err(error) => {
                assert!(
                    start.elapsed() < Duration::from_secs(20),
                    "Refresh did not read the list: {error}"
                );
                std::thread::sleep(Duration::from_millis(1));
            }
        }
    };
    write(work.path().join("a.c"), "int Beta;\n").unwrap();
    list.write_all(b"a.c\n").unwrap();
    drop(list);
    session.expect(b"cscope: 1 lines\na.c Beta 1 int Beta;\n>> ");
    std::fs::remove_file(work.path().join("files")).unwrap();
    write(work.path().join("files"), "a.c\n").unwrap();
    write(work.path().join("a.c"), "int Gamma;\n").unwrap();
    session.query(false, "1Gamma", &[("a.c", "Gamma", 1, "int Gamma;")]);
    session.send(b"q\n");
    session.finish();
}

#[cfg(unix)]
#[test]
fn automatic_refresh_keeps_unreadable_source_facts_and_retries_next_request() {
    use std::os::unix::fs::PermissionsExt;
    let work = tempfile::tempdir().unwrap();
    let source = work.path().join("a.c");
    write(&source, "int Alpha;\n").unwrap();
    build(work.path(), &["a.c"]);
    let mut session = Session::start(work.path(), &["-l", "-f", "i.db"]);
    session.expect(b">> ");
    std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o0)).unwrap();
    if std::fs::File::open(&source).is_ok() {
        std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o600)).unwrap();
        session.send(b"q\n");
        session.finish();
        return;
    }
    session.send(b"1Alpha\n");
    session.expect_line(|line| line.starts_with("ripscope: cannot read a.c:"));
    session.expect(b"cscope: 1 lines\na.c Alpha 1 int Alpha;\n>> ");
    std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o600)).unwrap();
    write(&source, "int Beta;\n").unwrap();
    session.query(false, "1Beta", &[("a.c", "Beta", 1, "int Beta;")]);
    session.send(b"q\n");
    session.finish();
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn caseless_identifiers_keep_every_prefix_spelling_and_one_definition_per_line() {
    for newline in ["\n", "\r\n"] {
        let work = tempfile::tempdir().unwrap();
        let source = "int kerProbe;\nint KerProbe;\nint kErProbe;\nint KErProbe;\nint keRProbe;\nint KeRProbe;\nint kERProbe;\nint KERProbe, kerProbe;\nint kerProbed;\nint a, A;\nint _a1Probe, _A1Probe;\nint __;\n";
        write(work.path().join("case.c"), source.replace('\n', newline)).unwrap();
        build(work.path(), &["case.c"]);
        std::fs::remove_file(work.path().join("case.c")).unwrap();
        for (pattern, expected) in [
            (
                "kErPrObE",
                "case.c kErPrObE 1 int kerProbe;\ncase.c kErPrObE 2 int KerProbe;\ncase.c kErPrObE 3 int kErProbe;\ncase.c kErPrObE 4 int KErProbe;\ncase.c kErPrObE 5 int keRProbe;\ncase.c kErPrObE 6 int KeRProbe;\ncase.c kErPrObE 7 int kERProbe;\ncase.c kErPrObE 8 int KERProbe, kerProbe;\n",
            ),
            ("A", "case.c A 10 int a, A;\n"),
            ("_a1pROBE", "case.c _a1pROBE 11 int _a1Probe, _A1Probe;\n"),
            ("__", "case.c __ 12 int __;\n"),
        ] {
            let output = run(work.path(), &["-dLC", "-f", "i.db", "-1", pattern]);
            assert!(output.status.success(), "{:?}", output);
            assert_eq!(output.stdout, expected.as_bytes(), "{pattern}");
            assert!(output.stderr.is_empty(), "{:?}", output);
        }
    }
}

#[test]
fn exact_stored_requests_keep_rows_while_fields_patterns_and_case_change() {
    for newline in ["\n", "\r\n"] {
        let work = tempfile::tempdir().unwrap();
        let source = "int Alpha;\nint alpha;\nint leaf(void) { return Alpha; }\nvoid change(void) { Alpha = leaf(); }\n";
        write(work.path().join("case.c"), source.replace('\n', newline)).unwrap();
        build(work.path(), &["case.c"]);
        std::fs::remove_file(work.path().join("case.c")).unwrap();
        for json in [false, true] {
            let mut args = vec!["-dl", "-f", "i.db"];
            if json {
                args.push("--json");
            }
            let mut session = Session::start(work.path(), &args);
            let ready: &[u8] = if json {
                b"{\"type\":\"ready\"}\n"
            } else {
                b">> "
            };
            session.expect(ready);
            for (request, rows) in [
                ("1Alpha", "case.c Alpha 1 int Alpha;\n"),
                (
                    "9Alpha",
                    "case.c change 4 void change(void ) { Alpha = leaf(); }\n",
                ),
                (
                    "3leaf",
                    "case.c change 4 void change(void ) { Alpha = leaf(); }\n",
                ),
                (
                    "2change",
                    "case.c leaf 4 void change(void ) { Alpha = leaf(); }\n",
                ),
                (
                    "0Alpha",
                    "case.c <global> 1 int Alpha;\ncase.c leaf 3 int leaf(void ) { return Alpha; }\ncase.c change 4 void change(void ) { Alpha = leaf(); }\n",
                ),
                ("1absent", ""),
                ("c", ""),
                (
                    "1ALPHA",
                    "case.c ALPHA 1 int Alpha;\ncase.c ALPHA 2 int alpha;\n",
                ),
                (
                    "3LEAF",
                    "case.c change 4 void change(void ) { Alpha = leaf(); }\n",
                ),
                ("c", ""),
                ("1ALPHA", ""),
                ("1alpha", "case.c alpha 2 int alpha;\n"),
            ] {
                session.send(format!("{request}\n").as_bytes());
                if request != "c" {
                    if json {
                        session.expect(
                            format!(
                                "{{\"type\":\"result\",\"count\":{}}}\n",
                                rows.lines().count()
                            )
                            .as_bytes(),
                        );
                        for row in rows.lines() {
                            let parts: Vec<_> = row.splitn(4, ' ').collect();
                            let row = serde_json::json!([
                                parts[0],
                                parts[1],
                                parts[2].parse::<u32>().unwrap(),
                                parts[3]
                            ]);
                            session.expect(format!("{row}\n").as_bytes());
                        }
                    } else if rows.is_empty() {
                        session.expect(b"Unable to search database\n");
                    } else {
                        session.expect(
                            format!("cscope: {} lines\n{rows}", rows.lines().count()).as_bytes(),
                        );
                    }
                }
                session.expect(ready);
            }
            session.send(b"q\n");
            session.finish();
        }
    }
}

#[test]
fn readiness_checks_metadata_and_the_path_list_before_line_or_json_ready() {
    for defect in ["selection", "revision", "missing-analyzer", "path-list"] {
        let work = tempfile::tempdir().unwrap();
        write(work.path().join("a.c"), "int Alpha;\n").unwrap();
        write(work.path().join("b.c"), "int Beta;\n").unwrap();
        build(work.path(), &["a.c", "b.c"]);
        let index = work.path().join("i.db");
        let connection = rusqlite::Connection::open(&index).unwrap();
        match defect {
            "selection" => {
                connection
                    .execute("UPDATE meta SET value=x'00' WHERE key='selection'", [])
                    .unwrap();
            }
            "revision" => {
                connection
                    .execute("UPDATE meta SET value='bad' WHERE key='revision'", [])
                    .unwrap();
            }
            "missing-analyzer" => {
                connection
                    .execute("DELETE FROM meta WHERE key='analyzer'", [])
                    .unwrap();
            }
            "path-list" => {
                // The second path's length runs past the list's end.
                connection
                    .execute(
                        "UPDATE meta SET value=x'03612e6309622e63' WHERE key='paths'",
                        [],
                    )
                    .unwrap();
            }
            _ => unreachable!(),
        }
        drop(connection);
        let query = run(work.path(), &["-dL", "-f", "i.db", "-7", ".*"]);
        assert!(
            !query.status.success() && query.stdout.is_empty(),
            "{defect}: {query:?}"
        );
        for json in [false, true] {
            let mut args = vec!["-dl", "-f", "i.db"];
            if json {
                args.push("--json");
            }
            let startup = run(work.path(), &args);
            assert!(!startup.status.success(), "{defect}: {startup:?}");
            assert!(startup.stdout.is_empty(), "{defect}: {startup:?}");
            assert_eq!(startup.stderr, query.stderr, "{defect}: {startup:?}");
        }
    }
}

#[test]
fn readiness_keeps_paths_with_unreadable_analyses_or_empty_published_membership() {
    for state in ["other-analyzer", "unreadable-analyzer", "no-files"] {
        let work = tempfile::tempdir().unwrap();
        write(work.path().join("a.c"), "int Alpha;\n").unwrap();
        write(work.path().join("b.c"), "int Beta;\n").unwrap();
        build(work.path(), &["a.c", "b.c"]);
        let connection = rusqlite::Connection::open(work.path().join("i.db")).unwrap();
        let change = match state {
            "other-analyzer" => "UPDATE meta SET value=x'00' WHERE key='analyzer'",
            "unreadable-analyzer" => "UPDATE meta SET value=1.5 WHERE key='analyzer'",
            "no-files" => {
                "DELETE FROM files; DELETE FROM names; DELETE FROM units;
                 UPDATE meta SET value=x'' WHERE key='paths';"
            }
            _ => unreachable!(),
        };
        connection.execute_batch(change).unwrap();
        drop(connection);
        let files = if state == "no-files" {
            &b""[..]
        } else {
            &b"a.c <unknown> 1 <unknown>\nb.c <unknown> 1 <unknown>\n"[..]
        };
        let query = run(work.path(), &["-dL", "-f", "i.db", "-7", ".*"]);
        assert!(query.status.success(), "{state}: {query:?}");
        assert_eq!(query.stdout, files, "{state}: {query:?}");
        assert!(query.stderr.is_empty(), "{state}: {query:?}");
        for json in [false, true] {
            let mut args = vec!["-dl", "-f", "i.db"];
            if json {
                args.push("--json");
            }
            let startup = run(work.path(), &args);
            assert!(startup.status.success(), "{state}: {startup:?}");
            let ready = if json {
                &b"{\"type\":\"ready\"}\n"[..]
            } else {
                &b">> "[..]
            };
            assert_eq!(startup.stdout, ready, "{state}: {startup:?}");
            assert!(startup.stderr.is_empty(), "{state}: {startup:?}");
        }
    }
}

#[test]
fn split_calls_keep_exact_rows_in_both_stored_interfaces_without_sources() {
    for newline in ["\n", "\r\n"] {
        let work = tempfile::tempdir().unwrap();
        let source =
            include_str!("fixtures/scanner/probes/74-split-calls.c").replace('\n', newline);
        write(work.path().join("case.c"), source).unwrap();
        build(work.path(), &["case.c"]);
        std::fs::remove_file(work.path().join("case.c")).unwrap();
        let mut session = Session::start(work.path(), &["-dl", "-f", "i.db"]);
        session.expect(b">> ");
        for (field, pattern, rows) in [
            ("2", "split", "case.c leaf 5 return (leaf\n"),
            ("2", "assigned", "case.c leaf 10 if ((result = (leaf\n"),
            (
                "3",
                "leaf",
                concat!(
                    "case.c ordinary 2 int ordinary(void ) { return leaf(1); }\n",
                    "case.c grouped 3 int grouped(void ) { return (leaf(1)); }\n",
                    "case.c split 5 return (leaf\n",
                    "case.c assigned 10 if ((result = (leaf\n",
                ),
            ),
            ("1", "split", "case.c split 4 int split(void ) {\n"),
        ] {
            let answer = run(
                work.path(),
                &["-dL", "-f", "i.db", &format!("-{field}"), pattern],
            );
            assert!(
                answer.status.success() && answer.stderr.is_empty(),
                "{answer:?}"
            );
            assert_eq!(answer.stdout, rows.as_bytes());
            session.send(format!("{field}{pattern}\n").as_bytes());
            session.expect(format!("cscope: {} lines\n{rows}>> ", rows.lines().count()).as_bytes());
            let json = run(
                work.path(),
                &["-dL", "--json", "-f", "i.db", &format!("-{field}"), pattern],
            );
            assert!(json.status.success() && json.stderr.is_empty(), "{json:?}");
            let parsed: Vec<serde_json::Value> = json
                .stdout
                .split(|byte| *byte == b'\n')
                .filter(|row| !row.is_empty())
                .map(|row| serde_json::from_slice(row).unwrap())
                .collect();
            let expected: Vec<_> = rows
                .lines()
                .map(|row| {
                    let columns: Vec<_> = row.splitn(4, ' ').collect();
                    serde_json::json!([
                        columns[0],
                        columns[1],
                        columns[2].parse::<u32>().unwrap(),
                        columns[3]
                    ])
                })
                .collect();
            assert_eq!(parsed, expected);
        }
        session.send(b"q\n");
        session.finish();
    }
}

/// Run with an actual scanner-32 executable to cover analyzer migration and warmed
/// posting/block caches, rather than editing an index's recorded analyzer identity.
#[test]
#[ignore = "requires RIPSCOPE_PREVIOUS_BINARY pointing to scanner 32"]
fn split_calls_refresh_migrates_a_warmed_previous_product_index() {
    let previous = std::env::var_os("RIPSCOPE_PREVIOUS_BINARY").expect("preceding product");
    for newline in ["\n", "\r\n"] {
        let work = tempfile::tempdir().unwrap();
        let source = "int f(void) { return leaf\n(1); }\n".replace('\n', newline);
        write(work.path().join("case.c"), source).unwrap();
        let built = Command::new(&previous)
            .current_dir(work.path())
            .args(["-bk", "-f", "i.db", "case.c"])
            .env_remove("INCLUDEDIRS")
            .env_remove("INCDIR")
            .captured()
            .unwrap();
        assert!(
            built.status.success() && built.stderr.is_empty(),
            "{built:?}"
        );
        let metadata = || index_metadata(work.path());
        let before = metadata();
        assert!(before.2.starts_with(b"ripscope-scanner-32+"));
        let mut session = Session::start(work.path(), &["-dl", "-f", "i.db"]);
        session.expect(b">> ");
        session.send(b"2f\n3leaf\n");
        session.expect(b"Unable to search database\n>> Unable to search database\n>> ");
        assert_eq!(metadata(), before);
        session.send(b"r\n");
        session.expect(b"\n>> ");
        let after = metadata();
        assert!(after.0 > before.0);
        assert_ne!(after.1, before.1);
        assert!(after.2.starts_with(b"ripscope-scanner-37+"));
        for _ in 0..2 {
            session.send(b"2f\n3leaf\n");
            session.expect(b"cscope: 1 lines\ncase.c leaf 1 int f(void ) { return leaf\n>> ");
            session.expect(b"cscope: 1 lines\ncase.c f 1 int f(void ) { return leaf\n>> ");
            session.send(b"r\n");
            session.expect(b"\n>> ");
            assert_eq!(metadata(), after, "unchanged Refresh keeps the revision");
        }
        session.send(b"q\n");
        session.finish();
    }
}

/// Supply the actual preceding product executable, rather than relabelling a
/// candidate index. This keeps scanner-only migrations testable at the CLI seam.
#[test]
#[ignore = "requires RIPSCOPE_PREVIOUS_BINARY pointing to the preceding product"]
fn macro_constructor_refresh_migrates_a_warmed_previous_product_index() {
    let previous =
        std::env::var_os("RIPSCOPE_PREVIOUS_BINARY").expect("preceding product executable");
    let source = include_str!("fixtures/scanner/probes/73-macro-constructor-heads.c")
        .lines()
        .take(17)
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    for newline in ["\n", "\r\n"] {
        let work = tempfile::tempdir().expect("work");
        write(work.path().join("case.c"), source.replace('\n', newline)).unwrap();
        let built = Command::new(&previous)
            .current_dir(work.path())
            .args(["-bk", "-f", "i.db", "case.c"])
            .env_remove("INCLUDEDIRS")
            .env_remove("INCDIR")
            .captured()
            .unwrap();
        assert!(
            built.status.success() && built.stderr.is_empty(),
            "{built:?}"
        );
        let metadata = || index_metadata(work.path());
        let before = metadata();
        assert!(before.2.starts_with(b"ripscope-scanner-29+"));
        let fields = [
            ("3", "CppConcat"),
            ("2", "GENERATED"),
            ("1", "CppConcat"),
            ("1", "helper|ordinary"),
            ("3", "helper"),
        ];
        let old: Vec<_> = fields
            .iter()
            .map(|(field, pattern)| {
                let answer = run(
                    work.path(),
                    &["-dL", "-f", "i.db", &format!("-{field}"), pattern],
                );
                assert!(
                    answer.status.success() && answer.stderr.is_empty(),
                    "{answer:?}"
                );
                answer.stdout
            })
            .collect();
        assert!(String::from_utf8_lossy(&old[0]).contains(" GENERATED 5 "));
        assert!(String::from_utf8_lossy(&old[0]).contains(" GENERATED_OPAQUE 16 "));
        let mut session = Session::start(work.path(), &["-dl", "-f", "i.db"]);
        session.expect(b">> ");
        let ask = |session: &mut Session, field: &str, pattern: &str, rows: &[u8]| {
            session.send(format!("{field}{pattern}\n").as_bytes());
            session.expect(
                format!(
                    "cscope: {} lines\n",
                    rows.iter().filter(|&&b| b == b'\n').count()
                )
                .as_bytes(),
            );
            session.expect(rows);
            session.expect(b">> ");
        };
        for ((field, pattern), rows) in fields.iter().zip(&old) {
            ask(&mut session, field, pattern, rows);
        }
        assert_eq!(
            metadata(),
            before,
            "startup and cache warming must not update"
        );
        session.send(b"r\n");
        session.expect(b"\n>> ");
        let after = metadata();
        assert!(after.0 > before.0, "Refresh must publish a revision");
        assert_ne!(after.1, before.1);
        assert!(after.2.starts_with(b"ripscope-scanner-30+"));
        let new = [
            b"case.c ordinary 10 int ordinary(void ) { return CppConcat(helper()); }\n".to_vec(),
            b"case.c helper 6 { return helper(); }\n".to_vec(),
            old[2].clone(),
            old[3].clone(),
            old[4].clone(),
        ];
        for ((field, pattern), rows) in fields.iter().zip(&new) {
            ask(&mut session, field, pattern, rows);
            assert_eq!(
                run(
                    work.path(),
                    &["-dL", "-f", "i.db", &format!("-{field}"), pattern]
                )
                .stdout,
                *rows
            );
        }
        session.send(b"r\n");
        session.expect(b"\n>> ");
        assert_eq!(metadata(), after, "unchanged Refresh preserves publication");
        session.send(b"q\n");
        session.finish();
        println!(
            "{newline:?}: scanner 29 -> 30, revision {} -> {}; warmed queries and unchanged Refresh pass",
            before.0, after.0
        );
    }
}

#[test]
fn vim_prepends_its_path_to_text_rows_and_acknowledges_errors() {
    let work = tempfile::tempdir().expect("work");
    let root = work.path().to_str().expect("root").replace('\\', "/");
    write(work.path().join("a.c"), "int Alpha;\n").expect("source");
    build(work.path(), &["a.c"]);
    // As Vim starts it: `cscopeprg -dl -f DB -P<path>`, with the owned configuration's flag.
    let prefix = format!("-P{root}");
    let mut session = Session::start(work.path(), &["--vim-errors", "-dl", "-f", "i.db", &prefix]);
    session.expect(b">> ");
    // Stored rows keep the names the index holds; cscope prepends only where it reads files.
    session.send(b"1Alpha\n");
    session.expect(b"cscope: 1 lines\na.c Alpha 1 int Alpha;\n>> ");
    session.send(b"4Alpha\n");
    session.expect(format!("cscope: 1 lines\n{root}/a.c <unknown> 1 int Alpha;\n>> ").as_bytes());
    // An unusable request: Vim shows the message once it reads the RETURN prompt.
    session.send(b"1[\n");
    session.expect(b"Unable to search database\n");
    session.expect_line(|line| line.starts_with("ripscope: ") && !line.contains('>'));
    session.expect(b"Press the RETURN key to continue:");
    session.send(b"\n");
    session.expect(b">> ");
    session.send(b"q\n");
    session.finish();
}

#[test]
fn vim_acknowledgement_eof_commands_and_oversized_lines_keep_the_session_sequence() {
    let work = tempfile::tempdir().unwrap();
    write(work.path().join("a.c"), "int Alpha;\n").unwrap();
    build(work.path(), &["a.c"]);
    for reply in [
        None,
        Some(b"1Alpha\n".to_vec()),
        Some([vec![b'x'; 4099], b"\n1Alpha\n".to_vec()].concat()),
    ] {
        let mut session = Session::start(work.path(), &["--vim-errors", "-dl", "-f", "i.db"]);
        session.expect(b">> ");
        session.send(b"1[\n");
        session.expect(b"Unable to search database\nripscope: the query pattern cannot be used\nPress the RETURN key to continue:");
        if let Some(reply) = reply {
            session.send(&reply);
            session.expect(b">> cscope: 1 lines\na.c Alpha 1 int Alpha;\n>> ");
            session.send(b"q\n");
        }
        // Closing input at the RETURN prompt ends without another ready prompt.
        // A nonempty reply is executed once; an oversized reply is drained whole.
        session.finish();
    }
}

#[test]
fn warnings_precede_an_answer_and_vim_is_told_after_it() {
    let work = tempfile::tempdir().expect("work");
    write(work.path().join("a.c"), "int Alpha;\n").expect("source");
    let mut sources = vec!["a.c".to_owned()];
    // Enough unreadable files that their warnings would overflow Vim's buffer before its
    // RETURN prompt (about 1,000 bytes), were they written there.
    for n in 0..20 {
        let name = format!("gone_with_a_long_name_{n:02}.c");
        write(work.path().join(&name), "int AlphaGone;\n").expect("source");
        sources.push(name);
    }
    let names: Vec<&str> = sources.iter().map(String::as_str).collect();
    build(work.path(), &names);
    for name in &sources[1..] {
        std::fs::remove_file(work.path().join(name)).expect("removed");
    }

    // One-shot (xcscope): the note comes first, and never looks like a row.
    let (ok, output) = merged(work.path(), &["-dL", "-f", "i.db", "-4", "Alpha"]);
    assert!(ok);
    let output = String::from_utf8_lossy(&output);
    let lines: Vec<&str> = output.lines().collect();
    assert_eq!(lines.len(), 21, "{output}");
    for line in &lines[..20] {
        assert!(line.starts_with("ripscope: cannot read gone_"), "{output}");
        assert!(
            line.split(' ')
                .nth(2)
                .is_none_or(|word| word.parse::<u64>().is_err())
        );
    }
    assert_eq!(lines[20], "a.c <unknown> 1 int Alpha;");

    // A session writes them before the answer too, where Vim passes over them; with
    // --vim-errors a short fixed notice after the rows has Vim show that something went
    // wrong.
    for vim in [false, true] {
        let mut args = vec!["-dl", "-f", "i.db"];
        if vim {
            args.insert(0, "--vim-errors");
        }
        let mut session = Session::start(work.path(), &args);
        session.expect(b">> ");
        session.send(b"4Alpha\n");
        for _ in 0..20 {
            session.expect_line(|line| line.starts_with("ripscope: cannot read gone_"));
        }
        session.expect(b"cscope: 1 lines\na.c <unknown> 1 int Alpha;\n");
        if vim {
            session.expect_line(|line| line.starts_with("ripscope: ") && line.len() < 200);
            session.expect(b"Press the RETURN key to continue:");
            session.send(b"\n");
        }
        session.expect(b">> ");
        session.send(b"q\n");
        session.finish();
    }
}

#[test]
fn a_session_answers_every_request_and_sees_new_revisions() {
    let work = tempfile::tempdir().expect("work");
    write(work.path().join("a.c"), "int Alpha;\nint Beta;\n").expect("source");
    build(work.path(), &["a.c"]);
    let mut session = Session::start(work.path(), &["-dl", "-f", "i.db"]);
    session.expect(b">> ");
    session.send(b"0Alpha|Beta\r\n");
    session.expect(b"cscope: 2 lines\na.c <global> 1 int Alpha;\na.c <global> 2 int Beta;\n>> ");
    let mut oversized = b"1".to_vec();
    oversized.extend(std::iter::repeat_n(b'A', 32_000));
    oversized.push(b'\n');
    // Nothing found, as cscope says it.
    session.send(b"1Missing\n");
    session.expect(b"Unable to search database\n>> ");
    // A request that cannot be answered also gets its reason on standard error.
    for (request, reason) in [
        (&b"1[\n"[..], "invalid query pattern: invalid syntax"),
        (
            b"1\xff\n",
            "invalid query pattern: a symbol pattern is printable ASCII, a file or include \
             pattern holds no carriage return, and no pattern holds a NUL or a line break",
        ),
        (b"1A\0lpha\n", "the query pattern holds a NUL byte"),
        (&oversized, "the query request is too long"),
    ] {
        session.send(request);
        session.expect(b"Unable to search database\n");
        session.expect_line(|line| line == format!("ripscope: {reason}\n"));
        session.expect(b">> ");
    }
    // cscope's line mode answers field 5 as a text search.
    session.send(b"5Beta\n");
    session.expect(b"cscope: 1 lines\na.c <unknown> 2 int Beta;\n>> ");
    // A definition pattern loses its trailing blanks, as Vim sends `:cs find g Alpha `.
    session.send(b"1Alpha \n");
    session.expect(b"cscope: 1 lines\na.c Alpha 1 int Alpha;\n>> ");
    // A command is known by its first byte, as in cscope.
    session.send(b"cxyz\n1alpha\ncxyz\n");
    session.expect(b">> cscope: 1 lines\na.c alpha 1 int Alpha;\n>> >> ");
    // cscope's commands to change its file list are answered, but change nothing.
    session.send(b"Fb.c\n");
    session.expect(b"\n");
    session.expect_line(|line| line.starts_with("ripscope: "));
    session.expect(b">> ");
    // An unknown command is reported on standard error, and the prompt returns.
    session.send(b"x\n");
    session.expect_line(|line| line.starts_with("ripscope: "));
    session.expect(b">> ");
    // Another process publishes; the next query sees it.
    write(work.path().join("a.c"), "int Delta;\n").expect("edit");
    build(work.path(), &["a.c"]);
    session.send(b"1Delta\n");
    session.expect(b"cscope: 1 lines\na.c Delta 1 int Delta;\n>> ");
    session.send(b"quit\n");
    session.finish();
}

#[test]
fn includers_and_files_keep_order_across_refresh_and_index_replacement() {
    for newline in ["\n", "\r\n"] {
        for json in [false, true] {
            let work = tempfile::tempdir().unwrap();
            let save = |path: &str, text: &str| {
                write(work.path().join(path), text.replace('\n', newline)).unwrap();
            };
            let mut source = "#include \"old.h\"\n".to_owned();
            for n in 0..1000 {
                source.push_str(&format!("int value_{n};\n"));
            }
            source.push_str("#include \"old.h\"\n");
            save("a.c", &source);
            save("b.c", "#include \"old.h\"\n");
            save("old.h", "#include <leaf.h>\nint Old;\n");
            let built = command(work.path(), &["-bk", "-f", "i.db", "a.c", "b.c"])
                .env("RIPSCOPE_BLOCK_SIZE", "256")
                .captured()
                .unwrap();
            assert!(built.status.success(), "{built:?}");
            let mut args = vec!["-dl", "-f", "i.db"];
            if json {
                args.push("--json");
            }
            let mut session = Session::start(work.path(), &args);
            let ready = if json {
                &b"{\"type\":\"ready\"}\n"[..]
            } else {
                b">> "
            };
            session.expect(ready);
            let ask = |session: &mut Session, request: &str, rows: &[(&str, &str, u32, &str)]| {
                session.send(format!("{request}{newline}").as_bytes());
                let mut expected = Vec::new();
                if json {
                    writeln!(expected, "{{\"type\":\"result\",\"count\":{}}}", rows.len()).unwrap();
                    for &(path, context, line, text) in rows {
                        writeln!(
                            expected,
                            "{}",
                            serde_json::json!([path, context, line, text])
                        )
                        .unwrap();
                    }
                } else if rows.is_empty() {
                    expected.extend_from_slice(b"Unable to search database\n");
                } else {
                    writeln!(expected, "cscope: {} lines", rows.len()).unwrap();
                    for &(path, context, line, text) in rows {
                        writeln!(expected, "{path} {context} {line} {text}").unwrap();
                    }
                }
                expected.extend_from_slice(ready);
                session.expect(&expected);
            };
            let old = [
                ("a.c", "<global>", 1, "#include \"old.h\""),
                ("a.c", "<global>", 1002, "#include \"old.h\""),
                ("b.c", "<global>", 1, "#include \"old.h\""),
            ];
            let files = [
                ("a.c", "<unknown>", 1, "<unknown>"),
                ("b.c", "<unknown>", 1, "<unknown>"),
            ];
            for _ in 0..2 {
                ask(&mut session, "8old.h", &old);
                ask(&mut session, "7^[ab]\\.c$", &files);
                ask(&mut session, "8^old\\.h$", &old);
                ask(&mut session, "8absent.h", &[]);
            }
            session.send(b"c\n");
            session.expect(ready);
            ask(&mut session, "8OLD.H", &old);
            session.send(b"c\n");
            session.expect(ready);
            let before = index_metadata(work.path());
            session.send(b"r\n");
            session.expect(if json {
                b"{\"type\":\"rebuilt\"}\n"
            } else {
                b"\n"
            });
            session.expect(ready);
            assert_eq!(index_metadata(work.path()), before, "unchanged Refresh");
            save("a.c", "#include <new.h>\nint Changed;\n");
            save("old.h", "#include <newleaf.h>\nint Old;\n");
            ask(&mut session, "8old.h", &old);
            session.send(b"r\n");
            session.expect(if json {
                b"{\"type\":\"rebuilt\"}\n"
            } else {
                b"\n"
            });
            session.expect(ready);
            assert!(index_metadata(work.path()).0 > before.0);
            ask(
                &mut session,
                "8new.h",
                &[("a.c", "<global>", 1, "#include <new.h>")],
            );
            ask(&mut session, "8old.h", &[old[2]]);
            ask(&mut session, "8^leaf\\.h$", &[]);
            ask(
                &mut session,
                "8newleaf.h",
                &[("old.h", "<global>", 1, "#include <newleaf.h>")],
            );
            ask(&mut session, "7^[ab]\\.c$", &files);
            save("b.c", "#include <alternate.h>\n");
            build(work.path(), &["a.c", "b.c"]);
            ask(
                &mut session,
                "8alternate.h",
                &[("b.c", "<global>", 1, "#include <alternate.h>")],
            );
            ask(&mut session, "8old.h", &[]);
            for path in ["a.c", "b.c", "old.h"] {
                std::fs::remove_file(work.path().join(path)).unwrap();
            }
            ask(
                &mut session,
                "8new.h",
                &[("a.c", "<global>", 1, "#include <new.h>")],
            );
            ask(&mut session, "7^[ab]\\.c$", &files);
            let old_identity = index_metadata(work.path()).1;
            for path in ["i.db", "i.db-wal", "i.db-shm"] {
                let _ = std::fs::remove_file(work.path().join(path));
            }
            save("c.c", "#include <replacement.h>\n");
            build(work.path(), &["c.c"]);
            assert_ne!(index_metadata(work.path()).1, old_identity);
            ask(
                &mut session,
                "8replacement.h",
                &[("c.c", "<global>", 1, "#include <replacement.h>")],
            );
            ask(&mut session, "8new.h", &[]);
            ask(&mut session, "7.*", &[("c.c", "<unknown>", 1, "<unknown>")]);
            session.send(b"q\n");
            session.finish();
        }
    }
}

/// Includers answer from the rows the index records for each Include and read no Block:
/// damage in any Block, one that holds a matching Include among them, leaves the answer
/// whole, and `--verify` finds it.
#[test]
fn includers_read_no_blocks() {
    let mutations = ["UPDATE blocks SET image = X'00'", "DELETE FROM blocks"];
    for ((mutation, include_at_end), damaged) in mutations
        .into_iter()
        .flat_map(|mutation| [false, true].map(|at_end| (mutation, at_end)))
        .flat_map(|case| [1, 0].map(|block| (case, block)))
    {
        let work = tempfile::tempdir().unwrap();
        let mut source = "#include <target.h>\n".to_owned();
        for n in 0..100 {
            source.push_str(&format!("int value_{n};\n"));
        }
        if include_at_end {
            source.push_str("#include <target.h>\n");
        }
        write(work.path().join("a.c"), source).unwrap();
        write(work.path().join("b.c"), "#include <target.h>\n").unwrap();
        let built = command(work.path(), &["-bk", "-f", "i.db", "a.c", "b.c"])
            .env("RIPSCOPE_BLOCK_SIZE", "128")
            .captured()
            .unwrap();
        assert!(built.status.success(), "{built:?}");
        let index = rusqlite::Connection::open(work.path().join("i.db")).unwrap();
        index
            .execute_batch(&format!(
                "{mutation} WHERE id = (SELECT (unit << 20) + {damaged} FROM files \
                 WHERE path = CAST('a.c' AS BLOB))"
            ))
            .unwrap();
        drop(index);
        for path in ["a.c", "b.c"] {
            std::fs::remove_file(work.path().join(path)).unwrap();
        }
        let found = run(work.path(), &["-dL", "-f", "i.db", "-8", "target.h"]);
        assert!(found.status.success(), "{found:?}");
        let expected = if include_at_end {
            "a.c <global> 1 #include <target.h>\na.c <global> 102 #include <target.h>\n\
             b.c <global> 1 #include <target.h>\n"
        } else {
            "a.c <global> 1 #include <target.h>\nb.c <global> 1 #include <target.h>\n"
        };
        assert_eq!(
            String::from_utf8_lossy(&found.stdout),
            expected,
            "{damaged}"
        );
        assert_eq!(found.stderr, b"");
        let verified = run(work.path(), &["--verify", "-d", "-f", "i.db"]);
        assert!(!verified.status.success(), "{verified:?}");
    }
}

/// Leaves an index with nothing published, as readers see one an index build is making afresh
/// before its first commit: revision 0 and no files (its selection rule, analyzer and
/// identity stay).
fn unpublish(index: &Path) {
    rusqlite::Connection::open(index)
        .expect("index")
        .execute_batch(
            "UPDATE meta SET value = 0 WHERE key = 'revision';
             DELETE FROM files; DELETE FROM names; DELETE FROM units;",
        )
        .expect("unpublished");
}

/// An index that holds no published revision yet, as while another process makes it
/// afresh, is no index: a query fails as for a missing one and says why, instead of
/// answering nothing, and a session does not start. Once nothing holds it, a query
/// without `-d` builds it and answers.
#[test]
fn an_index_with_nothing_published_is_no_index_yet() {
    let work = tempfile::tempdir().expect("work");
    write(work.path().join("a.c"), "int Alpha;\n").expect("source");
    build(work.path(), &["a.c"]);
    unpublish(&work.path().join("i.db"));
    let unpublished = "ripscope: no index at i.db yet: nothing is published in it\n";
    for args in [
        &["-dL", "-f", "i.db", "-1", "Alpha"][..],
        &["-dl", "-f", "i.db"],
        &["--verify", "-d", "-f", "i.db"],
    ] {
        let output = run(work.path(), args);
        assert!(!output.status.success(), "{args:?}: {output:?}");
        assert!(output.stdout.is_empty(), "{args:?}: {output:?}");
        assert_eq!(
            String::from_utf8_lossy(&output.stderr),
            unpublished,
            "{args:?}"
        );
    }
    let writer = rusqlite::Connection::open(work.path().join("i.db")).expect("writer");
    writer.execute_batch("BEGIN IMMEDIATE").expect("held");
    let busy = run(work.path(), &["-L", "-f", "i.db", "-1", "Alpha"]);
    assert!(!busy.status.success(), "{busy:?}");
    assert!(busy.stdout.is_empty(), "{busy:?}");
    assert_eq!(
        String::from_utf8_lossy(&busy.stderr),
        "ripscope: no index at i.db yet: another process is building it\n"
    );
    drop(writer);
    let built = run(work.path(), &["-L", "-f", "i.db", "-1", "Alpha"]);
    assert!(built.status.success(), "{built:?}");
    assert_eq!(
        String::from_utf8_lossy(&built.stdout),
        "a.c Alpha 1 int Alpha;\n"
    );
}

/// Each session query reads the index as it is: one made afresh as soon as it publishes,
/// although it numbers its revisions from 1 again; until then the query fails and says
/// why, as a one-shot query does.
#[test]
fn a_session_reads_an_index_made_afresh_once_it_publishes() {
    let work = tempfile::tempdir().expect("work");
    write(work.path().join("a.c"), "int Alpha;\n").expect("source");
    write(work.path().join("b.c"), "int Beta;\n").expect("source");
    build(work.path(), &["a.c"]);
    let mut session = Session::start(work.path(), &["-dl", "-f", "i.db"]);
    session.expect(b">> ");
    session.send(b"7.*\n");
    session.expect(b"cscope: 1 lines\na.c <unknown> 1 <unknown>\n>> ");
    // Another process deletes the index and makes it afresh; until it commits, the new
    // index holds revision 0.
    std::fs::remove_file(work.path().join("i.db")).expect("old index removed");
    for log in ["i.db-wal", "i.db-shm"] {
        let _ = std::fs::remove_file(work.path().join(log));
    }
    build(work.path(), &["b.c"]);
    unpublish(&work.path().join("i.db"));
    session.send(b"7.*\n");
    session.expect(b"Unable to search database\n");
    session
        .expect_line(|line| line == "ripscope: no index at i.db yet: nothing is published in it\n");
    session.expect(b">> ");
    build(work.path(), &["b.c"]);
    session.send(b"7.*\n");
    session.expect(b"cscope: 1 lines\nb.c <unknown> 1 <unknown>\n>> ");
    session.send(b"q\n");
    session.finish();
}

/// Marks the index as one of an older schema version, as an earlier ripscope made it, so
/// that an index build replaces it with a new file instead of updating it in place.
fn make_older(index: &Path) {
    rusqlite::Connection::open(index)
        .expect("index")
        .execute_batch("PRAGMA user_version = 1; PRAGMA wal_checkpoint(TRUNCATE);")
        .expect("older");
}

/// An index build in another process replaces the index while an updating session waits
/// for its next request, just after it started and just after it answered `r`, and the
/// session's next query reads the new index. On Windows, which refuses to replace a file
/// a process holds open, the session lets go of the index it updated at once.
#[test]
fn an_index_build_replaces_the_index_while_a_session_waits() {
    let work = tempfile::tempdir().expect("work");
    for (name, text) in [("a.c", "Alpha"), ("b.c", "Beta"), ("c.c", "Gamma")] {
        write(work.path().join(name), format!("int {text};\n")).expect("source");
    }
    let index = work.path().join("i.db");
    build(work.path(), &["a.c"]);
    let mut session = Session::start(work.path(), &["-l", "-f", "i.db"]);
    session.expect(b">> ");
    make_older(&index);
    build(work.path(), &["b.c"]);
    session.query(false, "7.*", &[("b.c", "<unknown>", 1, "<unknown>")]);
    session.send(b"r\n");
    session.expect(b"\n>> ");
    make_older(&index);
    build(work.path(), &["c.c"]);
    session.query(false, "7.*", &[("c.c", "<unknown>", 1, "<unknown>")]);
    session.send(b"q\n");
    session.finish();
}

/// While the index cannot be read (here it is moved away), a session query fails and says
/// why, in each client's way; once the index is back, the next one answers.
#[test]
fn a_session_query_fails_while_the_index_cannot_be_read() {
    let work = tempfile::tempdir().expect("work");
    write(work.path().join("a.c"), "int Alpha;\n").expect("source");
    build(work.path(), &["a.c"]);
    let mut session = Session::start(work.path(), &["-dl", "-f", "i.db"]);
    session.expect(b">> ");
    session.send(b"1Alpha\n");
    session.expect(b"cscope: 1 lines\na.c Alpha 1 int Alpha;\n>> ");
    let (index, kept) = (work.path().join("i.db"), work.path().join("kept.db"));
    let mut json = Session::start(work.path(), &["-dl", "-f", "i.db", "--json"]);
    json.expect(b"{\"type\":\"ready\"}\n");
    let mut vim = Session::start(work.path(), &["-dl", "-f", "i.db", "--vim-errors"]);
    vim.expect(b">> ");
    std::fs::rename(&index, &kept).expect("move away");
    vim.send(b"1Alpha\n");
    vim.expect(
        b"Unable to search database\nripscope: the index cannot be read: no index at i.db\n\
          Press the RETURN key to continue:",
    );
    vim.send(b"\n");
    vim.expect(b">> ");
    session.send(b"1Alpha\n");
    session.expect(b"Unable to search database\n");
    session.expect_line(|line| line == "ripscope: no index at i.db\n");
    session.expect(b">> ");
    json.send(b"1Alpha\n");
    json.expect(b"{\"message\":\"no index at i.db\",\"type\":\"error\"}\n");
    json.expect(b"{\"type\":\"ready\"}\n");
    std::fs::rename(&kept, &index).expect("move back");
    session.send(b"1Alpha\n");
    session.expect(b"cscope: 1 lines\na.c Alpha 1 int Alpha;\n>> ");
    session.send(b"q\n");
    session.finish();
    json.send(b"q\n");
    json.finish();
    vim.send(b"q\n");
    vim.finish();
}

#[test]
fn json_sessions_frame_errors_and_empty_answers_apart() {
    let work = tempfile::tempdir().expect("work");
    write(work.path().join("a.c"), "int Alpha;\n").expect("source");
    build(work.path(), &["a.c"]);
    let output = command(work.path(), &["--json", "-dl", "-f", "i.db"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .start()
        .and_then(|mut child| {
            child
                .stdin
                .take()
                .expect("stdin")
                .write_all(b"1[\n1Missing\nx\nc\n1alpha\nr\nq\n")?;
            child.wait_with_output()
        })
        .expect("session");
    assert!(output.status.success(), "{output:?}");
    let frames: Vec<serde_json::Value> = output
        .stdout
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .map(|line| serde_json::from_slice(line).expect("frame"))
        .collect();
    let types: Vec<&str> = frames
        .iter()
        .map(|frame| frame.get("type").and_then(|t| t.as_str()).unwrap_or("row"))
        .collect();
    assert_eq!(
        types,
        [
            "ready", "error", "ready", "result", "ready", "error", "ready", "ready", "result",
            "row", "ready", "rebuilt", "ready"
        ]
    );
    assert_eq!(frames[3]["count"], 0);
    assert_eq!(frames[8]["count"], 1);
    // Field 1 shows the pattern as typed.
    assert_eq!(
        frames[9],
        serde_json::json!(["a.c", "alpha", 1, "int Alpha;"])
    );
}

/// While another process holds the index for writing, a query answers from the last
/// published revision and says so, and `r` rebuilds nothing: a JSON client gets an error
/// frame, not `rebuilt`.
#[test]
fn a_query_answers_from_the_last_revision_while_another_process_updates() {
    let work = tempfile::tempdir().expect("work");
    write(work.path().join("a.c"), "int Alpha;\n").expect("source");
    build(work.path(), &["a.c"]);
    write(work.path().join("a.c"), "int Beta;\n").expect("edit");
    let writer = rusqlite::Connection::open(work.path().join("i.db")).expect("writer");
    writer.execute_batch("BEGIN IMMEDIATE").expect("held");
    let busy = "another process is updating the index; answering from the last published revision";

    let query = run(work.path(), &["-L", "-f", "i.db", "-1", "Alpha"]);
    assert!(query.status.success(), "{query:?}");
    assert_eq!(
        String::from_utf8_lossy(&query.stdout),
        "a.c Alpha 1 int Alpha;\n"
    );
    assert_eq!(
        String::from_utf8_lossy(&query.stderr),
        format!("ripscope: {busy}\n")
    );
    let output = command(work.path(), &["--json", "-dl", "-f", "i.db"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .start()
        .and_then(|mut child| {
            child
                .stdin
                .take()
                .expect("stdin")
                .write_all(b"r\n1Alpha\nq\n")?;
            child.wait_with_output()
        })
        .expect("session");
    assert!(output.status.success(), "{output:?}");
    let frames: Vec<serde_json::Value> = output
        .stdout
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .map(|line| serde_json::from_slice(line).expect("frame"))
        .collect();
    assert_eq!(
        frames,
        [
            serde_json::json!({"type": "ready"}),
            serde_json::json!({"type": "error", "message": busy}),
            serde_json::json!({"type": "ready"}),
            serde_json::json!({"type": "result", "count": 1}),
            serde_json::json!(["a.c", "Alpha", 1, "int Alpha;"]),
            serde_json::json!({"type": "ready"}),
        ]
    );

    // Once the other process is done, the next query brings the index up to date.
    writer.execute_batch("ROLLBACK").expect("released");
    let query = run(work.path(), &["-L", "-f", "i.db", "-1", "Beta"]);
    assert!(query.status.success(), "{query:?}");
    assert_eq!(
        String::from_utf8_lossy(&query.stdout),
        "a.c Beta 1 int Beta;\n"
    );
    assert!(query.stderr.is_empty(), "{query:?}");
}

#[test]
fn text_rows_keep_tabs_and_crlf_endings_as_cscope_prints_them() {
    let work = tempfile::tempdir().expect("work");
    write(work.path().join("a.c"), b"\tint Alpha;\r\nint \x0cBeta;\n").expect("source");
    build(work.path(), &["a.c"]);
    let literal = run(work.path(), &["-dL", "-f", "i.db", "-4", "Alpha"]);
    assert_eq!(literal.stdout, b"a.c <unknown> 1 \tint Alpha;\r\n");
    // cscope matches the line with its carriage return.
    let anchored = run(work.path(), &["-dL", "-f", "i.db", "-6", "Alpha;$"]);
    assert_eq!(anchored.stdout, b"");
    // Other control bytes are escaped in the line protocol and kept exactly in JSON.
    let escaped = run(work.path(), &["-dL", "-f", "i.db", "-4", "Beta"]);
    assert_eq!(escaped.stdout, b"a.c <unknown> 2 int \\x0cBeta;\n");
    let json = run(work.path(), &["--json", "-dL", "-f", "i.db", "-4", "Alpha"]);
    let row: serde_json::Value = serde_json::from_slice(&json.stdout).expect("row");
    assert_eq!(row[3], "\tint Alpha;\r");
}

#[test]
fn editor_invocations_are_read_as_cscope_reads_them() {
    let work = tempfile::tempdir().expect("work");
    write(work.path().join("-odd.c"), "int Odd;\n").expect("source");
    write(work.path().join("a.c"), "int Alpha;\n").expect("source");
    // The owned adapters end their options with `--`.
    let built = run(work.path(), &["-bk", "-f", "i.db", "--", "-odd.c", "a.c"]);
    assert!(built.status.success(), "{built:?}");
    let odd = run(work.path(), &["-dL", "-f", "i.db", "-1", "Odd"]);
    assert_eq!(odd.stdout, b"-odd.c Odd 1 int Odd;\n");
    // Field 5 is a text search in line mode, as in cscope.
    assert_eq!(
        run(work.path(), &["-dL", "-f", "i.db", "-5", "Alpha"]).stdout,
        run(work.path(), &["-dL", "-f", "i.db", "-4", "Alpha"]).stdout
    );
    // xcscope's query shape: options, then `-f`, `-d`, `-L`, the field.
    let xcscope = run(
        work.path(),
        &["-k", "-q", "-f", "i.db", "-d", "-L", "-0", "Alpha"],
    );
    assert_eq!(xcscope.stdout, b"a.c <global> 1 int Alpha;\n");
    // Field 1 shows the pattern as each row's context, so the line protocol cannot carry
    // one with a blank; JSON can.
    let spaced = run(work.path(), &["-dL", "-f", "i.db", "-1", "(Alpha|a b)"]);
    assert!(spaced.status.success(), "{spaced:?}");
    assert_eq!(spaced.stdout, b"");
    assert!(String::from_utf8_lossy(&spaced.stderr).contains("--json"));
    let json = run(
        work.path(),
        &["--json", "-dL", "-f", "i.db", "-1", "(Alpha|a b)"],
    );
    let row: serde_json::Value = serde_json::from_slice(&json.stdout).expect("row");
    assert_eq!(
        row,
        serde_json::json!(["a.c", "(Alpha|a b)", 1, "int Alpha;"])
    );
    // `-d` never builds an index: without one there is nothing to answer from.
    let missing = run(work.path(), &["-dL", "-f", "missing.db", "-1", "Alpha"]);
    assert!(!missing.status.success(), "{missing:?}");
    assert!(!work.path().join("missing.db").exists());
    // Combinations no editor sends are refused before any index is touched.
    for args in [
        &["--json", "-bk", "-f", "other.db", "a.c"][..],
        &["--json", "--vim-errors", "-dl", "-f", "i.db"],
        &["--vim-errors", "-dL", "-f", "i.db", "-1", "Alpha"],
        &["-dL", "-f", "i.db", "-1", "Alpha", "--", "extra.c"],
    ] {
        let refused = run(work.path(), args);
        assert!(!refused.status.success(), "{args:?}");
        assert!(refused.stdout.is_empty(), "{args:?}");
    }
    assert!(!work.path().join("other.db").exists());
}

/// `--verify` brings the index up to date with every file read: while another process
/// updates it, it waits for that update instead of answering from the last revision.
#[test]
fn verify_waits_for_another_process_update_instead_of_skipping() {
    let work = tempfile::tempdir().expect("work");
    write(work.path().join("a.c"), "int Alpha;\n").expect("source");
    build(work.path(), &["a.c"]);
    write(work.path().join("a.c"), "int Beta;\n").expect("edit");
    let writer = rusqlite::Connection::open(work.path().join("i.db")).expect("writer");
    writer.execute_batch("BEGIN IMMEDIATE").expect("held");
    let verify = command(work.path(), &["--verify", "-f", "i.db"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .start()
        .expect("verify");
    std::thread::sleep(std::time::Duration::from_millis(500));
    writer.execute_batch("ROLLBACK").expect("released");
    let verified = verify.wait_with_output().expect("verified");
    assert!(verified.status.success(), "{verified:?}");
    assert!(verified.stderr.is_empty(), "{verified:?}");
    let query = run(work.path(), &["-dL", "-f", "i.db", "-1", "Beta"]);
    assert_eq!(query.stdout, b"a.c Beta 1 int Beta;\n", "{query:?}");
}

/// `--verify` of an index that cannot be written fails, where a query would answer from
/// it as it is: it must bring the index up to date.
#[test]
fn verify_of_an_index_that_cannot_be_written_fails() {
    let work = tempfile::tempdir().expect("work");
    write(work.path().join("a.c"), "int Alpha;\n").expect("source");
    build(work.path(), &["a.c"]);
    let index = work.path().join("i.db");
    let mut permissions = std::fs::metadata(&index).expect("index").permissions();
    permissions.set_readonly(true);
    std::fs::set_permissions(&index, permissions.clone()).expect("read-only");
    // Root may write any file on Unix: then there is nothing to check.
    let writable = std::fs::OpenOptions::new()
        .append(true)
        .open(&index)
        .is_ok();
    let verified = run(work.path(), &["--verify", "-f", "i.db"]);
    #[allow(clippy::permissions_set_readonly_false)]
    permissions.set_readonly(false);
    std::fs::set_permissions(&index, permissions).expect("writable");
    if writable {
        return;
    }
    assert_eq!(verified.status.code(), Some(1), "{verified:?}");
    assert!(verified.stdout.is_empty(), "{verified:?}");
    assert!(
        String::from_utf8_lossy(&verified.stderr).contains("cannot be written"),
        "{verified:?}"
    );
}

/// `INCLUDEDIRS` and `INCDIR` are read when an index is built, by `-b` and by
/// `--store-build` alike, and kept in its selection rule: a later update without them
/// still finds the headers there.
#[test]
fn the_include_environment_is_kept_in_the_selection_rule() {
    for build_option in ["-bk", "--store-build"] {
        let work = tempfile::tempdir().expect("work");
        for directory in ["listed", "system"] {
            std::fs::create_dir(work.path().join(directory)).expect("directory");
        }
        write(
            work.path().join("a.c"),
            "#include \"listed.h\"\n#include <system.h>\nint Alpha;\n",
        )
        .expect("source");
        write(work.path().join("listed/listed.h"), "int Listed;\n").expect("header");
        write(work.path().join("system/system.h"), "int System;\n").expect("header");
        let args: &[&str] = if build_option == "-bk" {
            &["-b", "-f", "i.db", "a.c"]
        } else {
            &["--store-build", "-f", "i.db", "a.c"]
        };
        let built = Command::new(env!("CARGO_BIN_EXE_ripscope"))
            .current_dir(work.path())
            .env("INCLUDEDIRS", "listed")
            .env("INCDIR", "system")
            .args(args)
            .captured()
            .expect("build");
        assert!(built.status.success(), "{build_option}: {built:?}");
        write(
            work.path().join("a.c"),
            "#include \"listed.h\"\n#include <system.h>\nint Beta;\n",
        )
        .expect("edit");
        for (name, file) in [("Listed", "listed/listed.h"), ("System", "system/system.h")] {
            let query = run(work.path(), &["-L", "-f", "i.db", "-1", name]);
            assert_eq!(
                String::from_utf8_lossy(&query.stdout),
                format!("{file} {name} 1 int {name};\n"),
                "{build_option}: {query:?}"
            );
        }
    }
}

/// Files whose stored analysis this ripscope cannot read are left out of the answers of
/// the fields that read analyses (0 to 3, 8 and 9), with one warning naming the way back,
/// while field 7 still lists them and a text search reads the files themselves, without a
/// word. A damaged analysis leaves its file out; an index
/// another version made, known by the analyzer it records, leaves every file out. A
/// session tells them apart the same way.
#[test]
fn files_whose_analysis_cannot_be_read_are_left_out_with_a_warning() {
    let work = tempfile::tempdir().expect("work");
    write(work.path().join("a.c"), "int Alpha;\n").expect("source");
    write(work.path().join("b.c"), "int Alpha2;\n").expect("source");
    build(work.path(), &["a.c", "b.c"]);
    let index = rusqlite::Connection::open(work.path().join("i.db")).expect("index");
    // a.c's unit image claims another image version, in an index this version made.
    let (block, mut image): (i64, Vec<u8>) = index
        .query_row(
            "SELECT blocks.id, blocks.image FROM blocks JOIN files ON files.unit = blocks.id >> 20
             WHERE files.path = CAST('a.c' AS BLOB)",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("unit");
    image[4] = 0xff;
    index
        .execute(
            "UPDATE blocks SET image = ?1 WHERE id = ?2",
            rusqlite::params![image, block],
        )
        .expect("image");
    let warning = "ripscope: damaged index: 1 indexed file cannot be read and is left out of \
                   the answers; rebuild it with -bu and the options it was built with\n";
    let stored = run(work.path(), &["-dL", "-f", "i.db", "-1", "Alpha.*"]);
    assert!(stored.status.success(), "{stored:?}");
    assert_eq!(stored.stdout, b"b.c Alpha.* 1 int Alpha2;\n", "{stored:?}");
    assert_eq!(String::from_utf8_lossy(&stored.stderr), warning);
    let files = "a.c <unknown> 1 <unknown>\nb.c <unknown> 1 <unknown>\n";
    let listed = run(work.path(), &["-dL", "-f", "i.db", "-7", ".*"]);
    assert!(listed.status.success(), "{listed:?}");
    assert_eq!(String::from_utf8_lossy(&listed.stdout), files);
    assert!(listed.stderr.is_empty(), "{listed:?}");
    let text = run(work.path(), &["-dL", "-f", "i.db", "-4", "Alpha"]);
    assert!(text.status.success(), "{text:?}");
    assert_eq!(
        String::from_utf8_lossy(&text.stdout),
        "a.c <unknown> 1 int Alpha;\nb.c <unknown> 1 int Alpha2;\n"
    );
    assert!(text.stderr.is_empty(), "{text:?}");
    let mut session = Session::start(work.path(), &["-dl", "-f", "i.db"]);
    session.expect(b">> ");
    session.send(b"7.*\n");
    session.expect(format!("cscope: 2 lines\n{files}>> ").as_bytes());
    session.send(b"4Alpha\n");
    session
        .expect(b"cscope: 2 lines\na.c <unknown> 1 int Alpha;\nb.c <unknown> 1 int Alpha2;\n>> ");
    session.send(b"1Alpha.*\n");
    session.expect_line(|line| line == warning);
    session.expect(b"cscope: 1 lines\nb.c Alpha.* 1 int Alpha2;\n>> ");
    session.send(b"q\n");
    session.finish();
    // The analyzer the index records names another unit image version.
    index
        .execute(
            "UPDATE meta SET value = CAST('ripscope-scanner-1/unit-99' AS BLOB)
             WHERE key = 'analyzer'",
            [],
        )
        .expect("analyzer");
    let other = "ripscope: 2 indexed files cannot be read by this version of ripscope and are \
                 left out of the answers; rebuild the index with -bu and the options it was \
                 built with\n";
    let stored = run(work.path(), &["-dL", "-f", "i.db", "-1", "Alpha.*"]);
    assert!(stored.status.success(), "{stored:?}");
    assert!(stored.stdout.is_empty(), "{stored:?}");
    assert_eq!(String::from_utf8_lossy(&stored.stderr), other);
    let listed = run(work.path(), &["-dL", "-f", "i.db", "-7", ".*"]);
    assert_eq!(String::from_utf8_lossy(&listed.stdout), files);
    assert!(listed.stderr.is_empty(), "{listed:?}");
}
