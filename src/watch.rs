//! A session's watch on the files its index was made from
//! ([ADR 0008](../docs/adr/0008-sessions-refresh-on-watched-changes.md)): it tells the
//! session before each query whether a Refresh could find anything changed, so that the
//! queries between changes answer at once from the published revision.
//!
//! What a Refresh reads is watched before it reads it. The watch learns the selection
//! rule and the revision a Refresh starts from once the Refresh holds the write lock and
//! before it looks at any file, and covers them then. What it learns only from the
//! revision the Refresh leaves (a file in a directory not watched before) is covered
//! afterwards, and the next query refreshes again, since a change in between went
//! unseen. A change reported after a Refresh began, or one the watch cannot rule out
//! because events were lost, leaves the next query to refresh. Where it cannot watch at
//! all, every query refreshes.
//!
//! Before each query the watch takes in every change made before the query arrived. On
//! Linux inotify queues an event before the system call that made the change returns,
//! so reading the queue until it is empty takes them all in. On Windows events reach a
//! watcher thread later, so the watch makes a file of its own and waits for its event,
//! which comes after those of every earlier change in the directory tree.

use crate::hashing::{HashMap, HashSet};
use crate::refresh::{self, Entry, Observer, Selection, Sources};
use crate::store::{self, FileStat, RevisionId, StoredFile};
use std::borrow::Cow;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

#[cfg(target_os = "linux")]
mod inotify;
#[cfg(target_os = "linux")]
use inotify::Backend;
#[cfg(not(any(target_os = "linux", windows)))]
mod unwatched;
#[cfg(not(any(target_os = "linux", windows)))]
use unwatched::Backend;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows::Backend;

#[cfg(test)]
mod tests;

/// What a backend reports.
#[derive(Debug)]
enum Happened {
    /// Events may have been lost, or what is watched may no longer be where it was:
    /// anything may have changed, and the next Refresh watches afresh.
    Lost,
    /// Something about a watched directory itself changed.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    Directory,
    /// The watch cannot work, for this reason: every query refreshes from now on.
    #[cfg_attr(not(windows), allow(dead_code))]
    Failed(String),
    /// A name in a watched directory, as the watched directory is spelled.
    Entry {
        path: PathBuf,
        kind: Kind,
        /// Whether it is a directory, where the event tells.
        directory: Option<bool>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    /// Created, or renamed to this name.
    Added,
    /// Deleted, or renamed from this name.
    Removed,
    /// Written, or its metadata changed.
    Changed,
}

/// How a backend covers a directory.
#[derive(Debug, PartialEq, Eq)]
enum Cover {
    /// Its changes are reported; `true` if it was not watched before.
    Watched(bool),
    /// It does not exist, and the nearest directory above it that does is watched;
    /// `true` if that was not watched before.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    Above(bool),
    /// Its changes are not reported: the names in it, or its times, are checked before
    /// each query.
    Unwatched,
}

/// Whether the watch covers a directory.
enum Watching {
    /// It is watched, or the nearest directory above it is while it does not exist;
    /// `true` if that was not so before.
    Watched(bool),
    /// No watch reaches it; `true` if that was not known before.
    Unwatched(bool),
}

pub(crate) struct Watch {
    /// The index's file name and its SQLite companions': a Refresh writes them.
    index: Vec<OsString>,
    covered: Option<Covered>,
    /// Whether a Refresh may find anything changed since the last one began.
    changed: bool,
    /// A Refresh began and has not finished: it failed, and the next query refreshes.
    refreshing: bool,
    /// Why changes cannot be watched: every query then refreshes.
    blind: Option<String>,
    told: bool,
    /// The revision the session's last Refresh left or its last query read: a query that
    /// reads another found another process's publication, which may have changed the
    /// selection rule or the names indexed, and the next query refreshes.
    known: Option<Known>,
}

enum Known {
    /// A Refresh left the revision with this number.
    Left(u64),
    Read(RevisionId),
}

/// What the watch covers for one rule.
struct Covered {
    rule: Rule,
    /// `None` once events were lost: the next Refresh starts a new one.
    backend: Option<Backend>,
    /// Each name a Refresh read or asked about, as [`key`] gives it.
    names: HashSet<Vec<u8>>,
    /// The directories above them, the same way.
    directories: HashSet<Vec<u8>>,
    /// The watched directories that hold them, as stored names spell them.
    parents: HashSet<Vec<u8>>,
    /// Directories that cannot be watched: the names in them, or their times, are polled.
    unwatched: HashSet<Vec<u8>>,
    /// Missing directories tried during this record.
    tried: HashSet<Vec<u8>>,
    polling: Polling,
    /// Where the files last recorded lead, being found on another thread.
    looking: Option<std::thread::JoinHandle<Vec<Vec<u8>>>>,
    /// A digest of the names of the revision last covered: until one is, a Refresh covers
    /// the revision it starts from before it reads anything.
    recorded: Option<[u8; 32]>,
}

/// What a selection rule makes a watch cover beyond the files and questions.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Rule {
    root: PathBuf,
    /// Automatic discovery, recursive or not.
    discovery: Option<bool>,
    /// The file list the rule reads, or reads when it is present.
    list: Option<Vec<u8>>,
}

impl Rule {
    fn of(selection: &Selection) -> Self {
        let (discovery, list) = match &selection.sources {
            Sources::Files(_) => (None, None),
            Sources::List(list) => (None, Some(list.clone())),
            Sources::Automatic { recursive } => {
                (Some(*recursive), Some(refresh::AUTOMATIC_LIST.to_vec()))
            }
        };
        Self {
            root: selection.root.clone(),
            discovery,
            list,
        }
    }
}

/// What the watch looks at itself before each query, where no watch reaches: the names a
/// Refresh read or asked about there, or the directories they are in. A directory's times
/// move when a name in it is made, removed or renamed, so they cover the names missing
/// there, and a directory missing below one looked at is covered by that one. In a
/// directory looked at by its times, only the names found are looked at themselves,
/// since an edit leaves their directory as it was.
#[derive(Default)]
struct Polling {
    directories: Vec<PolledDirectory>,
    /// Where each directory is in `directories`, as stored names spell it.
    places: HashMap<Vec<u8>, usize>,
    /// The root's identity, read once a directory is polled: `None` where it cannot be.
    root_identity: Option<Option<Vec<u8>>>,
}

/// A directory no watch reaches, and the names looked for in it.
struct PolledDirectory {
    /// As stored names spell it.
    spelled: Vec<u8>,
    /// Whether it is the root, whose times move with the files the session itself makes
    /// there: the names in it are looked at themselves.
    root: bool,
    itself: Polled,
    by: By,
    names: Vec<Polled>,
}

/// How a polled directory is looked at before each query.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum By {
    /// By its own times, with the names found in it.
    Times,
    /// By the times of the directory above it, which were seen before it was found
    /// missing, with any name found in it.
    Above,
    /// By every name looked for in it, found or missing.
    Names,
}

/// A directory is looked at by its times once this many names are missing in it. On
/// Windows a look at a directory costs about what looking for seven missing names does,
/// and its times move with any name made there, such as an editor's swap file, so a few
/// names missing in it are looked for themselves.
const MISSING_FOR_TIMES: usize = 8;

impl Polling {
    /// Polls a stored name in the directory that `directory` spells, and that directory
    /// from now on if it was not already. A directory is looked at before the names in it,
    /// so that a name made after it was found missing moves times already seen.
    fn add(&mut self, root: &Path, directory: &[u8], name: &[u8]) {
        let at = match self.places.get(directory) {
            Some(&at) => at,
            None => {
                let itself = Polled::new(place(root, directory));
                let root = self.is_root(root, directory, &itself.seen);
                self.places
                    .insert(directory.to_vec(), self.directories.len());
                self.directories.push(PolledDirectory {
                    spelled: directory.to_vec(),
                    root,
                    itself,
                    by: By::Names,
                    names: Vec::new(),
                });
                self.directories.len() - 1
            }
        };
        let name = Polled::new(refresh::below(root, name));
        self.directories[at].names.push(name);
        self.directories[at].by = self.by(&self.directories[at]);
    }

    /// Whether the directory that `directory` spells, just `seen`, is the root: spelled
    /// through it, or the same directory reached through a link, a substituted drive or
    /// another name for it. Where the root's identity cannot be read, any directory might
    /// be the root.
    fn is_root(&mut self, root: &Path, directory: &[u8], seen: &Seen) -> bool {
        if spells_root(root, directory) {
            return true;
        }
        let identity = self
            .root_identity
            .get_or_insert_with(|| match refresh::entry_of(root) {
                Ok(Entry::Directory(stat)) if !stat.identity.is_empty() => Some(stat.identity),
                _ => None,
            });
        match (identity, seen) {
            (None, _) => true,
            (Some(identity), Seen::Directory(stat)) => stat.identity == *identity,
            (Some(_), _) => false,
        }
    }

    /// How `directory` is looked at from now on, once it and the names in it were seen:
    /// one missing below a directory looked at by its times, seen before it, is covered by
    /// them, and one with many names missing is looked at by its own. Only a directory
    /// seen as one, or missing, has times to look at: one that could not be looked at
    /// itself has its names looked at.
    fn by(&self, directory: &PolledDirectory) -> By {
        if directory.root {
            return By::Names;
        }
        let above = parent_of(&directory.spelled);
        let covered = directory.itself.seen == Seen::Missing
            && above != directory.spelled
            && self.places.get(above).is_some_and(|&at| {
                let above = &self.directories[at];
                above.by == By::Times && matches!(above.itself.seen, Seen::Directory(_))
            });
        if covered {
            return By::Above;
        }
        let timed = matches!(directory.itself.seen, Seen::Directory(_) | Seen::Missing);
        let missing = directory.names.iter().filter(|name| !name.found()).count();
        if timed && missing >= MISSING_FOR_TIMES {
            By::Times
        } else {
            By::Names
        }
    }

    /// Looks at everything again, as a Refresh that reads it all begins: each directory
    /// after the one above it and before the names in it.
    fn again(&mut self) {
        let mut order: Vec<usize> = (0..self.directories.len()).collect();
        order.sort_by_key(|&at| self.directories[at].spelled.len());
        for at in order {
            let directory = &mut self.directories[at];
            directory.itself.again();
            for name in &mut directory.names {
                name.again();
            }
            self.directories[at].by = self.by(&self.directories[at]);
        }
    }

    /// Whether anything changed since it was last looked at.
    fn changed(&self) -> bool {
        self.directories.iter().any(|directory| {
            (directory.by == By::Times && directory.itself.changed())
                || directory
                    .names
                    .iter()
                    .any(|name| directory.looks_at(name) && name.changed())
        })
    }

    /// How many things [`Polling::changed`] looks at.
    #[cfg_attr(not(any(test, feature = "session-profile")), allow(dead_code))]
    fn looks(&self) -> usize {
        self.directories
            .iter()
            .map(|directory| {
                usize::from(directory.by == By::Times)
                    + directory
                        .names
                        .iter()
                        .filter(|name| directory.looks_at(name))
                        .count()
            })
            .sum()
    }
}

impl PolledDirectory {
    /// Whether a name in it is looked at itself.
    fn looks_at(&self, name: &Polled) -> bool {
        self.by == By::Names || name.found()
    }
}

/// A name or directory looked at before each query, and what it was before the Refresh
/// that read it.
struct Polled {
    path: PathBuf,
    seen: Seen,
    /// When it was seen: one modified this close to it may change again unnoticed within
    /// its timestamps' resolution.
    at: i64,
}

#[derive(Debug, PartialEq, Eq)]
enum Seen {
    File(FileStat),
    Directory(FileStat),
    Other,
    Missing,
    Failed(std::io::ErrorKind),
}

impl Seen {
    /// What a name or directory is now.
    fn of(path: &Path) -> Self {
        match refresh::entry_of(path) {
            Ok(Entry::File(stat, _)) => Self::File(stat),
            Ok(Entry::Directory(stat)) => Self::Directory(stat),
            Ok(Entry::Other) => Self::Other,
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
                ) =>
            {
                Self::Missing
            }
            Err(error) => Self::Failed(error.kind()),
        }
    }
}

/// Times this close to a reading are not trusted, as in a Refresh.
const RACY_NANOSECONDS: i64 = 2_000_000_000;

impl Polled {
    fn new(path: PathBuf) -> Self {
        let at = now();
        let seen = Seen::of(&path);
        Self { path, seen, at }
    }

    fn again(&mut self) {
        self.at = now();
        self.seen = Seen::of(&self.path);
    }

    /// Whether it was there when last seen, or could not be looked at.
    fn found(&self) -> bool {
        self.seen != Seen::Missing
    }

    fn changed(&self) -> bool {
        if let Seen::File(stat) | Seen::Directory(stat) = &self.seen
            && stat.modified.max(stat.changed) >= self.at - RACY_NANOSECONDS
        {
            return true;
        }
        Seen::of(&self.path) != self.seen
    }
}

impl Watch {
    /// A watch for a session on `index`, which covers nothing until a Refresh tells it
    /// what to cover: until then every query refreshes.
    pub(crate) fn new(index: &Path) -> Self {
        let name = index.file_name().map(OsString::from).unwrap_or_default();
        let index = std::iter::once("")
            .chain(store::LOGS)
            .map(|suffix| {
                let mut companion = name.clone();
                companion.push(suffix);
                companion
            })
            .collect();
        Self {
            index,
            covered: None,
            changed: true,
            refreshing: false,
            blind: None,
            told: false,
            known: None,
        }
    }

    /// Whether a Refresh may find anything changed since the last one began, after taking
    /// in every change made so far.
    pub(crate) fn changed(&mut self) -> bool {
        profile_span!("watch_changed");
        if std::mem::take(&mut self.refreshing) {
            self.changed = true;
        }
        if self.blind.is_some() {
            return true;
        }
        let Some(covered) = &mut self.covered else {
            return true;
        };
        let Some(backend) = &mut covered.backend else {
            return true;
        };
        for happened in backend.events() {
            match happened {
                Happened::Lost => {
                    covered.backend = None;
                    self.changed = true;
                    return true;
                }
                Happened::Directory => self.changed = true,
                Happened::Failed(reason) => {
                    self.blind = Some(reason);
                    return true;
                }
                Happened::Entry {
                    path,
                    kind,
                    directory,
                } => {
                    if covered.replaced(&path, kind) {
                        covered.backend = None;
                        self.changed = true;
                        return true;
                    }
                    if covered.relevant(&self.index, &path, kind, directory) {
                        self.changed = true;
                    }
                    if let Err(reason) = covered.discovered(&path, kind, directory) {
                        self.blind = Some(reason);
                        return true;
                    }
                }
            }
        }
        match covered.links() {
            Ok(anew) => self.changed |= anew,
            Err(reason) => {
                self.blind = Some(reason);
                return true;
            }
        }
        if !self.changed {
            profile_span!("watch_polled");
            profile_count!("watch_polled", covered.polling.looks());
            self.changed = covered.polling.changed();
        }
        self.changed
    }

    /// Notes the revision a query read.
    pub(crate) fn read(&mut self, revision: Option<&RevisionId>) {
        let Some(revision) = revision else {
            return;
        };
        let another = match &self.known {
            Some(Known::Left(number)) => *number != revision.number,
            Some(Known::Read(read)) => read != revision,
            None => false,
        };
        if another {
            self.changed = true;
            // Covered before the next Refresh reads it.
            if let Some(covered) = &mut self.covered {
                covered.recorded = None;
            }
        }
        self.known = Some(Known::Read(revision.clone()));
    }

    /// Puts away what the last check left, once its answer is sent: the Windows watch's
    /// marker file.
    pub(crate) fn tidy(&mut self) {
        if let Some(backend) = self.covered.as_mut().and_then(|c| c.backend.as_mut()) {
            backend.tidy();
        }
    }

    /// Why the watch cannot see changes, once.
    pub(crate) fn note(&mut self) -> Option<String> {
        if self.told {
            return None;
        }
        let reason = self.blind.as_ref()?;
        self.told = true;
        Some(format!(
            "cannot watch the sources for changes ({reason}); each query updates the index first"
        ))
    }
}

impl Observer for Watch {
    fn starting(&mut self, selection: &Selection, files: &[StoredFile], observed: Option<&[u8]>) {
        profile_span!("watch_starting");
        if self.blind.is_some() {
            return;
        }
        // Whatever happens from here on, this Refresh reads or the watch reports.
        self.changed = false;
        self.refreshing = true;
        let rule = Rule::of(selection);
        let started = match self.covered.take() {
            Some(covered) if covered.rule == rule && covered.backend.is_some() => Ok(covered),
            _ => Covered::start(rule),
        };
        let covered = match started {
            Ok(covered) => self.covered.insert(covered),
            Err(reason) => {
                self.blind = Some(reason);
                return;
            }
        };
        // This Refresh reads what is polled again: it is compared with what it is now.
        covered.polling.again();
        if covered.recorded.is_some() {
            return;
        }
        match covered.record(files, observed) {
            Ok(_) => covered.recorded = Some(digest(files, observed)),
            Err(reason) => self.blind = Some(reason),
        }
    }

    fn finished(
        &mut self,
        revision: u64,
        files: &[StoredFile],
        observed: &[u8],
        missing: &[Vec<u8>],
    ) {
        profile_span!("watch_finished");
        self.known = Some(Known::Left(revision));
        self.refreshing = false;
        if self.blind.is_some() {
            return;
        }
        let Some(covered) = self.covered.as_mut().filter(|c| c.backend.is_some()) else {
            return;
        };
        // A revision with the names already covered needs nothing more, apart from where
        // links lead and the named files still missing: an unchanged one, or one whose
        // files only changed their bytes. Anything covered only now, after the Refresh
        // read it, may have changed unseen, and the next query refreshes again.
        let names = digest(files, Some(observed));
        let covering = if covered.recorded == Some(names) {
            covered.links()
        } else {
            covered.recorded = Some(names);
            covered.record(files, Some(observed))
        };
        let covering = covering.and_then(|anew| {
            missing
                .iter()
                .try_fold(anew, |anew, name| Ok(covered.know(name)? | anew))
        });
        match covering {
            Ok(anew) => self.changed |= anew,
            Err(reason) => self.blind = Some(reason),
        }
    }
}

/// A session's watch, which a session that does not update has none of.
impl Observer for Option<Watch> {
    fn starting(&mut self, selection: &Selection, files: &[StoredFile], observed: Option<&[u8]>) {
        if let Some(watch) = self {
            watch.starting(selection, files, observed);
        }
    }

    fn finished(
        &mut self,
        revision: u64,
        files: &[StoredFile],
        observed: &[u8],
        missing: &[Vec<u8>],
    ) {
        if let Some(watch) = self {
            watch.finished(revision, files, observed, missing);
        }
    }
}

impl Covered {
    /// Watches what `rule` discovers from, and nothing else yet.
    fn start(rule: Rule) -> Result<Self, String> {
        let mut backend = Backend::new(&rule.root)?;
        if let Some(recursive) = rule.discovery {
            backend.discover(&rule.root, recursive)?;
        }
        let mut covered = Self::with(rule, Some(backend));
        // A directory moved below the root is seen from the root.
        if Backend::FOLLOWS_DIRECTORY {
            covered.watch(b"")?;
        }
        Ok(covered)
    }

    fn with(rule: Rule, backend: Option<Backend>) -> Self {
        Self {
            rule,
            backend,
            names: HashSet::default(),
            directories: HashSet::default(),
            parents: HashSet::default(),
            unwatched: HashSet::default(),
            tried: HashSet::default(),
            polling: Polling::default(),
            looking: None,
            recorded: None,
        }
    }

    /// Covers a revision's files, the questions its Refresh asked and the rule's list:
    /// whether any of them needed covering anew. Files not seen before are looked at for
    /// links on another thread, while the Refresh reads ([`Covered::links`]).
    fn record(&mut self, files: &[StoredFile], observed: Option<&[u8]>) -> Result<bool, String> {
        self.tried.clear();
        let mut anew = self.links()?;
        if let Some(list) = self.rule.list.clone() {
            anew |= self.know(&list)?;
        }
        // Files new to the watch are looked at for links. One that becomes a link later is
        // not followed: a change where it leads is noticed only by `r`.
        let mut unlooked = Vec::new();
        for file in files {
            if Backend::FOLLOWS_LINKS
                && !self
                    .names
                    .contains(name_key(&self.rule.root, &file.path).as_ref())
            {
                unlooked.push(file.path.clone());
            }
            anew |= self.know(&file.path)?;
        }
        if let Some(questions) = observed.and_then(refresh::questions) {
            for name in questions {
                anew |= self.know(name)?;
            }
        }
        if !unlooked.is_empty() {
            let root = self.rule.root.clone();
            self.looking = Some(std::thread::spawn(move || targets(&root, &unlooked)));
        }
        Ok(anew)
    }

    /// Covers where the files last looked at for links lead, once that is known: whether
    /// any of it needed covering anew.
    fn links(&mut self) -> Result<bool, String> {
        let Some(looking) = self.looking.take() else {
            return Ok(false);
        };
        let mut anew = false;
        for target in looking.join().unwrap_or_default() {
            anew |= self.know(&target)?;
        }
        Ok(anew)
    }

    /// Covers a stored name (relative to the root, or absolute): whether the name or its
    /// directory needed covering anew.
    fn know(&mut self, name: &[u8]) -> Result<bool, String> {
        let key = name_key(&self.rule.root, name);
        let unknown = !self.names.contains(key.as_ref());
        let mut anew = false;
        if unknown {
            let mut above = key.as_ref();
            let mut farther = Vec::new();
            while let Some(at) = above.iter().rposition(|byte| *byte == b'/') {
                above = &above[..at.max(1)];
                if !self.directories.insert(above.to_vec()) {
                    break;
                }
                farther.push(above.to_vec());
                if above == b"/" {
                    break;
                }
            }
            self.names.insert(key.into_owned());
            // Where a watch stays with the directory it watches, a directory above that is
            // moved or replaced is seen only from the directory above it.
            if Backend::FOLLOWS_DIRECTORY {
                for directory in farther.iter().skip(1) {
                    anew |= matches!(self.watch(directory)?, Watching::Watched(true));
                }
            }
        }
        let parent = parent_of(name);
        Ok(anew
            | match self.watch(parent)? {
                Watching::Watched(watched) => watched,
                // The names in a directory no watch reaches, or its times, are polled.
                Watching::Unwatched(newly) => {
                    let polled = unknown || newly;
                    if polled {
                        self.polling.add(&self.rule.root, parent, name);
                    }
                    polled
                }
            })
    }

    /// Watches the directory a stored name's directory part spells (empty for the root),
    /// unless it is already.
    fn watch(&mut self, directory: &[u8]) -> Result<Watching, String> {
        if self.unwatched.contains(directory) {
            return Ok(Watching::Unwatched(false));
        }
        if self.parents.contains(directory) || self.tried.contains(directory) {
            return Ok(Watching::Watched(false));
        }
        let path = place(&self.rule.root, directory);
        let backend = self.backend.as_mut().expect("a started watch");
        Ok(match backend.cover(&path)? {
            Cover::Watched(anew) => {
                self.parents.insert(directory.to_vec());
                Watching::Watched(anew)
            }
            // Watched once it exists: the directory above reports it made.
            Cover::Above(anew) => {
                self.tried.insert(directory.to_vec());
                Watching::Watched(anew)
            }
            Cover::Unwatched => {
                self.unwatched.insert(directory.to_vec());
                Watching::Unwatched(true)
            }
        })
    }

    /// Whether a change at `path` made, removed or renamed a directory above covered
    /// names: what is watched below it may no longer be where those names lead, so the
    /// next Refresh watches afresh.
    fn replaced(&self, path: &Path, kind: Kind) -> bool {
        kind != Kind::Changed && self.directories.contains(&key(&self.rule.root, path))
    }

    /// Whether a change at `path` can change what a Refresh finds.
    fn relevant(
        &self,
        index: &[OsString],
        path: &Path,
        kind: Kind,
        directory: Option<bool>,
    ) -> bool {
        let Some(name) = path.file_name() else {
            return true;
        };
        if index.iter().any(|own| own == name) {
            return false;
        }
        let key = key(&self.rule.root, path);
        if self.names.contains(&key) {
            return true;
        }
        if self.directories.contains(&key) {
            return !(Backend::ECHOES_ENTRIES && kind == Kind::Changed);
        }
        if !self.discovers(&key) {
            return false;
        }
        refresh::is_source_name(name.as_encoded_bytes())
            || match directory {
                Some(directory) => directory,
                // A directory renamed into the tree brings files no event names.
                None => kind == Kind::Added && path.is_dir(),
            }
    }

    /// Whether discovery looks for sources where `key` is.
    fn discovers(&self, key: &[u8]) -> bool {
        let Some(recursive) = self.rule.discovery else {
            return false;
        };
        let inside = !key.is_empty() && !refresh::outside(key);
        inside && (recursive || !key.contains(&b'/'))
    }

    /// Watches a directory that appeared where discovery recurses.
    fn discovered(
        &mut self,
        path: &Path,
        kind: Kind,
        directory: Option<bool>,
    ) -> Result<(), String> {
        if kind == Kind::Removed
            || directory != Some(true)
            || self.rule.discovery != Some(true)
            || !self.discovers(&key(&self.rule.root, path))
        {
            return Ok(());
        }
        match &mut self.backend {
            Some(backend) => backend.discover(path, true),
            None => Ok(()),
        }
    }
}

/// Where those of `names` that are symbolic links lead: a change there passes no watched
/// directory. Linux only: on Windows links to files need privileges, and Git checks them
/// out as plain files by default.
fn targets(root: &Path, names: &[Vec<u8>]) -> Vec<Vec<u8>> {
    let threads = refresh::checking_threads(names.len());
    refresh::each_on_threads(names, threads, |name| {
        let path = refresh::below(root, name);
        let link = std::fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink());
        link.then(|| std::fs::canonicalize(&path).ok()).flatten()
    })
    .into_iter()
    .flatten()
    .map(|target| target.into_os_string().into_encoded_bytes())
    .collect()
}

/// What identifies the names a revision's record holds: its files' paths, in order, and
/// the questions its Refresh asked.
fn digest(files: &[StoredFile], observed: Option<&[u8]>) -> [u8; 32] {
    let mut digest = blake3::Hasher::new();
    for file in files {
        digest.update(&(file.path.len() as u64).to_le_bytes());
        digest.update(&file.path);
    }
    digest.update(observed.unwrap_or_default());
    digest.finalize().into()
}

/// The directory part of a stored name, without its last `/`: `/` for a name in the file
/// system's root, empty for one in the index's root.
fn parent_of(name: &[u8]) -> &[u8] {
    match name.iter().rposition(|byte| *byte == b'/') {
        Some(0) => b"/",
        Some(at) => &name[..at],
        None => b"",
    }
}

/// Where the directory part of a stored name leads.
fn place(root: &Path, directory: &[u8]) -> PathBuf {
    match directory {
        b"" => root.to_path_buf(),
        directory => refresh::below(root, directory),
    }
}

/// Whether the directory a stored name's directory part spells is the root, once each
/// `..` in it takes away the name before it, as Windows reads a path.
fn spells_root(root: &Path, directory: &[u8]) -> bool {
    let mut lexical = PathBuf::new();
    for component in place(root, directory).components() {
        match component {
            std::path::Component::ParentDir => {
                lexical.pop();
            }
            std::path::Component::CurDir => {}
            component => lexical.push(component),
        }
    }
    key(root, &lexical) == key(root, root)
}

/// How the watch knows a path: relative to the root when below it, else whole, with `/`
/// between names; on Windows, where names are compared without case, in lower case.
fn key(root: &Path, path: &Path) -> Vec<u8> {
    #[cfg(windows)]
    {
        // `canonicalize` gives verbatim paths (`\\?\C:\...`), which name the same files.
        let fold = |path: &Path| {
            let path = path.to_string_lossy().replace('\\', "/").to_lowercase();
            if let Some(share) = path.strip_prefix("//?/unc/") {
                format!("//{share}")
            } else if let Some(local) = path.strip_prefix("//?/") {
                local.to_owned()
            } else {
                path
            }
        };
        let (root, path) = (fold(root), fold(path));
        let root = root.trim_end_matches('/');
        match path
            .strip_prefix(root)
            .and_then(|rest| rest.strip_prefix('/'))
        {
            Some(rest) => rest.as_bytes().to_vec(),
            None => path.into_bytes(),
        }
    }
    #[cfg(not(windows))]
    {
        let root = root.as_os_str().as_encoded_bytes();
        let path = path.as_os_str().as_encoded_bytes();
        let root = root.strip_suffix(b"/").unwrap_or(root);
        match path
            .strip_prefix(root)
            .and_then(|rest| rest.strip_prefix(b"/"))
        {
            Some(rest) => rest.to_vec(),
            None => path.to_vec(),
        }
    }
}

/// [`key`] for a stored name, which is relative to the root or absolute.
fn name_key<'n>(root: &Path, name: &'n [u8]) -> Cow<'n, [u8]> {
    if cfg!(windows) || name.starts_with(b"/") {
        return Cow::Owned(key(root, &refresh::below(root, name)));
    }
    Cow::Borrowed(name)
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| {
            i64::try_from(duration.as_nanos()).unwrap_or(i64::MAX)
        })
}
