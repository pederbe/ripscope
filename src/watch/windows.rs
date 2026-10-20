//! The Windows watch: `ReadDirectoryChangesW` on the whole tree below the root, through
//! the `notify` crate. Its events reach a watcher thread some time after each change, so
//! before each query the watch makes a file of its own in the root and waits for that
//! file's event: the system reports a tree's changes in order, so every change made
//! before the query has been reported by then. Names outside the tree, and below a
//! directory reached through a junction or link, are polled, or their directory's times
//! where many of them are missing.

use super::{Cover, Happened, Kind};
use crate::hashing::HashMap;
use notify::event::{EventKind, ModifyKind, RenameMode};
use notify::{RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::{Duration, Instant};

/// How long the watch waits for its own file's event before it stops trusting the
/// watcher: the next Refresh starts another.
const SETTLE: Duration = Duration::from_millis(500);

pub(super) struct Backend {
    root: PathBuf,
    /// The root as the file system resolves it, if it can.
    resolved: Option<PathBuf>,
    _watcher: notify::RecommendedWatcher,
    events: Receiver<notify::Result<notify::Event>>,
    serial: u64,
    /// The marker file the last check made, removed once its answer is sent.
    marker: Option<PathBuf>,
    /// Whether a marker's event has come: until one has, a marker whose event never
    /// comes means the file system reports no changes at all.
    settled: bool,
    /// Whether each directory below the root resolves to where its path spells: changes
    /// below one reached through a junction or link are reported by the path it leads to,
    /// if at all.
    inside: HashMap<PathBuf, bool>,
}

impl Backend {
    /// The whole tree is watched by path.
    pub(super) const FOLLOWS_DIRECTORY: bool = false;
    /// Links to files need privileges, and Git checks them out as plain files.
    pub(super) const FOLLOWS_LINKS: bool = false;
    /// A directory is reported changed whenever a name in it is made or removed, which
    /// that name's own event reports.
    pub(super) const ECHOES_ENTRIES: bool = true;

    pub(super) fn new(root: &Path) -> Result<Self, String> {
        let (sender, events) = channel();
        let mut watcher = notify::recommended_watcher(sender)
            .map_err(|error| format!("watching {}: {error}", root.display()))?;
        watcher
            .watch(root, RecursiveMode::Recursive)
            .map_err(|error| format!("watching {}: {error}", root.display()))?;
        Ok(Self {
            root: root.to_path_buf(),
            resolved: std::fs::canonicalize(root).ok(),
            _watcher: watcher,
            events,
            serial: 0,
            marker: None,
            settled: false,
            inside: HashMap::default(),
        })
    }

    /// The whole tree below the root is watched already.
    pub(super) fn discover(&mut self, _directory: &Path, _recursive: bool) -> Result<(), String> {
        Ok(())
    }

    pub(super) fn cover(&mut self, directory: &Path) -> Result<Cover, String> {
        if directory == self.root {
            return Ok(Cover::Watched(false));
        }
        let spelled = super::key(&self.root, directory);
        if crate::refresh::outside(&spelled) {
            return Ok(Cover::Unwatched);
        }
        let resolved = self.resolved.as_ref();
        let inside = *self
            .inside
            .entry(directory.to_path_buf())
            .or_insert_with(|| match (std::fs::canonicalize(directory), resolved) {
                (Ok(directory), Some(root)) => super::key(root, &directory) == spelled,
                // A directory that does not exist is reported once it is made.
                _ => true,
            });
        Ok(if inside {
            Cover::Watched(false)
        } else {
            Cover::Unwatched
        })
    }

    pub(super) fn tidy(&mut self) {
        if let Some(marker) = self.marker.take() {
            let _ = std::fs::remove_file(marker);
        }
    }

    /// Every change reported up to now, once the watch's own file's event has come.
    pub(super) fn events(&mut self) -> Vec<Happened> {
        self.tidy();
        let mut happened = Vec::new();
        self.serial += 1;
        let name = format!(".ripscope-{}-{}.sync", std::process::id(), self.serial);
        let marker = self.root.join(&name);
        // A marker an earlier process of the same number left goes first.
        let made = std::fs::File::create_new(&marker).or_else(|error| {
            if error.kind() != std::io::ErrorKind::AlreadyExists {
                return Err(error);
            }
            let _ = std::fs::remove_file(&marker);
            std::fs::File::create_new(&marker)
        });
        if let Err(error) = made {
            happened.push(Happened::Failed(format!(
                "cannot make a file in {} to wait for its event: {error}",
                self.root.display()
            )));
            return happened;
        }
        let deadline = Instant::now() + SETTLE;
        let settled = loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.events.recv_timeout(left) {
                Ok(Ok(event)) => {
                    if event.need_rescan() {
                        happened.push(Happened::Lost);
                        continue;
                    }
                    if event
                        .paths
                        .iter()
                        .any(|path| path.file_name() == Some(name.as_ref()))
                    {
                        break true;
                    }
                    happened.extend(entries(event));
                }
                Ok(Err(_)) => happened.push(Happened::Lost),
                Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => break false,
            }
        };
        // Removed after the answer is sent, as nothing waits on that.
        self.marker = Some(marker);
        if settled {
            self.settled = true;
        } else if self.settled {
            happened.push(Happened::Lost);
        } else {
            happened.push(Happened::Failed(format!(
                "no change events come from {}",
                self.root.display()
            )));
        }
        happened
    }
}

impl Drop for Backend {
    fn drop(&mut self) {
        self.tidy();
    }
}

fn entries(event: notify::Event) -> impl Iterator<Item = Happened> {
    let kind = match event.kind {
        EventKind::Create(_) | EventKind::Modify(ModifyKind::Name(RenameMode::To)) => Kind::Added,
        EventKind::Remove(_) | EventKind::Modify(ModifyKind::Name(RenameMode::From)) => {
            Kind::Removed
        }
        _ => Kind::Changed,
    };
    event.paths.into_iter().map(move |path| Happened::Entry {
        path,
        kind,
        directory: None,
    })
}
