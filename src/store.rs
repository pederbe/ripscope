//! The Store (docs/architecture/storage.md): one SQLite file at the
//! `-f` path holding the current index revision. It keeps the selection rule, the indexed
//! files in database order with what change detection needs, each file's Analysis unit as
//! blocks of its lines (ADR 0011), the units' include operands and their name postings,
//! but no copies of the sources. A query reads only what it needs
//! through a [`Reader`].
//!
//! An update holds SQLite's write lock from start to commit, so index builds wait for each
//! other and new units go into the transaction as they are made; readers keep the
//! revision they started with (write-ahead logging). Integrity rests on SQLite. The
//! Store opens only its own files: an absent path or an empty file becomes a new index,
//! and anything else that is not a ripscope index, a cscope database included, is refused
//! without being touched.

use crate::hashing::{HashMap, HashSet};
use crate::unit::{AnalysisUnit, Placed, Scope, Split, UnitError};
use index_path::{IndexPath, beside, for_sqlite};
use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior, params};
use std::fmt;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

mod index_path;
mod location;
mod postings;
#[cfg(test)]
mod tests;

#[cfg(test)]
std::thread_local! {
    static BEFORE_UPDATE: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const {
        std::cell::RefCell::new(None)
    };
    #[cfg(unix)]
    static BEFORE_COMPANION_STAT: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const {
        std::cell::RefCell::new(None)
    };
}

/// Installs a one-shot gate after the next companion is opened on this thread.
#[cfg(all(test, unix))]
fn before_next_companion_stat(gate: impl FnOnce() + 'static) {
    BEFORE_COMPANION_STAT.with(|slot| {
        assert!(
            slot.borrow_mut().replace(Box::new(gate)).is_none(),
            "only one next-companion gate per thread"
        );
    });
}

/// Installs a one-shot gate immediately before this thread's next write-lock attempt.
#[cfg(test)]
pub(crate) fn before_next_update(gate: impl FnOnce() + 'static) {
    BEFORE_UPDATE.with(|slot| {
        assert!(
            slot.borrow_mut().replace(Box::new(gate)).is_none(),
            "only one next-update gate per thread"
        );
    });
}

/// `PRAGMA application_id`: "RIPS".
const APPLICATION_ID: i32 = 0x5249_5053;
/// `PRAGMA user_version`: the schema version.
const SCHEMA_VERSION: i32 = 10;

/// SQLite's last-close WAL cleanup briefly takes the main file's exclusive lock.
/// Setup uses a 50 ms SQLite busy timeout, not a wall-clock deadline. Ordinary
/// reads and the caller's separate writer-lock waits keep their existing policy.
const OPEN_WAIT: Duration = Duration::from_millis(50);

/// How many bytes of image a block of a unit's lines holds before the next line starts
/// another (ADR 0011), in the indexes this version makes. An index keeps its own in
/// `meta`, which its updates use. The environment variable `RIPSCOPE_BLOCK_SIZE` sets
/// another when an index is made, so that tests can store a block for each line.
const BLOCK_SIZE: usize = 16 << 10;

/// The bits of a block's id that hold its number in its unit: block `number` of unit `id`
/// has the id `id << BLOCK_BITS | number`, so a unit has at most 2^20 blocks.
const BLOCK_BITS: u32 = 20;

const SCHEMA: &str = "
CREATE TABLE meta (key TEXT PRIMARY KEY, value ANY NOT NULL) STRICT;
-- Each Analysis unit, with how many blocks hold its lines.
CREATE TABLE units (
    id INTEGER PRIMARY KEY,
    key BLOB NOT NULL UNIQUE,
    blocks INTEGER NOT NULL
) STRICT;
-- A unit's lines in blocks (ADR 0011): each an Analysis unit image of a run of its lines,
-- with the macro and the function that run starts inside (NULL for none). Block `number`
-- of unit `id` has the id id << 20 | number, so a unit's blocks are one range of ids, in
-- order, deleted with it.
CREATE TABLE blocks (
    id INTEGER PRIMARY KEY,
    macro BLOB,
    function BLOB,
    image BLOB NOT NULL
) STRICT;
CREATE TRIGGER unit_blocks AFTER DELETE ON units WHEN old.id BETWEEN 0 AND 8796093022207
BEGIN
    DELETE FROM blocks WHERE id BETWEEN old.id << 20 AND (old.id << 20) + 1048575;
END;
-- Each distinct include operand the units hold, without its delimiters, so that an
-- includer query matches each once.
CREATE TABLE operands (
    id INTEGER PRIMARY KEY,
    operand BLOB NOT NULL UNIQUE
) STRICT;
-- Each unit's include operands, in order, with where each sits: its block, line, the
-- line's record in the block and the name's index in the line. Each also keeps its
-- includer row: the name its line's reading resumes at after the row, and the text the row
-- shows, so that an includer query reads no blocks (ADR 0011).
CREATE TABLE includes (
    unit INTEGER NOT NULL REFERENCES units(id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    operand INTEGER NOT NULL REFERENCES operands(id),
    quoted INTEGER NOT NULL,
    block INTEGER NOT NULL,
    line INTEGER NOT NULL,
    record INTEGER NOT NULL,
    name INTEGER NOT NULL,
    after INTEGER NOT NULL,
    text BLOB NOT NULL,
    PRIMARY KEY (unit, position)
) STRICT, WITHOUT ROWID;
-- An includer query reads the includes of the operands it matches from this index alone.
CREATE INDEX includes_by_operand
    ON includes(operand, unit, block, line, record, name, after, text);
CREATE TABLE files (
    ordinal INTEGER PRIMARY KEY,
    -- Not unique: a file a list names twice is indexed twice, as cscope does.
    path BLOB NOT NULL,
    unit INTEGER NOT NULL REFERENCES units(id),
    size INTEGER NOT NULL,
    modified INTEGER NOT NULL,
    changed INTEGER NOT NULL,
    identity BLOB NOT NULL,
    hash BLOB NOT NULL,
    verified INTEGER NOT NULL
) STRICT;
CREATE INDEX files_by_unit ON files(unit);
-- Each distinct name the units hold, with the units that hold it, its roles in each and
-- the blocks a query for it reads (store/postings.rs). Kept in step with the units by the
-- updates.
-- Keep posting lists out of the name-key index: a large list in an interior node of a
-- WITHOUT ROWID table makes unrelated lookups read its overflow pages too.
CREATE TABLE names (
    name BLOB PRIMARY KEY,
    postings BLOB NOT NULL
) STRICT;
INSERT INTO meta VALUES ('revision', 0), ('selection', x''), ('analyzer', x''),
    ('identity', randomblob(16)), ('paths', x'');
";

/// How a damaged index is recovered.
pub(crate) const REBUILD: &str = "rebuild it with -bu and the options it was built with";

/// How an index is opened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Access {
    Read,
    /// For an index build: also creates the index when the path is absent or an empty
    /// file, and makes it afresh when it is a ripscope index of another schema version.
    Write,
    /// For an update of the index as it was built: like `Write`, except that an index of
    /// another schema version is refused, since the selection rule it holds is needed.
    Update,
}

#[derive(Debug)]
pub(crate) enum StoreError {
    /// The path holds something other than a ripscope index; it was not touched.
    NotAnIndex(PathBuf),
    /// On Windows, the path's file name holds a `:`, which names a stream of another file.
    Name(PathBuf),
    /// The path is a symbolic link to nothing.
    Dangling(PathBuf, PathBuf),
    /// Reading needs an existing index.
    Missing(PathBuf),
    /// The index uses another schema version; rebuild it.
    Version(i32),
    /// Another process is updating the index.
    Busy,
    /// The index file cannot be written, so it cannot be updated.
    ReadOnly(PathBuf),
    /// The index, or a log file beside it, is open in another process (on Windows) or
    /// cannot be removed, so the index cannot be replaced.
    InUse(PathBuf),
    /// A file SQLite keeps beside the index cannot be made writable again.
    Companion(PathBuf),
    /// A file SQLite keeps beside the index is a link, has another name or is not a file,
    /// so SQLite would write through it.
    Unsafe(PathBuf),
    /// A reader cannot create SQLite's shared-memory file beside the index.
    SharedMemory(PathBuf),
    /// The index would live where SQLite's write-ahead log is unsafe.
    Location(PathBuf, &'static str),
    /// Where the index would live could not be checked.
    Unchecked(PathBuf, std::io::Error),
    /// SQLite cannot read the index file; an index build with `-u` replaces it.
    Damaged(rusqlite::Error),
    Unit(UnitError),
    /// A name's stored postings do not decode.
    Postings,
    /// The revision's path list does not decode.
    Paths,
    /// The index's block size cannot be read, so an update cannot split the units it
    /// adds; one that analyses every file again records it again.
    BlockSize,
    Sqlite(rusqlite::Error),
    Io(std::io::Error),
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotAnIndex(path) => write!(
                f,
                "{} is not a ripscope index; it was left unchanged",
                path.display()
            ),
            Self::Name(path) => {
                write!(f, "{}: a Windows file name cannot hold ':'", path.display())
            }
            Self::Dangling(path, target) => write!(
                f,
                "{} is a symbolic link to {}, which does not exist",
                path.display(),
                target.display()
            ),
            Self::Missing(path) => write!(f, "no index at {}", path.display()),
            Self::Version(version) => write!(
                f,
                "the index has schema version {version}, and this ripscope reads version \
                 {SCHEMA_VERSION}; rebuild it"
            ),
            Self::Busy => f.write_str("another process is updating the index"),
            Self::ReadOnly(path) => write!(f, "the index {} cannot be written", path.display()),
            Self::InUse(path) => write!(
                f,
                "{} is open in another process or cannot be removed, so the index cannot \
                 be replaced; build the index again once it is closed or removed",
                path.display()
            ),
            Self::Companion(path) => write!(
                f,
                "{} cannot be written, so the index cannot be updated; it must be writable \
                 like the index (its owner can change that)",
                path.display()
            ),
            Self::Unsafe(path) => write!(
                f,
                "{} is not a plain file (a link, a file with another name, or not a file); \
                 SQLite would write through it, so the index is not opened; remove it",
                path.display()
            ),
            Self::SharedMemory(path) => write!(
                f,
                "cannot read the index {0}: SQLite must create {0}-wal and {0}-shm beside it, \
                 and that directory cannot be written; copy the index somewhere writable and \
                 give it with -f",
                path.display()
            ),
            Self::Location(path, place) => write!(
                f,
                "{} is {place}, where the index's write-ahead log is unsafe; keep the index in \
                 a local folder that is not synchronised",
                path.display()
            ),
            Self::Unchecked(path, error) => write!(
                f,
                "cannot tell whether {} is a safe place for the index: {error}",
                path.display()
            ),
            Self::Damaged(error) => write!(f, "damaged index: {error}; {REBUILD}"),
            Self::Unit(error) => write!(f, "damaged index: {error}; {REBUILD}"),
            Self::Postings => write!(
                f,
                "damaged index: its name postings cannot be read; {REBUILD}"
            ),
            Self::BlockSize => write!(f, "damaged index: its block size cannot be read; {REBUILD}"),
            Self::Paths => write!(f, "damaged index: its path list cannot be read; {REBUILD}"),
            Self::Sqlite(error) => write!(f, "index database: {error}"),
            Self::Io(error) => write!(f, "index file: {error}"),
        }
    }
}

impl std::error::Error for StoreError {}

impl From<rusqlite::Error> for StoreError {
    fn from(error: rusqlite::Error) -> Self {
        match error.sqlite_error_code() {
            Some(rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked) => {
                Self::Busy
            }
            Some(rusqlite::ErrorCode::DatabaseCorrupt | rusqlite::ErrorCode::NotADatabase) => {
                Self::Damaged(error)
            }
            _ => Self::Sqlite(error),
        }
    }
}

impl From<UnitError> for StoreError {
    fn from(error: UnitError) -> Self {
        Self::Unit(error)
    }
}

/// What change detection compares for a file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FileStat {
    pub size: u64,
    /// Modification and status-change times, in nanoseconds since the epoch.
    pub modified: i64,
    pub changed: i64,
    /// Whatever identifies the file on its file system: an inode number, or on Windows
    /// the volume serial number and file index.
    pub identity: Vec<u8>,
}

/// An indexed file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StoredFile {
    /// As displayed in rows: raw bytes.
    pub path: Vec<u8>,
    pub unit: i64,
    pub stat: FileStat,
    /// SHA-256 of the bytes analysed.
    pub hash: [u8; 32],
    /// When those bytes were last read and hashed: the start of that update, in
    /// nanoseconds since the epoch. Metadata alone is trusted only for a file whose
    /// modification time is safely older.
    pub verified: i64,
}

impl StoredFile {
    /// The same file with the same facts, whenever its bytes were last read.
    fn same_facts(&self, other: &Self) -> bool {
        self.path == other.path
            && self.unit == other.unit
            && self.stat == other.stat
            && self.hash == other.hash
    }
}

/// An include operand without its delimiters, and whether it was quoted.
pub(crate) type Include = (Vec<u8>, bool);

/// Which revision of which index. An index made afresh at the same path numbers its
/// revisions from 1 again, and its identity tells it from the earlier one; sessions keep
/// the blocks they decode by the identity.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct RevisionId {
    /// Random bytes written when the index was created, and again by each update that
    /// analyses every file again; empty for an index made before ripscope wrote them,
    /// which only its number tells apart.
    pub identity: Vec<u8>,
    pub number: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Published {
    Revision(u64),
    /// Nothing changed; the revision stays.
    Unchanged(u64),
}

pub(crate) struct Store {
    connection: Connection,
    /// Whether a reader has read this connection's index before.
    read: std::cell::Cell<bool>,
    /// Opening it made the index afresh in a folder OneDrive synchronises.
    made_in_onedrive: bool,
}

impl Store {
    pub(crate) fn open(path: &Path, access: Access) -> Result<Self, StoreError> {
        profile_span!("store_open");
        Self::inspect(path)?.open(access)
    }

    /// What is at `path` now, looked at once to tell whether opening it makes a new index
    /// and then to open it.
    pub(crate) fn inspect(path: &Path) -> Result<Inspected, StoreError> {
        let index = IndexPath::new(path)?;
        let state = state_at(index.file())?;
        Ok(Inspected { index, state })
    }

    fn open_at(index: &IndexPath, state: FileState, access: Access) -> Result<Self, StoreError> {
        let path = index.given();
        if access != Access::Read {
            let directory = index.directory();
            let shown = || std::path::absolute(directory).unwrap_or_else(|_| directory.to_owned());
            match location::unsafe_place(directory) {
                Ok(None) => {}
                Ok(Some(place)) => return Err(StoreError::Location(shown(), place)),
                Err(error) => return Err(StoreError::Unchecked(shown(), error)),
            }
        }
        let store = match (state, access) {
            (FileState::Index(version), _) if version == SCHEMA_VERSION => {
                // Before SQLite opens them, which it does to read the schema.
                remove_journal(index)?;
                check_companions(index, access != Access::Read && may_write(index.file()))?;
                Self::with(open_index(index.file()).map_err(|error| explained(index, error))?)
            }
            // Only an index build makes afresh an index of another schema version: an
            // update needs the selection rule the index holds.
            (FileState::Index(version), Access::Read | Access::Update) => {
                return Err(StoreError::Version(version));
            }
            (FileState::Index(_), Access::Write) => Self::made(index, Placing::Replace)?,
            (FileState::Absent | FileState::Empty, Access::Write | Access::Update) => {
                Self::made(index, Placing::New)?
            }
            (FileState::Absent | FileState::Empty, Access::Read) => {
                return Err(StoreError::Missing(path.to_owned()));
            }
            (FileState::Foreign, _) => return Err(StoreError::NotAnIndex(path.to_owned())),
        };
        // SQLite opens a file it may not write for reading only.
        if access != Access::Read && store.connection.is_readonly(rusqlite::MAIN_DB)? {
            return Err(StoreError::ReadOnly(path.to_owned()));
        }
        if access == Access::Read {
            read_only(&store.connection)?;
        }
        Ok(store)
    }

    /// A new, empty index made at `index` (see [`create`]).
    fn made(index: &IndexPath, placing: Placing) -> Result<Self, StoreError> {
        let connection = create(index, placing)?;
        Ok(Self {
            made_in_onedrive: location::in_onedrive(index.directory()),
            ..Self::with(connection)
        })
    }

    /// Whether opening it made the index afresh in a folder OneDrive synchronises, which
    /// may lock the index or copy it while it is written (ADR 0005).
    pub(crate) fn made_in_onedrive(&self) -> bool {
        self.made_in_onedrive
    }

    /// The same index, read only as [`Access::Read`] opens it, once an update in this
    /// process is done with it: a session or query reads from it rather than opening the
    /// index again.
    pub(crate) fn into_reader(self) -> Result<Self, StoreError> {
        // A reader leaves the write-ahead log as it is when it closes, so what the update
        // wrote would stay in it, and every connection opened later would read all of it
        // again: on Windows, where a session opens the index for each query, a kernel
        // query took 8 ms instead of 1.
        let log = self
            .connection
            .path()
            .and_then(|path| std::fs::metadata(format!("{path}-wal")).ok());
        if log.is_some_and(|log| log.len() > 0) {
            trim_log(&self);
        }
        read_only(&self.connection)?;
        Ok(self)
    }

    fn with(connection: Connection) -> Self {
        Self {
            connection,
            read: std::cell::Cell::new(false),
            made_in_onedrive: false,
        }
    }

    /// The latest published revision: its number, and its index's identity, read
    /// together in one statement.
    pub(crate) fn revision_id(&self) -> Result<RevisionId, StoreError> {
        let (number, identity) = self.connection.query_row(
            "SELECT value, (SELECT value FROM meta WHERE key = 'identity') \
             FROM meta WHERE key = 'revision'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        Ok(RevisionId {
            identity: identity_in(identity),
            number: revision_in(number),
        })
    }

    /// The current revision, as one read transaction held while the snapshot lives.
    pub(crate) fn snapshot(&self) -> Result<Snapshot<'_>, StoreError> {
        let transaction = self.connection.unchecked_transaction()?;
        let current = Current::read(&transaction)?;
        Ok(Snapshot {
            transaction,
            current,
        })
    }

    /// The current revision, read for one query as one read transaction held while the
    /// reader lives. It never reads the whole file list at once.
    pub(crate) fn reader(&self) -> Result<Reader<'_>, StoreError> {
        profile_span!("reader_metadata");
        // A Session reads for every request, and compiling these statements each time
        // took a third of a cheap request's work. SQLite compiles a statement kept for
        // reuse without its faster lookaside memory, though, which would slow a One-shot
        // query's only read, so only later reads keep them.
        let cached = self.read.replace(true);
        let transaction = ReadTransaction::begin(&self.connection, cached)?;
        let value = |key: &str| {
            with_statement(
                &transaction,
                cached,
                "SELECT value FROM meta WHERE key = ?1",
                |statement| statement.query_row([key], |row| row.get(0)),
            )
        };
        let id = RevisionId {
            identity: identity_in(
                value("identity")
                    .optional()?
                    .unwrap_or(rusqlite::types::Value::Null),
            ),
            number: revision_in(value("revision")?),
        };
        let selection = blob_in(value("selection")?);
        let analyzer = blob_in(value("analyzer")?);
        Ok(Reader {
            transaction,
            id,
            selection,
            analyzer,
        })
    }

    /// Starts an update, waiting up to `wait` for another process's. The update holds
    /// the write lock until it is finished or dropped (which abandons it).
    pub(crate) fn update(&mut self, wait: Duration) -> Result<Update<'_>, StoreError> {
        self.connection.busy_timeout(wait)?;
        #[cfg(test)]
        if let Some(gate) = BEFORE_UPDATE.with(|slot| slot.borrow_mut().take()) {
            gate();
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate);
        let transaction = transaction?;
        let current = Current::read(&transaction)?;
        let fresh =
            transaction.query_row("SELECT NOT EXISTS (SELECT 1 FROM names)", [], |row| {
                row.get(0)
            })?;
        let block_size = block_size(&transaction)?;
        Ok(Update {
            transaction,
            current,
            discarded: false,
            pending: postings::Pending::default(),
            fresh,
            block_size,
            observing: None,
        })
    }

    /// Replaces a ripscope index that SQLite can no longer read with a new, empty one
    /// (the recovery an index build with `-u` makes). Anything else at `path` is refused
    /// untouched.
    pub(crate) fn replace(path: &Path) -> Result<Self, StoreError> {
        let Inspected { index, state } = Self::inspect(path)?;
        match state {
            FileState::Index(_) => Self::made(&index, Placing::Replace),
            FileState::Absent | FileState::Empty => Self::open_at(&index, state, Access::Write),
            FileState::Foreign => Err(StoreError::NotAnIndex(path.to_owned())),
        }
    }

    /// `--verify`: SQLite's full integrity and foreign-key checks, and the block size,
    /// which an update that analyses every file again would record again. Returns the
    /// problems found; a sound database has none.
    pub(crate) fn verify(&self) -> Result<Vec<String>, StoreError> {
        let lines = |sql: &str| -> Result<Vec<String>, StoreError> {
            let mut statement = self.connection.prepare(sql)?;
            let lines = statement
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(lines)
        };
        let mut problems: Vec<String> = lines("PRAGMA integrity_check")?
            .into_iter()
            .filter(|line| line != "ok")
            .collect();
        if problems.is_empty() {
            problems = lines(
                "SELECT 'the index refers to analyses it no longer holds' \
                 FROM pragma_foreign_key_check LIMIT 1",
            )?;
        }
        if problems.is_empty() && block_size(&self.connection)?.is_none() {
            problems.push(NO_BLOCK_SIZE.to_owned());
        }
        Ok(problems)
    }
}

/// What a revision records about itself.
struct Current {
    revision: u64,
    selection: Vec<u8>,
    analyzer: Vec<u8>,
    files: Vec<StoredFile>,
    /// What the Refresh that left this revision observed, if it was clean: see
    /// [`Update::observed`].
    observed: Option<Vec<u8>>,
}

impl Current {
    fn read(connection: &Connection) -> Result<Self, StoreError> {
        let revision = revision_of(connection)?;
        let mut statement = connection.prepare(
            "SELECT path, unit, size, modified, changed, identity, hash, verified
             FROM files ORDER BY ordinal",
        )?;
        let files = statement
            .query_map([], |row| {
                // Read in place: an update reads every file's.
                let hash = row.get_ref(6)?;
                let hash = hash.as_blob().map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(6, hash.data_type(), Box::new(error))
                })?;
                Ok(StoredFile {
                    path: row.get(0)?,
                    unit: row.get(1)?,
                    stat: FileStat {
                        size: u64::try_from(row.get::<_, i64>(2)?).unwrap_or_default(),
                        modified: row.get(3)?,
                        changed: row.get(4)?,
                        identity: row.get(5)?,
                    },
                    hash: hash.try_into().unwrap_or_default(),
                    verified: row.get(7)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let observed = connection
            .query_row("SELECT value FROM meta WHERE key = 'observed'", [], |row| {
                row.get::<_, rusqlite::types::Value>(0)
            })
            .optional()?
            .and_then(|value| match value {
                rusqlite::types::Value::Blob(bytes) => Some(bytes),
                _ => None,
            });
        Ok(Self {
            revision,
            selection: meta_blob(connection, "selection")?,
            analyzer: meta_blob(connection, "analyzer")?,
            files,
            observed,
        })
    }
}

/// A value `meta` records as bytes; empty when it holds something else.
fn meta_blob(connection: &Connection, key: &str) -> Result<Vec<u8>, StoreError> {
    let value = connection.query_row("SELECT value FROM meta WHERE key = ?1", [key], |row| {
        row.get(0)
    })?;
    Ok(blob_in(value))
}

/// The bytes in a `meta` value; empty when it holds something else.
fn blob_in(value: rusqlite::types::Value) -> Vec<u8> {
    match value {
        rusqlite::types::Value::Blob(bytes) => bytes,
        _ => Vec::new(),
    }
}

/// A revision's indexed paths, in database order, as `meta` keeps them in one value so
/// that a query reads them at once: each path's length as a variable-length integer, then
/// its bytes.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Paths {
    bytes: Vec<u8>,
    /// Where each path starts and ends in `bytes`.
    spans: Vec<(usize, usize)>,
}

impl Paths {
    fn encode<'p>(paths: impl Iterator<Item = &'p [u8]>) -> Vec<u8> {
        let mut bytes = Vec::new();
        for path in paths {
            let mut length = path.len() as u64;
            while length >= 0x80 {
                bytes.push((length as u8) | 0x80);
                length >>= 7;
            }
            bytes.push(length as u8);
            bytes.extend_from_slice(path);
        }
        bytes
    }

    fn decode(bytes: Vec<u8>) -> Option<Self> {
        let mut spans = Vec::new();
        let mut at = 0;
        while at < bytes.len() {
            let (mut length, mut shift) = (0usize, 0);
            loop {
                let byte = *bytes.get(at)?;
                at += 1;
                length |= usize::from(byte & 0x7f).checked_shl(shift)?;
                if byte < 0x80 {
                    break;
                }
                shift += 7;
            }
            let end = at.checked_add(length).filter(|end| *end <= bytes.len())?;
            spans.push((at, end));
            at = end;
        }
        Some(Self { bytes, spans })
    }

    /// The paths, in database order.
    pub(crate) fn iter(&self) -> impl Iterator<Item = &[u8]> {
        self.spans
            .iter()
            .map(|&(start, end)| &self.bytes[start..end])
    }
}

/// A digest of a revision's file rows, which a record of what its Refresh observed holds.
fn files_digest(files: &[StoredFile]) -> [u8; 32] {
    let mut bytes = Vec::new();
    for file in files {
        for part in [&file.path[..], &file.stat.identity, &file.hash] {
            bytes.extend_from_slice(&(part.len() as u64).to_le_bytes());
            bytes.extend_from_slice(part);
        }
        for number in [
            file.unit,
            i64::try_from(file.stat.size).unwrap_or(i64::MAX),
            file.stat.modified,
            file.stat.changed,
            file.verified,
        ] {
            bytes.extend_from_slice(&number.to_le_bytes());
        }
    }
    blake3::hash(&bytes).into()
}

/// Records what a clean Refresh observed, or removes the record of an earlier one.
fn record_observed(connection: &Connection, observed: Option<&[u8]>) -> Result<(), StoreError> {
    match observed {
        Some(observed) => connection.execute(
            "INSERT OR REPLACE INTO meta VALUES ('observed', ?1)",
            [observed],
        )?,
        None => connection.execute("DELETE FROM meta WHERE key = 'observed'", [])?,
    };
    Ok(())
}

/// The revision number `meta` records; an unreadable one counts as none yet.
fn revision_of(connection: &Connection) -> Result<u64, StoreError> {
    let value =
        connection.query_row("SELECT value FROM meta WHERE key = 'revision'", [], |row| {
            row.get(0)
        })?;
    Ok(revision_in(value))
}

/// The revision number in `meta`'s value for it.
fn revision_in(value: rusqlite::types::Value) -> u64 {
    match value {
        rusqlite::types::Value::Integer(revision) => u64::try_from(revision).unwrap_or(0),
        _ => 0,
    }
}

/// The identity in `meta`'s value for it; empty for an index made before ripscope wrote
/// one.
fn identity_in(value: rusqlite::types::Value) -> Vec<u8> {
    match value {
        rusqlite::types::Value::Blob(identity) => identity,
        _ => Vec::new(),
    }
}

/// What `--verify` says of an index whose block size cannot be read.
const NO_BLOCK_SIZE: &str = "the index's block size cannot be read";

/// The block size `meta` records, when it is a positive integer.
fn block_size(connection: &Connection) -> Result<Option<usize>, StoreError> {
    let value: Option<rusqlite::types::Value> = connection
        .query_row(
            "SELECT value FROM meta WHERE key = 'block size'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    Ok(match value {
        Some(rusqlite::types::Value::Integer(size)) => {
            usize::try_from(size).ok().filter(|size| *size > 0)
        }
        _ => None,
    })
}

/// A block of a stored unit's lines (ADR 0011): its number in the unit, the scope its
/// first line starts in, and its lines, an Analysis unit of their own.
pub(crate) struct Block {
    pub number: u32,
    macro_name: Option<Box<[u8]>>,
    function: Option<Box<[u8]>>,
    pub lines: AnalysisUnit,
}

impl Block {
    /// A whole unit as one block, from the file scope.
    pub(crate) fn whole(lines: AnalysisUnit) -> Self {
        Self {
            number: 0,
            macro_name: None,
            function: None,
            lines,
        }
    }

    /// The scope its first line starts in.
    pub(crate) fn start(&self) -> Scope<'_> {
        Scope {
            macro_name: self.macro_name.as_deref(),
            function: self.function.as_deref(),
        }
    }
}

/// Damage to a stored unit.
fn damaged_unit() -> StoreError {
    StoreError::Unit(UnitError::Damaged)
}

/// The id of unit `id`'s first block; none for an id too large to have blocks.
fn first_block(id: i64) -> Option<i64> {
    (0..1 << (63 - BLOCK_BITS))
        .contains(&id)
        .then_some(id << BLOCK_BITS)
}

/// A unit's stored count of blocks: damage when it has none or more than a unit can.
fn counted(blocks: i64) -> Result<u32, StoreError> {
    u32::try_from(blocks)
        .ok()
        .filter(|blocks| (1..=1 << BLOCK_BITS).contains(blocks))
        .ok_or_else(damaged_unit)
}

/// How many blocks unit `id` has: damage when it has no row, no block or too many.
fn block_count(connection: &Connection, id: i64) -> Result<u32, StoreError> {
    let blocks: Option<i64> = connection
        .prepare_cached("SELECT blocks FROM units WHERE id = ?1")?
        .query_row([id], |row| row.get(0))
        .optional()?;
    counted(blocks.ok_or_else(damaged_unit)?)
}

/// Unit `id`'s blocks, each checked: all of them, in order, or those `which` lists, in its
/// order, given its `count` of blocks when the caller has read it. A block missing, or
/// beyond the unit's count (even one the index still holds), is damage, as is one that does
/// not decode. `lazily`, a block's lines are checked once something first asks whether
/// they are sound ([`AnalysisUnit::checked`]) instead of now.
fn read_blocks(
    connection: &Connection,
    id: i64,
    which: Option<&[u32]>,
    count: Option<u32>,
    lazily: bool,
) -> Result<Vec<Block>, StoreError> {
    profile_span!("read_decode_blocks");
    let first = first_block(id).ok_or_else(damaged_unit)?;
    let block = |number, row: &rusqlite::Row<'_>| -> Result<Block, StoreError> {
        let scope = |column| -> rusqlite::Result<Option<Box<[u8]>>> {
            Ok(row
                .get::<_, Option<Vec<u8>>>(column)?
                .map(Vec::into_boxed_slice))
        };
        let image: Vec<u8> = row.get("image")?;
        profile_count!("block_bytes_loaded", image.len());
        profile_count!("blocks_loaded", 1);
        Ok(Block {
            number,
            macro_name: scope("macro")?,
            function: scope("function")?,
            // For a query, checked whole on the thread that first walks it.
            lines: if lazily {
                AnalysisUnit::unchecked(image.into_boxed_slice())?
            } else {
                AnalysisUnit::from_image(image.into_boxed_slice())?
            },
        })
    };
    let mut blocks = Vec::new();
    if let Some(which) = which {
        let count = match count {
            Some(count) => count,
            None => block_count(connection, id)?,
        };
        let mut statement =
            connection.prepare_cached("SELECT macro, function, image FROM blocks WHERE id = ?1")?;
        for &number in which {
            if number >= count {
                return Err(damaged_unit());
            }
            let mut rows = statement.query([first + i64::from(number)])?;
            let row = rows.next()?.ok_or_else(damaged_unit)?;
            blocks.push(block(number, row)?);
        }
        return Ok(blocks);
    }
    // Every block the unit's range of ids holds: they must be blocks 0 to count - 1, in
    // order, and no more.
    let count = match count {
        Some(count) => count,
        None => block_count(connection, id)?,
    };
    let mut statement = connection.prepare_cached(
        "SELECT id, macro, function, image FROM blocks WHERE id BETWEEN ?1 AND ?1 + 1048575 \
         ORDER BY id",
    )?;
    let mut rows = statement.query([first])?;
    while let Some(row) = rows.next()? {
        let number = u32::try_from(blocks.len()).map_err(|_| damaged_unit())?;
        if row.get::<_, i64>(0)? != first + i64::from(number) || number >= count {
            return Err(damaged_unit());
        }
        blocks.push(block(number, row)?);
    }
    if blocks.len() != count as usize {
        return Err(damaged_unit());
    }
    Ok(blocks)
}

/// The names whose lists may hold the unit of `blocks`, as they tell: those in their
/// lines, and those of the scopes they start in.
fn names_of(blocks: &[Block]) -> Vec<&[u8]> {
    let mut names: Vec<&[u8]> = Vec::new();
    for block in blocks {
        names.extend(block.lines.postings().into_iter().map(|(name, _)| name));
        let start = block.start();
        names.extend([start.macro_name, start.function].into_iter().flatten());
    }
    names.sort_unstable();
    names.dedup();
    names
}

/// A digest of a name's posting in one unit, which `--verify` sums over every posting,
/// in any order: a mix of its words, cheap where a keyed hash would cost a second over the
/// Linux kernel's 30 million postings, and finished so that each input bit reaches every
/// output bit and damage does not cancel out by chance.
fn digest(
    name: &[u8],
    unit: i64,
    roles: u32,
    blocks: impl Iterator<Item = u32>,
    rows: &[u8],
) -> u64 {
    const K: u64 = 0x9e37_79b9_7f4a_7c15;
    let mut hash = (name.len() as u64).wrapping_mul(K);
    let mut mix = |word: u64| hash = (hash.rotate_left(26) ^ word).wrapping_mul(K);
    // Bytes go in eight at a time, the last word padded with zeros.
    fn words(bytes: &[u8]) -> impl Iterator<Item = u64> + '_ {
        let (words, rest) = bytes.as_chunks::<8>();
        let mut last = [0u8; 8];
        last[..rest.len()].copy_from_slice(rest);
        let last = u64::from_le_bytes(last);
        words
            .iter()
            .map(|word| u64::from_le_bytes(*word))
            .chain([last])
    }
    words(name).for_each(&mut mix);
    mix(unit as u64);
    mix(u64::from(roles));
    let mut count = 0;
    for block in blocks {
        mix(u64::from(block));
        count += 1;
    }
    mix(count);
    words(rows).for_each(&mut mix);
    mix(rows.len() as u64);
    // The finish of splitmix64.
    let mut z = hash;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// A digest of one include row, which `--verify` sums over every row in any order, as
/// [`digest`] does for postings.
fn include_digest(
    unit: i64,
    position: i64,
    quoted: bool,
    block: u32,
    include: Recorded<'_>,
) -> u64 {
    let (operand, at, after, text) = include;
    let place = [position as u32, block, at.line, at.record, at.name, after];
    digest(operand, unit, u32::from(quoted), place.into_iter(), text)
}

/// An include row's operand, place, resuming name and text, as [`include_digest`] reads it.
type Recorded<'r> = (&'r [u8], Placed, u32, &'r [u8]);

/// One revision, read in one transaction.
pub(crate) struct Snapshot<'s> {
    transaction: rusqlite::Transaction<'s>,
    current: Current,
}

impl Snapshot<'_> {
    pub(crate) fn revision(&self) -> u64 {
        self.current.revision
    }

    pub(crate) fn selection(&self) -> &[u8] {
        &self.current.selection
    }

    /// The indexed files, in database order.
    pub(crate) fn files(&self) -> &[StoredFile] {
        &self.current.files
    }

    /// The analyzer that produced the revision's units.
    pub(crate) fn analyzer(&self) -> &[u8] {
        &self.current.analyzer
    }

    /// A stored unit, its blocks joined.
    pub(crate) fn unit(&self, id: i64) -> Result<AnalysisUnit, StoreError> {
        let blocks = read_blocks(&self.transaction, id, None, None, false)?;
        Ok(AnalysisUnit::join(blocks.iter().map(|block| &block.lines))?)
    }

    /// `--verify`'s check of the stored analyses, in one pass over the units: each unit's
    /// blocks run from 0 without a gap, decode, and are those that splitting its lines
    /// again with the index's block size gives, start scopes included; no block belongs
    /// to no unit; and the name postings are exactly those of the units, compared by an
    /// order-independent digest of both. Gives the problems found: the indexed files
    /// whose analysis is damaged, in database order, and only when there are none,
    /// postings that do not match.
    pub(crate) fn check_analyses(&self) -> Result<Vec<String>, StoreError> {
        let Some(size) = block_size(&self.transaction)? else {
            return Ok(vec![NO_BLOCK_SIZE.to_owned()]);
        };
        let mut problems = Vec::new();
        let strays: i64 = self.transaction.query_row(
            "SELECT count(*) FROM blocks WHERE NOT EXISTS (SELECT 1 FROM units \
             WHERE units.id = blocks.id >> 20 AND (blocks.id & 1048575) < units.blocks)",
            [],
            |row| row.get(0),
        )?;
        if strays > 0 {
            problems.push("the index holds blocks of no analysis".to_owned());
        }
        let indexed = self.current.files.iter().map(|file| &file.path[..]);
        let listed = Paths::decode(meta_blob(&self.transaction, "paths")?);
        if listed.is_none_or(|listed| !listed.iter().eq(indexed)) {
            problems.push("the path list does not match the indexed files".to_owned());
        }
        let (mut derived, mut derived_count) = (0u64, 0u64);
        let (mut derived_includes, mut derived_include_count) = (0u64, 0u64);
        let mut damaged = HashMap::default();
        let units: Vec<i64> = self
            .transaction
            .prepare("SELECT id FROM units")?
            .query_map([], |row| row.get(0))?
            .collect::<Result<_, _>>()?;
        for id in units {
            let checked =
                read_blocks(&self.transaction, id, None, None, false).and_then(|blocks| {
                    let blocks: Vec<_> = blocks
                        .iter()
                        .map(|block| (&block.lines, block.start()))
                        .collect();
                    let postings =
                        AnalysisUnit::check_split(&blocks, size).ok_or_else(damaged_unit)?;
                    let mut position = 0;
                    for (number, (lines, _)) in (0u32..).zip(&blocks) {
                        for placed in lines.placed_includes() {
                            let recorded = (placed.operand, placed.at, placed.after, placed.text);
                            let include =
                                include_digest(id, position, placed.quoted, number, recorded);
                            derived_includes = derived_includes.wrapping_add(include);
                            derived_include_count += 1;
                            position += 1;
                        }
                    }
                    for (name, roles, blocks, rows) in postings.iter() {
                        let blocks = blocks.iter().copied();
                        derived = derived.wrapping_add(digest(name, id, roles, blocks, rows));
                        derived_count += 1;
                    }
                    Ok(())
                });
            match checked {
                Ok(()) => {}
                Err(StoreError::Unit(error)) => {
                    damaged.insert(id, error);
                }
                Err(error) => return Err(error),
            }
        }
        for file in &self.current.files {
            if let Some(error) = damaged.get(&file.unit) {
                let path = String::from_utf8_lossy(&file.path);
                problems.push(format!("{path}: damaged analysis: {error}"));
            }
        }
        if !damaged.is_empty() {
            if problems.is_empty() {
                problems.push("the index holds damaged analyses no file uses".to_owned());
            }
            return Ok(problems);
        }
        let (mut stored, mut stored_count) = (0u64, 0u64);
        let mut names = self
            .transaction
            .prepare("SELECT name, postings FROM names")?;
        let mut rows = names.query([])?;
        let mut decoded = true;
        while let Some(row) = rows.next()? {
            let blob = |column| {
                row.get_ref(column)?
                    .as_blob()
                    .map_err(rusqlite::Error::from)
            };
            let name = blob(0)?;
            let Some(list) = postings::decode(blob(1)?) else {
                decoded = false;
                break;
            };
            for posting in list {
                let blocks = posting.blocks();
                let rows = posting.rows();
                stored =
                    stored.wrapping_add(digest(name, posting.unit, posting.roles, blocks, rows));
                stored_count += 1;
            }
        }
        if !decoded || (derived, derived_count) != (stored, stored_count) {
            problems.push("the name postings do not match the stored analyses".to_owned());
        }
        drop(rows);
        let (mut stored_includes, mut stored_include_count) = (0u64, 0u64);
        let mut includes = self.transaction.prepare(
            "SELECT unit, operands.operand, block, line, record, name, after, text, position, \
             quoted FROM includes JOIN operands ON operands.id = includes.operand",
        )?;
        let mut rows = includes.query([])?;
        while let Some(row) = rows.next()? {
            let unit: i64 = row.get(0)?;
            let (position, quoted): (i64, i64) = (row.get(8)?, row.get(9)?);
            let read = included_of(row, 2).and_then(|included| Ok((operand_of(row)?, included)));
            let Ok((operand, included)) = read else {
                stored_include_count = u64::MAX;
                break;
            };
            let recorded = (operand, included.at, included.after, &included.text[..]);
            let include = include_digest(unit, position, quoted == 1, included.block, recorded);
            stored_includes = stored_includes.wrapping_add(include);
            stored_include_count += 1;
        }
        if (derived_includes, derived_include_count) != (stored_includes, stored_include_count) {
            problems.push("the include records do not match the stored analyses".to_owned());
        }
        Ok(problems)
    }
}

/// A deferred read transaction that rolls back when dropped, as rusqlite's does, with its
/// `BEGIN` and `ROLLBACK` statements kept for reuse when `cached`.
struct ReadTransaction<'c> {
    connection: &'c Connection,
    cached: bool,
}

impl<'c> ReadTransaction<'c> {
    fn begin(connection: &'c Connection, cached: bool) -> Result<Self, StoreError> {
        with_statement(connection, cached, "BEGIN", |statement| {
            statement.execute([])
        })?;
        Ok(Self { connection, cached })
    }
}

impl std::ops::Deref for ReadTransaction<'_> {
    type Target = Connection;

    fn deref(&self) -> &Connection {
        self.connection
    }
}

impl Drop for ReadTransaction<'_> {
    fn drop(&mut self) {
        // SQLite ends the transaction itself after some errors.
        if !self.connection.is_autocommit() {
            let _ = with_statement(self.connection, self.cached, "ROLLBACK", |statement| {
                statement.execute([])
            });
        }
    }
}

/// Runs `f` on `sql`, compiled once and kept for reuse when `cached`.
fn with_statement<T>(
    connection: &Connection,
    cached: bool,
    sql: &str,
    f: impl FnOnce(&mut rusqlite::Statement<'_>) -> rusqlite::Result<T>,
) -> rusqlite::Result<T> {
    if cached {
        f(&mut *connection.prepare_cached(sql)?)
    } else {
        f(&mut connection.prepare(sql)?)
    }
}

/// One revision, read for one query in one transaction: its identity, selection rule and
/// analyzer, and on request the files the query needs, in database order.
pub(crate) struct Reader<'s> {
    transaction: ReadTransaction<'s>,
    id: RevisionId,
    selection: Vec<u8>,
    analyzer: Vec<u8>,
}

impl Reader<'_> {
    /// The revision's number and its index's identity.
    pub(crate) fn revision_id(&self) -> &RevisionId {
        &self.id
    }

    pub(crate) fn selection(&self) -> &[u8] {
        &self.selection
    }

    /// The analyzer that produced the revision's units.
    pub(crate) fn analyzer(&self) -> &[u8] {
        &self.analyzer
    }

    /// Every indexed path, in database order.
    pub(crate) fn paths(&self) -> Result<Paths, StoreError> {
        profile_span!("read_paths");
        let bytes = meta_blob(&self.transaction, "paths")?;
        Paths::decode(bytes).ok_or(StoreError::Paths)
    }

    /// Every indexed file and its unit's id, in database order.
    pub(crate) fn files(&self) -> Result<Files, StoreError> {
        let mut statement = self
            .transaction
            .prepare_cached("SELECT path, unit FROM files ORDER BY ordinal")?;
        let files = statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<Result<_, _>>()?;
        Ok(files)
    }

    /// The files whose unit holds `name` in one of the roles `mask` gives
    /// ([`crate::unit::Mark::role`]), with their unit's id, in database order, and for each
    /// such unit the blocks its postings list for the name: those a query for it reads.
    /// Its list unread, for anything but a busy index, is [`StoreError::Postings`].
    #[cfg(test)]
    pub(crate) fn holding(&self, name: &[u8], mask: u32) -> Result<(Files, Listed), StoreError> {
        let listed = self.holding_listed(name, mask)?;
        let units = listed.keys().copied().collect();
        Ok((self.files_of(&units)?, listed))
    }

    /// [`Self::holding`]'s units, each with the blocks its postings list for the name.
    #[cfg(test)]
    pub(crate) fn holding_listed(&self, name: &[u8], mask: u32) -> Result<Listed, StoreError> {
        self.holding_rows(name, mask).map(|(listed, _)| listed)
    }

    /// [`Self::holding_listed`], with each unit's answer rows for the name
    /// ([`crate::unit::Occurrences`]).
    pub(crate) fn holding_rows(
        &self,
        name: &[u8],
        mask: u32,
    ) -> Result<(Listed, HashMap<i64, Vec<u8>>), StoreError> {
        let listed = || -> Result<(Listed, HashMap<i64, Vec<u8>>), StoreError> {
            let stored: Option<Vec<u8>> = self
                .transaction
                .prepare_cached("SELECT postings FROM names WHERE name = ?1")?
                .query_row([name], |row| row.get(0))
                .optional()?;
            let Some(stored) = stored else {
                return Ok(Default::default());
            };
            let list = postings::decode(&stored).ok_or_else(postings::damaged)?;
            let (mut listed, mut rows) = (Listed::default(), HashMap::default());
            for posting in list.into_iter().filter(|posting| posting.roles & mask != 0) {
                listed.insert(posting.unit, posting.blocks().collect());
                rows.insert(posting.unit, posting.rows().to_vec());
            }
            Ok((listed, rows))
        };
        listed().map_err(postings::unread)
    }

    /// The units that hold a name in one of the roles `wanted` gives for it (none
    /// for a name the query does not read): one
    /// pass over every name, or over those starting with one of `prefixes` when every name
    /// the query reads does, decoding only the lists of those it reads. Names or lists
    /// unread, for anything but a busy index, are [`StoreError::Postings`].
    pub(crate) fn holding_any_units(
        &self,
        prefixes: Option<&[Vec<u8>]>,
        mut wanted: impl FnMut(&[u8]) -> u32,
    ) -> Result<HashSet<i64>, StoreError> {
        profile_span!("holding_any");
        let mut listed = || -> Result<HashSet<i64>, StoreError> {
            let mut units = HashSet::default();
            let mut read = |rows: &mut rusqlite::Rows<'_>| -> Result<(), StoreError> {
                while let Some(row) = rows.next()? {
                    let blob = |column| {
                        row.get_ref(column)?
                            .as_blob()
                            .map_err(rusqlite::Error::from)
                    };
                    let mask = wanted(blob(0)?);
                    if mask == 0 {
                        continue;
                    }
                    let list = postings::decode(blob(1)?).ok_or_else(postings::damaged)?;
                    units.extend(
                        list.into_iter()
                            .filter(|posting| posting.roles & mask != 0)
                            .map(|posting| posting.unit),
                    );
                }
                Ok(())
            };
            let Some(prefixes) = prefixes else {
                let mut statement = self
                    .transaction
                    .prepare_cached("SELECT name, postings FROM names")?;
                read(&mut statement.query([])?)?;
                return Ok(units);
            };
            for prefix in prefixes {
                match after_prefix(prefix) {
                    Some(after) => {
                        let mut statement = self.transaction.prepare_cached(
                            "SELECT name, postings FROM names WHERE name >= ?1 AND name < ?2",
                        )?;
                        read(&mut statement.query((prefix, &after))?)?;
                    }
                    None => {
                        let mut statement = self
                            .transaction
                            .prepare_cached("SELECT name, postings FROM names WHERE name >= ?1")?;
                        read(&mut statement.query([prefix])?)?;
                    }
                }
            }
            Ok(units)
        };
        listed().map_err(postings::unread)
    }

    /// The files whose unit is one of `units`, with their unit's id, in database order:
    /// looked up one unit at a time for a few, else picked out of every file.
    pub(crate) fn files_of(&self, units: &HashSet<i64>) -> Result<Files, StoreError> {
        profile_span!("select_files");
        if units.len() > 2_000 {
            let mut files = self.files()?;
            files.retain(|(_, unit)| units.contains(unit));
            return Ok(files);
        }
        let mut statement = self
            .transaction
            .prepare_cached("SELECT ordinal, path FROM files WHERE unit = ?1")?;
        let mut found: Vec<(i64, Vec<u8>, i64)> = Vec::new();
        for &unit in units {
            let rows = statement.query_map([unit], |row| Ok((row.get(0)?, row.get(1)?, unit)))?;
            for row in rows {
                found.push(row?);
            }
        }
        found.sort_unstable_by_key(|(ordinal, _, _)| *ordinal);
        Ok(found
            .into_iter()
            .map(|(_, path, unit)| (path, unit))
            .collect())
    }

    /// The files whose unit includes an operand (without its delimiters) that `matches`
    /// accepts, with their unit's id, in database order. Each distinct operand is asked
    /// once.
    pub(crate) fn including(
        &self,
        matches: impl FnMut(&[u8]) -> bool,
    ) -> Result<(Files, Includers), StoreError> {
        let mut includers = self.including_listed(matches)?;
        in_place(&mut includers);
        Ok((
            self.files_of(&includers.keys().copied().collect())?,
            includers,
        ))
    }

    /// [`Self::including`]'s units, each with where its matching includes sit.
    pub(crate) fn including_listed(
        &self,
        mut matches: impl FnMut(&[u8]) -> bool,
    ) -> Result<Includers, StoreError> {
        profile_span!("include_scan");
        let mut listed = Includers::default();
        for (id, operand) in self.operand_directory()? {
            if matches(&operand) {
                for (unit, included) in self.included_by(id)? {
                    listed.entry(unit).or_default().push(included);
                }
            }
        }
        Ok(listed)
    }

    /// Each distinct include operand (without its delimiters), with its id, in no
    /// particular order.
    pub(crate) fn operand_directory(&self) -> Result<Vec<(i64, Vec<u8>)>, StoreError> {
        let mut statement = self
            .transaction
            .prepare_cached("SELECT id, operand FROM operands")?;
        let mut rows = statement.query([])?;
        let mut operands = Vec::new();
        while let Some(row) = rows.next()? {
            operands.push((row.get(0)?, operand_of(row)?.to_vec()));
        }
        Ok(operands)
    }

    /// The includes of the operand with id `operand`: each with its unit and where it
    /// sits, in unit order.
    pub(crate) fn included_by(&self, operand: i64) -> Result<Vec<(i64, Included)>, StoreError> {
        let mut statement = self.transaction.prepare_cached(
            "SELECT unit, block, line, record, name, after, text FROM includes \
             WHERE operand = ?1 ORDER BY operand, unit, block, line, record, name",
        )?;
        let mut rows = statement.query([operand])?;
        let mut included = Vec::new();
        while let Some(row) = rows.next()? {
            included.push((row.get(0)?, included_of(row, 1)?));
        }
        Ok(included)
    }

    /// Every file and its unit's id, with its place in database order.
    pub(crate) fn every_file(&self) -> Result<Vec<(i64, Vec<u8>, i64)>, StoreError> {
        let mut statement = self
            .transaction
            .prepare_cached("SELECT ordinal, path, unit FROM files")?;
        let files = statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
            .collect::<Result<_, _>>()?;
        Ok(files)
    }

    /// How many files are indexed.
    pub(crate) fn count(&self) -> Result<usize, StoreError> {
        let count: i64 = self
            .transaction
            .query_row("SELECT count(*) FROM files", [], |row| row.get(0))?;
        Ok(usize::try_from(count).unwrap_or_default())
    }

    /// A stored unit's key, the same in any index for the same analysis, and how many
    /// blocks hold its lines.
    pub(crate) fn key(&self, id: i64) -> Result<(Vec<u8>, u32), StoreError> {
        let row: Option<(Vec<u8>, i64)> = self
            .transaction
            .prepare_cached("SELECT key, blocks FROM units WHERE id = ?1")?
            .query_row([id], |row| Ok((row.get(0)?, row.get(1)?)))
            .optional()?;
        let (key, blocks) = row.ok_or_else(damaged_unit)?;
        Ok((key, counted(blocks)?))
    }

    /// A stored unit's blocks, each checked: all of them, in order, or those `which`
    /// lists, in its order, given its `count` of blocks when the caller has read it. A
    /// block missing, beyond the unit's count, or that does not decode is damage.
    pub(crate) fn blocks(
        &self,
        id: i64,
        which: Option<&[u32]>,
        count: Option<u32>,
    ) -> Result<Vec<Block>, StoreError> {
        read_blocks(&self.transaction, id, which, count, true)
    }
}

/// An include row's operand, borrowed: column 1, a blob in an intact index.
fn operand_of<'r>(row: &'r rusqlite::Row<'_>) -> Result<&'r [u8], rusqlite::Error> {
    match row.get_ref(1)? {
        rusqlite::types::ValueRef::Blob(operand) => Ok(operand),
        other => Err(rusqlite::Error::InvalidColumnType(
            1,
            "operand".into(),
            other.data_type(),
        )),
    }
}

/// Where an include row's operand sits and its includer row: its block, within a unit's
/// 2^20 blocks, line, record and name, then the name reading resumes at and the row's
/// text, in the six columns from `first`.
fn included_of(row: &rusqlite::Row<'_>, first: usize) -> Result<Included, StoreError> {
    let number = |column| -> Result<u32, StoreError> {
        let value: i64 = row.get(first + column)?;
        u32::try_from(value).map_err(|_| damaged_unit())
    };
    let text = match row.get_ref(first + 5)? {
        rusqlite::types::ValueRef::Blob(text) => text,
        _ => return Err(damaged_unit()),
    };
    Ok(Included {
        block: Some(number(0)?)
            .filter(|block| *block < 1 << BLOCK_BITS)
            .ok_or_else(damaged_unit)?,
        at: Placed {
            line: number(1)?,
            record: number(2)?,
            name: number(3)?,
        },
        after: number(4)?,
        text: text.into(),
    })
}

/// The first bytes after every byte string that starts with `prefix`, in SQLite's order
/// of blobs: none when the prefix is all 0xFF bytes.
fn after_prefix(prefix: &[u8]) -> Option<Vec<u8>> {
    let last = prefix.iter().rposition(|byte| *byte != u8::MAX)?;
    let mut after = prefix[..=last].to_vec();
    after[last] += 1;
    Some(after)
}

/// Distinct include operands, each with the units that include it and the number of the
/// block that does.
/// Where an include sits, its block and its place in the block ([`Placed`]), and its
/// includer row ([`crate::unit::PlacedInclude`]).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Included {
    pub block: u32,
    pub at: Placed,
    pub after: u32,
    pub text: std::sync::Arc<[u8]>,
}

/// For each unit with a matching include, where each sits.
pub(crate) type Includers = HashMap<i64, Vec<Included>>;

/// Puts each unit's includes in `includers` in order, without repeats.
pub(crate) fn in_place(includers: &mut Includers) {
    for included in includers.values_mut() {
        included.sort_unstable();
        included.dedup();
    }
}

/// Indexed files, each with its unit's id, in database order.
pub(crate) type Files = Vec<(Vec<u8>, i64)>;

/// For each unit an exact name's postings hold, the blocks they list for it, in order.
pub(crate) type Listed = HashMap<i64, Vec<u32>>;

/// An update in progress: the committed revision it starts from, and the write lock.
pub(crate) struct Update<'s> {
    transaction: rusqlite::Transaction<'s>,
    current: Current,
    /// Every stored unit was dropped, so the update publishes whatever it finds.
    discarded: bool,
    /// The name postings of the units it adds and drops, yet to be stored.
    pending: postings::Pending,
    /// The index holds no name postings yet (an index build afresh, or a discarding
    /// update), so the first batch has no list to read before writing one.
    fresh: bool,
    /// The index's block size, which the units it adds are split by; `None` when it
    /// cannot be read, so that only an update that discards every unit adds any.
    block_size: Option<usize>,
    /// What this update's Refresh observed, which `finish` records with the revision.
    observing: Option<Vec<u8>>,
}

impl Update<'_> {
    /// The published selection rule seen after acquiring the write lock; `None` when
    /// this transaction starts from an index that has published nothing yet.
    pub(crate) fn selection(&self) -> Option<&[u8]> {
        (self.current.revision != 0).then_some(&self.current.selection)
    }

    /// The revision number being updated; 0 for an index nothing was published to.
    pub(crate) fn revision(&self) -> u64 {
        self.current.revision
    }

    /// The revision being updated.
    pub(crate) fn files(&self) -> &[StoredFile] {
        &self.current.files
    }

    /// What the clean Refresh that left the current revision observed of the selection
    /// and the file system, unless every unit was dropped since: a Refresh that observes
    /// the same, with every file unchanged, would publish the same revision.
    pub(crate) fn observed(&self) -> Option<&[u8]> {
        let stored = self
            .current
            .observed
            .as_deref()
            .filter(|_| !self.discarded)?;
        let (revision, rest) = stored.split_at_checked(8)?;
        let (files, observed) = rest.split_at_checked(32)?;
        (revision == self.current.revision.to_le_bytes()
            && files == files_digest(&self.current.files))
        .then_some(observed)
    }

    /// Records what this Refresh observed with the revision it finishes, or that it was
    /// not clean.
    pub(crate) fn observing(&mut self, observed: Option<Vec<u8>>) {
        self.observing = observed;
    }

    /// The analyzer that produced the revision's units.
    pub(crate) fn analyzer(&self) -> &[u8] {
        &self.current.analyzer
    }

    /// Stores the name postings in batches of `batch`, so that tests cross batches; set
    /// before the update collects any.
    #[cfg(test)]
    pub(crate) fn batch(&mut self, batch: usize) {
        assert!(
            self.pending.is_empty(),
            "the batch is set before postings are collected"
        );
        self.pending = postings::Pending::with_batch(batch);
    }

    /// Drops every stored unit, so each file is analysed again and nothing damaged is
    /// kept (an index build with `-u`, or another analyzer's units). It gives the index a
    /// new identity, since sessions keep blocks by identity and unit key, and the blocks
    /// of a key it had may now be split by another size; and an index whose block size
    /// cannot be read the one a new index gets.
    pub(crate) fn discard(&mut self) -> Result<(), StoreError> {
        self.transaction.execute_batch(
            "DELETE FROM files; DELETE FROM names; DELETE FROM blocks; DELETE FROM units;
             DELETE FROM operands;
             INSERT OR REPLACE INTO meta VALUES ('identity', randomblob(16));",
        )?;
        if self.block_size.is_none() {
            let size = new_block_size();
            self.transaction.execute_batch(&format!(
                "INSERT OR REPLACE INTO meta VALUES ('block size', {size});"
            ))?;
            self.block_size = Some(size);
        }
        self.discarded = true;
        self.fresh = true;
        self.pending = postings::Pending::with_batch(self.pending.batch());
        Ok(())
    }

    /// Every stored unit's include operands, in order, read in one pass.
    pub(crate) fn every_include(
        &self,
    ) -> Result<crate::hashing::HashMap<i64, Vec<Include>>, StoreError> {
        let mut statement = self.transaction.prepare_cached(
            "SELECT unit, operands.operand, quoted FROM includes \
                 JOIN operands ON operands.id = includes.operand ORDER BY unit, position",
        )?;
        let mut rows = statement.query([])?;
        let mut every = crate::hashing::HashMap::<i64, Vec<Include>>::default();
        while let Some(row) = rows.next()? {
            let include = (row.get(1)?, row.get::<_, i64>(2)? != 0);
            every.entry(row.get(0)?).or_default().push(include);
        }
        Ok(every)
    }

    /// A stored unit's include operands, in order.
    #[cfg(test)]
    pub(crate) fn includes(&self, unit: i64) -> Result<Vec<Include>, StoreError> {
        let mut statement = self.transaction.prepare_cached(
            "SELECT operands.operand, quoted FROM includes \
             JOIN operands ON operands.id = includes.operand WHERE unit = ?1 ORDER BY position",
        )?;
        let includes = statement
            .query_map([unit], |row| Ok((row.get(0)?, row.get::<_, i64>(1)? != 0)))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(includes)
    }

    /// The size units are split into blocks by; [`StoreError::BlockSize`] when the index
    /// has no readable one.
    pub(crate) fn block_size(&self) -> Result<usize, StoreError> {
        self.block_size.ok_or(StoreError::BlockSize)
    }

    /// Stores a unit under `key` as [`Update::add_split`] does, unless one is stored
    /// already, splitting it only then.
    #[cfg(test)]
    pub(crate) fn add_unit(
        &mut self,
        key: &[u8; 32],
        unit: &AnalysisUnit,
    ) -> Result<i64, StoreError> {
        if let Some(id) = self.unit_of(key)? {
            return Ok(id);
        }
        let split = unit.split(self.block_size()?);
        self.insert_split(key, &split)
    }

    /// Stores a unit under `key`, its lines in the blocks `split` gives for
    /// [`Update::block_size`] with their start scopes, with its include operands and name
    /// postings, unless one is stored already; returns its id. A unit that needs more
    /// than 2^20 blocks, or an id too large for its blocks, is too large.
    pub(crate) fn add_split(
        &mut self,
        key: &[u8; 32],
        split: &Split<'_>,
    ) -> Result<i64, StoreError> {
        if let Some(id) = self.unit_of(key)? {
            return Ok(id);
        }
        self.insert_split(key, split)
    }

    /// The id of the unit stored under `key`, if one is.
    fn unit_of(&self, key: &[u8; 32]) -> Result<Option<i64>, StoreError> {
        Ok(self
            .transaction
            .prepare_cached("SELECT id FROM units WHERE key = ?1")?
            .query_row([&key[..]], |row| row.get(0))
            .optional()?)
    }

    fn insert_split(&mut self, key: &[u8; 32], split: &Split<'_>) -> Result<i64, StoreError> {
        let too_large = || StoreError::Unit(UnitError::TooLarge);
        if split.blocks.len() > 1 << BLOCK_BITS {
            return Err(too_large());
        }
        self.transaction
            .prepare_cached("INSERT INTO units (key, blocks) VALUES (?1, ?2)")?
            .execute(params![&key[..], split.blocks.len() as i64])?;
        let id = self.transaction.last_insert_rowid();
        let first = first_block(id).ok_or_else(too_large)?;
        let mut insert = self.transaction.prepare_cached(
            "INSERT INTO blocks (id, macro, function, image) VALUES (?1, ?2, ?3, ?4)",
        )?;
        for (number, (block, start)) in (0..).zip(&split.blocks) {
            insert.execute(params![
                first + number,
                start.macro_name,
                start.function,
                block.image()
            ])?;
        }
        let mut insert = self.transaction.prepare_cached(
            "INSERT INTO includes (unit, position, operand, quoted, block, line, record, name, \
             after, text) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        )?;
        let mut known = self
            .transaction
            .prepare_cached("SELECT id FROM operands WHERE operand = ?1")?;
        let mut added = self
            .transaction
            .prepare_cached("INSERT INTO operands (operand) VALUES (?1)")?;
        // A unit's includes are its blocks' includes, in order.
        let mut position = 0i64;
        for (number, (block, _)) in (0i64..).zip(&split.blocks) {
            for placed in block.placed_includes() {
                let operand = match known.query_row([placed.operand], |row| row.get::<_, i64>(0)) {
                    Ok(id) => id,
                    Err(rusqlite::Error::QueryReturnedNoRows) => {
                        added.execute([placed.operand])?;
                        self.transaction.last_insert_rowid()
                    }
                    Err(error) => return Err(error.into()),
                };
                insert.execute(params![
                    id,
                    position,
                    operand,
                    i64::from(placed.quoted),
                    number,
                    placed.at.line,
                    placed.at.record,
                    placed.at.name,
                    placed.after,
                    placed.text
                ])?;
                position += 1;
            }
        }
        for (name, roles, blocks, rows) in split.postings.iter() {
            self.pending.add(name, id, roles, blocks, rows);
        }
        if self.pending.full() {
            self.pending.store(&self.transaction, self.fresh)?;
            self.fresh = false;
        }
        Ok(id)
    }

    /// Commits `files` (in database order) as the next revision, or keeps the current one
    /// when nothing differs but when files were last read, which it records in place:
    /// readers never use it.
    pub(crate) fn finish(
        mut self,
        selection: &[u8],
        analyzer: &[u8],
        files: &[StoredFile],
    ) -> Result<Published, StoreError> {
        let current = &self.current;
        if !self.discarded
            && current.selection == selection
            && current.analyzer == analyzer
            && current.files.len() == files.len()
            && current
                .files
                .iter()
                .zip(files)
                .all(|(stored, file)| stored.same_facts(file))
        {
            let read_again: Vec<(usize, i64)> = current
                .files
                .iter()
                .zip(files)
                .enumerate()
                .filter(|(_, (stored, file))| stored.verified != file.verified)
                .map(|(ordinal, (_, file))| (ordinal, file.verified))
                .collect();
            let observing = self.observing.as_deref().map(|observing| {
                [
                    &current.revision.to_le_bytes()[..],
                    &files_digest(files),
                    observing,
                ]
                .concat()
            });
            let observed = self.current.observed != observing;
            let stamped = !read_again.is_empty();
            if observed {
                record_observed(&self.transaction, observing.as_deref())?;
            }
            if !read_again.is_empty() {
                // The postings of any unit added go in with it.
                self.pending.store(&self.transaction, self.fresh)?;
                let mut update = self
                    .transaction
                    .prepare("UPDATE files SET verified = ?2 WHERE ordinal = ?1")?;
                for (ordinal, verified) in read_again {
                    update.execute(params![
                        i64::try_from(ordinal).unwrap_or(i64::MAX),
                        verified
                    ])?;
                }
                drop(update);
            }
            if observed || stamped {
                self.transaction.commit()?;
            }
            return Ok(Published::Unchanged(current.revision));
        }
        let transaction = self.transaction;
        let observing = self.observing.as_deref().map(|observing| {
            let next = self.current.revision + 1;
            [&next.to_le_bytes()[..], &files_digest(files), observing].concat()
        });
        record_observed(&transaction, observing.as_deref())?;
        transaction.execute("DELETE FROM files", [])?;
        {
            let mut insert = transaction.prepare(
                "INSERT INTO files (ordinal, path, unit, size, modified, changed, identity, hash,
                 verified) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            )?;
            for (ordinal, file) in files.iter().enumerate() {
                insert.execute(params![
                    i64::try_from(ordinal).unwrap_or(i64::MAX),
                    file.path,
                    file.unit,
                    i64::try_from(file.stat.size).unwrap_or(i64::MAX),
                    file.stat.modified,
                    file.stat.changed,
                    file.stat.identity,
                    &file.hash[..],
                    file.verified,
                ])?;
            }
        }
        drop_orphans(&transaction, &mut self.pending, self.fresh)?;
        let next = self.current.revision + 1;
        let set = |key: &str, value: &dyn rusqlite::ToSql| {
            transaction.execute(
                "UPDATE meta SET value = ?2 WHERE key = ?1",
                params![key, value],
            )
        };
        set("revision", &i64::try_from(next).unwrap_or(i64::MAX))?;
        set(
            "paths",
            &Paths::encode(files.iter().map(|file| &file.path[..])),
        )?;
        set("selection", &selection)?;
        set("analyzer", &analyzer)?;
        transaction.commit()?;
        Ok(Published::Revision(next))
    }
}

/// Deletes the units no file uses any more, with their blocks and name postings, and
/// stores these removals with the postings `pending` holds, in one pass: each unit's names
/// found from its blocks, or, for units whose blocks cannot be read, searched for in every
/// list at once.
fn drop_orphans(
    transaction: &rusqlite::Transaction<'_>,
    pending: &mut postings::Pending,
    fresh: bool,
) -> Result<(), StoreError> {
    let orphans: Vec<i64> = transaction
        .prepare("SELECT id FROM units WHERE id NOT IN (SELECT unit FROM files)")?
        .query_map([], |row| row.get(0))?
        .collect::<Result<_, _>>()?;
    let mut undecodable = HashSet::default();
    for id in orphans {
        match read_blocks(transaction, id, None, None, false) {
            Ok(blocks) => {
                for name in names_of(&blocks) {
                    pending.remove(name, id);
                }
                if pending.full() {
                    pending.store(transaction, fresh)?;
                }
            }
            Err(StoreError::Unit(_)) => {
                undecodable.insert(id);
            }
            Err(error) => return Err(error),
        }
    }
    pending.store(transaction, fresh)?;
    if !undecodable.is_empty() {
        postings::purge(transaction, &undecodable)?;
    }
    transaction.execute_batch(
        "DELETE FROM units WHERE id NOT IN (SELECT unit FROM files);
         DELETE FROM operands WHERE NOT EXISTS
             (SELECT 1 FROM includes WHERE includes.operand = operands.id);",
    )?;
    Ok(())
}

/// An index path and what was at it when [`Store::inspect`] looked.
pub(crate) struct Inspected {
    index: IndexPath,
    state: FileState,
}

impl Inspected {
    /// Whether opening it for `access` makes a new index: the path is absent or an empty
    /// file, or an index build finds an index of another schema version there.
    pub(crate) fn makes_new(&self, access: Access) -> bool {
        match self.state {
            FileState::Absent | FileState::Empty => access != Access::Read,
            FileState::Index(version) => version != SCHEMA_VERSION && access == Access::Write,
            FileState::Foreign => false,
        }
    }

    /// Opens it for `access`, as what was at the path when it was looked at.
    pub(crate) fn open(self, access: Access) -> Result<Store, StoreError> {
        Store::open_at(&self.index, self.state, access)
    }
}

#[derive(Clone, Copy)]
enum FileState {
    Absent,
    Empty,
    /// A ripscope index of this schema version.
    Index(i32),
    Foreign,
}

/// What is at `path`, judged from the file's first bytes alone: a SQLite header with
/// ripscope's application id is an index.
fn state_at(path: &Path) -> Result<FileState, StoreError> {
    let mut file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(FileState::Absent);
        }
        Err(error) => return Err(StoreError::Io(error)),
    };
    if !file.metadata().map_err(StoreError::Io)?.is_file() {
        return Ok(FileState::Foreign);
    }
    let mut header = [0u8; 100];
    let mut read = 0;
    while read < header.len() {
        match file.read(&mut header[read..]).map_err(StoreError::Io)? {
            0 => break,
            count => read += count,
        }
    }
    if read == 0 {
        return Ok(FileState::Empty);
    }
    let sqlite = read == header.len() && header.starts_with(b"SQLite format 3\0");
    let field = |at: usize| {
        i32::from_be_bytes([header[at], header[at + 1], header[at + 2], header[at + 3]])
    };
    Ok(if sqlite && field(68) == APPLICATION_ID {
        FileState::Index(field(60))
    } else {
        FileState::Foreign
    })
}

/// Makes `connection` a reader's.
fn read_only(connection: &Connection) -> Result<(), StoreError> {
    // Readers may still recover the write-ahead log after a writer crashed.
    connection.execute_batch("PRAGMA query_only = ON;")?;
    // Leave the WAL files for the next query instead of checkpointing and deleting them on
    // every close. Writers keep their normal cleanup, and on Windows a Session lets go of
    // its handle after each query, so that index replacement can go ahead.
    connection.set_db_config(
        rusqlite::config::DbConfig::SQLITE_DBCONFIG_NO_CKPT_ON_CLOSE,
        true,
    )?;
    Ok(())
}

/// Opens the index file at `path` (the file itself, not a link to it).
fn open_index(path: &Path) -> Result<Connection, StoreError> {
    let connection = open_sqlite(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    connection.busy_timeout(OPEN_WAIT)?;
    connection.execute_batch(
        "PRAGMA foreign_keys = ON; PRAGMA trusted_schema = OFF;
         PRAGMA synchronous = FULL;",
    )?;
    connection.busy_timeout(Duration::ZERO)?;
    // A Session's queries and each request's transaction and metadata lookup already
    // fill the 16 statements rusqlite keeps by default, and a Refresh adds its own. Room
    // for all of them keeps any from being compiled again on every request.
    connection.set_prepared_statement_cache_capacity(32);
    Ok(connection)
}

/// Opens the path checked by `IndexPath` with SQLite. Windows needs its long-path
/// buffer as well as the extended-length spelling. That VFS keeps `win32`'s locking.
fn open_sqlite(path: &Path, flags: OpenFlags) -> Result<Connection, StoreError> {
    let path = for_sqlite(path);
    #[cfg(windows)]
    {
        Connection::open_with_flags_and_vfs(path, flags, "win32-longpath").map_err(StoreError::from)
    }
    #[cfg(not(windows))]
    Connection::open_with_flags(path, flags).map_err(StoreError::from)
}

/// How a new index goes into place.
#[derive(Clone, Copy)]
enum Placing {
    /// Where there is no index: never over one another process made meanwhile.
    New,
    /// Over an index of another schema version, or one SQLite cannot read.
    Replace,
}

/// Makes a new, empty index at `path`: written in full beside it and then moved into
/// place, so an interrupted creation never leaves a half-made file there. Where there was
/// no index, one that another process made meanwhile is used instead, so of two index
/// builds that start at once the second waits for the first one's update.
///
/// Through a symbolic link, the new index takes the place of the file the link names,
/// beside it, and the link stays: every update through a link writes that file.
fn create(index: &IndexPath, placing: Placing) -> Result<Connection, StoreError> {
    let path = index.file();
    // Anyone who may write the directory could move a new index over it, but one made by
    // a user who may not write the old one could lock its owner out.
    if matches!(placing, Placing::Replace) && !may_write(path) {
        return Err(StoreError::ReadOnly(index.given().to_owned()));
    }
    let staging = index.beside(&format!(".new-{}", std::process::id()));
    remove_database(&staging);
    let placed = write_new(&staging, path, placing).and_then(|()| match placing {
        Placing::New => place_new(&staging, path),
        Placing::Replace => replace_index(&staging, path),
    });
    remove_database(&staging);
    placed?;
    check_companions(index, false)?;
    open_index(path)
}

/// Writes a new, empty index at `staging`. The file is created here, before SQLite writes
/// it, so that it gets the permissions any new file gets, as cscope's database does (SQLite
/// would make it 0644, less the umask): a umask or a default ACL can share it with a group.
/// On Unix an index that replaces the one at `path` then takes that one's permissions:
/// its group and permission bits, and its owner where the process may give it away (as
/// root), as SQLite does for the files it keeps beside a database.
fn write_new(staging: &Path, path: &Path, placing: Placing) -> Result<(), StoreError> {
    // Kept open until SQLite has written and closed the file, so that the permissions
    // change through this handle.
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(staging)
        .map_err(StoreError::Io)?;
    write_empty(staging)?;
    #[cfg(unix)]
    if let Placing::Replace = placing {
        use std::os::unix::fs::{MetadataExt, PermissionsExt, fchown};
        let old = std::fs::metadata(path).map_err(StoreError::Io)?;
        // Best effort: only root may give a file away, and only a member of a group may
        // give it to that group.
        if fchown(&file, Some(old.uid()), Some(old.gid())).is_err() {
            let _ = fchown(&file, None, Some(old.gid()));
        }
        file.set_permissions(std::fs::Permissions::from_mode(old.mode() & 0o777))
            .map_err(StoreError::Io)?;
    }
    #[cfg(not(unix))]
    let _ = (file, path, placing);
    Ok(())
}

/// Moves the new index at `staging` over the index at `path`, whose log files go with it:
/// SQLite would replay a log left beside the new index into it, so the move happens only
/// once they are gone. On Windows none of them can be removed or replaced while another
/// process has it open (a query reading the old index, or a virus scanner reading the new
/// one), so the move waits up to ten seconds for them: a scanner busy with the files a
/// test suite makes held one for more than two. On Unix a log that cannot be
/// removed (another user's, in a directory with the sticky bit) stops it at once.
fn replace_index(staging: &Path, path: &Path) -> Result<(), StoreError> {
    let mut waits = 0;
    loop {
        remove_logs(path);
        let left = LOGS
            .iter()
            .map(|log| beside(path, log))
            .find(|log| std::fs::symlink_metadata(log).is_ok());
        let blocked = match left {
            Some(log) => log,
            None => match std::fs::rename(staging, path) {
                Ok(()) => return Ok(()),
                // Access denied, or a sharing violation (32).
                Err(error)
                    if cfg!(windows)
                        && (error.kind() == std::io::ErrorKind::PermissionDenied
                            || error.raw_os_error() == Some(32)) =>
                {
                    path.to_owned()
                }
                Err(error) => return Err(StoreError::Io(error)),
            },
        };
        if cfg!(unix) || waits == 200 {
            return Err(StoreError::InUse(blocked));
        }
        waits += 1;
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// The block size a new index records: [`BLOCK_SIZE`], or the positive number of bytes
/// `RIPSCOPE_BLOCK_SIZE` gives.
#[cfg(not(test))]
fn new_block_size() -> usize {
    std::env::var("RIPSCOPE_BLOCK_SIZE")
        .ok()
        .and_then(|size| size.parse::<u32>().ok())
        .filter(|size| *size > 0)
        .map_or(BLOCK_SIZE, |size| size as usize)
}

/// Unit tests set an index's block size themselves, whatever the environment holds.
#[cfg(test)]
fn new_block_size() -> usize {
    BLOCK_SIZE
}

/// Writes an empty index at `path`, its header, schema and log mode complete.
fn write_empty(path: &Path) -> Result<(), StoreError> {
    let connection = open_sqlite(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    let block_size = new_block_size();
    connection.execute_batch(&format!(
        "PRAGMA application_id = {APPLICATION_ID};
         BEGIN;
         {SCHEMA}
         INSERT INTO meta VALUES ('block size', {block_size});
         PRAGMA user_version = {SCHEMA_VERSION};
         COMMIT;
         PRAGMA journal_mode = WAL;
         PRAGMA wal_checkpoint(TRUNCATE);"
    ))?;
    connection
        .close()
        .map_err(|(_, error)| StoreError::from(error))
}

/// Puts the new index at `staging` where there is none, linking it so that an index
/// another process put there meanwhile is never replaced (a file system without links
/// moves it instead).
fn place_new(staging: &Path, path: &Path) -> Result<(), StoreError> {
    // An empty file at the path holds nothing to keep.
    if std::fs::metadata(path).is_ok_and(|metadata| metadata.is_file() && metadata.len() == 0) {
        let _ = std::fs::remove_file(path);
    }
    match std::fs::hard_link(staging, path) {
        Ok(()) => {
            // Log files left by an index deleted without them would be read as this one's.
            remove_logs(path);
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => match state_at(path)? {
            FileState::Index(version) if version == SCHEMA_VERSION => Ok(()),
            FileState::Index(version) => Err(StoreError::Version(version)),
            FileState::Absent | FileState::Empty | FileState::Foreign => {
                Err(StoreError::NotAnIndex(path.to_owned()))
            }
        },
        Err(_) => std::fs::rename(staging, path).map_err(StoreError::Io),
    }
}

/// A reader's failure to create SQLite's shared-memory file beside the index, told by its
/// cause.
fn explained(index: &IndexPath, error: StoreError) -> StoreError {
    match error {
        StoreError::Sqlite(ref sqlite)
            if matches!(
                sqlite.sqlite_error_code(),
                Some(rusqlite::ErrorCode::ReadOnly | rusqlite::ErrorCode::CannotOpen)
            ) && !(index.beside("-wal").exists() && index.beside("-shm").exists())
                && directory_writable(index.directory()) == Some(false) =>
        {
            StoreError::SharedMemory(index.file().to_owned())
        }
        error => error,
    }
}

/// Whether this process may write the file at `path`.
fn may_write(path: &Path) -> bool {
    #[cfg(unix)]
    {
        rustix::fs::access(path, rustix::fs::Access::WRITE_OK).is_ok()
    }
    #[cfg(not(unix))]
    {
        std::fs::OpenOptions::new().write(true).open(path).is_ok()
    }
}

/// Whether this process may create files in `directory`, when this can be checked
/// without writing there. On Windows keep SQLite's error instead of guessing its cause.
fn directory_writable(directory: &Path) -> Option<bool> {
    #[cfg(unix)]
    {
        Some(rustix::fs::access(directory, rustix::fs::Access::WRITE_OK).is_ok())
    }
    #[cfg(not(unix))]
    {
        let _ = directory;
        None
    }
}

/// SQLite's log files beside the index, before SQLite opens anything: each must be absent
/// or a plain file (a regular file with one name, not a link). SQLite writes through a
/// hard link, and on Windows through a symbolic link, so it would empty or overwrite
/// another file; anything else refuses the index, for a query as for an index build.
///
/// With `repair`, for a process that may write the index: SQLite gives the files it keeps
/// beside the index the index file's permissions, so on Unix a reader of an index file it
/// could not write leaves them read-only, and every later update would fail. They are
/// made writable again, with the index file's read and write bits (on Windows its
/// attributes), as SQLite would have made them, through the handle that was inspected.
#[cfg(unix)]
fn check_companions(index: &IndexPath, repair: bool) -> Result<(), StoreError> {
    for suffix in ["-wal", "-shm"] {
        check_companion(index, &index.beside(suffix), repair)?;
    }
    Ok(())
}

#[cfg(unix)]
fn check_companion(index: &IndexPath, companion: &Path, repair: bool) -> Result<(), StoreError> {
    use rustix::fs::{Access, FileType, Mode, OFlags};
    use std::os::unix::fs::PermissionsExt;
    // Last-close cleanup may unlink an opened companion before fstat. Recheck
    // its path, at most three times in total, without weakening link checks.
    for _ in 0..3 {
        // Without following a link, and non-blocking, so a FIFO cannot hold the open.
        let flags = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC;
        let file = match rustix::fs::open(companion, flags, Mode::empty()) {
            Ok(file) => file,
            Err(rustix::io::Errno::NOENT) => return Ok(()),
            // A link, on Linux (other systems give other errors, left to SQLite).
            Err(rustix::io::Errno::LOOP) => return Err(StoreError::Unsafe(companion.to_owned())),
            // SQLite will say what is wrong with a file it cannot open either.
            Err(_) => return Ok(()),
        };
        #[cfg(test)]
        if let Some(gate) = BEFORE_COMPANION_STAT.with(|slot| slot.borrow_mut().take()) {
            gate();
        }
        let stat = rustix::fs::fstat(&file).map_err(|error| StoreError::Io(error.into()))?;
        if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile {
            return Err(StoreError::Unsafe(companion.to_owned()));
        }
        if stat.st_nlink == 0 {
            drop(file);
            continue;
        }
        if stat.st_nlink != 1 {
            return Err(StoreError::Unsafe(companion.to_owned()));
        }
        if !repair || rustix::fs::access(companion, Access::WRITE_OK).is_ok() {
            return Ok(());
        }
        let mode = std::fs::metadata(index.file())
            .map_err(StoreError::Io)?
            .permissions()
            .mode()
            & 0o777;
        rustix::fs::fchmod(&file, Mode::from_raw_mode(mode))
            .map_err(|_| StoreError::Companion(companion.to_owned()))?;
        return Ok(());
    }
    Err(StoreError::Busy)
}

/// On Windows a link is a name-surrogate reparse point (a symbolic link or a junction);
/// other reparse points, such as OneDrive's files on demand, are files.
#[cfg(not(unix))]
fn check_companions(index: &IndexPath, repair: bool) -> Result<(), StoreError> {
    use std::os::windows::fs::OpenOptionsExt;
    use winx::file::{AccessMode, Flags};
    for suffix in ["-wal", "-shm"] {
        let companion = index.beside(suffix);
        // The name itself, not what a link names; a directory opens too.
        let access = if repair {
            AccessMode::FILE_READ_ATTRIBUTES | AccessMode::FILE_WRITE_ATTRIBUTES
        } else {
            AccessMode::FILE_READ_ATTRIBUTES
        };
        let file = match std::fs::OpenOptions::new()
            .access_mode(access.bits())
            .custom_flags(
                (Flags::FILE_FLAG_OPEN_REPARSE_POINT | Flags::FILE_FLAG_BACKUP_SEMANTICS).bits(),
            )
            .open(&companion)
        {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            // SQLite will say what is wrong with a file it cannot open either.
            Err(_) => continue,
        };
        let information = winx::winapi_util::file::information(&file).map_err(StoreError::Io)?;
        let metadata = file.metadata().map_err(StoreError::Io)?;
        let kind = metadata.file_type();
        if kind.is_dir() || kind.is_symlink() || information.number_of_links() != 1 {
            return Err(StoreError::Unsafe(companion));
        }
        if !repair || !metadata.permissions().readonly() {
            continue;
        }
        let failed = || StoreError::Companion(companion.clone());
        // The index file's attributes: that file is writable, so they are never all clear,
        // and attributes of 0 would leave the log file's attributes as they are.
        let permissions = std::fs::metadata(index.file())
            .map_err(|_| failed())?
            .permissions();
        file.set_permissions(permissions).map_err(|_| failed())?;
        let cleared = file
            .metadata()
            .is_ok_and(|metadata| !metadata.permissions().readonly());
        if !cleared {
            return Err(failed());
        }
    }
    Ok(())
}

/// The files SQLite keeps beside a database: its write-ahead log and shared memory, and
/// the rollback journal a tool that switched the index out of WAL mode would leave.
pub(crate) const LOGS: [&str; 3] = ["-wal", "-shm", "-journal"];

/// Removes a rollback journal beside the index. An index is kept in WAL mode and never
/// has one, but SQLite reads one it finds there on every open, even to read, and would
/// play it back into the index, and act on what it names. Only the name beside the index
/// goes: a link is removed, not what it names.
fn remove_journal(index: &IndexPath) -> Result<(), StoreError> {
    let journal = index.beside("-journal");
    match std::fs::remove_file(&journal) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(StoreError::Io(std::io::Error::new(
            error.kind(),
            format!("cannot remove {}: {error}", journal.display()),
        ))),
    }
}

/// Removes SQLite's log files for the database at `path`; best effort.
fn remove_logs(path: &Path) {
    for log in LOGS {
        let _ = std::fs::remove_file(beside(path, log));
    }
}

/// Removes a database file and its log files; best effort.
fn remove_database(path: &Path) {
    let _ = std::fs::remove_file(path);
    remove_logs(path);
}

/// Trims the write-ahead log after a large update; best effort, as readers may hold it.
pub(crate) fn trim_log(store: &Store) {
    let _ = store
        .connection
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
}
