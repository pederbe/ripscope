use super::*;
use crate::scanner::Corrections;
use crate::store::Access;

fn write(root: &Path, name: &str, text: &str) {
    let path = root.join(name);
    std::fs::create_dir_all(path.parent().expect("parent")).expect("directory");
    std::fs::write(path, text).expect("source");
}

fn selection(root: &Path, sources: Sources, include_dirs: &[&str]) -> Selection {
    Selection {
        root: root.to_owned(),
        sources,
        include_dirs: include_dirs.iter().map(|d| d.as_bytes().to_vec()).collect(),
        environment_dirs: Vec::new(),
        system_includes: false,
        system_dir: b"/usr/include".to_vec(),
    }
}

fn run(store: &mut Store, rule: &Selection) -> Report {
    build(store, rule, options(Trust::Metadata)).expect("index built")
}

fn options(trust: Trust) -> Options {
    Options {
        unreadable: Unreadable::Keep,
        trust,
        wait: Duration::ZERO,
        analyzer: Analyzer::PRODUCT,
    }
}

fn paths(store: &Store) -> Vec<String> {
    store
        .snapshot()
        .expect("snapshot")
        .files()
        .iter()
        .map(|file| String::from_utf8_lossy(&file.path).into_owned())
        .collect()
}

/// Publishes real source files through an update whose write lock is already held.
fn publish_files(update: Update<'_>, rule: &Selection, names: &[&str]) -> Published {
    let mut update = update;
    let analyzer = Analyzer::PRODUCT;
    let files: Vec<StoredFile> = names
        .iter()
        .map(|name| {
            let path = rule.root.join(name);
            let bytes = std::fs::read(&path).expect("source");
            let hash: [u8; 32] = Sha256::digest(&bytes).into();
            let unit = analyzer.analyse(name.as_bytes(), &bytes).expect("analysis");
            let unit = update
                .add_unit(&analyzer.key(name.as_bytes(), &hash), &unit)
                .expect("stored analysis");
            StoredFile {
                path: name.as_bytes().to_vec(),
                unit,
                stat: stat_of(&path).expect("metadata").expect("source file").0,
                hash,
                verified: now(),
            }
        })
        .collect();
    update
        .finish(&rule.encode(), &analyzer.identity(), &files)
        .expect("published")
}

/// The publisher commits after Refresh reaches its write-lock attempt. A rule read
/// before that attempt must therefore see the old revision, exposing that ordering.
fn refresh_after_observation(
    store: &mut Store,
    invoked: &Selection,
    expected: Option<Selection>,
    publish: impl FnOnce(),
) -> Result<Report, RefreshError> {
    let (ready, readiness) = std::sync::mpsc::channel();
    let (release, released) = std::sync::mpsc::channel();
    std::thread::scope(|threads| {
        let waiting = threads.spawn(move || {
            assert_eq!(
                stored_rule(&store.snapshot().expect("snapshot")).unwrap(),
                expected
            );
            store::before_next_update(move || {
                ready.send(()).expect("refresh reached write-lock attempt");
                released
                    .recv_timeout(Duration::from_secs(5))
                    .expect("publication completed before write-lock attempt");
            });
            refresh(
                store,
                invoked,
                Options {
                    wait: Duration::from_secs(5),
                    ..options(Trust::Bytes)
                },
                &mut (),
            )
        });
        readiness
            .recv_timeout(Duration::from_secs(5))
            .expect("refresh reached write-lock attempt");
        publish();
        release.send(()).expect("write-lock attempt released");
        waiting.join().expect("refresh thread")
    })
}

#[test]
fn a_waiting_refresh_preserves_the_rule_of_the_first_published_revision() {
    let directory = tempfile::tempdir().expect("directory");
    let root = directory.path();
    write(root, "top.c", "int Top;\n");
    write(root, "sub/nested.c", "int Nested;\n");
    let index = root.join("index.db");
    let mut publisher = Store::open(&index, Access::Write).expect("publisher");
    let mut refreshing = Store::open(&index, Access::Update).expect("refreshing");
    let recursive = selection(root, Sources::Automatic { recursive: true }, &[]);
    let invoked = selection(root, Sources::Automatic { recursive: false }, &[]);
    let held = publisher
        .update(Duration::ZERO)
        .expect("publisher's write lock");
    let report = refresh_after_observation(&mut refreshing, &invoked, None, || {
        assert!(matches!(
            publish_files(held, &recursive, &["sub/nested.c", "top.c"]),
            Published::Revision(1)
        ));
    })
    .expect("refreshed after publication");
    assert_eq!(paths(&refreshing), ["sub/nested.c", "top.c"]);
    assert_eq!(
        stored_rule(&refreshing.snapshot().expect("snapshot")).unwrap(),
        Some(recursive)
    );
    assert_eq!(
        (report.revision, report.published, report.files),
        (1, false, 2)
    );
}

#[test]
fn a_waiting_refresh_preserves_a_rule_changed_by_another_index_build() {
    let directory = tempfile::tempdir().expect("directory");
    let root = directory.path();
    write(root, "a.c", "int Alpha;\n");
    write(root, "b.c", "int Beta;\n");
    write(root, "c.c", "int Gamma;\n");
    let index = root.join("index.db");
    let old_rule = selection(root, Sources::Files(vec![b"a.c".to_vec()]), &[]);
    let new_rule = selection(
        root,
        Sources::Files(vec![b"b.c".to_vec(), b"c.c".to_vec()]),
        &[],
    );
    let mut publisher = Store::open(&index, Access::Write).expect("publisher");
    run(&mut publisher, &old_rule);
    let mut refreshing = Store::open(&index, Access::Update).expect("refreshing");
    let held = publisher
        .update(Duration::ZERO)
        .expect("publisher's write lock");
    let report =
        refresh_after_observation(&mut refreshing, &old_rule, Some(old_rule.clone()), || {
            assert!(matches!(
                publish_files(held, &new_rule, &["b.c", "c.c"]),
                Published::Revision(2)
            ));
        })
        .expect("refreshed after publication");
    assert_eq!(paths(&refreshing), ["b.c", "c.c"]);
    assert_eq!(
        stored_rule(&refreshing.snapshot().expect("snapshot")).unwrap(),
        Some(new_rule)
    );
    assert_eq!(
        (report.revision, report.published, report.files),
        (2, false, 2)
    );
}

#[test]
fn a_refresh_uses_the_invocation_rule_if_nothing_is_published_at_its_write_lock() {
    let directory = tempfile::tempdir().expect("directory");
    let root = directory.path();
    write(root, "a.c", "int Alpha;\n");
    write(root, "b.c", "int Beta;\n");
    let index = root.join("index.db");
    let invoked = selection(root, Sources::Files(vec![b"b.c".to_vec()]), &[]);
    let mut publisher = Store::open(&index, Access::Write).expect("publisher");
    let mut refreshing = Store::open(&index, Access::Update).expect("refreshing");
    let held = publisher
        .update(Duration::ZERO)
        .expect("publisher's write lock");
    let report = refresh_after_observation(&mut refreshing, &invoked, None, || drop(held))
        .expect("first publication");
    assert_eq!(paths(&refreshing), ["b.c"]);
    assert_eq!(
        stored_rule(&refreshing.snapshot().expect("snapshot")).unwrap(),
        Some(invoked)
    );
    assert_eq!(
        (report.revision, report.published, report.files),
        (1, true, 1)
    );
}

#[test]
fn a_waiting_refresh_rejects_a_published_rule_that_cannot_be_decoded() {
    let directory = tempfile::tempdir().expect("directory");
    let root = directory.path();
    write(root, "a.c", "int Alpha;\n");
    let index = root.join("index.db");
    let invoked = selection(root, Sources::Files(vec![b"a.c".to_vec()]), &[]);
    let mut publisher = Store::open(&index, Access::Write).expect("publisher");
    run(&mut publisher, &invoked);
    let mut refreshing = Store::open(&index, Access::Update).expect("refreshing");
    let held = publisher
        .update(Duration::ZERO)
        .expect("publisher's write lock");
    let files = held.files().to_vec();
    let failed =
        refresh_after_observation(&mut refreshing, &invoked, Some(invoked.clone()), || {
            held.finish(
                b"not a selection rule",
                &Analyzer::PRODUCT.identity(),
                &files,
            )
            .expect("unreadable rule published");
        });
    assert!(matches!(failed, Err(RefreshError::Selection(_))));
    assert_eq!(publisher.revision_id().expect("revision").number, 2);
    assert_eq!(paths(&refreshing), ["a.c"]);
    assert!(stored_rule(&refreshing.snapshot().expect("snapshot")).is_err());
}

#[test]
fn files_follow_the_reference_database_order() {
    let directory = tempfile::tempdir().expect("directory");
    let root = directory.path();
    write(root, "src/b.c", "#include \"local.h\"\nint b;\n");
    write(
        root,
        "src/a.c",
        "#include <sys.h>\n#include \"missing.h\"\nint a;\n",
    );
    write(root, "local.h", "#include <deep.h>\nint local;\n");
    write(root, "inc/sys.h", "int sys;\n");
    write(root, "inc/deep.h", "int deep;\n");
    write(root, "other/deep.h", "int other;\n");
    let mut store = Store::open(&root.join("index.db"), Access::Write).expect("index");
    let rule = selection(
        root,
        Sources::Files(vec![b"./src/b.c".to_vec(), b"src//a.c".to_vec()]),
        &["inc", "other"],
    );
    let report = run(&mut store, &rule);
    assert_eq!((report.revision, report.analysed), (1, 5));
    // Selected files sorted, then each include level sorted; the first directory that
    // has an included file wins.
    assert_eq!(
        paths(&store),
        ["src/a.c", "src/b.c", "inc/sys.h", "local.h", "inc/deep.h"]
    );
    let snapshot = store.snapshot().expect("snapshot");
    assert_eq!(Selection::decode(snapshot.selection()), Some(rule));
}

#[test]
fn a_refresh_analyses_only_what_changed_and_drops_deleted_files() {
    let directory = tempfile::tempdir().expect("directory");
    let root = directory.path();
    write(root, "a.c", "int a;\n");
    write(root, "b.c", "int b;\n");
    let mut store = Store::open(&root.join("index.db"), Access::Write).expect("index");
    let rule = selection(
        root,
        Sources::Files(vec![b"a.c".to_vec(), b"b.c".to_vec()]),
        &[],
    );
    // Files changed an hour ago are trusted by their metadata after one hashing.
    let hour_ago = std::time::SystemTime::now() - Duration::from_secs(3600);
    for name in ["a.c", "b.c"] {
        std::fs::File::options()
            .write(true)
            .open(root.join(name))
            .and_then(|file| file.set_modified(hour_ago))
            .expect("older mtime");
    }
    run(&mut store, &rule);
    let again = run(&mut store, &rule);
    assert_eq!((again.analysed, again.published), (0, false));
    write(root, "b.c", "int b2;\n");
    std::fs::remove_file(root.join("a.c")).expect("removed");
    let changed = run(&mut store, &rule);
    assert_eq!(changed.analysed, 1);
    assert!(changed.unreadable.is_empty());
    assert_eq!(paths(&store), ["b.c"]);
    let snapshot = store.snapshot().expect("snapshot");
    let b = snapshot.unit(snapshot.files()[0].unit).expect("unit");
    let names: Vec<Vec<u8>> = b
        .lines()
        .flat_map(|line| {
            line.names()
                .map(|name| name.text.to_vec())
                .collect::<Vec<_>>()
        })
        .collect();
    assert_eq!(names, [b"b2".to_vec()]);
}

#[test]
fn metadata_is_trusted_only_for_files_hashed_well_after_their_last_change() {
    let directory = tempfile::tempdir().expect("directory");
    let root = directory.path();
    write(root, "a.c", "int a;\n");
    // A change time an hour ahead keeps every update inside the racy window, however
    // slowly the test runs.
    let ahead = std::time::SystemTime::now() + Duration::from_secs(60 * 60);
    std::fs::File::options()
        .write(true)
        .open(root.join("a.c"))
        .and_then(|file| file.set_modified(ahead))
        .expect("later mtime");
    let mut store = Store::open(&root.join("index.db"), Access::Write).expect("index");
    let rule = selection(root, Sources::Files(vec![b"a.c".to_vec()]), &[]);
    run(&mut store, &rule);
    let verified = store.snapshot().expect("snapshot").files()[0].verified;
    // Just written: the bytes are hashed again, and the verification moves on.
    run(&mut store, &rule);
    let later = store.snapshot().expect("snapshot").files()[0].verified;
    assert!(later > verified);
    // An edit of the same size within the same timestamp is still caught.
    let modified = std::fs::metadata(root.join("a.c"))
        .and_then(|m| m.modified())
        .expect("mtime");
    write(root, "a.c", "int b;\n");
    std::fs::File::options()
        .write(true)
        .open(root.join("a.c"))
        .and_then(|file| file.set_modified(modified))
        .expect("same mtime");
    assert_eq!(run(&mut store, &rule).analysed, 1);
}

/// An update that finds nothing changed writes nothing to the index.
#[test]
fn an_unchanged_update_writes_nothing() {
    let directory = tempfile::tempdir().expect("directory");
    let root = directory.path();
    write(root, "a.c", "int a;\n");
    std::fs::File::options()
        .write(true)
        .open(root.join("a.c"))
        .and_then(|file| {
            file.set_modified(std::time::SystemTime::now() - Duration::from_secs(3600))
        })
        .expect("mtime");
    let mut store = Store::open(&root.join("index.db"), Access::Write).expect("index");
    let rule = selection(root, Sources::Files(vec![b"a.c".to_vec()]), &[]);
    run(&mut store, &rule);
    run(&mut store, &rule);
    let files = store.snapshot().expect("snapshot").files().to_vec();
    let log = || std::fs::metadata(root.join("index.db-wal")).map_or(0, |log| log.len());
    let written = log();
    assert_eq!(run(&mut store, &rule).analysed, 0);
    assert_eq!(store.snapshot().expect("snapshot").files(), files);
    assert_eq!(log(), written);
}

#[test]
fn a_file_re_created_with_the_same_size_and_times_is_analysed_again() {
    let directory = tempfile::tempdir().expect("directory");
    let root = directory.path();
    let hour_ago = std::time::SystemTime::now() - Duration::from_secs(3600);
    let write_old = |text: &str| {
        write(root, "a.c", text);
        std::fs::File::options()
            .write(true)
            .open(root.join("a.c"))
            .and_then(|file| file.set_modified(hour_ago))
            .expect("mtime");
    };
    write_old("int a;\n");
    let mut store = Store::open(&root.join("index.db"), Access::Write).expect("index");
    let rule = selection(root, Sources::Files(vec![b"a.c".to_vec()]), &[]);
    run(&mut store, &rule);
    assert_eq!(run(&mut store, &rule).analysed, 0);
    #[cfg(windows)]
    let before = store.snapshot().expect("snapshot").files()[0].stat.clone();
    #[cfg(windows)]
    let created = std::fs::metadata(root.join("a.c"))
        .and_then(|metadata| metadata.created())
        .expect("creation time");
    // Deleted and written again at once, with the same size and modification time, as
    // an archive extraction does.
    std::fs::remove_file(root.join("a.c")).expect("removed");
    write_old("int b;\n");
    #[cfg(windows)]
    {
        use std::os::windows::fs::FileTimesExt;
        // As NTFS usually does for a name deleted moments before: only the identity tells
        // the files apart. (On Linux the inode number is often reused; ctime tells.)
        std::fs::File::options()
            .write(true)
            .open(root.join("a.c"))
            .and_then(|file| file.set_times(std::fs::FileTimes::new().set_created(created)))
            .expect("creation time");
        let after = stat_of(&root.join("a.c")).expect("stat").expect("a file").0;
        assert_ne!(before.identity, after.identity);
        assert_eq!(
            (before.size, before.modified, before.changed),
            (after.size, after.modified, after.changed)
        );
    }
    assert_eq!(run(&mut store, &rule).analysed, 1);
}

#[cfg(windows)]
#[test]
fn a_file_another_process_holds_without_sharing_is_still_seen() {
    use std::os::windows::fs::OpenOptionsExt;
    let directory = tempfile::tempdir().expect("directory");
    let root = directory.path();
    write(root, "a.c", "int a;\n");
    std::fs::File::options()
        .write(true)
        .open(root.join("a.c"))
        .and_then(|file| {
            file.set_modified(std::time::SystemTime::now() - Duration::from_secs(3600))
        })
        .expect("mtime");
    let mut store = Store::open(&root.join("index.db"), Access::Write).expect("index");
    let rule = selection(root, Sources::Files(vec![b"a.c".to_vec()]), &[]);
    let selected = options(Trust::Metadata);
    build(&mut store, &rule, selected).unwrap();
    let original_marks = first_marks(&store);
    let held = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(root.join("a.c"))
        .expect("held");
    let report = build(&mut store, &rule, selected).unwrap();
    // Its metadata is read through a handle with no access rights, and is unchanged.
    assert_eq!((report.analysed, report.unreadable.len()), (0, 0));
    assert_eq!(paths(&store), ["a.c"]);
    assert_eq!(first_marks(&store), original_marks);
    drop(held);
    assert_eq!(
        refresh(&mut store, &rule, selected, &mut ())
            .unwrap()
            .analysed,
        0
    );
}

#[test]
fn trusting_bytes_reads_every_file_and_analyses_what_changed() {
    let directory = tempfile::tempdir().expect("directory");
    let root = directory.path();
    let hour_ago = std::time::SystemTime::now() - Duration::from_secs(3600);
    let write_old = |name: &str, text: &str| {
        write(root, name, text);
        std::fs::File::options()
            .write(true)
            .open(root.join(name))
            .and_then(|file| file.set_modified(hour_ago))
            .expect("mtime");
    };
    write_old("a.c", "int a;\n");
    write_old("b.c", "int b;\n");
    let mut store = Store::open(&root.join("index.db"), Access::Write).expect("index");
    let rule = selection(
        root,
        Sources::Files(vec![b"a.c".to_vec(), b"b.c".to_vec()]),
        &[],
    );
    run(&mut store, &rule);
    // Rewritten in place with the same size and modification time: a stat may miss it,
    // a rehash does not.
    write_old("a.c", "int c;\n");
    let report = refresh(&mut store, &rule, options(Trust::Bytes), &mut ()).expect("refreshed");
    assert_eq!(report.analysed, 1);
    // Trusting nothing analyses every file again and publishes, even when nothing changed.
    let report = refresh(&mut store, &rule, options(Trust::Nothing), &mut ()).expect("refreshed");
    assert_eq!((report.analysed, report.published), (2, true));
}

#[test]
fn a_different_analyzer_analyses_everything_again() {
    let directory = tempfile::tempdir().expect("directory");
    let root = directory.path();
    write(root, "a.c", "int a;\n");
    let mut store = Store::open(&root.join("index.db"), Access::Write).expect("index");
    let rule = selection(root, Sources::Files(vec![b"a.c".to_vec()]), &[]);
    run(&mut store, &rule);
    let update = store.update(Duration::ZERO).expect("update");
    let files = update.files().to_vec();
    assert_eq!(update.analyzer(), Analyzer::PRODUCT.identity());
    update
        .finish(&rule.encode(), b"an older analyzer", &files)
        .expect("published");
    assert_eq!(run(&mut store, &rule).analysed, 1);
    assert_eq!(run(&mut store, &rule).analysed, 0);
}

/// The first file's names as `mark` + `name`.
fn first_marks(store: &Store) -> Vec<String> {
    let snapshot = store.snapshot().expect("snapshot");
    let unit = snapshot.unit(snapshot.files()[0].unit).expect("unit");
    unit.lines()
        .flat_map(|line| {
            line.names()
                .map(|name| {
                    format!(
                        "{}{}",
                        char::from(name.mark.byte()),
                        String::from_utf8_lossy(name.text)
                    )
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

#[test]
fn the_correction_set_decides_the_reading_and_the_identity() {
    let directory = tempfile::tempdir().expect("directory");
    let root = directory.path();
    // A designated entry: the reference reads `b` as a global definition.
    write(root, "a.c", "struct s x = {\n  .a = 1,\n  .b = 2,\n};\n");
    let mut store = Store::open(&root.join("index.db"), Access::Write).expect("index");
    let rule = selection(root, Sources::Files(vec![b"a.c".to_vec()]), &[]);
    let with = |corrections| Options {
        analyzer: Analyzer::with(corrections),
        ..options(Trust::Metadata)
    };
    let reference =
        refresh(&mut store, &rule, with(Corrections::default()), &mut ()).expect("refreshed");
    assert_eq!(reference.analysed, 1);
    assert!(first_marks(&store).contains(&"gb".to_owned()));
    let enabled =
        refresh(&mut store, &rule, with(Corrections::ENABLED), &mut ()).expect("refreshed");
    assert_eq!(enabled.analysed, 1, "a new Correction set analyses again");
    assert!(!first_marks(&store).contains(&"gb".to_owned()));
    assert_eq!(
        refresh(&mut store, &rule, with(Corrections::ENABLED), &mut ())
            .expect("refreshed")
            .analysed,
        0
    );
}

#[test]
fn directories_are_searched_by_cscope_suffix() {
    let directory = tempfile::tempdir().expect("directory");
    let root = directory.path();
    for name in [
        "a.c",
        "b.h",
        "c.y",
        "d.txt",
        "s.old.c",
        "sub/e.c",
        "sub/deeper/f.l",
    ] {
        write(root, name, "int x;\n");
    }
    let mut flat = Store::open(&root.join("flat.db"), Access::Write).expect("index");
    run(
        &mut flat,
        &selection(root, Sources::Automatic { recursive: false }, &[]),
    );
    assert_eq!(paths(&flat), ["a.c", "b.h", "c.y"]);
    let mut deep = Store::open(&root.join("deep.db"), Access::Write).expect("index");
    run(
        &mut deep,
        &selection(root, Sources::Automatic { recursive: true }, &[]),
    );
    assert_eq!(
        paths(&deep),
        ["a.c", "b.h", "c.y", "sub/deeper/f.l", "sub/e.c"]
    );
}

#[test]
fn a_list_file_is_read_again_at_each_refresh() {
    let directory = tempfile::tempdir().expect("directory");
    let root = directory.path();
    write(root, "a.c", "int a;\n");
    write(root, "b.c", "int b;\n");
    write(root, "files.txt", "-k\na.c\n");
    let mut store = Store::open(&root.join("index.db"), Access::Write).expect("index");
    let rule = selection(root, Sources::List(b"files.txt".to_vec()), &[]);
    run(&mut store, &rule);
    assert_eq!(paths(&store), ["a.c"]);
    write(root, "files.txt", "a.c\n\"b.c\"\n");
    run(&mut store, &rule);
    assert_eq!(paths(&store), ["a.c", "b.c"]);
}

#[test]
fn an_include_matches_names_exactly_even_where_case_does_not_matter() {
    let directory = tempfile::tempdir().expect("directory");
    let root = directory.path();
    write(root, "src/vxworks.h", "#include <vxWorks.h>\nint v;\n");
    let mut store = Store::open(&root.join("index.db"), Access::Write).expect("index");
    let rule = selection(
        root,
        Sources::Files(vec![b"src/vxworks.h".to_vec()]),
        &["src"],
    );
    run(&mut store, &rule);
    assert_eq!(paths(&store), ["src/vxworks.h"]);
}

#[test]
fn list_files_hold_names_quoted_names_and_cscope_options() {
    let contents =
        parse_list(b"-k\n-q a.c \"b c.h\"\n-I inc -Iinc2 \"d\\\"e.c\"\n\n").expect("list");
    assert_eq!(
        contents,
        ListContents {
            names: vec![b"a.c".to_vec(), b"b c.h".to_vec(), b"d\"e.c".to_vec()],
            include_dirs: vec![b"inc".to_vec(), b"inc2".to_vec()],
            kernel: true,
        }
    );
    assert!(parse_list(b"a.c -I\n").is_err());
}

#[test]
fn list_files_may_hold_the_options_that_change_no_row() {
    // `-c` and `-p N` concern cscope's own database and full-screen display; `-p` takes
    // its number from the same entry or the next one, and stands alone without one.
    let contents = parse_list(b"-c\n-p 3 a.c\n-p2 b.c -p \"4\" c.c\n").expect("list");
    assert_eq!(
        contents,
        ListContents {
            names: vec![b"a.c".to_vec(), b"b.c".to_vec(), b"c.c".to_vec()],
            ..ListContents::default()
        }
    );
    let alone = parse_list(b"a.c -p\n-p -I inc b.c\n-p d.c\n").expect("list");
    assert_eq!(
        alone,
        ListContents {
            names: vec![b"a.c".to_vec(), b"b.c".to_vec(), b"d.c".to_vec()],
            include_dirs: vec![b"inc".to_vec()],
            kernel: false,
        }
    );
    assert_eq!(
        parse_list(b"a.c\n-T\n"),
        Err("line 2: option -T is not supported".into())
    );
}

#[test]
fn a_list_file_can_add_include_directories_and_turn_off_the_system_one() {
    let directory = tempfile::tempdir().expect("directory");
    let root = directory.path();
    write(root, "a.c", "#include <x.h>\n#include <y.h>\nint a;\n");
    write(root, "listed/x.h", "int x;\n");
    write(root, "system/y.h", "int y;\n");
    write(root, "files.txt", "-I listed\na.c\n");
    let mut rule = selection(root, Sources::List(b"files.txt".to_vec()), &[]);
    rule.system_includes = true;
    rule.system_dir = b"system".to_vec();
    let mut store = Store::open(&root.join("index.db"), Access::Write).expect("index");
    run(&mut store, &rule);
    assert_eq!(paths(&store), ["a.c", "listed/x.h", "system/y.h"]);
    write(root, "files.txt", "-k -I listed\na.c\n");
    run(&mut store, &rule);
    assert_eq!(paths(&store), ["a.c", "listed/x.h"]);
}

#[test]
fn automatic_discovery_reads_cscope_files_when_it_is_there() {
    let directory = tempfile::tempdir().expect("directory");
    let root = directory.path();
    let sources = Sources::Automatic { recursive: true };
    assert!(matches!(
        sources.chosen(root),
        Chosen::Directory { recursive: true }
    ));
    write(root, "cscope.files", "a.c\n");
    assert!(matches!(
        sources.chosen(root),
        Chosen::List(b"cscope.files")
    ));
    assert_eq!(
        Sources::named(Some(b"l.txt".to_vec()), vec![b"a.c".to_vec()], false),
        Sources::List(b"l.txt".to_vec())
    );
}

#[test]
fn names_repeat_as_cscope_repeats_them() {
    let directory = tempfile::tempdir().expect("directory");
    let root = directory.path();
    write(root, "a.c", "int a;\n");
    write(root, "b.c", "int b;\n");
    write(root, "list.txt", "b.c\na.c\nb.c\n");
    std::fs::create_dir(root.join("dir.c")).expect("a directory named like a source");
    let mut store = Store::open(&root.join("index.db"), Access::Write).expect("index");
    // A list's repeated name is indexed twice.
    run(
        &mut store,
        &selection(root, Sources::List(b"list.txt".to_vec()), &[]),
    );
    assert_eq!(paths(&store), ["a.c", "b.c", "b.c"]);
    // On the command line a found name counts once; a missing name, or one that is not a
    // regular file, is reported each time it is given, in the order given and in canonical
    // form, as cscope's `compath` leaves it.
    let names = [
        "a.c",
        "./gone.c",
        "a.c",
        "gone.c",
        "dir.c",
        "sub/../other.c",
    ];
    let rule = selection(
        root,
        Sources::Files(names.iter().map(|n| n.as_bytes().to_vec()).collect()),
        &[],
    );
    let report = run(&mut store, &rule);
    assert_eq!(paths(&store), ["a.c"]);
    assert_eq!(
        report.missing,
        ["gone.c", "gone.c", "dir.c", "other.c"].map(|n| n.as_bytes().to_vec())
    );
    // Nothing found: the update fails, reporting every missing name, and publishes nothing.
    let rule = selection(root, Sources::Files(vec![b"gone.c".to_vec()]), &[]);
    let failed = build(&mut store, &rule, options(Trust::Metadata));
    assert!(
        matches!(failed, Err(RefreshError::NoSources(ref report)) if report.missing == [b"gone.c".to_vec()])
    );
    assert_eq!(paths(&store), ["a.c"]);
}

#[test]
fn a_new_index_is_made_only_when_the_rule_finds_a_file() {
    let directory = tempfile::tempdir().expect("directory");
    let root = directory.path();
    let named = |names: &[&str]| {
        selection(
            root,
            Sources::Files(names.iter().map(|n| n.as_bytes().to_vec()).collect()),
            &[],
        )
    };
    // Nothing in the directory, and nothing named that exists.
    let empty = selection(root, Sources::Automatic { recursive: true }, &[]);
    assert!(matches!(
        check_sources(&empty),
        Err(RefreshError::NoSources(ref report)) if report.missing.is_empty()
    ));
    std::fs::create_dir(root.join("dir.c")).expect("a directory named like a source");
    assert!(matches!(
        check_sources(&named(&["./gone.c", "dir.c"])),
        Err(RefreshError::NoSources(ref report))
            if report.missing == [b"gone.c".to_vec(), b"dir.c".to_vec()]
    ));
    // One file is enough, and a list is read as the refresh reads it.
    write(root, "a.c", "int a;\n");
    check_sources(&named(&["gone.c", "a.c"])).expect("a.c");
    check_sources(&empty).expect("discovered");
    write(root, "list.txt", "gone.c\n");
    let listed = selection(root, Sources::List(b"list.txt".to_vec()), &[]);
    assert!(matches!(
        check_sources(&listed),
        Err(RefreshError::NoSources(_))
    ));
    let unlisted = selection(root, Sources::List(b"missing.txt".to_vec()), &[]);
    assert!(matches!(
        check_sources(&unlisted),
        Err(RefreshError::Selection(_))
    ));
}

#[cfg(unix)]
#[test]
fn a_fifo_is_not_a_source_and_never_waited_on() {
    use std::os::unix::fs::PermissionsExt;
    let directory = tempfile::tempdir().expect("directory");
    let root = directory.path();
    let made = std::process::Command::new("mkfifo")
        .arg(root.join("pipe.c"))
        .status()
        .expect("mkfifo");
    assert!(made.success());
    let named = selection(root, Sources::Files(vec![b"pipe.c".to_vec()]), &[]);
    assert!(matches!(
        check_sources(&named),
        Err(RefreshError::NoSources(ref report)) if report.missing == [b"pipe.c".to_vec()]
    ));
    // cscope scans the directory when cscope.files cannot be read.
    write(root, "a.c", "int a;\n");
    write(root, "cscope.files", "gone.c\n");
    std::fs::set_permissions(
        root.join("cscope.files"),
        std::fs::Permissions::from_mode(0o0),
    )
    .expect("unreadable");
    if std::fs::File::open(root.join("cscope.files")).is_err() {
        let automatic = Sources::Automatic { recursive: false };
        assert!(matches!(
            automatic.chosen(root),
            Chosen::Directory { recursive: false }
        ));
    }
}

#[test]
fn a_directory_holds_only_the_relative_paths_below_it() {
    assert!(within(b"a.c", b"", false));
    assert!(within(b"src/a.c", b"", true));
    assert!(!within(b"src/a.c", b"", false));
    assert!(within(b"src/a.c", b"src", false));
    assert!(within(b"src/sub/a.c", b"src", true));
    assert!(!within(b"src/sub/a.c", b"src", false));
    assert!(!within(b"srcx/a.c", b"src", true));
    for outside in [&b"/usr/include/stdio.h"[..], b"../inc/x.h", b".."] {
        assert!(!within(outside, b"", true));
    }
    #[cfg(windows)]
    {
        assert!(!within(b"C:x.c", b"", true));
        assert!(!within(b"C:/src/x.c", b"", true));
    }
}

#[test]
fn paths_are_kept_in_one_spelling() {
    assert_eq!(canonical_path(b"./src//a.c"), b"src/a.c");
    #[cfg(windows)]
    for (given, kept) in [
        (&b"src\\..\\lib\\a.c"[..], &b"lib/a.c"[..]),
        (b"C:\\work\\a.c", b"C:/work/a.c"),
        (b"c:\\work\\a.c", b"C:/work/a.c"),
        (b"\\rooted\\a.c", b"/rooted/a.c"),
        // `..` never climbs above the drive or the share.
        (b"C:\\Users\\me\\..\\..\\..\\a.c", b"C:/a.c"),
        (b"C:\\..", b"C:/"),
        (b"C:dir\\..\\a.c", b"C:a.c"),
        (b"\\\\server\\share\\dir\\..\\a.c", b"//server/share/a.c"),
        (b"//server/share/../a.c", b"//server/share/a.c"),
        (b"\\\\server\\share", b"//server/share"),
        // A verbatim or device path keeps its spelling.
        (b"\\\\?\\C:\\a.c", b"\\\\?\\C:\\a.c"),
        (b"\\\\.\\pipe\\x", b"\\\\.\\pipe\\x"),
    ] {
        assert_eq!(
            canonical_path(given),
            kept,
            "{}",
            String::from_utf8_lossy(given)
        );
    }
    #[cfg(not(windows))]
    assert_eq!(canonical_path(b"a\\b.c"), b"a\\b.c");
}

/// A file's reading depends on its own bytes alone: an edited source or header, a new
/// `#include` line or a newly listed file analyses only the files that changed or are new.
#[test]
fn an_edit_or_a_new_file_analyses_only_what_changed() {
    let work = tempfile::tempdir().unwrap();
    let root = work.path();
    write(
        root,
        "root.c",
        "#include \"macros.h\"\nAPI(int) wrapped(void) { return 1; }\n",
    );
    write(root, "neighbor.c", "int neighbor;\n");
    write(root, "macros.h", "#define API(type) type\n");
    write(root, "files.txt", "root.c\nneighbor.c\n");
    let rule = selection(root, Sources::List(b"files.txt".to_vec()), &[]);
    let mut store = Store::open(&root.join("i.db"), Access::Write).unwrap();
    assert_eq!(run(&mut store, &rule).analysed, 3);
    write(root, "neighbor.c", "int changed;\n");
    assert_eq!(run(&mut store, &rule).analysed, 1);
    write(root, "macros.h", "#define API(type) API(type)\n");
    assert_eq!(run(&mut store, &rule).analysed, 1);
    // A new include line, here of a header that does not exist.
    write(root, "neighbor.c", "#include \"missing.h\"\nint changed;\n");
    assert_eq!(run(&mut store, &rule).analysed, 1);
    write(root, "added.c", "int added;\n");
    write(root, "files.txt", "root.c\nneighbor.c\nadded.c\n");
    let report = run(&mut store, &rule);
    assert_eq!((report.files, report.analysed), (4, 1));
    assert_eq!(run(&mut store, &rule).analysed, 0);
}

/// Observe actual SQLite insertions, including anything Publication later deletes.
fn inserted_keys(index: &Path) -> Vec<Vec<u8>> {
    let connection = rusqlite::Connection::open(index).unwrap();
    let keys = connection
        .prepare("SELECT key FROM insertions ORDER BY rowid")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    connection.execute("DELETE FROM insertions", []).unwrap();
    keys
}

/// A name a list gives twice is indexed twice, as in cscope, and its reading is analysed
/// for each but stored once.
#[test]
fn repeated_list_names_keep_their_counts_and_store_each_reading_once() {
    let work = tempfile::tempdir().unwrap();
    let root = work.path();
    write(
        root,
        "root.c",
        "#include \"api.h\"\nAPI(int) wrapped(void) { return called(); }\n",
    );
    write(root, "api.h", "#define API(type) type\n");
    write(root, "neighbor.c", "int neighbor;\n");
    write(
        root,
        "files.txt",
        "root.c\nmissing.c\nneighbor.c\nroot.c\nother.c\nmissing.c\n",
    );
    let rule = selection(root, Sources::List(b"files.txt".to_vec()), &[]);
    let index = root.join("i.db");
    let mut store = Store::open(&index, Access::Write).unwrap();
    rusqlite::Connection::open(&index)
        .unwrap()
        .execute_batch(
            "CREATE TABLE insertions (key BLOB);
             CREATE TRIGGER inserted AFTER INSERT ON units BEGIN
                 INSERT INTO insertions VALUES (new.key);
             END;",
        )
        .unwrap();
    for (trust, count) in [
        (Trust::Metadata, 4),
        (Trust::Metadata, 1),
        (Trust::Nothing, 4),
    ] {
        let report = refresh(&mut store, &rule, options(trust), &mut ()).unwrap();
        assert_eq!(report.analysed, count);
        assert_eq!(paths(&store), ["neighbor.c", "root.c", "root.c", "api.h"]);
        assert_eq!(
            report.missing,
            [
                b"missing.c".to_vec(),
                b"other.c".to_vec(),
                b"missing.c".to_vec()
            ]
        );
        let keys = inserted_keys(&index);
        let connection = rusqlite::Connection::open(&index).unwrap();
        let final_keys: Vec<Vec<u8>> = connection
            .prepare("SELECT key FROM units")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert!(
            keys.iter().all(|key| final_keys.contains(key)),
            "every stored reading is used"
        );
        assert_eq!(keys.len(), if count == 4 { 3 } else { 1 });
        write(root, "neighbor.c", "int changed;\n");
    }
}

/// A Refresh that finds the selection, every file and every include's resolution as the
/// last clean Refresh left them stops early. A header that now shadows another, an
/// edited source or a header that is gone is still read as a full Refresh reads it.
#[test]
fn an_unchanged_refresh_stops_early_and_any_change_is_read_in_full() {
    let work = tempfile::tempdir().unwrap();
    let root = work.path();
    let age = |name: &str| {
        std::fs::File::options()
            .write(true)
            .open(root.join(name))
            .and_then(|file| {
                file.set_modified(std::time::SystemTime::now() - Duration::from_secs(3600))
            })
            .expect("mtime");
    };
    write(root, "a.c", "#include \"x.h\"\nint a;\n");
    write(root, "two/x.h", "int two;\n");
    std::fs::create_dir(root.join("one")).unwrap();
    for name in ["a.c", "two/x.h"] {
        age(name);
    }
    let rule = selection(root, Sources::Files(vec![b"a.c".to_vec()]), &["one", "two"]);
    let mut store = Store::open(&root.join("i.db"), Access::Write).unwrap();
    let stopped = || STOPPED_EARLY.with(std::cell::Cell::get);
    assert!(run(&mut store, &rule).published);
    assert_eq!(paths(&store), ["a.c", "two/x.h"]);
    let before = stopped();
    let report = run(&mut store, &rule);
    assert!(!report.published);
    assert_eq!((report.files, stopped()), (2, before + 1));
    // A header earlier in the search path now holds the include.
    write(root, "one/x.h", "int one;\n");
    age("one/x.h");
    assert!(run(&mut store, &rule).published);
    assert_eq!(paths(&store), ["a.c", "one/x.h"]);
    assert_eq!(stopped(), before + 1);
    assert!(!run(&mut store, &rule).published);
    assert_eq!(stopped(), before + 2);
    // An edited source.
    write(root, "a.c", "#include \"x.h\"\nint a, b;\n");
    age("a.c");
    assert_eq!(run(&mut store, &rule).analysed, 1);
    // A header that is gone.
    std::fs::remove_file(root.join("one/x.h")).unwrap();
    assert!(run(&mut store, &rule).published);
    assert_eq!(paths(&store), ["a.c", "two/x.h"]);
    assert_eq!(stopped(), before + 2);
}
