use super::*;
use crate::scanner::{self, Mode};
use std::path::Path;

fn unit_of(source: &[u8]) -> AnalysisUnit {
    AnalysisUnit::from_records(&scanner::scan(source, Mode::C)).expect("unit")
}

fn stat(size: u64) -> FileStat {
    FileStat {
        size,
        modified: 1,
        changed: 2,
        identity: vec![3],
    }
}

fn file(path: &str, unit: i64) -> StoredFile {
    StoredFile {
        path: path.as_bytes().to_vec(),
        unit,
        stat: stat(10),
        hash: [7; 32],
        verified: 5,
    }
}

/// The problems `--verify` finds in the stored analyses and their postings.
fn checked(store: &Store) -> Vec<String> {
    let snapshot = store.snapshot().expect("snapshot");
    snapshot.check_analyses().expect("checked")
}

/// Damages the blocks of the unit stored under `key`.
fn damage(store: &Store, key: u8) {
    store
        .connection
        .execute(
            "UPDATE blocks SET image = x'00' WHERE id >> 20 = (SELECT id FROM units WHERE key = ?1)",
            [&[key; 32][..]],
        )
        .expect("damaged");
}

/// Publishes `files`, each given with its unit's key and source.
fn publish(store: &mut Store, files: &[(&str, u8, &[u8])]) -> Published {
    let mut update = store.update(Duration::ZERO).expect("update");
    let rows: Vec<StoredFile> = files
        .iter()
        .map(|(path, key, source)| {
            let unit = update
                .add_unit(&[*key; 32], &unit_of(source))
                .expect("unit");
            file(path, unit)
        })
        .collect();
    update
        .finish(b"rule", b"analyzer", &rows)
        .expect("published")
}

#[test]
fn an_absent_path_or_an_empty_file_becomes_an_empty_index() {
    let directory = tempfile::tempdir().expect("directory");
    let absent = directory.path().join("absent.db");
    let empty = directory.path().join("empty.db");
    std::fs::write(&empty, b"").expect("empty file");
    for path in [&absent, &empty] {
        let store = Store::open(path, Access::Write).expect("created");
        let snapshot = store.snapshot().expect("snapshot");
        assert_eq!(snapshot.revision(), 0);
        assert!(snapshot.files().is_empty());
        drop(snapshot);
        drop(store);
        Store::open(path, Access::Read).expect("reopened");
    }
}

/// One look at the path tells an index build or an update whether opening it makes a new
/// index, and opens what that look found.
#[test]
fn one_look_tells_whether_an_open_makes_a_new_index_and_then_opens_it() {
    let directory = tempfile::tempdir().expect("directory");
    let path = directory.path().join("i.db");
    let absent = Store::inspect(&path).expect("inspected");
    assert!(absent.makes_new(Access::Write));
    assert!(absent.makes_new(Access::Update));
    assert!(!absent.makes_new(Access::Read));
    drop(absent.open(Access::Update).expect("created"));
    let index = Store::inspect(&path).expect("inspected");
    for access in [Access::Read, Access::Write, Access::Update] {
        assert!(!index.makes_new(access));
    }
    let store = index.open(Access::Read).expect("opened");
    assert_eq!(store.snapshot().expect("snapshot").revision(), 0);
}

/// Each index gets an identity of its own when it is created, which its revisions carry,
/// so an index made afresh at the same path is told from the one it replaced; an index
/// made before identities were written has none.
#[test]
fn each_new_index_has_an_identity_of_its_own() {
    let directory = tempfile::tempdir().expect("directory");
    let path = directory.path().join("index.db");
    let mut store = Store::open(&path, Access::Write).expect("created");
    let created = store.revision_id().expect("revision");
    assert_eq!((created.identity.len(), created.number), (16, 0));
    publish(&mut store, &[("a.c", 1, b"int a;\n")]);
    let published = RevisionId {
        identity: created.identity.clone(),
        number: 1,
    };
    assert_eq!(store.revision_id().expect("revision"), published);
    assert_eq!(store.reader().expect("reader").revision_id(), &published);
    drop(store);
    remove_database(&path);
    let afresh = Store::open(&path, Access::Write).expect("made afresh");
    let fresh = afresh.revision_id().expect("revision");
    assert_eq!((fresh.identity.len(), fresh.number), (16, 0));
    assert_ne!(fresh.identity, created.identity);
    afresh
        .connection
        .execute("DELETE FROM meta WHERE key = 'identity'", [])
        .expect("an index made before identities");
    assert_eq!(
        afresh.revision_id().expect("revision"),
        RevisionId::default()
    );
    assert_eq!(
        afresh.reader().expect("reader").revision_id(),
        &RevisionId::default()
    );
}

#[test]
fn a_published_revision_remains_readable_without_an_index_identity() {
    let directory = tempfile::tempdir().expect("directory");
    let mut store =
        Store::open(&directory.path().join("index.db"), Access::Write).expect("created");
    publish(&mut store, &[("a.c", 1, b"int a;\n")]);
    store
        .connection
        .execute("DELETE FROM meta WHERE key = 'identity'", [])
        .expect("an index made before identities");
    let reader = store.reader().expect("reader");
    assert_eq!(
        reader.revision_id(),
        &RevisionId {
            identity: Vec::new(),
            number: 1,
        }
    );
    assert_eq!(reader.selection(), b"rule");
    assert_eq!(reader.analyzer(), b"analyzer");
    assert!(reader.paths().expect("paths").iter().eq([&b"a.c"[..]]));
}

#[test]
fn a_query_reader_ends_its_transaction_when_dropped() {
    let directory = tempfile::tempdir().expect("directory");
    let mut store =
        Store::open(&directory.path().join("index.db"), Access::Write).expect("created");
    publish(&mut store, &[("a.c", 1, b"int a;\n")]);
    // As a Session's requests do, with the transaction's statements cached after the first.
    for _ in 0..2 {
        let reader = store.reader().expect("reader");
        assert!(!store.connection.is_autocommit());
        drop(reader);
        assert!(store.connection.is_autocommit());
    }
    publish(&mut store, &[("a.c", 2, b"int b;\n")]);
    assert_eq!(store.reader().expect("reader").revision_id().number, 2);
}

#[test]
fn query_readers_require_revision_selection_and_analyzer_metadata() {
    // A first read compiles its statements for one use, and later ones reuse them.
    for (key, read_before) in ["revision", "selection", "analyzer"]
        .into_iter()
        .flat_map(|key| [(key, false), (key, true)])
    {
        let directory = tempfile::tempdir().expect("directory");
        let mut store =
            Store::open(&directory.path().join("index.db"), Access::Write).expect("created");
        publish(&mut store, &[("a.c", 1, b"int a;\n")]);
        if read_before {
            drop(store.reader().expect("reader"));
        }
        store
            .connection
            .execute("DELETE FROM meta WHERE key = ?1", [key])
            .expect("removed required metadata");
        assert!(
            matches!(
                store.reader(),
                Err(StoreError::Sqlite(rusqlite::Error::QueryReturnedNoRows))
            ),
            "{key} {read_before}"
        );
        assert!(store.connection.is_autocommit(), "{key} {read_before}");
    }
}

#[test]
fn query_readers_keep_metadata_fallbacks_for_unreadable_values() {
    for (revision, read_before) in ["-1", "'bad'", "1.5", "x'00'"]
        .into_iter()
        .flat_map(|revision| [(revision, false), (revision, true)])
    {
        let directory = tempfile::tempdir().expect("directory");
        let mut store =
            Store::open(&directory.path().join("index.db"), Access::Write).expect("created");
        publish(&mut store, &[("a.c", 1, b"int a;\n")]);
        if read_before {
            drop(store.reader().expect("reader"));
        }
        store
            .connection
            .execute_batch(&format!(
                "UPDATE meta SET value = {revision} WHERE key = 'revision';
                 UPDATE meta SET value = 17 WHERE key = 'identity';
                 UPDATE meta SET value = 'bad' WHERE key = 'selection';
                 UPDATE meta SET value = 1.5 WHERE key = 'analyzer';"
            ))
            .expect("unreadable metadata");
        let reader = store.reader().expect("reader");
        assert_eq!(
            reader.revision_id(),
            &RevisionId::default(),
            "{revision} {read_before}"
        );
        assert!(reader.selection().is_empty(), "{revision} {read_before}");
        assert!(reader.analyzer().is_empty(), "{revision} {read_before}");
    }
}

#[test]
fn a_new_index_never_replaces_one_made_meanwhile() {
    let directory = tempfile::tempdir().expect("directory");
    let path = directory.path().join("index.db");
    let mut first = Store::open(&path, Access::Write).expect("first");
    publish(&mut first, &[("a.c", 1, b"int a;\n")]);
    // As when another index build made it between this one's look and its own creation.
    let index = IndexPath::new(&path).expect("path");
    let second = create(&index, Placing::New).expect("the first one's index");
    let second = Store::with(second);
    assert_eq!(second.snapshot().expect("snapshot").revision(), 1);
    let left: Vec<_> = std::fs::read_dir(directory.path())
        .expect("listing")
        .map(|entry| entry.expect("entry").file_name())
        .filter(|name| name.to_string_lossy().contains(".new-"))
        .collect();
    assert!(left.is_empty(), "{left:?}");
}

#[test]
fn anything_else_is_refused_and_left_unchanged() {
    let directory = tempfile::tempdir().expect("directory");
    let cscope = directory.path().join("cscope.out");
    let contents = b"cscope 15 /src              0000000123\n\t@a.c\n\n1 int x;\n".to_vec();
    std::fs::write(&cscope, &contents).expect("cscope database");
    let other = directory.path().join("other.db");
    Connection::open(&other)
        .and_then(|db| db.execute_batch("CREATE TABLE t (x); PRAGMA application_id = 7;"))
        .expect("other database");
    let other_bytes = std::fs::read(&other).expect("other bytes");
    for (path, bytes) in [(&cscope, &contents), (&other, &other_bytes)] {
        for access in [Access::Read, Access::Write] {
            assert!(matches!(
                Store::open(path, access),
                Err(StoreError::NotAnIndex(_))
            ));
        }
        assert_eq!(&std::fs::read(path).expect("still there"), bytes);
    }
    assert!(matches!(
        Store::open(&directory.path().join("missing.db"), Access::Read),
        Err(StoreError::Missing(_))
    ));
}

#[test]
fn another_schema_version_is_refused_for_reading_and_made_afresh_for_writing() {
    let directory = tempfile::tempdir().expect("directory");
    let path = directory.path().join("index.db");
    let mut store = Store::open(&path, Access::Write).expect("created");
    publish(&mut store, &[("a.c", 1, b"int a;\n")]);
    store
        .connection
        .execute_batch("PRAGMA user_version = 99; PRAGMA wal_checkpoint(TRUNCATE);")
        .expect("older schema");
    drop(store);
    // An update needs the selection rule the index holds, so only an index build makes it
    // afresh.
    for access in [Access::Read, Access::Update] {
        assert!(matches!(
            Store::open(&path, access),
            Err(StoreError::Version(99))
        ));
    }
    let store = Store::open(&path, Access::Write).expect("made afresh");
    assert!(store.snapshot().expect("snapshot").files().is_empty());
    drop(store);
    let reopened = Store::open(&path, Access::Update).expect("the new schema");
    assert_eq!(reopened.snapshot().expect("snapshot").revision(), 0);
}

/// An index of `SCHEMA_VERSION` 99 at `path`, which an index build replaces.
fn another_version(path: &Path) {
    let mut store = Store::open(path, Access::Write).expect("created");
    publish(&mut store, &[("a.c", 1, b"int a;\n")]);
    store
        .connection
        .execute_batch("PRAGMA user_version = 99; PRAGMA wal_checkpoint(TRUNCATE);")
        .expect("older schema");
}

/// A process that may not write an index never replaces it, though it may write the
/// directory: the index's owner would be locked out.
#[test]
fn only_a_process_that_may_write_an_index_replaces_it() {
    let directory = tempfile::tempdir().expect("directory");
    let path = directory.path().join("index.db");
    another_version(&path);
    let bytes = std::fs::read(&path).expect("index");
    let writable = |yes: bool| {
        let mut permissions = std::fs::metadata(&path).expect("index").permissions();
        permissions.set_readonly(!yes);
        std::fs::set_permissions(&path, permissions).expect("permissions");
    };
    writable(false);
    // Root may write any file on Unix: then there is nothing to check.
    if may_write(&path) {
        writable(true);
        return;
    }
    assert!(matches!(
        Store::open(&path, Access::Write),
        Err(StoreError::ReadOnly(_))
    ));
    assert!(matches!(
        Store::replace(&path),
        Err(StoreError::ReadOnly(_))
    ));
    writable(true);
    assert_eq!(std::fs::read(&path).expect("kept"), bytes);
    let names: Vec<_> = std::fs::read_dir(directory.path())
        .expect("listing")
        .map(|entry| entry.expect("entry").file_name())
        .collect();
    assert_eq!(names, ["index.db"]);
}

/// An index that replaces another keeps its permission bits, whatever the umask, and its
/// group, so rebuilding an index shared with a group keeps it shared.
#[cfg(unix)]
#[test]
fn a_replaced_index_keeps_its_permissions() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let directory = tempfile::tempdir().expect("directory");
    let path = directory.path().join("index.db");
    let mode = |set: Option<u32>| {
        if let Some(mode) = set {
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).expect("mode");
        }
        std::fs::metadata(&path)
            .expect("index")
            .permissions()
            .mode()
            & 0o777
    };
    another_version(&path);
    mode(Some(0o640));
    drop(Store::open(&path, Access::Write).expect("made afresh"));
    assert_eq!(mode(None), 0o640);
    mode(Some(0o660));
    drop(Store::replace(&path).expect("replaced"));
    assert_eq!(mode(None), 0o660);
    // The group, where this user belongs to another one to give the old index to.
    let group = |path: &Path| std::fs::metadata(path).expect("index").gid();
    let groups = std::process::Command::new("id")
        .arg("-G")
        .output()
        .expect("id");
    let other = String::from_utf8_lossy(&groups.stdout)
        .split_whitespace()
        .filter_map(|gid| gid.parse::<u32>().ok())
        .find(|gid| *gid != group(&path));
    if let Some(other) = other {
        std::os::unix::fs::chown(&path, None, Some(other)).expect("group");
        drop(Store::replace(&path).expect("replaced"));
        assert_eq!(group(&path), other);
        assert_eq!(mode(None), 0o660);
    }
}

/// The logs of the index being replaced go with it: a log left beside the new index would
/// be replayed into it. On Windows they cannot be removed while another process holds
/// them, so the replacement waits for it.
#[test]
fn a_replaced_index_takes_in_no_log_of_the_old_one() {
    let directory = tempfile::tempdir().expect("directory");
    let path = directory.path().join("index.db");
    let log = beside(&path, "-wal");
    another_version(&path);
    // A log with frames in it, as a writer that crashed leaves one. Here the writer's own
    // close takes them in (the old index becomes version 98), and the log is then written
    // back from the bytes it held.
    let frames = {
        let writer = Connection::open(&path).expect("writer");
        writer
            .execute_batch(
                "PRAGMA wal_autocheckpoint = 0; PRAGMA user_version = 98;
                 INSERT INTO meta VALUES ('stale', 1);",
            )
            .expect("frames");
        std::fs::read(&log).expect("log")
    };
    std::fs::write(&log, &frames).expect("stale log");
    #[cfg(windows)]
    let holder = {
        use std::os::windows::fs::OpenOptionsExt;
        // Read and write sharing without delete sharing, as SQLite opens files.
        const SHARE_READ_WRITE: u32 = 0x1 | 0x2;
        let held: Vec<std::fs::File> = [&path, &log]
            .into_iter()
            .map(|name| {
                std::fs::OpenOptions::new()
                    .read(true)
                    .share_mode(SHARE_READ_WRITE)
                    .open(name)
                    .expect("held")
            })
            .collect();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            drop(held);
        })
    };
    let store = Store::open(&path, Access::Write).expect("made afresh");
    #[cfg(windows)]
    holder.join().expect("holder");
    let stale: i64 = store
        .connection
        .query_row("SELECT count(*) FROM meta WHERE key = 'stale'", [], |row| {
            row.get(0)
        })
        .expect("meta");
    assert_eq!(stale, 0);
    drop(store);
    let reopened = Store::open(&path, Access::Update).expect("the new schema");
    assert_eq!(reopened.snapshot().expect("snapshot").revision(), 0);
}

/// A log file the replacement cannot remove stops it, and the old index stays as it was:
/// SQLite would take the log for the new index's.
#[test]
fn a_log_that_cannot_be_removed_stops_a_replacement() {
    let directory = tempfile::tempdir().expect("directory");
    let path = directory.path().join("index.db");
    another_version(&path);
    let bytes = std::fs::read(&path).expect("index");
    let log = beside(&path, "-wal");
    std::fs::create_dir(&log).expect("a directory where the log goes");
    #[cfg(windows)]
    let log = std::fs::canonicalize(log).expect("log path");
    match Store::open(&path, Access::Write) {
        Err(StoreError::InUse(blocked)) => assert_eq!(blocked, log),
        other => panic!("{:?}", other.map(|_| ())),
    }
    assert_eq!(std::fs::read(&path).expect("kept"), bytes);
    std::fs::remove_dir(&log).expect("log");
    let names: Vec<_> = std::fs::read_dir(directory.path())
        .expect("listing")
        .map(|entry| entry.expect("entry").file_name())
        .collect();
    assert_eq!(names, ["index.db"]);
}

/// A reader of the index being replaced delays the replacement, which on Windows waits
/// for it to close the index.
#[test]
fn a_replacement_waits_for_a_reader_of_the_old_index() {
    let directory = tempfile::tempdir().expect("directory");
    let path = directory.path().join("index.db");
    another_version(&path);
    let reader = Connection::open(&path).expect("reader");
    let version: i32 = reader
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("read");
    assert_eq!(version, 99);
    let holder = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        drop(reader);
    });
    let store = Store::open(&path, Access::Write).expect("made afresh");
    holder.join().expect("holder");
    assert_eq!(store.snapshot().expect("snapshot").revision(), 0);
}

#[test]
fn an_update_replaces_the_files_and_shares_units_by_key() {
    let directory = tempfile::tempdir().expect("directory");
    let path = directory.path().join("index.db");
    let mut store = Store::open(&path, Access::Write).expect("created");
    let published = publish(
        &mut store,
        &[
            ("b.c", 1, b"#include \"x.h\"\nint shared;\n"),
            ("a.c", 1, b"#include \"x.h\"\nint shared;\n"),
            ("c.c", 2, b"#include <sys/y.h>\nvoid f(void) { g(); }\n"),
        ],
    );
    assert_eq!(published, Published::Revision(1));
    let snapshot = store.snapshot().expect("snapshot");
    assert_eq!(snapshot.revision(), 1);
    assert_eq!(snapshot.selection(), b"rule");
    let paths: Vec<&[u8]> = snapshot.files().iter().map(|f| &f.path[..]).collect();
    assert_eq!(paths, [&b"b.c"[..], b"a.c", b"c.c"]);
    let files = snapshot.files().to_vec();
    assert_eq!(files[0].unit, files[1].unit);
    assert_ne!(files[0].unit, files[2].unit);
    assert_eq!(files[0].stat, stat(10));
    assert_eq!(
        snapshot.unit(files[2].unit).expect("unit"),
        unit_of(b"#include <sys/y.h>\nvoid f(void) { g(); }\n")
    );
    drop(snapshot);

    // The next revision keeps one unit, whose include operands stay readable without
    // its image, and drops the other.
    let update = store.update(Duration::ZERO).expect("update");
    assert_eq!(
        update.includes(files[2].unit).expect("includes"),
        [(b"sys/y.h".to_vec(), false)]
    );
    assert_eq!(
        update.includes(files[0].unit).expect("includes"),
        [(b"x.h".to_vec(), true)]
    );
    let published = update
        .finish(b"rule", b"analyzer", &[files[2].clone()])
        .expect("published");
    assert_eq!(published, Published::Revision(2));
    let count = |table: &str| -> i64 {
        store
            .connection
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .expect("count")
    };
    assert_eq!((count("units"), count("includes")), (1, 1));
    assert!(store.verify().expect("verified").is_empty());
}

/// A malformed Include operand is refused, whatever else matches.
#[test]
fn includer_lookup_refuses_non_blob_operands() {
    for (operand, kind) in [
        ("NULL", "Null"),
        ("'target.h'", "Text"),
        ("17", "Integer"),
        ("1.5", "Real"),
    ] {
        let directory = tempfile::tempdir().expect("directory");
        let mut store =
            Store::open(&directory.path().join("index.db"), Access::Write).expect("created");
        publish(&mut store, &[("a.c", 1, b"#include <target.h>\n")]);
        // Remove the STRICT constraint to inject values an intact table cannot hold.
        store
            .connection
            .execute_batch(&format!(
                "PRAGMA foreign_keys = OFF;
                 CREATE TABLE unreadable_operands AS SELECT * FROM operands;
                 DROP TABLE operands;
                 ALTER TABLE unreadable_operands RENAME TO operands;
                 INSERT INTO operands VALUES (99, {operand});"
            ))
            .expect("unreadable operand");
        let reader = store.reader().expect("reader");
        let error = reader
            .including(|operand| operand == b"target.h")
            .expect_err("a non-BLOB operand is refused");
        assert_eq!(
            error.to_string(),
            format!("index database: Invalid column type {kind} at index: 1, name: operand"),
            "{operand}"
        );
    }
}

/// The name postings: each unit's are stored with it, found by name and role, deleted
/// with the unit when no file uses it any more (from its image, or by search for one that
/// no longer decodes), and dropped by a discarding update; `--verify` finds any that do not
/// match the units.
#[test]
fn name_postings_follow_their_units() {
    let directory = tempfile::tempdir().expect("directory");
    let path = directory.path().join("index.db");
    let mut store = Store::open(&path, Access::Write).expect("created");
    publish(
        &mut store,
        &[
            ("a.c", 1, b"int shared;\nvoid f(void) { g(); }\n"),
            ("b.c", 2, b"int b(void) { return shared; }\n"),
            ("c.c", 3, b"int c;\n"),
        ],
    );
    let postings = |store: &Store| -> Vec<(Vec<u8>, i64, u32)> {
        let rows: Vec<(Vec<u8>, Vec<u8>)> = store
            .connection
            .prepare("SELECT name, postings FROM names ORDER BY name")
            .expect("statement")
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .expect("rows")
            .collect::<Result<_, _>>()
            .expect("postings");
        rows.into_iter()
            .flat_map(|(name, list)| {
                postings::decode(&list)
                    .expect("list")
                    .into_iter()
                    .map(|posting| (name.clone(), posting.unit, posting.roles))
                    .collect::<Vec<_>>()
            })
            .collect()
    };
    let count = |store: &Store| postings(store).len();
    assert_eq!(count(&store), 6);
    let holding = |store: &Store, name: &[u8], mask: u32| -> Vec<Vec<u8>> {
        let reader = store.reader().expect("reader");
        let (files, _) = reader.holding(name, mask).expect("holding");
        files.into_iter().map(|(path, _)| path).collect()
    };
    let global = crate::unit::Mark::Global.role(false);
    let plain = crate::unit::Mark::Plain.role(false);
    assert_eq!(holding(&store, b"shared", global), [b"a.c".to_vec()]);
    assert_eq!(
        holding(&store, b"shared", global | plain),
        [b"a.c".to_vec(), b"b.c".to_vec()]
    );
    assert!(holding(&store, b"g", global).is_empty());
    assert_eq!(checked(&store), Vec::<String>::new());

    // b.c's unit goes with its postings; c.c's no longer decodes, and its postings go
    // too.
    damage(&store, 3);
    let files = store.snapshot().expect("snapshot").files().to_vec();
    let update = store.update(Duration::ZERO).expect("update");
    update
        .finish(b"rule", b"analyzer", &files[..1])
        .expect("published");
    assert_eq!(count(&store), 3);
    assert_eq!(checked(&store), Vec::<String>::new());

    // A missing or a stray posting is found.
    let unmatched = ["the name postings do not match the stored analyses"];
    store
        .connection
        .execute("DELETE FROM names WHERE name = CAST('g' AS BLOB)", [])
        .expect("deleted");
    assert_eq!(checked(&store), unmatched);
    // Unit 99, with roles 1, in block 0.
    store
        .connection
        .execute(
            "INSERT INTO names VALUES (CAST('g' AS BLOB), x'63010100')",
            [],
        )
        .expect("inserted");
    assert_eq!(checked(&store), unmatched);

    let mut update = store.update(Duration::ZERO).expect("update");
    update.discard().expect("discarded");
    update.finish(b"rule", b"analyzer", &[]).expect("published");
    assert_eq!(count(&store), 0);
}

/// Postings stored in many small batches, an index build's first without reading and each
/// later one merged into the lists, and an update that adds and drops units across
/// batches, end as the postings of the units, and as one batch stores them: with a block
/// for each unit, and with a block for each line.
#[test]
fn postings_stored_in_batches_are_those_of_the_units() {
    for block_size in [BLOCK_SIZE, 1] {
        postings_in_batches(block_size);
    }
}

fn postings_in_batches(block_size: usize) {
    let directory = tempfile::tempdir().expect("directory");
    let sources: Vec<(String, u8, Vec<u8>)> = (0..12u8)
        .map(|n| {
            let source = format!(
                "int shared{};\nint f{n}(void) {{ return shared{} + common; }}\n",
                n % 3,
                (n + 1) % 3
            );
            (format!("f{n}.c"), n + 1, source.into_bytes())
        })
        .collect();
    let rows = |store: &Store| -> Vec<(Vec<u8>, Vec<u8>)> {
        store
            .connection
            .prepare("SELECT name, postings FROM names ORDER BY name")
            .expect("statement")
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .expect("rows")
            .collect::<Result<_, _>>()
            .expect("postings")
    };
    let build = |path: &Path, batch: usize, files: &[(String, u8, Vec<u8>)]| {
        let mut store = Store::open(path, Access::Write).expect("store");
        set_block_size(&store, block_size);
        let mut update = store.update(Duration::ZERO).expect("update");
        update.batch(batch);
        let rows: Vec<StoredFile> = files
            .iter()
            .map(|(path, key, source)| {
                let unit = update
                    .add_unit(&[*key; 32], &unit_of(source))
                    .expect("unit");
                file(path, unit)
            })
            .collect();
        update
            .finish(b"rule", b"analyzer", &rows)
            .expect("published");
        store
    };
    let small = build(&directory.path().join("small.db"), 3, &sources);
    let whole = build(&directory.path().join("whole.db"), usize::MAX, &sources);
    assert_eq!(rows(&small), rows(&whole));
    assert_eq!(checked(&small), Vec::<String>::new());
    // An update in small batches that keeps some files, drops others and adds new ones.
    let mut store = small;
    let kept = store.snapshot().expect("snapshot").files()[..6].to_vec();
    let mut update = store.update(Duration::ZERO).expect("update");
    update.batch(2);
    let mut files = kept;
    for n in 20..26u8 {
        let source = format!("int g{n};\nint common;\n");
        let unit = update
            .add_unit(&[n; 32], &unit_of(source.as_bytes()))
            .expect("unit");
        files.push(file(&format!("g{n}.c"), unit));
    }

    update
        .finish(b"rule", b"analyzer", &files)
        .expect("published");
    assert_eq!(checked(&store), Vec::<String>::new());
    let reader = store.reader().expect("reader");
    let (holding, listed) = reader.holding(b"common", u32::MAX).expect("holding");
    assert_eq!(holding.len(), 12);
    // `common` is on the second line of each file: in its second block when each line has
    // one.
    let second = u32::from(block_size == 1);
    for (_, unit) in &holding {
        assert_eq!(listed[unit], [second]);
    }
    drop(reader);
    // A unit added and dropped in one update leaves no posting, also while its postings
    // are still collected.
    let files = store.snapshot().expect("snapshot").files().to_vec();
    let mut update = store.update(Duration::ZERO).expect("update");
    update
        .add_unit(&[30; 32], &unit_of(b"int common;\nint unused;\n"))
        .expect("unit");
    let extra = unit_of(b"int extra;\n");
    let unit = update.add_unit(&[31; 32], &extra).expect("unit");
    let mut files = files;
    files.push(file("extra.c", unit));
    update
        .finish(b"rule", b"analyzer", &files)
        .expect("published");
    assert_eq!(checked(&store), Vec::<String>::new());
    // Each unit's blocks go with it.
    let blocks: i64 = store
        .connection
        .query_row(
            "SELECT count(*) - (SELECT sum(blocks) FROM units) FROM blocks",
            [],
            |row| row.get(0),
        )
        .expect("blocks");
    assert_eq!(blocks, 0);
}

/// Makes the index at `store`, which holds no unit yet, store blocks of `size` bytes.
fn set_block_size(store: &Store, size: usize) {
    store
        .connection
        .execute(
            "UPDATE meta SET value = ?1 WHERE key = 'block size'",
            [size as i64],
        )
        .expect("block size");
}

/// A macro over two lines, a function with a body over several, and a global after it.
const SCOPES: &[u8] = b"#define WRAP(x) \\\n\tcall(x)\nint f(void)\n{\n\tg();\n\th();\n}\nint k;\n";

/// Publishes `a.c`, holding [`SCOPES`], in an index of a block for each line; gives the
/// directory it lives in, the store and the unit's id.
fn scopes_in_blocks() -> (tempfile::TempDir, Store, i64) {
    let directory = tempfile::tempdir().expect("directory");
    let mut store =
        Store::open(&directory.path().join("index.db"), Access::Write).expect("created");
    set_block_size(&store, 1);
    publish(&mut store, &[("a.c", 1, SCOPES)]);
    let id = store.snapshot().expect("snapshot").files()[0].unit;
    (directory, store, id)
}

/// A unit is stored as blocks of its lines, each with the scope it starts in, read back
/// whole or as the blocks a list names; a name's postings list the blocks it occurs in and
/// those that start inside its body.
#[test]
fn units_are_stored_in_blocks_with_their_scopes() {
    let (_directory, store, id) = scopes_in_blocks();
    let unit = unit_of(SCOPES);
    let split = unit.split(1);
    let reader = store.reader().expect("reader");
    let blocks = reader.blocks(id, None, None).expect("blocks");
    let read: Vec<_> = blocks
        .iter()
        .map(|block| (block.number, &block.lines, block.start()))
        .collect();
    let made: Vec<_> = (0..)
        .zip(&split.blocks)
        .map(|(number, (lines, start))| (number, lines, *start))
        .collect();
    assert_eq!(read, made);
    assert_eq!(blocks.len(), unit.lines().count());
    let start = |name: &[u8]| {
        let holds = |block: &&Block| {
            block
                .lines
                .lines()
                .any(|line| line.names().any(|n| n.text == name))
        };
        blocks.iter().find(holds).expect("a block").start()
    };
    assert_eq!(start(b"call").macro_name, Some(&b"WRAP"[..]));
    assert_eq!(start(b"g").function, Some(&b"f"[..]));
    assert_eq!(start(b"k"), Scope::default());
    // Listed blocks come in the order asked; one beyond the unit's is damage.
    let listed = reader.blocks(id, Some(&[3, 1]), None).expect("listed");
    let numbers: Vec<u32> = listed.iter().map(|block| block.number).collect();
    assert_eq!(numbers, [3, 1]);
    assert_eq!(listed[0].lines, split.blocks[3].0);
    assert!(matches!(
        reader.blocks(id, Some(&[99]), None),
        Err(StoreError::Unit(UnitError::Damaged))
    ));
    // So is one beyond the unit's blocks that the index still holds, which also makes the
    // whole unit damaged.
    store
        .connection
        .execute("UPDATE units SET blocks = 3", [])
        .expect("fewer blocks");
    for which in [Some(&[3][..]), None] {
        assert!(matches!(
            reader.blocks(id, which, None),
            Err(StoreError::Unit(UnitError::Damaged))
        ));
    }
    assert_eq!(
        reader.blocks(id, Some(&[2]), None).expect("block 2").len(),
        1
    );
    store
        .connection
        .execute("UPDATE units SET blocks = ?1", [split.blocks.len() as i64])
        .expect("its blocks");
    let (files, listed) = reader.holding(b"f", u32::MAX).expect("holding");
    assert_eq!(files, [(b"a.c".to_vec(), id)]);
    let (_, _, made, _) = split
        .postings
        .iter()
        .find(|(name, _, _, _)| *name == b"f")
        .expect("f");
    assert_eq!(&listed[&id], made);
    assert!(made.len() > 2, "{made:?}");
    drop(reader);
    assert_eq!(
        store.snapshot().expect("snapshot").unit(id).expect("unit"),
        unit
    );
    // At the default size the unit is one block.
    let directory = tempfile::tempdir().expect("directory");
    let mut whole =
        Store::open(&directory.path().join("index.db"), Access::Write).expect("created");
    publish(&mut whole, &[("a.c", 1, SCOPES)]);
    let id = whole.snapshot().expect("snapshot").files()[0].unit;
    let blocks = whole
        .reader()
        .expect("reader")
        .blocks(id, None, None)
        .expect("blocks");
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].lines, unit);
}

/// `--verify` finds a path list that is not the indexed files' paths, and a query cannot
/// read one that does not decode.
#[test]
fn verify_finds_a_path_list_that_is_not_the_files() {
    let unmatched = "the path list does not match the indexed files";
    for damage in [
        "UPDATE meta SET value = x'03622e63' WHERE key = 'paths'",
        "UPDATE meta SET value = x'03612e6303612e63' WHERE key = 'paths'",
        "UPDATE meta SET value = x'' WHERE key = 'paths'",
        "UPDATE meta SET value = x'09612e63' WHERE key = 'paths'",
        "UPDATE meta SET value = 'a.c' WHERE key = 'paths'",
        "UPDATE files SET path = CAST('b.c' AS BLOB)",
    ] {
        let directory = tempfile::tempdir().expect("directory");
        let mut store =
            Store::open(&directory.path().join("index.db"), Access::Write).expect("created");
        publish(&mut store, &[("a.c", 1, b"int a;\n")]);
        assert_eq!(checked(&store), Vec::<String>::new());
        store.connection.execute_batch(damage).expect("damaged");
        assert_eq!(checked(&store), [unmatched], "{damage}");
    }
    let directory = tempfile::tempdir().expect("directory");
    let mut store =
        Store::open(&directory.path().join("index.db"), Access::Write).expect("created");
    publish(&mut store, &[("a.c", 1, b"int a;\n")]);
    store
        .connection
        .execute_batch("UPDATE meta SET value = x'09612e63' WHERE key = 'paths'")
        .expect("damaged");
    let error = store
        .reader()
        .expect("reader")
        .paths()
        .expect_err("refused");
    assert!(matches!(error, StoreError::Paths), "{error}");
}

/// `--verify` finds include records that are not their unit's includes: a place, an
/// operand, its delimiter or a position changed, one missing and one added.
#[test]
fn verify_finds_include_records_that_are_not_their_units() {
    let unmatched = "the include records do not match the stored analyses";
    let source = b"#include \"a.h\"\nint x;\n#include <b.h>\n";
    for damage in [
        "UPDATE includes SET record = record + 1 WHERE position = 1",
        "UPDATE includes SET line = line + 1 WHERE position = 0",
        "UPDATE includes SET name = name + 1",
        "UPDATE includes SET block = 0",
        "UPDATE operands SET operand = CAST('c.h' AS BLOB) WHERE operand = CAST('a.h' AS BLOB)",
        "UPDATE includes SET operand = (SELECT max(id) FROM operands) WHERE position = 0",
        "UPDATE includes SET quoted = 1 - quoted",
        "UPDATE includes SET position = position + 5",
        "DELETE FROM includes WHERE position = 1",
        "INSERT INTO includes SELECT unit, 9, operand, quoted, block, line, record, name, \
         after, text FROM includes WHERE position = 0",
        "UPDATE includes SET after = after + 1 WHERE position = 0",
        "UPDATE includes SET text = CAST('#include \"c.h\"' AS BLOB) WHERE position = 0",
    ] {
        let directory = tempfile::tempdir().expect("directory");
        let mut store =
            Store::open(&directory.path().join("index.db"), Access::Write).expect("created");
        set_block_size(&store, 1);
        publish(&mut store, &[("a.c", 1, source)]);
        assert_eq!(checked(&store), Vec::<String>::new());
        store.connection.execute_batch(damage).expect("damaged");
        assert_eq!(checked(&store), [unmatched], "{damage}");
    }
}

/// `--verify` finds blocks that are not what splitting their units' lines gives: one
/// missing, one of no unit, a count or a start scope changed, lines split by another
/// size, and a block list in the postings that disagrees; and a block size it cannot read.
#[test]
fn verify_finds_blocks_that_are_not_their_units_split() {
    let damaged = "a.c: damaged analysis: analysis unit image is damaged";
    let strays = "the index holds blocks of no analysis";
    let unreadable = "the index's block size cannot be read";
    let unmatched = "the name postings do not match the stored analyses";
    let first = "(SELECT id FROM units) << 20";
    let cases: [(String, &[&str]); 9] = [
        (
            format!("DELETE FROM blocks WHERE id = {first} | 2"),
            &[damaged],
        ),
        ("UPDATE units SET blocks = blocks + 1".into(), &[damaged]),
        // The unit's last block is then no unit's, and the unit damaged.
        (
            "UPDATE units SET blocks = blocks - 1".into(),
            &[strays, damaged],
        ),
        (
            format!(
                "INSERT INTO blocks SELECT id + (1 << 30), macro, function, image FROM blocks \
                 WHERE id = {first}"
            ),
            &[strays],
        ),
        (
            "UPDATE blocks SET function = CAST('other' AS BLOB) WHERE id = (SELECT max(id) \
             FROM blocks)"
                .into(),
            &[damaged],
        ),
        ("UPDATE blocks SET macro = NULL".into(), &[damaged]),
        (
            "UPDATE meta SET value = 16384 WHERE key = 'block size'".into(),
            &[damaged],
        ),
        (
            "UPDATE meta SET value = 'large' WHERE key = 'block size'".into(),
            &[unreadable],
        ),
        (
            "DELETE FROM meta WHERE key = 'block size'".into(),
            &[unreadable],
        ),
    ];
    for (damage, problems) in cases {
        let (_directory, store, _) = scopes_in_blocks();
        assert_eq!(checked(&store), Vec::<String>::new());
        store.connection.execute_batch(&damage).expect("damaged");
        assert_eq!(checked(&store), problems, "{damage}");
    }
    // `h` is called on one line, the only block its list holds: listing another block
    // instead is found.
    let (_directory, store, _) = scopes_in_blocks();
    let list: Vec<u8> = store
        .connection
        .query_row(
            "SELECT postings FROM names WHERE name = CAST('h' AS BLOB)",
            [],
            |row| row.get(0),
        )
        .expect("h's list");
    let mut other = list.clone();
    *other.last_mut().expect("a block") -= 1;
    assert_eq!(postings::decode(&other).map(|list| list.len()), Some(1));
    store
        .connection
        .execute(
            "UPDATE names SET postings = ?1 WHERE name = CAST('h' AS BLOB)",
            [other],
        )
        .expect("changed");
    assert_eq!(checked(&store), [unmatched]);
}

/// An update of an index whose block size cannot be read adds no unit: that fails with
/// [`StoreError::BlockSize`], while units stored already are its to publish, and
/// `--verify` reports it before any update. Discarding every unit (an index build with
/// `-u`, or another analyzer's units) gives the index a new identity, and one whose block
/// size cannot be read the size a new index gets, which the units it adds are split by;
/// abandoned, it records nothing. A block size that can be read is kept.
#[test]
fn only_an_update_that_discards_every_unit_records_an_unreadable_block_size() {
    let (_directory, mut store, _) = scopes_in_blocks();
    let before = store.revision_id().expect("identity");
    let mut update = store.update(Duration::ZERO).expect("update");
    update.discard().expect("discarded");
    let unit = update.add_unit(&[1; 32], &unit_of(SCOPES)).expect("unit");
    update
        .finish(b"rule", b"analyzer", &[file("a.c", unit)])
        .expect("published");
    assert_eq!(block_size(&store.connection).expect("read"), Some(1));
    let renewed = store.revision_id().expect("identity");
    assert_eq!(renewed.identity.len(), 16);
    assert_ne!(renewed.identity, before.identity);
    assert_eq!(renewed.number, before.number + 1);
    store
        .connection
        .execute("DELETE FROM meta WHERE key = 'block size'", [])
        .expect("deleted");
    assert_eq!(
        store.verify().expect("verified"),
        ["the index's block size cannot be read".to_owned()]
    );
    let mut update = store.update(Duration::ZERO).expect("update");
    assert!(matches!(
        update.add_unit(&[2; 32], &unit_of(b"int other;\n")),
        Err(StoreError::BlockSize)
    ));
    drop(update);
    publish(&mut store, &[("a.c", 1, SCOPES), ("b.c", 1, SCOPES)]);
    assert_eq!(block_size(&store.connection).expect("read"), None);
    assert_eq!(
        checked(&store),
        ["the index's block size cannot be read".to_owned()]
    );
    let mut update = store.update(Duration::ZERO).expect("update");
    update.discard().expect("discarded");
    drop(update);
    assert_eq!(block_size(&store.connection).expect("read"), None);
    assert_eq!(
        store.revision_id().expect("identity").identity,
        renewed.identity
    );
    let mut update = store.update(Duration::ZERO).expect("update");
    update.discard().expect("discarded");
    let unit = update.add_unit(&[1; 32], &unit_of(SCOPES)).expect("unit");
    update
        .finish(b"rule", b"analyzer", &[file("a.c", unit)])
        .expect("published");
    assert_eq!(
        block_size(&store.connection).expect("read"),
        Some(BLOCK_SIZE)
    );
    let after = store.revision_id().expect("identity").identity;
    assert_eq!(after.len(), 16);
    assert_ne!(after, renewed.identity);
    let blocks = store
        .reader()
        .expect("reader")
        .blocks(unit, None, None)
        .expect("blocks");
    assert_eq!(blocks.len(), 1);
    assert_eq!(checked(&store), Vec::<String>::new());
    assert_eq!(store.verify().expect("verified"), Vec::<String>::new());
}

/// The digest `--verify` sums over the postings changes with any one field of a posting,
/// and two postings that swap their roles do not cancel out.
#[test]
fn the_postings_digest_changes_with_any_field() {
    let mut seed = 0x9e37_79b9_7f4a_7c15u64;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    for _ in 0..20_000 {
        let name: Vec<u8> = (0..next() % 24)
            .map(|_| b'a' + (next() % 26) as u8)
            .collect();
        let unit = (next() % 70_000) as i64 + 1;
        let roles = (next() % 64) as u32;
        let blocks: Vec<u32> = (0..next() % 4).map(|block| block as u32 * 3).collect();
        let digest_of = |name: &[u8], unit, roles, blocks: &[u32]| {
            digest(name, unit, roles, blocks.iter().copied(), &[])
        };
        let one = digest_of(&name, unit, roles, &blocks);
        if let Some((first, rest)) = name.split_first() {
            let other: Vec<u8> = std::iter::once(first ^ 1)
                .chain(rest.iter().copied())
                .collect();
            assert_ne!(one, digest_of(&other, unit, roles, &blocks));
        }
        assert_ne!(one, digest_of(&name, unit + 1, roles, &blocks));
        assert_ne!(one, digest_of(&name, unit, roles ^ 1, &blocks));
        let more: Vec<u32> = blocks.iter().copied().chain([99]).collect();
        assert_ne!(one, digest_of(&name, unit, roles, &more));
        let (other_unit, other_roles) = (unit + 7, roles ^ 4);
        let before = one.wrapping_add(digest_of(&name, other_unit, other_roles, &blocks));
        let swapped = digest_of(&name, unit, other_roles, &blocks)
            .wrapping_add(digest_of(&name, other_unit, roles, &blocks));
        assert_ne!(before, swapped);
        let rows = digest(&name, unit, roles, blocks.iter().copied(), &[1]);
        assert_ne!(one, rows);
    }
}

/// Deleting a unit deletes its blocks and no other's, even for an id too large to have
/// blocks.
#[test]
fn a_unit_deletes_only_its_own_blocks() {
    let directory = tempfile::tempdir().expect("directory");
    let store = Store::open(&directory.path().join("index.db"), Access::Write).expect("created");
    store
        .connection
        .execute_batch(
            "INSERT INTO units VALUES (5, x'05', 1), (6, x'06', 1), ((1 << 44) + 5, x'07', 1);
             INSERT INTO blocks VALUES (5 << 20, NULL, NULL, x''), (6 << 20, NULL, NULL, x'');
             DELETE FROM units WHERE id = (1 << 44) + 5;
             DELETE FROM units WHERE id = 6;",
        )
        .expect("units and blocks");
    let left: Vec<i64> = store
        .connection
        .prepare("SELECT id >> 20 FROM blocks")
        .expect("statement")
        .query_map([], |row| row.get(0))
        .expect("blocks")
        .collect::<Result<_, _>>()
        .expect("ids");
    assert_eq!(left, [5]);
}

/// A unit whose id leaves no room for its blocks' is refused as too large.
#[test]
fn a_unit_id_too_large_for_its_blocks_is_refused() {
    let directory = tempfile::tempdir().expect("directory");
    let mut store =
        Store::open(&directory.path().join("index.db"), Access::Write).expect("created");
    store
        .connection
        .execute("INSERT INTO units VALUES ((1 << 43) - 1, x'00', 1)", [])
        .expect("a unit");
    let mut update = store.update(Duration::ZERO).expect("update");
    assert!(matches!(
        update.add_unit(&[1; 32], &unit_of(b"int a;\n")),
        Err(StoreError::Unit(UnitError::TooLarge))
    ));
}

#[test]
fn an_update_that_changes_nothing_publishes_nothing() {
    let directory = tempfile::tempdir().expect("directory");
    let path = directory.path().join("index.db");
    let mut store = Store::open(&path, Access::Write).expect("created");
    publish(&mut store, &[("a.c", 1, b"int a;\n")]);
    let update = store.update(Duration::ZERO).expect("update");
    let files = update.files().to_vec();
    assert_eq!(
        update
            .finish(b"rule", b"analyzer", &files)
            .expect("finished"),
        Published::Unchanged(1)
    );
}

/// Files read again whose facts did not change keep the revision; only when they were
/// last read is recorded, so `--verify` on an unchanged index publishes nothing.
#[test]
fn reading_unchanged_files_again_keeps_the_revision() {
    let directory = tempfile::tempdir().expect("directory");
    let path = directory.path().join("index.db");
    let mut store = Store::open(&path, Access::Write).expect("created");
    publish(
        &mut store,
        &[("a.c", 1, b"int a;\n"), ("b.c", 1, b"int b;\n")],
    );
    let update = store.update(Duration::ZERO).expect("update");
    let mut files = update.files().to_vec();
    files[1].verified += 5_000_000_000;
    assert_eq!(
        update
            .finish(b"rule", b"analyzer", &files)
            .expect("finished"),
        Published::Unchanged(1)
    );
    let snapshot = store.snapshot().expect("snapshot");
    assert_eq!(snapshot.revision(), 1);
    assert_eq!(snapshot.files(), &files[..]);
}

#[test]
fn a_reader_keeps_its_revision_while_another_process_updates() {
    let directory = tempfile::tempdir().expect("directory");
    let path = directory.path().join("index.db");
    let mut writer = Store::open(&path, Access::Write).expect("created");
    publish(&mut writer, &[("a.c", 1, b"int a;\n")]);
    let reader = Store::open(&path, Access::Read).expect("reader");
    let snapshot = reader.snapshot().expect("snapshot");
    publish(
        &mut writer,
        &[("a.c", 1, b"int a;\n"), ("b.c", 2, b"int b;\n")],
    );
    assert_eq!(snapshot.revision(), 1);
    assert_eq!(snapshot.files().len(), 1);
    snapshot
        .unit(snapshot.files()[0].unit)
        .expect("the old revision's unit");
    drop(snapshot);
    assert_eq!(reader.snapshot().expect("next snapshot").revision(), 2);
}

/// The streamed query reader can start under a writer, keep its complete old
/// revision through publication, and release it before the next query.
#[test]
fn a_streamed_reader_keeps_its_revision_through_active_write_and_publication() {
    let directory = tempfile::tempdir().expect("directory");
    let path = directory.path().join("index.db");
    let mut writer = Store::open(&path, Access::Write).expect("created");
    publish(&mut writer, &[("a.c", 1, b"int a;\n")]);
    let mut update = writer.update(Duration::ZERO).expect("write lock");
    let next_unit = update
        .add_unit(&[2; 32], &unit_of(b"int b;\n"))
        .expect("unpublished unit");
    let store = Store::open(&path, Access::Read).expect("open during update");
    let reader = store.reader().expect("reader during update");
    let files = reader.files().expect("old files");
    assert_eq!(reader.revision_id().number, 1);
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].0, b"a.c");
    assert_eq!(
        update
            .finish(b"rule", b"analyzer", &[file("b.c", next_unit)])
            .expect("publish while reader is active"),
        Published::Revision(2)
    );
    assert_eq!(reader.revision_id().number, 1);
    assert_eq!(reader.files().expect("same old files"), files);
    assert_eq!(
        reader.blocks(files[0].1, None, None).expect("old blocks")[0].lines,
        unit_of(b"int a;\n")
    );
    drop(reader);
    let next = store.reader().expect("next query");
    assert_eq!(next.revision_id().number, 2);
    assert!(
        next.paths()
            .expect("published files")
            .iter()
            .eq([&b"b.c"[..]])
    );
}

#[test]
fn a_damaged_unit_is_found_and_a_discarding_update_replaces_it() {
    let directory = tempfile::tempdir().expect("directory");
    let path = directory.path().join("index.db");
    let mut store = Store::open(&path, Access::Write).expect("created");
    publish(
        &mut store,
        &[("a.c", 1, b"int a;\n"), ("b.c", 2, b"int b;\n")],
    );
    damage(&store, 2);
    // SQLite itself sees nothing wrong; decoding does.
    assert_eq!(store.verify().expect("verified"), Vec::<String>::new());
    assert_eq!(
        checked(&store),
        ["b.c: damaged analysis: analysis unit image is damaged"]
    );

    // The same files and keys again: an ordinary update would keep the damaged unit, a
    // discarding one stores it afresh.
    let mut update = store.update(Duration::ZERO).expect("update");
    update.discard().expect("discarded");
    let rows: Vec<StoredFile> = [("a.c", 1, &b"int a;\n"[..]), ("b.c", 2, b"int b;\n")]
        .iter()
        .map(|(path, key, source)| {
            let unit = update
                .add_unit(&[*key; 32], &unit_of(source))
                .expect("unit");
            file(path, unit)
        })
        .collect();
    assert_eq!(
        update
            .finish(b"rule", b"analyzer", &rows)
            .expect("published"),
        Published::Revision(2)
    );
    assert_eq!(checked(&store), Vec::<String>::new());
}

#[test]
fn an_index_sqlite_cannot_read_is_replaced_only_by_request() {
    let directory = tempfile::tempdir().expect("directory");
    let path = directory.path().join("index.db");
    let mut store = Store::open(&path, Access::Write).expect("created");
    let files: Vec<(String, u8, Vec<u8>)> = (0..200u8)
        .map(|n| {
            (
                format!("f{n}.c"),
                n,
                format!("int value{n};\n").into_bytes(),
            )
        })
        .collect();
    let files: Vec<(&str, u8, &[u8])> = files
        .iter()
        .map(|(path, key, source)| (path.as_str(), *key, source.as_slice()))
        .collect();
    publish(&mut store, &files);
    trim_log(&store);
    drop(store);
    // Overwrite the pages after the first two, keeping the header ripscope recognises.
    let mut bytes = std::fs::read(&path).expect("index");
    assert!(bytes.len() > 3 * 4096, "{}", bytes.len());
    bytes[2 * 4096..].fill(0xa5);
    std::fs::write(&path, &bytes).expect("damaged");
    replace_after_verification(&path);
    // Anything that is not a ripscope index is never replaced.
    let other = directory.path().join("other.txt");
    std::fs::write(&other, b"notes").expect("other");
    assert!(matches!(
        Store::replace(&other),
        Err(StoreError::NotAnIndex(_))
    ));
    assert_eq!(std::fs::read(&other).expect("kept"), b"notes");
}

#[test]
fn a_reported_integrity_problem_closes_its_reader_before_replacement() {
    let directory = tempfile::tempdir().expect("directory");
    let path = directory.path().join("index.db");
    let mut store = Store::open(&path, Access::Write).expect("created");
    publish(&mut store, &[("a.c", 1, b"int value;\n")]);
    // Keep the schema readable, so verify returns problems alongside an open Store.
    store
        .connection
        .execute_batch("PRAGMA foreign_keys = OFF; DELETE FROM units;")
        .expect("dangling file reference");
    trim_log(&store);
    drop(store);
    assert!(
        replace_after_verification(&path),
        "verification must report problems"
    );
}

fn replace_after_verification(path: &Path) -> bool {
    let damaged = Store::open(path, Access::Write)
        .and_then(|store| store.verify().map(|problems| (store, problems)));
    let reported = match damaged {
        Ok((store, problems)) => {
            assert!(!problems.is_empty());
            drop(store);
            true
        }
        Err(error) => {
            assert!(matches!(error, StoreError::Damaged(_)), "{error}");
            false
        }
    };
    let replaced = Store::replace(path).expect("replaced");
    assert!(replaced.verify().expect("verified").is_empty());
    assert_eq!(replaced.snapshot().expect("snapshot").revision(), 0);
    reported
}

#[test]
fn a_second_update_waits_or_is_busy() {
    let directory = tempfile::tempdir().expect("directory");
    let path = directory.path().join("index.db");
    let mut first = Store::open(&path, Access::Write).expect("created");
    let mut second = Store::open(&path, Access::Write).expect("second");
    let holding = first.update(Duration::ZERO).expect("write lock");
    assert!(matches!(
        second.update(Duration::from_millis(50)),
        Err(StoreError::Busy)
    ));
    drop(holding);
    second
        .update(Duration::ZERO)
        .expect("updating after the lock")
        .finish(b"", b"", &[])
        .expect("finished");
}

/// Last-close WAL cleanup briefly takes the main file's exclusive lock. Holding
/// that lock through SQLite controls the same open-time shared-lock contention
/// without relying on the timing of another process's cleanup.
#[test]
fn opening_waits_briefly_for_a_main_file_lock_then_recovers_after_release() {
    let directory = tempfile::tempdir().expect("directory");
    let path = directory.path().join("index.db");
    let mut store = Store::open(&path, Access::Write).expect("created");
    publish(&mut store, &[("a.c", 1, b"int a;\n")]);
    drop(store);
    let lock = Connection::open(&path).expect("lock holder");
    lock.execute_batch("PRAGMA locking_mode = EXCLUSIVE; BEGIN EXCLUSIVE; COMMIT;")
        .expect("exclusive main-file lock");
    let started = std::time::Instant::now();
    assert!(matches!(
        Store::open(&path, Access::Read),
        Err(StoreError::Busy)
    ));
    let elapsed = started.elapsed();
    assert!(elapsed >= Duration::from_millis(50), "{elapsed:?}");
    assert!(elapsed < Duration::from_secs(2), "{elapsed:?}");
    drop(lock);
    let reader = Store::open(&path, Access::Read).expect("reopen after release");
    assert_eq!(
        reader
            .reader()
            .expect("published reader")
            .revision_id()
            .number,
        1
    );
    let wait: i64 = reader
        .connection
        .query_row("PRAGMA busy_timeout", [], |row| row.get(0))
        .expect("query wait");
    assert_eq!(
        wait, 0,
        "only opening waits for transient main-file contention"
    );
}

/// SQLite can unlink a companion between our no-follow open and descriptor
/// check. The detached descriptor no longer describes the path SQLite will use.
#[cfg(unix)]
#[test]
fn opening_accepts_a_companion_unlinked_after_it_was_opened() {
    for suffix in ["-wal", "-shm"] {
        for replaced in [false, true] {
            let directory = tempfile::tempdir().expect("directory");
            let path = directory.path().join("index.db");
            let mut store = Store::open(&path, Access::Write).expect("created");
            publish(&mut store, &[("a.c", 1, b"int a;\n")]);
            drop(store);
            let companion = beside(&path, suffix);
            std::fs::write(&companion, b"").expect("regular companion");
            before_next_companion_stat(move || {
                std::fs::remove_file(&companion).expect("concurrent last-close unlink");
                if replaced {
                    std::fs::write(&companion, b"").expect("concurrent new companion");
                }
            });
            let reader = Store::open(&path, Access::Read).expect("reader after companion unlink");
            assert_eq!(
                reader
                    .reader()
                    .expect("published reader")
                    .revision_id()
                    .number,
                1
            );
        }
    }
}

/// Reopening a detached descriptor must still validate the new path before
/// SQLite could write through an unsafe replacement.
#[cfg(unix)]
#[test]
fn an_unsafe_companion_replacement_after_open_is_refused_and_untouched() {
    use std::os::unix::fs::symlink;
    for suffix in ["-wal", "-shm"] {
        for replacement in ["symlink", "hardlink", "directory"] {
            let directory = tempfile::tempdir().expect("directory");
            let path = directory.path().join("index.db");
            let mut store = Store::open(&path, Access::Write).expect("created");
            publish(&mut store, &[("a.c", 1, b"int a;\n")]);
            drop(store);
            let companion = beside(&path, suffix);
            std::fs::write(&companion, b"").expect("regular companion");
            let target = directory.path().join("keep");
            std::fs::write(&target, b"must stay unchanged").expect("target");
            let changed = companion.clone();
            let protected = target.clone();
            before_next_companion_stat(move || {
                std::fs::remove_file(&changed).expect("concurrent unlink");
                match replacement {
                    "symlink" => symlink(&protected, &changed).expect("replacement symlink"),
                    "hardlink" => {
                        std::fs::hard_link(&protected, &changed).expect("replacement hardlink")
                    }
                    "directory" => std::fs::create_dir(&changed).expect("replacement directory"),
                    _ => unreachable!(),
                }
            });
            assert!(
                matches!(Store::open(&path, Access::Read), Err(StoreError::Unsafe(ref refused)) if refused == &companion)
            );
            assert_eq!(
                std::fs::read(&target).expect("protected target"),
                b"must stay unchanged"
            );
        }
    }
}

/// A repeatedly replaced companion is contention, not an unsafe path. The
/// safety check must terminate without opening SQLite or repairing stale files.
#[cfg(unix)]
#[test]
fn companion_unlink_contention_stops_after_three_attempts() {
    use std::cell::Cell;
    use std::rc::Rc;

    fn unlink_next(path: PathBuf, attempts: Rc<Cell<usize>>) {
        before_next_companion_stat(move || {
            attempts.set(attempts.get() + 1);
            std::fs::remove_file(&path).expect("concurrent unlink");
            std::fs::write(&path, b"").expect("concurrent replacement");
            if attempts.get() < 3 {
                unlink_next(path, attempts);
            }
        });
    }

    for suffix in ["-wal", "-shm"] {
        let directory = tempfile::tempdir().expect("directory");
        let path = directory.path().join("index.db");
        let mut store = Store::open(&path, Access::Write).expect("created");
        publish(&mut store, &[("a.c", 1, b"int a;\n")]);
        drop(store);
        let original = std::fs::read(&path).expect("published index");
        let companion = beside(&path, suffix);
        std::fs::write(&companion, b"").expect("regular companion");
        let attempts = Rc::new(Cell::new(0));
        unlink_next(companion, attempts.clone());
        assert!(matches!(
            Store::open(&path, Access::Write),
            Err(StoreError::Busy)
        ));
        assert_eq!(attempts.get(), 3, "three total open/fstat attempts");
        assert_eq!(std::fs::read(&path).expect("unchanged index"), original);
    }
}

#[test]
fn sqlite_is_given_a_name_it_cannot_read_as_a_uri() {
    assert_eq!(for_sqlite(Path::new("file:i.db")), Path::new("./file:i.db"));
    assert_eq!(
        for_sqlite(Path::new("file:i.db.new-1")),
        Path::new("./file:i.db.new-1")
    );
    // SQLite's test is case-sensitive, and only a name that begins with `file:` is one.
    for kept in ["FILE:i.db", "d/file:i.db", "i.db"] {
        assert_eq!(for_sqlite(Path::new(kept)), Path::new(kept), "{kept}");
    }
}

#[cfg(windows)]
#[test]
fn a_windows_sqlite_open_failure_does_not_guess_directory_permissions() {
    let directory = tempfile::tempdir().expect("directory");
    let index = IndexPath::new(&directory.path().join("i.db")).expect("path");
    for code in [
        rusqlite::ffi::SQLITE_CANTOPEN,
        rusqlite::ffi::SQLITE_READONLY,
    ] {
        let error = rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(code),
            Some("original cause".to_owned()),
        );
        let explained = explained(&index, StoreError::Sqlite(error));
        assert!(matches!(explained, StoreError::Sqlite(_)), "{explained}");
        assert!(explained.to_string().contains("original cause"));
    }
}

#[cfg(windows)]
#[test]
fn a_windows_index_path_may_hold_a_colon_only_in_its_drive_or_share() {
    use super::index_path::name_allowed;
    for allowed in [
        r"C:\x\i.db",
        r"\\?\C:\x\i.db",
        r"\\server\share\i.db",
        r"C:i.db",
        "i.db",
    ] {
        assert!(name_allowed(Path::new(allowed)).is_ok(), "{allowed}");
    }
    for refused in ["file:i.db", r"C:\x\a:b", r"x\y:z\i.db"] {
        assert!(name_allowed(Path::new(refused)).is_err(), "{refused}");
    }
}

/// The names a prefix range holds end before the first byte string after the prefix.
#[test]
fn a_prefix_range_ends_after_every_name_with_the_prefix() {
    assert_eq!(after_prefix(b"kmem_"), Some(b"kmem`".to_vec()));
    assert_eq!(after_prefix(b"a\xff\xff"), Some(b"b".to_vec()));
    assert_eq!(after_prefix(b"\xff"), None);
}
