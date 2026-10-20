//! The Linux watch: one inotify queue for every watched directory, read until it is empty
//! before each query. The kernel queues an event before the system call that made the
//! change returns, so an empty queue means no change was made since it was last read.

use super::{Cover, Happened, Kind};
use crate::hashing::{HashMap, HashSet};
use rustix::fd::OwnedFd;
use rustix::fs::inotify::{self, CreateFlags, ReadFlags, WatchFlags};
use rustix::io::Errno;
use std::mem::MaybeUninit;
use std::path::{Path, PathBuf};

pub(super) struct Backend {
    root: PathBuf,
    queue: OwnedFd,
    /// Each watch's directory, as each spelling it was added under names it.
    watches: HashMap<i32, Vec<PathBuf>>,
    added: HashSet<PathBuf>,
}

/// What a directory's watch reports: its names created, deleted, renamed, written or
/// given other metadata, and the directory itself moved.
const EVENTS: WatchFlags = WatchFlags::CREATE
    .union(WatchFlags::DELETE)
    .union(WatchFlags::MODIFY)
    .union(WatchFlags::CLOSE_WRITE)
    .union(WatchFlags::MOVED_FROM)
    .union(WatchFlags::MOVED_TO)
    .union(WatchFlags::ATTRIB)
    .union(WatchFlags::DELETE_SELF)
    .union(WatchFlags::MOVE_SELF)
    .union(WatchFlags::EXCL_UNLINK)
    .union(WatchFlags::ONLYDIR);

impl Backend {
    /// A watch stays with the directory it watches when the directory is moved.
    pub(super) const FOLLOWS_DIRECTORY: bool = true;
    /// A stored file may be a link to a file in a directory no other name leads to.
    pub(super) const FOLLOWS_LINKS: bool = true;
    /// A directory's own event says it changed, not that a name in it did.
    pub(super) const ECHOES_ENTRIES: bool = false;

    pub(super) fn new(root: &Path) -> Result<Self, String> {
        if let Some(system) = remote(root) {
            return Err(format!(
                "the sources are on {system}, which does not report changes made elsewhere"
            ));
        }
        let queue = inotify::init(CreateFlags::NONBLOCK | CreateFlags::CLOEXEC)
            .map_err(|error| format!("inotify: {error}"))?;
        Ok(Self {
            root: root.to_path_buf(),
            queue,
            watches: HashMap::default(),
            added: HashSet::default(),
        })
    }

    /// Watches `directory`, and with `recursive` every directory below it, as discovery
    /// walks them: each before it is listed, so that nothing made in it meanwhile is missed.
    pub(super) fn discover(&mut self, directory: &Path, recursive: bool) -> Result<(), String> {
        let mut pending = vec![directory.to_path_buf()];
        while let Some(directory) = pending.pop() {
            match self.add(&directory) {
                Ok(_) => {}
                Err(error) => {
                    limit(error)?;
                    continue;
                }
            }
            if !recursive {
                continue;
            }
            let Ok(entries) = std::fs::read_dir(&directory) else {
                continue;
            };
            for entry in entries.flatten() {
                if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                    pending.push(entry.path());
                }
            }
        }
        Ok(())
    }

    /// Watches `directory`, or where it does not exist the nearest directory above it.
    /// One outside the root on a network file system is not watched: changes made there
    /// by other machines are not reported.
    pub(super) fn cover(&mut self, directory: &Path) -> Result<Cover, String> {
        let outside = crate::refresh::outside(&super::key(&self.root, directory));
        if outside && !self.added.contains(directory) && remote(directory).is_some() {
            return Ok(Cover::Unwatched);
        }
        let mut at = directory;
        let mut above = false;
        loop {
            match self.add(at) {
                Ok(anew) if above => return Ok(Cover::Above(anew)),
                Ok(anew) => return Ok(Cover::Watched(anew)),
                Err(Errno::NOENT | Errno::NOTDIR) => match at.parent() {
                    Some(parent) if !parent.as_os_str().is_empty() => {
                        at = parent;
                        above = true;
                    }
                    _ => return Ok(Cover::Unwatched),
                },
                Err(error) => {
                    limit(error)?;
                    return Ok(Cover::Unwatched);
                }
            }
        }
    }

    /// Whether `directory` was watched anew.
    fn add(&mut self, directory: &Path) -> Result<bool, Errno> {
        if self.added.contains(directory) {
            return Ok(false);
        }
        let watch = inotify::add_watch(&self.queue, directory, EVENTS)?;
        self.watches
            .entry(watch)
            .or_default()
            .push(directory.to_path_buf());
        self.added.insert(directory.to_path_buf());
        Ok(true)
    }

    /// Nothing is left between checks.
    pub(super) fn tidy(&mut self) {}

    /// Every change queued since the last call.
    pub(super) fn events(&mut self) -> Vec<Happened> {
        let mut happened = Vec::new();
        let mut buffer = [MaybeUninit::<u8>::uninit(); 16 * 1024];
        let mut reader = inotify::Reader::new(&self.queue, &mut buffer);
        loop {
            let event = match reader.next() {
                Ok(event) => event,
                Err(Errno::AGAIN) => return happened,
                Err(Errno::INTR) => continue,
                Err(_) => {
                    happened.push(Happened::Lost);
                    return happened;
                }
            };
            let flags = event.events();
            if flags.contains(ReadFlags::QUEUE_OVERFLOW) {
                happened.push(Happened::Lost);
                return happened;
            }
            // A watched directory was removed, or moved: its spellings no longer name it. One
            // moved with a directory above it is seen from that one, which is watched too.
            if flags.intersects(ReadFlags::IGNORED | ReadFlags::MOVE_SELF | ReadFlags::UNMOUNT) {
                if self.watches.contains_key(&event.wd()) {
                    happened.push(Happened::Lost);
                    return happened;
                }
                continue;
            }
            let Some(spellings) = self.watches.get(&event.wd()) else {
                continue;
            };
            let Some(name) = event.file_name() else {
                happened.push(Happened::Directory);
                continue;
            };
            let kind = if flags.intersects(ReadFlags::CREATE | ReadFlags::MOVED_TO) {
                Kind::Added
            } else if flags.intersects(ReadFlags::DELETE | ReadFlags::MOVED_FROM) {
                Kind::Removed
            } else {
                Kind::Changed
            };
            let name =
                <std::ffi::OsStr as std::os::unix::ffi::OsStrExt>::from_bytes(name.to_bytes());
            for spelling in spellings {
                happened.push(Happened::Entry {
                    path: spelling.join(name),
                    kind,
                    directory: Some(flags.contains(ReadFlags::ISDIR)),
                });
            }
        }
    }
}

/// The kind of network file system `directory` is on, if it is on one: changes other
/// machines make there are not reported. FUSE file systems are counted among them, as
/// sshfs is one.
fn remote(directory: &Path) -> Option<&'static str> {
    // As linux/magic.h numbers them.
    const REMOTE: [(u32, &str); 14] = [
        (0x6969, "NFS"),
        (0x517b, "SMB"),
        (0xff53_4d42, "CIFS"),
        (0xfe53_4d42, "SMB"),
        (0x0102_1997, "9P"),
        (0x6573_5546, "a FUSE file system"),
        (0x00c3_6400, "Ceph"),
        (0x5346_414f, "AFS"),
        (0x6b41_4653, "AFS"),
        (0x7375_7245, "Coda"),
        (0x786f_4256, "a VirtualBox shared folder"),
        (0x0bd0_0bd0, "Lustre"),
        (0x4750_4653, "GPFS"),
        (0x0116_1970, "GFS2"),
    ];
    let system = rustix::fs::statfs(directory).ok()?.f_type as u32;
    REMOTE
        .iter()
        .find(|(magic, _)| *magic == system)
        .map(|(_, name)| *name)
}

/// Whether an error watching a directory leaves the watch unable to cover what it must.
fn limit(error: Errno) -> Result<(), String> {
    match error {
        Errno::NOSPC => {
            Err("the inotify watch limit is reached; raise fs.inotify.max_user_watches".into())
        }
        Errno::NOMEM => Err("inotify is out of memory".into()),
        _ => Ok(()),
    }
}
