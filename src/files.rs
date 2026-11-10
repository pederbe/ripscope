//! Read finite regular files, following named symlinks. Opening a FIFO must not wait
//! for a writer before its type can be checked.

use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

/// Opens a path without blocking on a FIFO. Callers must check its type before reading.
pub(crate) fn open(path: &Path) -> io::Result<File> {
    #[cfg(unix)]
    {
        use rustix::fs::{Mode, OFlags};
        let flags = OFlags::RDONLY | OFlags::NONBLOCK | OFlags::CLOEXEC | OFlags::NOCTTY;
        Ok(rustix::fs::open(path, flags, Mode::empty())?.into())
    }
    #[cfg(not(unix))]
    {
        File::open(path)
    }
}

/// Whether the opened handle names a regular file, rather than a pipe or device.
pub(crate) fn is_regular(file: &File) -> io::Result<bool> {
    #[cfg(windows)]
    if !winx::winapi_util::file::typ(file)?.is_disk() {
        return Ok(false);
    }
    Ok(file.metadata()?.is_file())
}

pub(crate) fn open_regular(path: &Path) -> io::Result<File> {
    let file = open(path)?;
    if !is_regular(&file)? {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "not a regular file",
        ));
    }
    Ok(file)
}

pub(crate) fn read(path: &Path) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    open_regular(path)?.read_to_end(&mut bytes)?;
    Ok(bytes)
}
