//! Refresh (docs/architecture/storage.md): re-applies the
//! selection rule, analyses only what changed and publishes the next revision in one
//! transaction.
//!
//! Files are ordered as the reference cscope orders its database: the selected files
//! sorted by byte value, then each level of the include closure sorted the same way.
//! Paths are stored as cscope displays them, in canonical form. A deleted file is
//! removed; a file that cannot be read keeps its last unit in a refresh and is tried
//! again next time, and an index build leaves it out. When the
//! analyzer changed (its version, image version or Correction set), no stored unit is
//! kept, so an unreadable file is then left out too: an index never mixes two readings.

use crate::analyzer::Analyzer;
use crate::hashing::{HashMap, HashSet};
use crate::scanner;
use crate::store::{self, FileStat, Include, Published, Store, StoreError, StoredFile, Update};
use crate::unit::AnalysisUnit;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::time::Duration;

mod includes;
use includes::IncludeResolver;

/// How many threads check `items` files or include questions at a startup: one for each 256,
/// up to the cores. Starting a thread and waking a core for it costs about as much as
/// checking a hundred or more of them on a laptop, so a small project's are checked on
/// this thread alone.
pub(crate) fn checking_threads(items: usize) -> usize {
    if items <= ITEMS_PER_CHECKING_THREAD {
        return 1;
    }
    let cores = std::thread::available_parallelism().map_or(1, usize::from);
    items.div_ceil(ITEMS_PER_CHECKING_THREAD).clamp(1, cores)
}

const ITEMS_PER_CHECKING_THREAD: usize = 256;

#[cfg(test)]
mod tests;

/// Whether `analyzer` made the revision's analyses; if not, the next update analyses
/// every file again.
pub(crate) fn same_analyzer(snapshot: &store::Snapshot<'_>, analyzer: Analyzer) -> bool {
    snapshot.analyzer() == analyzer.identity()
}

/// The selection rule an index was built with; `None` for an index nothing has been
/// published to yet. A rule that cannot be read is an error, never replaced by the
/// command line's: that would index other files.
pub(crate) fn stored_rule(
    snapshot: &store::Snapshot<'_>,
) -> Result<Option<Selection>, RefreshError> {
    if snapshot.revision() == 0 {
        return Ok(None);
    }
    decode_rule(snapshot.selection()).map(Some)
}

/// A published revision's selection rule, from its stored bytes.
pub(crate) fn decode_rule(stored: &[u8]) -> Result<Selection, RefreshError> {
    Selection::decode(stored).ok_or_else(|| {
        RefreshError::Selection(format!(
            "the index's selection rule cannot be read; {}",
            store::REBUILD
        ))
    })
}

/// A file modified this close to the time its bytes were last hashed may have changed
/// again within the file system's timestamp resolution, so its metadata alone is not
/// trusted (file systems with coarse timestamps).
const RACY_NANOSECONDS: i64 = 2_000_000_000;

/// How long an update waits for another process's to finish.
pub(crate) const WAIT: Duration = Duration::from_secs(60);

/// Which files an index covers, persisted with it so refreshes re-apply it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Selection {
    /// The directory the index build ran in; relative names are resolved against it.
    pub root: PathBuf,
    pub sources: Sources,
    /// `-I` directories from the command line, searched for included files first.
    pub include_dirs: Vec<Vec<u8>>,
    /// Directories from the `INCLUDEDIRS` environment variable at index build time,
    /// searched after those a list file names.
    pub environment_dirs: Vec<Vec<u8>>,
    /// Search the system directory last (cscope's default; `-k` turns it off).
    pub system_includes: bool,
    /// `/usr/include`, or `INCDIR` from the environment at index build time.
    pub system_dir: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Sources {
    /// Files named on the command line.
    Files(Vec<Vec<u8>>),
    /// A list file (`-i`), read again at each refresh.
    List(Vec<u8>),
    /// cscope's automatic discovery, applied again at each refresh:
    /// `cscope.files` in the directory the index is built in if it can be opened for
    /// reading, otherwise the directory's source files, and with `recursive` its
    /// subdirectories'.
    Automatic { recursive: bool },
}

/// The list automatic discovery reads when present.
pub(crate) const AUTOMATIC_LIST_NAME: &str = "cscope.files";
pub(crate) const AUTOMATIC_LIST: &[u8] = AUTOMATIC_LIST_NAME.as_bytes();

/// What a selection rule names at one refresh.
enum Chosen<'s> {
    Files(&'s [Vec<u8>]),
    List(&'s [u8]),
    Directory { recursive: bool },
}

impl Sources {
    /// The sources an index build names, chosen as cscope chooses them: a list given with
    /// `-i` (file names beside it are not read), else the files named, else automatic
    /// discovery.
    pub(crate) fn named(list: Option<Vec<u8>>, files: Vec<Vec<u8>>, recursive: bool) -> Self {
        match list {
            Some(list) => Self::List(list),
            None if !files.is_empty() => Self::Files(files),
            None => Self::Automatic { recursive },
        }
    }

    /// What the sources name now: automatic discovery reads `cscope.files` when `root`
    /// holds one that can be opened for reading, as cscope checks it (`dir.c`, `vpaccess`);
    /// a directory passes that check too, and then names nothing.
    fn chosen(&self, root: &Path) -> Chosen<'_> {
        match self {
            Self::Files(files) => Chosen::Files(files),
            Self::List(list) => Chosen::List(list),
            Self::Automatic { recursive } => {
                let list = root.join(path_of(AUTOMATIC_LIST));
                if list.is_dir() || std::fs::File::open(&list).is_ok() {
                    Chosen::List(AUTOMATIC_LIST)
                } else {
                    Chosen::Directory {
                        recursive: *recursive,
                    }
                }
            }
        }
    }
}

impl Selection {
    pub(crate) fn encode(&self) -> Vec<u8> {
        let bytes = |value: &[u8]| Value::from(value.to_vec());
        let sources = match &self.sources {
            Sources::Files(files) => {
                json!({"files": files.iter().map(|f| bytes(f)).collect::<Vec<_>>()})
            }
            Sources::List(list) => json!({"list": bytes(list)}),
            Sources::Automatic { recursive } => json!({"automatic": recursive}),
        };
        json!({
            "root": bytes(self.root.as_os_str().as_encoded_bytes()),
            "sources": sources,
            "include_dirs": self.include_dirs.iter().map(|d| bytes(d)).collect::<Vec<_>>(),
            "environment_dirs": self.environment_dirs.iter().map(|d| bytes(d)).collect::<Vec<_>>(),
            "system_includes": self.system_includes,
            "system_dir": bytes(&self.system_dir),
        })
        .to_string()
        .into_bytes()
    }

    pub(crate) fn decode(encoded: &[u8]) -> Option<Self> {
        let value: Value = serde_json::from_slice(encoded).ok()?;
        let bytes = |value: &Value| -> Option<Vec<u8>> {
            value
                .as_array()?
                .iter()
                .map(|byte| byte.as_u64().and_then(|b| u8::try_from(b).ok()))
                .collect()
        };
        let sources = &value["sources"];
        let sources = if let Some(files) = sources.get("files") {
            Sources::Files(files.as_array()?.iter().map(bytes).collect::<Option<_>>()?)
        } else if let Some(list) = sources.get("list") {
            Sources::List(bytes(list)?)
        } else {
            Sources::Automatic {
                recursive: sources.get("automatic")?.as_bool()?,
            }
        };
        Some(Self {
            // The root was written from this platform's own path bytes.
            root: path_of(&bytes(&value["root"])?),
            sources,
            include_dirs: value["include_dirs"]
                .as_array()?
                .iter()
                .map(bytes)
                .collect::<Option<_>>()?,
            environment_dirs: value["environment_dirs"]
                .as_array()?
                .iter()
                .map(bytes)
                .collect::<Option<_>>()?,
            system_includes: value["system_includes"].as_bool()?,
            system_dir: bytes(&value["system_dir"])?,
        })
    }
}

/// What happens to a selected file that exists but cannot be read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Unreadable {
    /// A refresh keeps its last unit and tries it again next time.
    Keep,
    /// An index build leaves it out.
    Drop,
}

/// What a refresh did.
#[derive(Debug, Default)]
pub(crate) struct Report {
    pub revision: u64,
    /// A new revision was published.
    pub published: bool,
    pub files: usize,
    pub analysed: usize,
    /// Named files (on the command line or in a list) that do not exist or are not
    /// regular files (cscope's "cannot find file"), each time it is named, in canonical
    /// form and in the order named.
    pub missing: Vec<Vec<u8>>,
    /// Files and directories that could not be read.
    pub unreadable: Vec<(Vec<u8>, String)>,
}

#[derive(Debug)]
pub(crate) enum RefreshError {
    Store(StoreError),
    /// The selection rule cannot be applied: its file list cannot be read or parsed.
    Selection(String),
    /// None of the selected files can be indexed. The report says what was found on the
    /// way: cscope names each missing file before it gives up.
    NoSources(Box<Report>),
}

impl std::fmt::Display for RefreshError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Store(error) => error.fmt(f),
            Self::Selection(message) => f.write_str(message),
            Self::NoSources(_) => f.write_str("no source files found"),
        }
    }
}

impl From<StoreError> for RefreshError {
    fn from(error: StoreError) -> Self {
        Self::Store(error)
    }
}

/// How much of what the index holds a refresh keeps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Trust {
    /// A file whose metadata is unchanged, and whose bytes were hashed safely after its
    /// last change, keeps its analysis without being read. On Windows, whose metadata
    /// holds no change time, an edit made in place that keeps the size and restores the
    /// modification time is noticed only by `--verify` or `-u` (ADR 0007). No ordinary
    /// tool makes one.
    Metadata,
    /// Every file is read and hashed; one whose bytes are unchanged keeps its analysis
    /// (`--verify`).
    Bytes,
    /// Nothing: every file is analysed again and every stored analysis dropped (`-u`,
    /// cscope's "assume every file changed", which also recovers a damaged index).
    Nothing,
}

/// How a refresh treats the files it looks at.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Options {
    /// What happens to a listed file that cannot be read.
    pub unreadable: Unreadable,
    pub trust: Trust,
    /// How long to wait for another process's update.
    pub wait: Duration,
    /// What files are read with: `Analyzer::PRODUCT` for the product, the reference
    /// reading for the compass to compare with cscope.
    pub analyzer: Analyzer,
}

/// What a Refresh tells a session's watch ([`crate::watch`]): the rule and the revision it
/// starts from, before it looks at any file, and what the revision it leaves was made from.
pub(crate) trait Observer {
    /// The write lock is held and nothing has been looked at yet. `observed` holds the
    /// questions the last clean Refresh asked of the file system (see [`questions`]).
    fn starting(&mut self, selection: &Selection, files: &[StoredFile], observed: Option<&[u8]>);
    /// The Refresh left revision `revision` with these files, made from these answers,
    /// and found the `missing` named files absent. A Refresh that fails says nothing
    /// more after [`Observer::starting`].
    fn finished(
        &mut self,
        revision: u64,
        files: &[StoredFile],
        observed: &[u8],
        missing: &[Vec<u8>],
    );
}

/// Nobody watching.
impl Observer for () {
    fn starting(&mut self, _: &Selection, _: &[StoredFile], _: Option<&[u8]>) {}
    fn finished(&mut self, _: u64, _: &[StoredFile], _: &[u8], _: &[Vec<u8>]) {}
}

/// The names a record of include observations asks about, found or missing; `None` for a
/// record that cannot be read.
pub(crate) fn questions(observed: &[u8]) -> Option<Vec<&[u8]>> {
    let parsed = includes::questions(observed)?;
    Some(parsed.into_iter().map(|(name, _)| name).collect())
}

/// Re-applies the published selection rule, using `invoked` only if the transaction
/// sees no published revision after acquiring its write lock, and tells `observer` what
/// it starts from and what it leaves.
pub(crate) fn refresh(
    store: &mut Store,
    invoked: &Selection,
    options: Options,
    observer: &mut dyn Observer,
) -> Result<Report, RefreshError> {
    apply(store, invoked, options, true, observer)
}

/// Builds the index with the invocation's selection rule, replacing a published rule.
pub(crate) fn build(
    store: &mut Store,
    selection: &Selection,
    options: Options,
) -> Result<Report, RefreshError> {
    apply(store, selection, options, false, &mut ())
}

fn apply(
    store: &mut Store,
    invoked: &Selection,
    options: Options,
    use_stored_rule: bool,
    observer: &mut dyn Observer,
) -> Result<Report, RefreshError> {
    // Taken before any file is looked at, so an edit made during the update is never
    // taken for one made before its bytes were read.
    let stamp = now();
    profile_span!("refresh");
    let report = {
        let update = store.update(options.wait)?;
        let stored = if use_stored_rule {
            update.selection().map(decode_rule).transpose()?
        } else {
            None
        };
        let selection = stored.as_ref().unwrap_or(invoked);
        // The record of what the last clean Refresh observed, checked once: a digest of
        // what it observed of its selection, then the questions it asked.
        let observed = update.observed().map(<[u8]>::to_vec);
        let questions = observed
            .as_deref()
            .and_then(|observed| observed.get(OBSERVING..));
        observer.starting(selection, update.files(), questions);
        publish_captured(update, selection, options, stamp, observed, observer)?
    };
    if report.published {
        profile_span!("refresh_trim_log");
        store::trim_log(store);
    }
    Ok(report)
}

/// Owns capture through Publication beneath the caller's write lock and rule choice.
/// Dropping the real update on any error rolls back the whole attempt.
fn publish_captured(
    mut update: Update<'_>,
    selection: &Selection,
    options: Options,
    stamp: i64,
    observed: Option<Vec<u8>>,
    observer: &mut dyn Observer,
) -> Result<Report, RefreshError> {
    let Options {
        unreadable,
        trust,
        analyzer,
        ..
    } = options;
    // Another analyzer's units are all analysed again, so they go first, with their
    // name postings, rather than one by one once no file uses them.
    if trust == Trust::Nothing || update.analyzer() != analyzer.identity() {
        update.discard()?;
    }
    let keeps = trust != Trust::Nothing && update.analyzer() == analyzer.identity();
    let mut report = Report::default();
    let (selected, includes) = {
        profile_span!("refresh_selected");
        let kept = if keeps { update.files() } else { &[] };
        selected(selection, unreadable, kept, &mut report)?
    };
    let observing = observing(selection, analyzer, &selected, &includes);
    let mut work = Work {
        selection,
        includes,
        unreadable,
        rehash: trust != Trust::Metadata,
        analyzer,
        // Filled once the Refresh does not stop early, which needs no lookup by path.
        previous: HashMap::default(),
        stamp,
        update,
        report,
        kept_includes: None,
    };
    // A Refresh that sees the same selection, every file unchanged and the file system
    // answering every include's resolution as the last clean Refresh saw it would
    // publish the same revision: it stops here.
    if trust == Trust::Metadata
        && work.report.missing.is_empty()
        && work.report.unreadable.is_empty()
    {
        // Units another analyzer made were let go above, and with them the record.
        let observed = observed
            .filter(|_| keeps)
            .and_then(|observed| observed.strip_prefix(&observing[..]).map(<[u8]>::to_vec));
        if let Some(observed) = observed
            && work.unchanged(&observed)
        {
            #[cfg(test)]
            STOPPED_EARLY.with(|stopped| stopped.set(stopped.get() + 1));
            // The revision stays as it is, with nothing to record: the update is let go.
            let update = work.update;
            observer.finished(update.revision(), update.files(), &observed, &[]);
            return Ok(Report {
                revision: update.revision(),
                published: false,
                files: update.files().len(),
                ..Report::default()
            });
        }
    }
    if keeps {
        work.previous = work
            .update
            .files()
            .iter()
            .map(|file| (file.path.clone(), file.clone()))
            .collect();
    }
    let files = {
        profile_span!("refresh_closure");
        work.closure(selected)?
    };
    let Work {
        mut update,
        report,
        includes,
        ..
    } = work;
    let clean = report.missing.is_empty() && report.unreadable.is_empty();
    let observed = includes.observations();
    update.observing(clean.then(|| [&observing[..], &observed].concat()));
    let files: Vec<_> = files.into_iter().map(DiscoveredFile::stored).collect();
    let published = {
        profile_span!("refresh_finish");
        update.finish(&selection.encode(), &analyzer.identity(), &files)?
    };
    let (revision, published) = match published {
        Published::Revision(revision) => (revision, true),
        Published::Unchanged(revision) => (revision, false),
    };
    observer.finished(revision, &files, &observed, &report.missing);
    Ok(Report {
        revision,
        published,
        files: files.len(),
        ..report
    })
}

/// The logical cores this process may use.
fn cores() -> usize {
    std::thread::available_parallelism().map_or(1, usize::from)
}

/// `work` done for each of `items` on up to `threads` threads, the results in the items'
/// order. Each thread takes the next item as it finishes one, so uneven items do not
/// leave threads waiting.
pub(crate) fn each_on_threads<'t, T: Sync, R: Send>(
    items: &'t [T],
    threads: usize,
    work: impl Fn(&'t T) -> R + Sync,
) -> Vec<R> {
    let threads = threads.min(items.len());
    if threads <= 1 {
        return items.iter().map(work).collect();
    }
    use std::sync::atomic::{AtomicUsize, Ordering};
    let next = AtomicUsize::new(0);
    std::thread::scope(|scope| {
        let workers: Vec<_> = (0..threads)
            .map(|_| {
                let (next, work) = (&next, &work);
                scope.spawn(move || {
                    let mut done = Vec::new();
                    loop {
                        let at = next.fetch_add(1, Ordering::Relaxed);
                        let Some(item) = items.get(at) else {
                            return done;
                        };
                        done.push((at, work(item)));
                    }
                })
            })
            .collect();
        let mut done: Vec<_> = workers
            .into_iter()
            .flat_map(|worker| worker.join().expect("worker thread"))
            .collect();
        done.sort_unstable_by_key(|(at, _)| *at);
        done.into_iter().map(|(_, result)| result).collect()
    })
}

/// A file the Refresh found, with its stored unit and the include operands discovery
/// follows from it.
struct DiscoveredFile {
    path: Vec<u8>,
    stat: FileStat,
    hash: [u8; 32],
    verified: i64,
    unit: i64,
    includes: Includes,
}

/// A file's include operands, shared by the files that keep one stored unit.
type Includes = std::sync::Arc<[Include]>;

impl DiscoveredFile {
    fn kept(file: StoredFile, includes: Includes) -> Self {
        Self {
            path: file.path,
            stat: file.stat,
            hash: file.hash,
            verified: file.verified,
            unit: file.unit,
            includes,
        }
    }

    fn stored(self) -> StoredFile {
        StoredFile {
            path: self.path,
            unit: self.unit,
            stat: self.stat,
            hash: self.hash,
            verified: self.verified,
        }
    }
}

#[cfg(test)]
std::thread_local! {
    // How many Refreshes on this thread found nothing changed and stopped early.
    static STOPPED_EARLY: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

struct Work<'w, 's> {
    selection: &'w Selection,
    includes: IncludeResolver,
    unreadable: Unreadable,
    rehash: bool,
    analyzer: Analyzer,
    previous: HashMap<Vec<u8>, StoredFile>,
    /// When this update started.
    stamp: i64,
    update: Update<'s>,
    report: Report,
    /// The stored units' include operands, read in one pass when a file first keeps its
    /// unit: units are only dropped once the update finishes.
    kept_includes: Option<HashMap<i64, Includes>>,
}

impl Work<'_, '_> {
    /// Whether every file of the current revision is unchanged by its metadata and the
    /// file system answers every question `observed` records as before. A file modified
    /// close to its last reading is read again, and the Refresh goes on.
    fn unchanged(&mut self, observed: &[u8]) -> bool {
        profile_span!("refresh_unchanged");
        let files = self.update.files();
        let (root, analyzer) = (&self.selection.root, self.analyzer);
        let unchanged = each_on_threads(files, checking_threads(files.len()), |file| {
            matches!(
                probe(root, &file.path, Some(file), false, analyzer),
                Probe::Unchanged { .. }
            )
        });
        unchanged.into_iter().all(|unchanged| unchanged) && self.includes.observes(observed)
    }

    /// The include operands of a stored unit a file keeps.
    fn kept_includes(&mut self, unit: i64) -> Result<Includes, RefreshError> {
        if self.kept_includes.is_none() {
            let every = self.update.every_include()?;
            let shared = every.into_iter().map(|(unit, list)| (unit, list.into()));
            self.kept_includes = Some(shared.collect());
        }
        let every = self.kept_includes.as_ref().expect("read above");
        Ok(every
            .get(&unit)
            .cloned()
            .unwrap_or_else(|| Vec::new().into()))
    }

    /// The selected files and their include closure, level by level, each level sorted.
    /// As in cscope, a name a list gives twice is indexed twice, a name given twice on the
    /// command line once, and a named file that is missing or not a regular file is
    /// reported each time it is named, in canonical form and in the order named.
    fn closure(&mut self, selected: Selected) -> Result<Vec<DiscoveredFile>, RefreshError> {
        let Selected { names, named, once } = selected;
        // The names in the order named, to report the missing ones as cscope does.
        let given = named.then(|| names.clone());
        let mut paths = names;
        paths.sort();
        if once {
            paths.dedup();
        }
        let mut listed: HashSet<Vec<u8>> = paths.iter().cloned().collect();
        let mut missing = HashSet::default();
        let mut files = Vec::new();
        // Batches large enough to keep every core busy, small enough to bound the new
        // units held in memory.
        let batch = 4096;
        let mut first_level = true;
        while !paths.is_empty() {
            let mut next = BTreeSet::new();
            for paths in paths.chunks(batch) {
                let probes = self.probe_all(paths);
                let stored = self.store_units(&probes)?;
                for ((path, probe), unit) in paths.iter().zip(probes).zip(stored) {
                    if first_level && given.is_some() && matches!(probe, Probe::Missing) {
                        missing.insert(path.clone());
                    }
                    let settled = {
                        profile_span!("refresh_settle");
                        self.settle(path, probe, unit)?
                    };
                    let Some(file) = settled else {
                        continue;
                    };
                    profile_span!("refresh_discover");
                    for (operand, quoted) in file.includes.iter() {
                        if let Some(found) = self.includes.discover(operand, *quoted, &listed)
                            && listed.insert(found.clone())
                        {
                            next.insert(found);
                        }
                    }
                    files.push(file);
                }
            }
            if first_level {
                if let Some(given) = &given {
                    self.report.missing = given
                        .iter()
                        .filter(|name| missing.contains(*name))
                        .cloned()
                        .collect();
                }
                // cscope stops when none of the selected files can be indexed; the index
                // keeps what it had.
                if files.is_empty() {
                    return Err(RefreshError::NoSources(Box::new(std::mem::take(
                        &mut self.report,
                    ))));
                }
            }
            first_level = false;
            paths = next.into_iter().collect();
        }
        Ok(files)
    }

    /// Probes files on all cores, keeping their order: any of them may need reading and
    /// analysing, which costs far more than starting a thread.
    fn probe_all(&self, paths: &[Vec<u8>]) -> Vec<Probe> {
        profile_span!("refresh_probe_all");
        let (root, previous, rehash, analyzer) = (
            &self.selection.root,
            &self.previous,
            self.rehash,
            self.analyzer,
        );
        each_on_threads(paths, cores(), |path| {
            probe(root, path, previous.get(path), rehash, analyzer)
        })
    }

    /// Stores the units of the files among `probes` that were analysed, in their order,
    /// and gives each of those files its stored unit. Splitting units into blocks took most
    /// of the time this thread spent storing them, so each run of them is split on every
    /// core first, a run at a time to bound the blocks held at once.
    fn store_units(&mut self, probes: &[Probe]) -> Result<Vec<Option<i64>>, RefreshError> {
        profile_span!("refresh_store_units");
        let mut stored = Vec::with_capacity(probes.len());
        for probes in probes.chunks(512) {
            let units: Vec<_> = probes
                .iter()
                .filter_map(|probe| match probe {
                    Probe::Analysed { key, unit, .. } => Some((key, unit)),
                    _ => None,
                })
                .collect();
            let mut ids = Vec::with_capacity(units.len());
            if !units.is_empty() {
                let size = self.update.block_size()?;
                let splits = each_on_threads(&units, cores(), |(_, unit)| unit.split(size));
                for ((key, _), split) in units.iter().zip(&splits) {
                    ids.push(self.update.add_split(key, split)?);
                }
            }
            let mut ids = ids.into_iter();
            stored.extend(probes.iter().map(|probe| {
                matches!(probe, Probe::Analysed { .. }).then(|| ids.next().expect("a stored unit"))
            }));
        }
        Ok(stored)
    }

    /// The file's entry and its include operands, from its probe and, for a file analysed
    /// again, the unit [`Work::store_units`] stored; `None` for a file that is left out.
    fn settle(
        &mut self,
        path: &[u8],
        probe: Probe,
        stored: Option<i64>,
    ) -> Result<Option<DiscoveredFile>, RefreshError> {
        let previous = self.previous.get(path).cloned();
        Ok(match (probe, previous) {
            (Probe::Missing, _) => None,
            (Probe::Failed(error), previous) => {
                self.report.unreadable.push((path.to_vec(), error));
                match previous {
                    Some(previous) if self.unreadable == Unreadable::Keep => {
                        let includes = self.kept_includes(previous.unit)?;
                        Some(DiscoveredFile::kept(previous, includes))
                    }
                    _ => None,
                }
            }
            (Probe::Unchanged { stat }, Some(previous)) => {
                let includes = self.kept_includes(previous.unit)?;
                Some(DiscoveredFile::kept(
                    StoredFile { stat, ..previous },
                    includes,
                ))
            }
            (Probe::SameBytes { stat }, Some(previous)) => {
                let includes = self.kept_includes(previous.unit)?;
                let verified = self.stamp;
                Some(DiscoveredFile::kept(
                    StoredFile {
                        stat,
                        verified,
                        ..previous
                    },
                    includes,
                ))
            }
            (
                Probe::Analysed {
                    stat,
                    hash,
                    includes,
                    ..
                },
                _,
            ) => {
                self.report.analysed += 1;
                Some(DiscoveredFile {
                    path: path.to_vec(),
                    unit: stored.expect("an analysed file's unit is stored"),
                    stat,
                    hash,
                    verified: self.stamp,
                    includes: includes.into(),
                })
            }
            (Probe::Unchanged { .. } | Probe::SameBytes { .. }, None) => None,
        })
    }
}

/// The selected source names, in canonical form.
fn selected(
    selection: &Selection,
    policy: Unreadable,
    previous: &[StoredFile],
    report: &mut Report,
) -> Result<(Selected, IncludeResolver), RefreshError> {
    let root = &selection.root;
    let mut list_dirs = Vec::new();
    let mut system_includes = selection.system_includes;
    let (names, named, once) = match selection.sources.chosen(root) {
        Chosen::Files(files) => (files.to_vec(), true, true),
        Chosen::List(list) => {
            let contents = read_list(root, list)?;
            list_dirs = contents.include_dirs;
            if contents.kernel {
                system_includes = false;
            }
            (contents.names, true, false)
        }
        Chosen::Directory { recursive } => {
            let (mut found, unreadable) = discover(root, recursive);
            for directory in unreadable {
                // The files of a directory that cannot be read are treated like
                // files that cannot be read: the sources it held keep their last facts.
                if policy == Unreadable::Keep {
                    let mut kept = HashSet::default();
                    found.extend(
                        previous
                            .iter()
                            .map(|file| &file.path)
                            .filter(|path| {
                                within(path, &directory, recursive)
                                    && is_source_name(file_name(path))
                                    && kept.insert(path.as_slice())
                            })
                            .cloned(),
                    );
                }
                let shown = if directory.is_empty() {
                    b".".to_vec()
                } else {
                    directory
                };
                report
                    .unreadable
                    .push((shown, "cannot read the directory".into()));
            }
            (found, false, true)
        }
    };
    let names: Vec<Vec<u8>> = names.iter().map(|name| canonical_path(name)).collect();
    if names.is_empty() {
        return Err(RefreshError::NoSources(Box::new(std::mem::take(report))));
    }
    Ok((
        Selected { names, named, once },
        IncludeResolver::new(selection, &list_dirs, system_includes),
    ))
}

/// The length of [`observing`]'s digest, which begins a revision's record of what its
/// Refresh observed.
const OBSERVING: usize = 32;

/// What a Refresh records of its selection before it looks at any file: its rule, its
/// analyzer, the names it selected and how includes are searched for, hashed.
fn observing(
    selection: &Selection,
    analyzer: Analyzer,
    selected: &Selected,
    includes: &IncludeResolver,
) -> [u8; OBSERVING] {
    let mut hash = HashInput(Vec::new());
    for part in [
        selection.encode(),
        analyzer.identity(),
        includes.configuration(),
    ] {
        hash.update((part.len() as u64).to_le_bytes());
        hash.update(part);
    }
    hash.update([u8::from(selected.named), u8::from(selected.once)]);
    for name in &selected.names {
        hash.update((name.len() as u64).to_le_bytes());
        hash.update(name);
    }
    blake3::hash(&hash.0).into()
}

/// A hash's input, collected whole and hashed at once, which lets BLAKE3 hash many
/// chunks in parallel lanes.
struct HashInput(Vec<u8>);

impl HashInput {
    fn update(&mut self, bytes: impl AsRef<[u8]>) {
        self.0.extend_from_slice(bytes.as_ref());
    }
}

/// The names a selection rule gives at one refresh.
struct Selected {
    /// In canonical form, in the order named or found.
    names: Vec<Vec<u8>>,
    /// Named on the command line or in a list, rather than found in a directory.
    named: bool,
    /// Each file is indexed once however often it is named (all but lists).
    once: bool,
}

/// Checks, before a new index is made for `selection`, that it finds a file to index: an
/// index build that finds none then leaves no index behind, as cscope stops before it
/// writes its database. A named file counts when it is a regular file that can be opened
/// for reading, as in cscope; the error names each one that is not.
pub(crate) fn check_sources(selection: &Selection) -> Result<(), RefreshError> {
    let root = &selection.root;
    let mut report = Report::default();
    // Whether the file can be indexed, telling the report why not.
    let check = |name: Vec<u8>, named: bool, report: &mut Report| {
        let full = root.join(path_of(&name));
        // Stated first: opening a FIFO would wait for a writer.
        if !std::fs::metadata(&full).is_ok_and(|metadata| metadata.is_file()) {
            if named {
                report.missing.push(name);
            }
            return false;
        }
        match std::fs::File::open(&full) {
            Ok(_) => true,
            Err(error) => {
                report.unreadable.push((name, error.to_string()));
                false
            }
        }
    };
    let found = match selection.sources.chosen(root) {
        Chosen::Files(files) => files
            .iter()
            .any(|name| check(canonical_path(name), true, &mut report)),
        Chosen::List(list) => read_list(root, list)?
            .names
            .iter()
            .any(|name| check(canonical_path(name), true, &mut report)),
        Chosen::Directory { recursive } => walk(root, recursive, |name| {
            if check(name, false, &mut report) {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        })
        .is_break(),
    };
    if found {
        Ok(())
    } else {
        Err(RefreshError::NoSources(Box::new(report)))
    }
}

/// The file list `list`, read from `root`.
fn read_list(root: &Path, list: &[u8]) -> Result<ListContents, RefreshError> {
    let bytes = std::fs::read(root.join(path_of(list))).map_err(|error| {
        RefreshError::Selection(format!(
            "cannot read the file list {}: {error}",
            String::from_utf8_lossy(list)
        ))
    })?;
    parse_list(&bytes).map_err(|error| {
        RefreshError::Selection(format!(
            "file list {}: {error}",
            String::from_utf8_lossy(list)
        ))
    })
}

/// The last name of a `/`-separated path.
fn file_name(path: &[u8]) -> &[u8] {
    path.rsplit(|byte| *byte == b'/').next().unwrap_or(path)
}

/// Whether a name relative to the directory the index is built in leads out of it: it
/// climbs out (`../`), is rooted (`/usr/include/...`) or has a Windows drive.
pub(crate) fn outside(name: &[u8]) -> bool {
    name.first() == Some(&b'/')
        || name == b".."
        || name.starts_with(b"../")
        || (cfg!(windows) && name.get(1) == Some(&b':'))
}

/// Whether the canonical `path` lies in `directory`, relative to the directory the index
/// is built in (that directory itself when empty): directly, or with `recursive` at any
/// depth. A path that climbs out (`../`), is rooted (`/usr/include/...`) or has a Windows
/// drive lies outside.
fn within(path: &[u8], directory: &[u8], recursive: bool) -> bool {
    if outside(path) {
        return false;
    }
    let rest = if directory.is_empty() {
        path
    } else {
        match path
            .strip_prefix(directory)
            .and_then(|rest| rest.strip_prefix(b"/"))
        {
            Some(rest) => rest,
            None => return false,
        }
    };
    recursive || !rest.contains(&b'/')
}

/// What a file's metadata and bytes say about it, worked out on any thread.
enum Probe {
    /// Its metadata matches the stored file's, and its bytes were hashed safely after its
    /// last modification.
    Unchanged { stat: FileStat },
    /// Its bytes hash as the stored file's did.
    SameBytes { stat: FileStat },
    Analysed {
        stat: FileStat,
        hash: [u8; 32],
        key: [u8; 32],
        unit: AnalysisUnit,
        includes: Vec<Include>,
    },
    /// It does not exist, or is not a regular file (cscope's "cannot find file").
    Missing,
    /// It exists but cannot be read.
    Failed(String),
}

fn probe(
    root: &Path,
    path: &[u8],
    previous: Option<&StoredFile>,
    rehash: bool,
    analyzer: Analyzer,
) -> Probe {
    let full = below(root, path);
    let failed = |error: std::io::Error| match error.kind() {
        std::io::ErrorKind::NotFound => Probe::Missing,
        _ => Probe::Failed(error.to_string()),
    };
    let (stat, opened) = match stat_of(&full) {
        Ok(Some(found)) => found,
        Ok(None) => return Probe::Missing,
        Err(error) => return failed(error),
    };
    if let Some(previous) = previous
        && !rehash
        && previous.stat == stat
        && stat.modified < previous.verified - RACY_NANOSECONDS
    {
        return Probe::Unchanged { stat };
    }
    let bytes = match read_bytes(&full, opened) {
        Ok(bytes) => bytes,
        Err(error) => return failed(error),
    };
    let hash: [u8; 32] = Sha256::digest(&bytes).into();
    if previous.is_some_and(|previous| previous.hash == hash) {
        return Probe::SameBytes { stat };
    }
    match analyzer.analyse(path, &bytes) {
        Ok(unit) => Probe::Analysed {
            stat,
            hash,
            key: analyzer.key(path, &hash),
            includes: unit.includes(),
            unit,
        },
        Err(error) => Probe::Failed(error.to_string()),
    }
}

/// A path as cscope stores it (its canonical form). On Windows `\` separates names as
/// `/` does and is stored as `/`, as directory discovery and includes spell paths, so a
/// path given with either separator is one path; a drive or share (`C:`,
/// `//server/share`) stays in front, and `..` never climbs above it. A verbatim or
/// device path (`\\?\`, `\\.\`) keeps its spelling.
fn canonical_path(path: &[u8]) -> Vec<u8> {
    if !cfg!(windows) {
        return scanner::canonical_path(path);
    }
    if path.starts_with(b"\\\\?\\") || path.starts_with(b"\\\\.\\") {
        return path.to_vec();
    }
    let mut path = forward_slashes(path.to_vec());
    let prefix = windows_prefix(&path);
    // Windows reads a drive letter in either case; `C:` is the spelling it gives back.
    if prefix == 2 {
        path[0].make_ascii_uppercase();
    }
    let (prefix, rest) = path.split_at(prefix);
    let mut canonical = prefix.to_vec();
    if prefix.is_empty() {
        canonical = scanner::canonical_path(rest);
    } else if rest.first() == Some(&b'/') {
        // Rooted below the drive or share, where `..` stops.
        let mut below = scanner::canonical_path(rest);
        while below.starts_with(b"/..") && matches!(below.get(3), None | Some(b'/')) {
            below.drain(..3);
        }
        if below.is_empty() {
            below.push(b'/');
        }
        canonical.extend(below);
    } else if !rest.is_empty() {
        canonical.extend(scanner::canonical_path(rest));
    }
    canonical
}

/// The length of a Windows path's drive (`C:`) or share (`//server/share`) prefix, or 0.
fn windows_prefix(path: &[u8]) -> usize {
    if path.len() >= 2 && path[1] == b':' && path[0].is_ascii_alphabetic() {
        return 2;
    }
    if path.starts_with(b"//") && path.get(2).is_some_and(|byte| *byte != b'/') {
        // The prefix ends where the third name would start.
        let mut separators = path[2..]
            .iter()
            .enumerate()
            .filter(|(_, byte)| **byte == b'/');
        return separators.nth(1).map_or(path.len(), |(at, _)| at + 2);
    }
    0
}

/// On Windows `\` separates names as `/` does; `/` is kept, as cscope and directory
/// discovery spell paths.
fn forward_slashes(mut path: Vec<u8>) -> Vec<u8> {
    if cfg!(windows) {
        for byte in &mut path {
            if *byte == b'\\' {
                *byte = b'/';
            }
        }
    }
    path
}

/// A stored path as the file system's: on Unix its bytes exactly; on Windows, where a
/// path given in bytes that are not UTF-8 cannot name a file, as UTF-8.
pub(crate) fn path_of(bytes: &[u8]) -> PathBuf {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        PathBuf::from(std::ffi::OsStr::from_bytes(bytes))
    }
    #[cfg(not(unix))]
    PathBuf::from(String::from_utf8_lossy(bytes).into_owned())
}

/// `name` below `root`, as `root.join(path_of(name))` makes it but in one allocation: a
/// freshness check makes one for every file and include question it asks about.
pub(crate) fn below(root: &Path, name: &[u8]) -> PathBuf {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let name = Path::new(std::ffi::OsStr::from_bytes(name));
        let mut path = PathBuf::with_capacity(root.as_os_str().len() + 1 + name.as_os_str().len());
        path.push(root);
        path.push(name);
        path
    }
    #[cfg(not(unix))]
    root.join(path_of(name))
}

/// What a file list holds.
#[derive(Debug, Default, PartialEq, Eq)]
struct ListContents {
    names: Vec<Vec<u8>>,
    include_dirs: Vec<Vec<u8>>,
    /// `-k`: leave out the system directory.
    kernel: bool,
}

/// A file list as cscope reads one: whitespace-separated names, `"quoted names"` with `\"`
/// and `\\` escaped, and the options `-I DIR` (or `-IDIR`) and `-k`. `-q`, `-c` and `-p N`
/// change nothing here: they ask for cscope's fast index, which ripscope always has, stop
/// cscope compressing its own database, and set how many path components its full-screen
/// display shows. A `-p` with no number after it stands alone, as cscope leaves an
/// unfinished option. `-T` would change cscope's answers, so a list naming it is refused.
fn parse_list(bytes: &[u8]) -> Result<ListContents, String> {
    let mut contents = ListContents::default();
    for (number, line) in bytes.split(|byte| *byte == b'\n').enumerate() {
        // The option, `-I` or `-p`, whose value may be the next entry on this line.
        let mut awaiting = None;
        let mut at = 0;
        loop {
            while line.get(at).is_some_and(u8::is_ascii_whitespace) {
                at += 1;
            }
            if at >= line.len() {
                break;
            }
            let (token, quoted) = if line[at] == b'"' {
                let mut token = Vec::new();
                at += 1;
                while at < line.len() && line[at] != b'"' {
                    if line[at] == b'\\' && matches!(line.get(at + 1), Some(b'"' | b'\\')) {
                        at += 1;
                    }
                    token.push(line[at]);
                    at += 1;
                }
                at += 1;
                (token, true)
            } else {
                let start = at;
                while at < line.len() && !line[at].is_ascii_whitespace() {
                    at += 1;
                }
                (line[start..at].to_vec(), false)
            };
            match awaiting.take() {
                Some(b'I') => {
                    contents.include_dirs.push(token);
                    continue;
                }
                Some(b'p') if token.first().is_some_and(u8::is_ascii_digit) => continue,
                _ => {}
            }
            if quoted || token.first() != Some(&b'-') {
                contents.names.push(token);
                continue;
            }
            match token.get(1) {
                Some(b'I') if token.len() > 2 => contents.include_dirs.push(token[2..].to_vec()),
                Some(&option @ (b'I' | b'p')) if token.len() == 2 => awaiting = Some(option),
                Some(b'k') => contents.kernel = true,
                Some(b'q' | b'c' | b'p') => {}
                _ => {
                    return Err(format!(
                        "line {}: option {} is not supported",
                        number + 1,
                        String::from_utf8_lossy(&token)
                    ));
                }
            }
        }
        if awaiting == Some(b'I') {
            return Err(format!("line {}: -I needs a directory", number + 1));
        }
    }
    Ok(contents)
}

/// Source files in `root`, and with `recursive` in its subdirectories, chosen by name as
/// cscope chooses them; and the directories that could not be read.
fn discover(root: &Path, recursive: bool) -> (Vec<Vec<u8>>, Vec<Vec<u8>>) {
    let mut found = Vec::new();
    let walked = walk(root, recursive, |path| {
        found.push(path);
        ControlFlow::Continue(())
    });
    let unreadable = match walked {
        ControlFlow::Continue(unreadable) => unreadable,
        ControlFlow::Break(()) => Vec::new(),
    };
    (found, unreadable)
}

/// Visits the source files `discover` finds, until `visit` breaks; otherwise gives the
/// directories that could not be read.
fn walk(
    root: &Path,
    recursive: bool,
    mut visit: impl FnMut(Vec<u8>) -> ControlFlow<()>,
) -> ControlFlow<(), Vec<Vec<u8>>> {
    let slashes = |path: &Path| forward_slashes(path.as_os_str().as_encoded_bytes().to_vec());
    let mut unreadable = Vec::new();
    let mut pending = vec![PathBuf::new()];
    while let Some(relative) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(root.join(&relative)) else {
            unreadable.push(slashes(&relative));
            continue;
        };
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            let path = relative.join(entry.file_name());
            if kind.is_dir() && recursive {
                pending.push(path);
            } else if kind.is_file() && is_source_name(entry.file_name().as_encoded_bytes()) {
                visit(slashes(&path))?;
            }
        }
    }
    ControlFlow::Continue(unreadable)
}

/// cscope's source suffixes; `s.name` and `S.name` (SCCS files) are left out.
pub(crate) fn is_source_name(name: &[u8]) -> bool {
    let Some(dot) = name.iter().rposition(|byte| *byte == b'.') else {
        return false;
    };
    if name.get(1) == Some(&b'.') && dot != 1 && matches!(name.first(), Some(b's' | b'S')) {
        return false;
    }
    matches!(
        &name[dot + 1..],
        b"c" | b"h"
            | b"l"
            | b"y"
            | b"C"
            | b"G"
            | b"H"
            | b"L"
            | b"bp"
            | b"qc"
            | b"qh"
            | b"sd"
            | b"cc"
            | b"hh"
            | b"tcc"
            | b"cpp"
            | b"cxx"
            | b"hpp"
            | b"hxx"
    )
}

/// A regular file's stat, or `None` for anything else. On Windows the file is opened to
/// learn its identity, and the handle is returned to read the bytes through.
pub(crate) fn stat_of(full: &Path) -> std::io::Result<Option<(FileStat, Option<std::fs::File>)>> {
    Ok(match entry_of(full)? {
        Entry::File(stat, opened) => Some((stat, opened)),
        Entry::Directory(_) | Entry::Other => None,
    })
}

/// What a path leads to, with the stat change detection compares for a regular file or a
/// directory: a directory's times move when a name in it is made, removed or renamed.
pub(crate) enum Entry {
    /// A regular file, and on Windows the file opened to learn its identity.
    File(FileStat, Option<std::fs::File>),
    Directory(FileStat),
    /// Anything else, such as a FIFO. Windows opens all but directories as files.
    #[cfg_attr(windows, allow(dead_code))]
    Other,
}

/// What `full` leads to, following links.
#[cfg(unix)]
pub(crate) fn entry_of(full: &Path) -> std::io::Result<Entry> {
    use std::os::unix::fs::MetadataExt;
    // A stat, not an open: opening a FIFO would wait for a writer.
    let metadata = std::fs::metadata(full)?;
    // The inode number alone: the device number can change across mounts (btrfs).
    let identity = metadata.ino().to_le_bytes().to_vec();
    Ok(if metadata.is_file() {
        Entry::File(file_stat(&metadata, identity), None)
    } else if metadata.is_dir() {
        Entry::Directory(file_stat(&metadata, identity))
    } else {
        Entry::Other
    })
}

#[cfg(windows)]
pub(crate) fn entry_of(full: &Path) -> std::io::Result<Entry> {
    use std::os::windows::fs::OpenOptionsExt;
    use winx::file::Flags;
    const FILE_ATTRIBUTE_DIRECTORY: u64 = 0x10;
    // FILETIME counts 100 ns from 1601; a stat counts nanoseconds from 1970.
    const FROM_1601: u64 = 116_444_736_000_000_000;
    // With no access rights, as `std::fs::metadata` opens it: this works where reading is
    // denied or another process holds the file without sharing it. The flag also opens a
    // directory.
    let file = std::fs::OpenOptions::new()
        .access_mode(0)
        .custom_flags(Flags::FILE_FLAG_BACKUP_SEMANTICS.bits())
        .open(full)?;
    let information = winx::winapi_util::file::information(&file)?;
    let nanoseconds = |time: Option<u64>| {
        time.and_then(|time| time.checked_sub(FROM_1601))
            .map_or(0, |time| {
                i64::try_from(time).map_or(i64::MAX, |t| t.saturating_mul(100))
            })
    };
    // The volume serial number and file index tell a re-created file or directory from the
    // one it replaced, where the size and times come out the same (NTFS gives one created
    // within seconds under a deleted one's name that one's creation time).
    let mut identity = information.volume_serial_number().to_le_bytes().to_vec();
    identity.extend_from_slice(&information.file_index().to_le_bytes());
    let stat = FileStat {
        size: information.file_size(),
        modified: nanoseconds(information.last_write_time()),
        changed: nanoseconds(information.creation_time()),
        identity,
    };
    Ok(
        if information.file_attributes() & FILE_ATTRIBUTE_DIRECTORY != 0 {
            Entry::Directory(stat)
        } else {
            Entry::File(stat, Some(file))
        },
    )
}

/// The file's bytes. On Windows they are read through the file that was stat'ed, reopened
/// for reading.
#[cfg(windows)]
fn read_bytes(full: &Path, opened: Option<std::fs::File>) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    use std::os::windows::io::AsHandle;
    use winx::file::{AccessMode, Flags, ShareMode};
    let Some(file) = opened else {
        return std::fs::read(full);
    };
    let mut reader = winx::file::reopen_file(
        file.as_handle(),
        AccessMode::FILE_GENERIC_READ,
        ShareMode::FILE_SHARE_READ | ShareMode::FILE_SHARE_WRITE | ShareMode::FILE_SHARE_DELETE,
        Flags::empty(),
    )?;
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes)?;
    Ok(bytes)
}

#[cfg(not(windows))]
fn read_bytes(full: &Path, _opened: Option<std::fs::File>) -> std::io::Result<Vec<u8>> {
    std::fs::read(full)
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn entry_of(full: &Path) -> std::io::Result<Entry> {
    let metadata = std::fs::metadata(full)?;
    Ok(if metadata.is_file() {
        Entry::File(file_stat(&metadata, Vec::new()), None)
    } else if metadata.is_dir() {
        Entry::Directory(file_stat(&metadata, Vec::new()))
    } else {
        Entry::Other
    })
}

#[cfg(not(windows))]
fn file_stat(metadata: &std::fs::Metadata, identity: Vec<u8>) -> FileStat {
    let nanoseconds = |time: std::io::Result<std::time::SystemTime>| {
        time.ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |duration| {
                i64::try_from(duration.as_nanos()).unwrap_or(i64::MAX)
            })
    };
    #[cfg(unix)]
    let changed = {
        use std::os::unix::fs::MetadataExt;
        metadata.ctime() * 1_000_000_000 + metadata.ctime_nsec()
    };
    #[cfg(not(unix))]
    let changed = nanoseconds(metadata.created());
    FileStat {
        size: metadata.len(),
        modified: nanoseconds(metadata.modified()),
        changed,
        identity,
    }
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| {
            i64::try_from(duration.as_nanos()).unwrap_or(i64::MAX)
        })
}
