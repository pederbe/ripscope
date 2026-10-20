//! The index a session holds open between its queries, so that a query skips opening the
//! index, reading its schema and preparing its statements again.
//!
//! Each query still reads the index's current revision in a transaction of its own. On
//! Unix an index replaced by another file (an index build moves a new one over it) is
//! noticed before each query, by the path naming another file, and opened afresh: until
//! then the file replaced keeps its disk space. On Windows a file a process holds open
//! cannot be deleted, moved or replaced, so there a session lets go of the index as soon
//! as it has read it or updated it, as it did before it held one.

use crate::store::{Access, Store, StoreError};
use std::cell::RefCell;
use std::path::{Path, PathBuf};

/// Whether a session lets go of the index as soon as it is done with it: on Windows,
/// where holding it would refuse anyone deleting, moving or replacing the index.
const LET_GO: bool = cfg!(windows);

pub(crate) struct HeldIndex {
    path: PathBuf,
    held: RefCell<Option<Held>>,
}

struct Held {
    store: Store,
    file: FileIdentity,
}

impl HeldIndex {
    pub(crate) fn new(path: &Path) -> Self {
        Self {
            path: path.to_owned(),
            held: RefCell::new(None),
        }
    }

    /// Runs `read` on the index, opened as it is now: the one held, unless the path now
    /// names another file, else opened afresh.
    pub(crate) fn with<R>(
        &self,
        read: impl FnOnce(&Store) -> Result<R, StoreError>,
    ) -> Result<R, StoreError> {
        let file = FileIdentity::of(&self.path);
        let mut held = self.held.take();
        if held
            .as_ref()
            .is_some_and(|held| file.is_none() || held.file != file)
        {
            held = None;
        }
        let held = match held {
            Some(held) => held,
            // The file is looked at before it is opened: if it is replaced in between,
            // the next query sees another file and opens it again.
            None => Held {
                store: Store::open(&self.path, Access::Read)?,
                file,
            },
        };
        let result = read(&held.store);
        if result.is_ok() && !LET_GO {
            self.held.replace(Some(held));
        }
        result
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// Holds `store`, opened for reading in this process on the file `file` named before
    /// it was opened, as if a query had opened it. On Windows it is let go of at once.
    pub(crate) fn adopt(&self, store: Store, file: FileIdentity) {
        if !LET_GO {
            self.held.replace(Some(Held { store, file }));
        }
    }

    /// Lets go of the index, for an update in this process that may replace it.
    pub(crate) fn release(&self) {
        self.held.take();
    }
}

/// Which file a path names, for noticing an index replaced while it was held: on Unix
/// its device and inode. On Windows a held file cannot be replaced, so every file is the
/// same.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct FileIdentity(Option<(u64, u64)>);

impl FileIdentity {
    /// The file `path` names now. Taken before a store is opened, so that a file replaced
    /// in between is opened again by the next query.
    #[cfg(unix)]
    pub(crate) fn of(path: &Path) -> Self {
        use std::os::unix::fs::MetadataExt;
        Self(
            std::fs::metadata(path)
                .ok()
                .map(|metadata| (metadata.dev(), metadata.ino())),
        )
    }

    #[cfg(not(unix))]
    pub(crate) fn of(_path: &Path) -> Self {
        Self(Some((0, 0)))
    }

    fn is_none(&self) -> bool {
        self.0.is_none()
    }
}
