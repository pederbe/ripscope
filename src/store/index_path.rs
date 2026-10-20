//! The index's file as the user named it and as SQLite opens it: SQLite on Unix follows a
//! symbolic link to the file it names and keeps its log files beside that file, so
//! everything the Store does beside the index, and the check of where it lives, uses the
//! same file. Windows' SQLite does not follow links, so there SQLite is handed that file.

use super::StoreError;
use std::borrow::Cow;
use std::path::{Component, Path, PathBuf};

/// As many links as SQLite follows (SQLITE_MAX_SYMLINK).
const MOST_LINKS: usize = 200;

pub(super) struct IndexPath {
    /// As the user gave it, for messages about the index.
    given: PathBuf,
    /// The file it names: the path given, or where its symbolic links lead.
    file: PathBuf,
}

impl IndexPath {
    /// The index at `given`. Its final name's symbolic links are followed, each read from
    /// its own directory; directories on the way are left to the kernel, which reaches
    /// the same files through them. A link that leads nowhere is refused, and so is one
    /// that changed while it was read.
    pub(super) fn new(given: &Path) -> Result<Self, StoreError> {
        name_allowed(given)?;
        let mut file = given.to_owned();
        let mut links = 0;
        while std::fs::symlink_metadata(&file).is_ok_and(|metadata| metadata.is_symlink()) {
            links += 1;
            if links > MOST_LINKS {
                return Err(StoreError::Io(std::io::Error::other(format!(
                    "{}: too many symbolic links",
                    given.display()
                ))));
            }
            let target = std::fs::read_link(&file).map_err(StoreError::Io)?;
            file = match file.parent() {
                Some(directory) if target.is_relative() => directory.join(target),
                _ => target,
            };
        }
        if links > 0 {
            // Windows reads `..` in a path by name, not through the directory a link
            // names, so there the file is the one the kernel opens through the link.
            if cfg!(windows)
                && let Ok(opened) = std::fs::canonicalize(given)
            {
                file = opened;
                name_allowed(&file)?;
            }
            if std::fs::symlink_metadata(&file).is_err() {
                return Err(StoreError::Dangling(given.to_owned(), file));
            }
            // What the kernel opens through the link must be the file read here; a link
            // changed meanwhile, or one the kernel refuses to follow, fails.
            if !same_file(given, &file).map_err(StoreError::Io)? {
                return Err(StoreError::Io(std::io::Error::other(format!(
                    "{}: the symbolic link changed while it was read",
                    given.display()
                ))));
            }
        }
        // All companion checks and file operations must use the spelling SQLite opens:
        // canonicalizing only at its open would bypass checks through a Windows alias.
        #[cfg(windows)]
        {
            file = windows_file(&file).map_err(StoreError::Io)?;
            name_allowed(&file)?;
        }
        Ok(Self {
            given: given.to_owned(),
            file,
        })
    }

    pub(super) fn given(&self) -> &Path {
        &self.given
    }

    /// The index file itself.
    pub(super) fn file(&self) -> &Path {
        &self.file
    }

    /// The file's name with `suffix` appended, beside it: SQLite's log files, and the
    /// staging file a new index is written to.
    pub(super) fn beside(&self, suffix: &str) -> PathBuf {
        beside(&self.file, suffix)
    }

    /// The directory the file is in.
    pub(super) fn directory(&self) -> &Path {
        directory_of(&self.file)
    }
}

/// The file's extended-length Windows path, including when it has not been created yet.
/// `absolute` applies ordinary Win32 name rules before a canonical parent supplies the
/// verbatim prefix; an explicitly verbatim name retains its own spelling rules.
#[cfg(windows)]
fn windows_file(path: &Path) -> std::io::Result<PathBuf> {
    let absolute = std::path::absolute(path)?;
    match std::fs::canonicalize(&absolute) {
        Ok(file) => Ok(file),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let Some(name) = absolute.file_name() else {
                return Err(error);
            };
            Ok(std::fs::canonicalize(directory_of(&absolute))?.join(name))
        }
        Err(error) => Err(error),
    }
}

/// `path` with `suffix` appended to its name.
pub(super) fn beside(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

/// The directory `path` is in: `.` for a bare name.
pub(super) fn directory_of(path: &Path) -> &Path {
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    }
}

/// `path` as SQLite must be given it to open that file. The bundled SQLite reads a name
/// that begins with `file:` as a URI, so a relative one is given as `./file:...`.
pub(super) fn for_sqlite(path: &Path) -> Cow<'_, Path> {
    if path.is_relative() && path.as_os_str().as_encoded_bytes().starts_with(b"file:") {
        Cow::Owned(Path::new(".").join(path))
    } else {
        Cow::Borrowed(path)
    }
}

/// On Windows a `:` after a path's drive or share names a stream of another file (`a:b`
/// is the stream `b` of `a`), which SQLite cannot keep its log files beside, so such an
/// index path is refused before anything is made.
pub(super) fn name_allowed(path: &Path) -> Result<(), StoreError> {
    let stream = cfg!(windows)
        && path.components().any(|component| {
            matches!(component, Component::Normal(name)
                if name.as_encoded_bytes().contains(&b':'))
        });
    if stream {
        return Err(StoreError::Name(path.to_owned()));
    }
    Ok(())
}

/// Whether the kernel opens one file at `a`, following links, and at `b`.
#[cfg(unix)]
fn same_file(a: &Path, b: &Path) -> std::io::Result<bool> {
    use std::os::unix::fs::MetadataExt;
    let (a, b) = (std::fs::metadata(a)?, std::fs::metadata(b)?);
    Ok((a.dev(), a.ino()) == (b.dev(), b.ino()))
}

/// Whether the kernel opens one file at `a`, following links, and at `b`: on Windows a
/// file's identity is its volume's serial number and its index there.
#[cfg(not(unix))]
fn same_file(a: &Path, b: &Path) -> std::io::Result<bool> {
    let identity = |path: &Path| -> std::io::Result<(u64, u64)> {
        let information = winx::winapi_util::file::information(&std::fs::File::open(path)?)?;
        Ok((information.volume_serial_number(), information.file_index()))
    };
    Ok(identity(a)? == identity(b)?)
}
