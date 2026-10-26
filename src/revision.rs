//! The index revision a query reads (CONTEXT.md): the files it needs in database order and
//! their Analysis units, read from the index in one transaction for that query, or made
//! in memory for the compass's `--units-query` and tests. It is the one place queries
//! read a revision: which files a query needs (looked up in the index's name postings or
//! include operands), which blocks of their units are read (ADR 0011), each unit's once
//! however many files share it, and which files are left out and why. It is the
//! [`Source`] stored queries answer from.
//!
//! A query reads only what it needs: its paths alone for field 7 and text searches, the
//! blocks its postings list for an exact name it asks for, every block of the files
//! holding a name its pattern matches, of the files with a matching include operand for
//! field 8, or of every file (the oracle). It reads them in parts, which other threads may
//! walk while it reads the next, so that a query holds a few parts' blocks at a time
//! ([`Revision::read_in_parts`]). A session keeps the blocks it decoded, up to a budget,
//! for the queries after.

use crate::analyzer::Analyzer;
use crate::engine::{Indexed, Reads, Source};
use crate::refresh;
use crate::store::{
    Access, Block, Included, Includers, Paths, Reader, RevisionId, Store, StoreError,
};
use crate::unit::AnalysisUnit;
use std::collections::HashMap;
use std::convert::Infallible;
use std::path::{Path, PathBuf};
use std::sync::Arc;

mod held;
pub(crate) use held::{FileIdentity, HeldIndex};

/// The index a query reads: opened for that query alone, or held by a session.
#[derive(Clone, Copy)]
pub(crate) enum Index<'i> {
    Path(&'i Path),
    Held(&'i HeldIndex),
    /// The index at the path, already open for reading in this process.
    Opened(&'i Path, &'i Store),
}

impl<'i> From<&'i Path> for Index<'i> {
    fn from(path: &'i Path) -> Self {
        Self::Path(path)
    }
}

impl<'i> From<&'i PathBuf> for Index<'i> {
    fn from(path: &'i PathBuf) -> Self {
        Self::Path(path)
    }
}

impl<'i> From<&'i HeldIndex> for Index<'i> {
    fn from(held: &'i HeldIndex) -> Self {
        Self::Held(held)
    }
}

impl Index<'_> {
    /// Runs `read` on the index as it is now.
    fn with<R>(self, read: impl FnOnce(&Store) -> Result<R, StoreError>) -> Result<R, StoreError> {
        match self {
            Self::Path(path) => read(&Store::open(path, Access::Read)?),
            Self::Held(held) => held.with(read),
            Self::Opened(_, store) => read(store),
        }
    }
}

/// How many bytes of block images a part of a revision read in parts holds before it ends.
/// Walked on the thread that read them, small parts answered fastest, probably because a
/// unit walked soon after it is decoded is still in the processor's caches and its memory
/// is used again: on the Linux kernel, parts of 1 MiB answered 3 to 10% faster than parts
/// of 64 MiB, in a fifth to two fifths of the memory, and parts of one unit no faster.
/// Walked on other threads, they were not measured again.
pub(crate) const PART: usize = 1 << 20;

/// The revision as one query reads it.
pub(crate) struct Revision {
    root: PathBuf,
    /// The files read, in database order, each with the slot of its unit; none for a
    /// revision read for its paths.
    files: Vec<(Vec<u8>, usize)>,
    /// For a revision read for its paths, every indexed path, in database order.
    paths: Option<Arc<Paths>>,
    /// Each distinct unit read: the blocks of it the query reads, in order, each checked
    /// once, or why they cannot be read; none for a revision read for its paths.
    units: Option<Vec<Result<Vec<Arc<Block>>, StoreError>>>,
    /// How many files this version cannot read, since another version made their units.
    other_version: usize,
    /// The name postings the query looked up cannot be read, so it read every file.
    postings_damaged: bool,
    /// For an exact name, each unit's answer rows for it, by slot
    /// ([`crate::unit::Occurrences`]).
    rows: Option<Vec<Vec<u8>>>,
    /// For field 8, where each unit's matching includes sit, by slot.
    included: Option<Vec<Vec<Included>>>,
}

/// What a query leaves out of its answer because it cannot read a file's unit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct LeftOut {
    /// Files whose units another version of ripscope made.
    pub other_version: usize,
    /// Files whose unit is damaged or cannot be read from the index.
    pub damaged: usize,
}

impl std::ops::AddAssign for LeftOut {
    fn add_assign(&mut self, other: Self) {
        self.other_version += other.other_version;
        self.damaged += other.damaged;
    }
}

impl Revision {
    /// The index's current revision, as `reads` needs it: the paths alone for field 7 and
    /// text searches, else the files that may hold what the query reads, with the blocks of
    /// their units it reads, those of a unit several files share read once. `units` keeps
    /// what a session decoded between its queries. An index that holds no published
    /// revision is no index yet.
    pub(crate) fn read<'i>(
        index: impl Into<Index<'i>>,
        reads: &Reads<'_>,
        units: &mut Units,
    ) -> Result<Self, String> {
        let mut whole = None;
        let read = Self::read_in_parts(index, reads, units, usize::MAX, |revision| {
            whole = Some(revision);
            Ok::<(), Infallible>(())
        })?;
        let Ok(()) = read;
        Ok(whole.expect("a revision read whole comes in one part"))
    }

    /// The index's current revision as [`Revision::read`] reads it, given to `each` in one
    /// part or more, in database order, each part's blocks let go once `each` drops the
    /// part (unless `units` keeps them). A part ends, between files, once the blocks it
    /// read hold `part` bytes of images ([`PART`] for a query); every file comes in one part
    /// when the query's context carries from file to file ([`Reads::Every`]). The outer
    /// error is the index's, the inner one the first that `each` gives, which ends the
    /// reading.
    pub(crate) fn read_in_parts<'i, E>(
        index: impl Into<Index<'i>>,
        reads: &Reads<'_>,
        units: &mut Units,
        part: usize,
        each: impl FnMut(Self) -> Result<(), E>,
    ) -> Result<Result<(), E>, String> {
        profile_span!("revision_read_in_parts");
        let index = index.into();
        let mut read = None;
        index
            .with(|store| {
                read = Some(Self::read_opened(index, store, reads, units, part, each));
                Ok(())
            })
            .map_err(|e| e.to_string())?;
        read.expect("read once the index is open")
    }

    /// [`Self::read_in_parts`] from the opened `store`.
    fn read_opened<E>(
        index: Index<'_>,
        store: &Store,
        reads: &Reads<'_>,
        units: &mut Units,
        part: usize,
        mut each: impl FnMut(Self) -> Result<(), E>,
    ) -> Result<Result<(), E>, String> {
        let reader = store.reader().map_err(|e| e.to_string())?;
        units.read = Some(reader.revision_id().clone());
        if reader.revision_id().number == 0 {
            return Err(match index {
                Index::Path(path) | Index::Opened(path, _) => unpublished(path),
                Index::Held(held) => unpublished(held.path()),
            });
        }
        let root = units.root(&reader).map_err(|e| e.to_string())?;
        let failed = |error: StoreError| error.to_string();
        let mut postings_damaged = false;
        // For an exact name, the blocks its postings list in each unit; for anything else,
        // every block of each unit read.
        let mut unit_rows: Option<crate::hashing::HashMap<i64, Vec<u8>>> = None;
        let mut includers: Option<Includers> = None;
        let (files, listed) = match reads {
            Reads::Paths => {
                let paths = units.paths(&reader).map_err(failed)?;
                return Ok(each(Self {
                    root,
                    files: Vec::new(),
                    paths: Some(paths),
                    units: None,
                    other_version: 0,
                    postings_damaged: false,
                    rows: None,
                    included: None,
                }));
            }
            // A revision never mixes two analyzers, so its analyzer tells whether this
            // version can read any of its units.
            _ if !Analyzer::reads_units_of(reader.analyzer()) => {
                return Ok(each(Self {
                    root,
                    files: Vec::new(),
                    paths: None,
                    units: Some(Vec::new()),
                    other_version: reader.count().map_err(failed)?,
                    postings_damaged: false,
                    rows: None,
                    included: None,
                }));
            }
            Reads::Names { roles, pattern } => {
                let found = match pattern.exact_name() {
                    Some(name) => {
                        reader
                            .holding_rows(name, roles.mask())
                            .and_then(|(listed, rows)| {
                                let files = units.files(&reader, listed.keys().copied())?;
                                unit_rows = Some(rows);
                                Ok((files, Some(listed)))
                            })
                    }
                    None => {
                        let mut wanted = roles.wanted(pattern);
                        let prefixes = pattern.name_prefixes();
                        reader
                            .holding_any_units(prefixes, |name| wanted.roles(name))
                            .and_then(|found| Ok((units.files(&reader, found)?, None)))
                    }
                };
                match found {
                    // Every file answers the same, only slower.
                    Err(StoreError::Postings) => {
                        postings_damaged = true;
                        reader.files().map(|files| (files, None))
                    }
                    found => found,
                }
            }
            Reads::Includes { pattern } => {
                let mut folded = Vec::new();
                let matches = |operand: &[u8]| pattern.matches_name(operand, &mut folded);
                let found = if units.budget == 0 {
                    reader.including(matches)
                } else {
                    units
                        .including(&reader, matches)
                        .and_then(|found| Ok((units.files(&reader, found.keys().copied())?, found)))
                };
                // The includes' recorded rows answer, so no blocks are read.
                found.map(|(files, found)| {
                    includers = Some(found);
                    (files, None)
                })
            }
            Reads::Every => reader.files().map(|files| (files, None)),
        }
        .map_err(failed)?;
        let part = if matches!(reads, Reads::Every) {
            usize::MAX
        } else {
            part
        };
        let mut files = files.into_iter().peekable();
        loop {
            let (mut slots, mut read, mut bytes) = (HashMap::new(), Vec::new(), 0);
            let mut rows = unit_rows.as_ref().map(|_| Vec::new());
            let mut included = includers.as_ref().map(|_| Vec::new());
            let mut taken = Vec::new();
            while (taken.is_empty() || bytes < part)
                && let Some((path, unit)) = files.next()
            {
                let slot = *slots.entry(unit).or_insert_with(|| {
                    let which = listed
                        .as_ref()
                        .map(|listed| listed.get(&unit).map_or(&[][..], Vec::as_slice));
                    let blocks = if includers.is_some() {
                        Ok(Vec::new())
                    } else {
                        units.get(&reader, unit, which)
                    };
                    bytes += blocks.as_ref().map_or(0, |blocks| bytes_of(blocks));
                    read.push(blocks);
                    // A unit's files can fall in several parts, each with its rows.
                    if let (Some(rows), Some(by_unit)) = (rows.as_mut(), unit_rows.as_ref()) {
                        rows.push(by_unit.get(&unit).cloned().unwrap_or_default());
                    }
                    if let (Some(included), Some(by_unit)) = (included.as_mut(), &includers) {
                        included.push(by_unit.get(&unit).cloned().unwrap_or_default());
                    }
                    read.len() - 1
                });
                taken.push((path, slot));
            }
            let revision = Self {
                root: root.clone(),
                files: taken,
                paths: None,
                units: Some(read),
                other_version: 0,
                postings_damaged,
                rows,
                included,
            };
            if let Err(error) = each(revision) {
                return Ok(Err(error));
            }
            if files.peek().is_none() {
                return Ok(Ok(()));
            }
        }
    }

    /// Files analysed in memory, read in the order given, which the caller makes the
    /// database order, each unit as one block.
    pub(crate) fn in_memory(files: Vec<(Vec<u8>, AnalysisUnit)>) -> Self {
        let (files, units) = files
            .into_iter()
            .enumerate()
            .map(|(slot, (path, unit))| {
                let blocks = vec![Arc::new(Block::whole(unit))];
                ((path, slot), Ok(blocks))
            })
            .unzip();
        Self {
            root: PathBuf::new(),
            files,
            paths: None,
            units: Some(units),
            other_version: 0,
            postings_damaged: false,
            rows: None,
            included: None,
        }
    }

    /// The bytes of block images it holds, each distinct unit's once.
    pub(crate) fn image_bytes(&self) -> usize {
        self.units
            .iter()
            .flatten()
            .flatten()
            .map(|blocks| bytes_of(blocks))
            .sum()
    }

    /// The directory the index was built in, below which relative paths are read.
    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    /// The paths read, in database order: every indexed path for field 7 and text searches.
    pub(crate) fn paths(&self) -> impl Iterator<Item = &[u8]> {
        let listed = self.paths.iter().flat_map(|paths| paths.iter());
        (self.files.iter().map(|(path, _)| path.as_slice())).chain(listed)
    }

    /// The blocks read of the files whose unit can be read, in database order and each
    /// file's in order. A unit that cannot be (another version's, a damaged one, or one
    /// SQLite fails to read) leaves its file out of the answer rather than failing the
    /// query.
    pub(crate) fn files(&self) -> Vec<Indexed<'_>> {
        let units = self.units();
        let mut files = Vec::new();
        for (path, slot) in &self.files {
            if let Ok(blocks) = sound(&units[*slot]) {
                files.extend(blocks.iter().map(|block| Indexed {
                    path,
                    unit: &block.lines,
                    start: block.start(),
                    first: block.number == 0,
                }));
            }
        }
        files
    }

    /// What the query leaves out because it cannot read a file's unit: nothing, when it
    /// reads the paths alone.
    pub(crate) fn left_out(&self) -> LeftOut {
        LeftOut {
            other_version: self.other_version,
            damaged: self.units.as_deref().map_or(0, |units| {
                self.files
                    .iter()
                    .filter(|(_, slot)| sound(&units[*slot]).is_err())
                    .count()
            }),
        }
    }

    /// Whether the name postings the query looked up cannot be read, so that it read
    /// every file: the answer is whole, and the index damaged.
    pub(crate) fn postings_damaged(&self) -> bool {
        self.postings_damaged
    }

    /// Why the query leaves files out, if it does: the compass compares every file, so
    /// `--store-query` stops on it.
    pub(crate) fn failure(&self) -> Option<String> {
        if self.other_version > 0 {
            let files = self.other_version;
            return Some(format!(
                "{files} indexed {} analysed by another version of ripscope; rebuild the \
                 index with -bu and the options it was built with",
                if files == 1 { "file was" } else { "files were" },
            ));
        }
        let units = self.units();
        self.files
            .iter()
            .find_map(|(_, slot)| sound(&units[*slot]).err())
    }

    /// The units, which a revision read for its paths does not hold: it answers only the
    /// queries that read none.
    fn units(&self) -> &[Result<Vec<Arc<Block>>, StoreError>] {
        self.units
            .as_deref()
            .expect("a revision read for its paths answers only field 7 and text searches")
    }
}

impl Source for Revision {
    fn paths(&self) -> impl Iterator<Item = &[u8]> {
        Revision::paths(self)
    }

    fn occurrences(&self) -> Option<Vec<crate::engine::Occurring<'_>>> {
        let rows = self.rows.as_ref()?;
        let units = self.units();
        Some(
            self.files
                .iter()
                .filter_map(|(path, slot)| {
                    let blocks = sound(&units[*slot]).ok()?;
                    Some((path.as_slice(), blocks, rows[*slot].as_slice()))
                })
                .collect(),
        )
    }

    fn included(&self) -> Option<Vec<crate::engine::Including<'_>>> {
        let included = self.included.as_ref()?;
        Some(
            self.files
                .iter()
                .map(|(path, slot)| (path.as_slice(), included[*slot].as_slice()))
                .collect(),
        )
    }

    /// The blocks read of the files whose unit can be read: those the revision was read
    /// for.
    fn read(&self, _reads: &Reads<'_>) -> Vec<Indexed<'_>> {
        self.files()
    }
}

/// A unit's blocks read, if they could be read and every one of them is sound, checked
/// once for each block however many queries and threads ask.
fn sound(unit: &Result<Vec<Arc<Block>>, StoreError>) -> Result<&[Arc<Block>], String> {
    let blocks = unit.as_ref().map_err(StoreError::to_string)?;
    for block in blocks {
        block
            .lines
            .checked()
            .map_err(|error| StoreError::Unit(error).to_string())?;
    }
    Ok(blocks)
}

/// One revision's distinct include operands with their ids, and the includes of each
/// operand a query matched, by id.
struct Operands {
    directory: Vec<(i64, Vec<u8>)>,
    kept: HashMap<i64, Arc<[(i64, Included)]>>,
}

/// The bytes of `blocks`' images.
fn bytes_of(blocks: &[Arc<Block>]) -> usize {
    blocks.iter().map(|block| block.lines.image().len()).sum()
}

/// The warning for files another version's units leave out, until the index is rebuilt.
pub(crate) fn other_version(files: usize) -> String {
    format!(
        "{files} indexed {} cannot be read by this version of ripscope and {} left out of \
         the answers; rebuild the index with -bu and the options it was built with",
        if files == 1 { "file" } else { "files" },
        if files == 1 { "is" } else { "are" },
    )
}

/// The blocks a session keeps decoded between its queries, up to a budget of image bytes,
/// the least recently used unit's going first. They are kept by index identity, unit key
/// and block number: a key holds the same analysis, in the same blocks, wherever one
/// index stores it.
pub(crate) struct Units {
    budget: usize,
    bytes: usize,
    clock: u64,
    /// By index identity and unit key.
    kept: HashMap<(Vec<u8>, Vec<u8>), Kept>,
    /// The key and block count of each unit id read from one revision, which are the same
    /// throughout it.
    keys: Option<(RevisionId, UnitKeys)>,
    /// One revision's distinct include operands, and the includes of those its queries
    /// matched.
    operands: Option<(RevisionId, Operands)>,
    /// One revision's files by unit, each with its place in database order.
    files: Option<(RevisionId, UnitFiles)>,
    /// One revision's paths, in database order.
    paths: Option<(RevisionId, Arc<Paths>)>,
    /// One revision's root, from its selection rule.
    root: Option<(RevisionId, PathBuf)>,
    /// The revision the last query read.
    read: Option<RevisionId>,
}

/// Each unit's files, with their places in database order.
type UnitFiles = HashMap<i64, Vec<(i64, Vec<u8>)>>;

/// Each unit id's key and block count.
type UnitKeys = HashMap<i64, (Vec<u8>, u32)>;

/// A kept unit: a slot for each of its blocks, filled once a query read it, whether a read
/// of every block checked the unit's range of ids, and when a query last used it.
struct Kept {
    blocks: Vec<Option<Arc<Block>>>,
    checked: bool,
    used: u64,
}

impl Kept {
    /// The bytes it keeps: its blocks' images, and its slots.
    fn bytes(&self) -> usize {
        let images: usize = self
            .blocks
            .iter()
            .flatten()
            .map(|block| block.lines.image().len())
            .sum();
        images + self.blocks.len() * std::mem::size_of::<Option<Arc<Block>>>()
    }
}

impl Units {
    /// A session's budget: a few hundred megabytes of images.
    pub(crate) const SESSION: usize = 256 << 20;

    /// Nothing kept: one query.
    pub(crate) fn none() -> Self {
        Self::with_budget(0)
    }

    pub(crate) fn with_budget(budget: usize) -> Self {
        Self {
            budget,
            bytes: 0,
            clock: 0,
            kept: HashMap::new(),
            keys: None,
            operands: None,
            files: None,
            paths: None,
            root: None,
            read: None,
        }
    }

    /// The revision the last query read, if it read one.
    pub(crate) fn read(&self) -> Option<&RevisionId> {
        self.read.as_ref()
    }

    /// The blocks of unit `id` of the revision `reader` reads that a query reads, every
    /// one or those `which` lists, in order: from those kept, or else from the index. What
    /// is kept changes only once they are read.
    fn get(
        &mut self,
        reader: &Reader<'_>,
        id: i64,
        which: Option<&[u32]>,
    ) -> Result<Vec<Arc<Block>>, StoreError> {
        if self.budget == 0 {
            let blocks = reader.blocks(id, which, None)?;
            return Ok(blocks.into_iter().map(Arc::new).collect());
        }
        let revision = reader.revision_id();
        if self.keys.as_ref().is_none_or(|(read, _)| read != revision) {
            self.keys = Some((revision.clone(), HashMap::new()));
        }
        let keys = &mut self.keys.as_mut().expect("set above").1;
        let (key, count) = match keys.get(&id) {
            Some(known) => known.clone(),
            None => {
                let found = reader.key(id)?;
                keys.insert(id, found.clone());
                found
            }
        };
        let key = (revision.identity.clone(), key);
        // A key holds the same blocks while the identity lasts; kept ones of another count
        // could only come from damage, and are let go.
        let kept = self
            .kept
            .get(&key)
            .filter(|kept| kept.blocks.len() == count as usize);
        let kept_block =
            |number: u32| kept.and_then(|kept| kept.blocks.get(number as usize)?.clone());
        let every: Vec<u32>;
        let wanted = match which {
            Some(which) => which,
            None => {
                every = (0..count).collect();
                &every
            }
        };
        let missing: Vec<u32> = wanted
            .iter()
            .copied()
            .filter(|number| kept_block(*number).is_none())
            .collect();
        profile_count!("cache_blocks_hit", wanted.len() - missing.len());
        profile_count!("cache_blocks_missed", missing.len());
        // The first read of every block checks the unit's whole range of ids, as a query
        // without a session does, so that no block beyond its count goes unnoticed.
        let whole = which.is_none() && !kept.is_some_and(|kept| kept.checked);
        let read = if whole {
            reader.blocks(id, None, Some(count))?
        } else if missing.is_empty() {
            Vec::new()
        } else {
            reader.blocks(id, Some(&missing), Some(count))?
        };
        self.clock += 1;
        let kept = self.kept.entry(key).or_insert_with(|| Kept {
            blocks: Vec::new(),
            checked: false,
            used: 0,
        });
        if kept.blocks.len() != count as usize {
            self.bytes -= kept.bytes();
            kept.blocks = vec![None; count as usize];
            kept.checked = false;
            self.bytes += kept.bytes();
        }
        kept.checked |= whole;
        kept.used = self.clock;
        for block in read {
            let slot = &mut kept.blocks[block.number as usize];
            if slot.is_none() {
                self.bytes += block.lines.image().len();
                *slot = Some(Arc::new(block));
            }
        }
        let blocks = wanted
            .iter()
            .map(|number| kept.blocks.get(*number as usize).cloned().flatten())
            .collect::<Option<Vec<_>>>()
            .ok_or(StoreError::Unit(crate::unit::UnitError::Damaged))?;
        if self.bytes > self.budget {
            self.trim();
        }
        Ok(blocks)
    }

    /// Every indexed path, in database order: for a session read once for each revision.
    fn paths(&mut self, reader: &Reader<'_>) -> Result<Arc<Paths>, StoreError> {
        if self.budget == 0 {
            return Ok(Arc::new(reader.paths()?));
        }
        let revision = reader.revision_id();
        if let Some((read, paths)) = &self.paths
            && read == revision
        {
            return Ok(Arc::clone(paths));
        }
        let paths = Arc::new(reader.paths()?);
        self.paths = Some((revision.clone(), Arc::clone(&paths)));
        Ok(paths)
    }

    /// The directory the revision `reader` reads was built in, from its selection rule,
    /// decoded once for each revision.
    fn root(&mut self, reader: &Reader<'_>) -> Result<PathBuf, refresh::RefreshError> {
        let revision = reader.revision_id();
        if let Some((read, root)) = &self.root
            && read == revision
        {
            return Ok(root.clone());
        }
        let root = refresh::decode_rule(reader.selection())?.root;
        self.root = Some((revision.clone(), root.clone()));
        Ok(root)
    }

    /// The files whose unit is one of `wanted`, with their unit's id, in database order:
    /// for a session from each revision's files read once, else looked up.
    fn files(
        &mut self,
        reader: &Reader<'_>,
        wanted: impl IntoIterator<Item = i64>,
    ) -> Result<crate::store::Files, StoreError> {
        if self.budget == 0 {
            return reader.files_of(&wanted.into_iter().collect());
        }
        let revision = reader.revision_id();
        if self.files.as_ref().is_none_or(|(read, _)| read != revision) {
            let mut by_unit = UnitFiles::default();
            for (ordinal, path, unit) in reader.every_file()? {
                by_unit.entry(unit).or_default().push((ordinal, path));
            }
            self.files = Some((revision.clone(), by_unit));
        }
        let by_unit = &self.files.as_ref().expect("read above").1;
        let mut found: Vec<(i64, &[u8], i64)> = wanted
            .into_iter()
            .flat_map(|unit| {
                by_unit
                    .get(&unit)
                    .into_iter()
                    .flatten()
                    .map(move |(ordinal, path)| (*ordinal, path.as_slice(), unit))
            })
            .collect();
        found.sort_unstable_by_key(|(ordinal, _, _)| *ordinal);
        Ok(found
            .into_iter()
            .map(|(_, path, unit)| (path.to_vec(), unit))
            .collect())
    }

    /// The units whose includes `matches` accepts the operand of, each with where those
    /// includes sit, in order: for a session from each revision's operands read once and
    /// the includes of each operand matched kept.
    fn including(
        &mut self,
        reader: &Reader<'_>,
        mut matches: impl FnMut(&[u8]) -> bool,
    ) -> Result<Includers, StoreError> {
        let revision = reader.revision_id();
        if self
            .operands
            .as_ref()
            .is_none_or(|(read, _)| read != revision)
        {
            let directory = reader.operand_directory()?;
            self.operands = Some((
                revision.clone(),
                Operands {
                    directory,
                    kept: HashMap::new(),
                },
            ));
        }
        let operands = &mut self.operands.as_mut().expect("read above").1;
        let mut found = Includers::default();
        for (id, operand) in &operands.directory {
            if !matches(operand) {
                continue;
            }
            let included = match operands.kept.get(id) {
                Some(included) => Arc::clone(included),
                None => {
                    let included: Arc<[(i64, Included)]> = reader.included_by(*id)?.into();
                    operands.kept.insert(*id, Arc::clone(&included));
                    included
                }
            };
            for (unit, at) in included.iter() {
                found.entry(*unit).or_default().push(at.clone());
            }
        }
        crate::store::in_place(&mut found);
        Ok(found)
    }

    /// Lets go of the least recently used units' blocks, down to three quarters of the
    /// budget.
    fn trim(&mut self) {
        let mut kept: Vec<_> = self.kept.drain().collect();
        kept.sort_unstable_by_key(|(_, kept)| std::cmp::Reverse(kept.used));
        self.bytes = 0;
        for (key, kept) in kept {
            let size = kept.bytes();
            if self.bytes + size > self.budget / 4 * 3 {
                continue;
            }
            self.bytes += size;
            self.kept.insert(key, kept);
        }
    }
}

/// Whether the index at `index` can be read and holds no published revision: another
/// process is making it afresh, or its first index build failed.
pub(crate) fn nothing_published(index: &Path) -> bool {
    Store::open(index, Access::Read)
        .and_then(|store| store.revision_id())
        .is_ok_and(|id: RevisionId| id.number == 0)
}

/// Why an index that holds no published revision answers no query: it is no index yet.
pub(crate) fn unpublished(index: &Path) -> String {
    format!(
        "no index at {} yet: nothing is published in it",
        index.display()
    )
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::analyzer::Analyzer;
    use crate::refresh::{Options, Sources, Trust, Unreadable};
    use crate::unit::UnitError;

    fn unit(source: &str) -> AnalysisUnit {
        Analyzer::PRODUCT
            .analyse(b"a.c", source.as_bytes())
            .unwrap()
    }

    /// Writes `sources` in `work` and builds an index there from a file list naming
    /// `listed`, as the product builds one.
    pub(crate) fn build(work: &Path, sources: &[(&str, &str)], listed: &[&str]) -> PathBuf {
        build_in_blocks(work, sources, listed, None)
    }

    /// [`build`], into an index that stores blocks of `block_size` bytes when given.
    pub(crate) fn build_in_blocks(
        work: &Path,
        sources: &[(&str, &str)],
        listed: &[&str],
        block_size: Option<usize>,
    ) -> PathBuf {
        refresh_in(work, sources, listed, block_size, Trust::Metadata).unwrap()
    }

    /// [`build_in_blocks`], trusting what `trust` says of the files the index holds, and
    /// giving the refresh's error.
    fn refresh_in(
        work: &Path,
        sources: &[(&str, &str)],
        listed: &[&str],
        block_size: Option<usize>,
        trust: Trust,
    ) -> Result<PathBuf, refresh::RefreshError> {
        for (name, source) in sources {
            std::fs::write(work.join(name), source).unwrap();
        }
        std::fs::write(work.join("files"), listed.join("\n")).unwrap();
        let index = work.join("i.db");
        let selection = refresh::Selection {
            root: work.to_owned(),
            sources: Sources::List(b"files".to_vec()),
            include_dirs: Vec::new(),
            environment_dirs: Vec::new(),
            system_includes: false,
            system_dir: Vec::new(),
        };
        let mut store = Store::open(&index, Access::Write).unwrap();
        if let Some(size) = block_size {
            rusqlite::Connection::open(&index)
                .unwrap()
                .execute(
                    "UPDATE meta SET value = ?1 WHERE key = 'block size'",
                    [i64::try_from(size).unwrap()],
                )
                .unwrap();
        }
        let options = Options {
            unreadable: Unreadable::Drop,
            trust,
            wait: refresh::WAIT,
            analyzer: Analyzer::PRODUCT,
        };
        refresh::refresh(&mut store, &selection, options, &mut ())?;
        Ok(index)
    }

    /// Every unit, as a query that reads every file does.
    fn every(index: &Path) -> Result<Revision, String> {
        Revision::read(index, &Reads::Every, &mut Units::none())
    }

    fn paths(revision: &Revision) -> Vec<&[u8]> {
        revision.paths().collect()
    }

    /// The files in database order, each unit read once however many files share it,
    /// and the directory the index is built in as the root.
    #[test]
    fn a_revision_reads_the_files_in_order_and_each_unit_once() {
        let work = tempfile::tempdir().unwrap();
        let index = build(
            work.path(),
            &[
                ("b.c", "int Beta;\n"),
                ("a.c", "int Beta;\n"),
                ("c.c", "int Gamma;\n"),
            ],
            &["b.c", "c.c", "a.c", "b.c"],
        );
        let revision = every(&index).unwrap();
        assert_eq!(revision.root(), work.path());
        assert_eq!(paths(&revision), [&b"a.c"[..], b"b.c", b"b.c", b"c.c"]);
        // a.c and b.c hold the same bytes, and b.c is listed twice: one unit for three
        // files, and one for c.c.
        assert_eq!(revision.units().len(), 2);
        let files = revision.files();
        assert_eq!(files.len(), 4);
        assert!(std::ptr::eq(files[0].unit, files[2].unit));
        assert!(*files[3].unit == unit("int Gamma;\n"));
        assert_eq!(revision.left_out(), LeftOut::default());
        assert!(revision.failure().is_none());
    }

    /// Read in parts, a revision gives every file once and in order, a part ending once
    /// the units it read hold the part's bytes of images (a unit its files share counted
    /// once). The oracle's reads, whose context carries from file to file, come in one
    /// part, as do the paths alone, and a query that finds no file gets one empty part. An
    /// error from a part ends the reading.
    #[test]
    fn a_revision_read_in_parts_gives_each_file_once_in_order() {
        use crate::engine::{Field, Semantics};
        use crate::query::{IdentifierCase, StoredQuery};
        let work = tempfile::tempdir().unwrap();
        let index = build(
            work.path(),
            &[
                ("b.c", "int Beta;\n"),
                ("a.c", "int Beta;\n"),
                ("c.c", "int Gamma;\n"),
            ],
            &["b.c", "c.c", "a.c", "b.c"],
        );
        let parts = |reads: &Reads<'_>, part: usize| -> Vec<Vec<String>> {
            let mut parts = Vec::new();
            let read =
                Revision::read_in_parts(&index, reads, &mut Units::none(), part, |revision| {
                    let paths = revision.paths().map(String::from_utf8_lossy);
                    parts.push(paths.map(|path| path.into_owned()).collect());
                    Ok::<(), Infallible>(())
                });
            let Ok(()) = read.unwrap();
            parts
        };
        fn query(pattern: &str) -> StoredQuery<'_> {
            StoredQuery::compile(
                Field::Symbols,
                pattern.as_bytes(),
                IdentifierCase::Sensitive,
            )
            .unwrap()
        }
        let every = query(".*");
        let every = every.reads(Semantics::Own);
        assert_eq!(parts(&every, 1), [["a.c"], ["b.c"], ["b.c"], ["c.c"]]);
        // A part of no bytes still takes a file.
        assert_eq!(parts(&every, 0), [["a.c"], ["b.c"], ["b.c"], ["c.c"]]);
        let shared = unit("int Beta;\n").image().len() + 1;
        assert_eq!(parts(&every, shared), [["a.c", "b.c", "b.c", "c.c"]]);
        assert_eq!(parts(&every, usize::MAX), [["a.c", "b.c", "b.c", "c.c"]]);
        assert_eq!(parts(&Reads::Every, 1), [["a.c", "b.c", "b.c", "c.c"]]);
        assert_eq!(parts(&Reads::Paths, 1), [["a.c", "b.c", "b.c", "c.c"]]);
        let none = query("Nothing");
        assert_eq!(
            parts(&none.reads(Semantics::Own), 1),
            [Vec::<String>::new()]
        );
        let mut calls = 0;
        let read = Revision::read_in_parts(&index, &every, &mut Units::none(), 1, |_| {
            calls += 1;
            Err("stop")
        });
        assert_eq!(read, Ok(Err("stop")));
        assert_eq!(calls, 1);
    }

    /// A query reads only the files whose unit holds a name its pattern matches (an exact
    /// name, a regular expression, or with `-C` either), in the roles its field reads, and
    /// field 8 only the files with a matching include.
    #[test]
    fn a_query_reads_only_the_files_that_hold_what_it_asks_for() {
        use crate::engine::Field;
        use crate::query::{IdentifierCase, StoredQuery};
        let work = tempfile::tempdir().unwrap();
        let index = build(
            work.path(),
            &[
                ("a.c", "#include \"b.h\"\nint Alpha;\n"),
                ("b.c", "int f(void) { return Alpha; }\n"),
                ("c.c", "int Gamma;\n"),
                ("b.h", "int Beta;\n"),
            ],
            &["a.c", "b.c", "c.c"],
        );
        let read_as = |case: IdentifierCase, digit: u8, pattern: &str| -> Vec<Vec<u8>> {
            let field = Field::from_digit(digit).unwrap();
            let query = StoredQuery::compile(field, pattern.as_bytes(), case).unwrap();
            let revision = Revision::read(
                &index,
                &query.reads(crate::engine::Semantics::Own),
                &mut Units::none(),
            )
            .unwrap();
            revision.paths().map(<[u8]>::to_vec).collect()
        };
        let read = |digit, pattern| read_as(IdentifierCase::Sensitive, digit, pattern);
        assert_eq!(read(b'0', "Alpha"), [b"a.c".to_vec(), b"b.c".to_vec()]);
        assert_eq!(read(b'1', "Alpha"), [b"a.c".to_vec()]);
        assert_eq!(read(b'3', "Alpha"), Vec::<Vec<u8>>::new());
        assert_eq!(read(b'8', "b\\.h"), [b"a.c".to_vec()]);
        assert_eq!(read(b'0', "Al.*"), [b"a.c".to_vec(), b"b.c".to_vec()]);
        assert_eq!(read(b'1', "Al.*"), [b"a.c".to_vec()]);
        assert_eq!(read(b'0', ".*a").len(), 4);
        assert_eq!(read(b'0', "Zz.*"), Vec::<Vec<u8>>::new());
        let caseless = |digit, pattern| read_as(IdentifierCase::AsciiInsensitive, digit, pattern);
        assert_eq!(caseless(b'0', "ALPHA"), [b"a.c".to_vec(), b"b.c".to_vec()]);
        assert_eq!(caseless(b'1', "al.*"), [b"a.c".to_vec()]);
        assert_eq!(read(b'7', "c").len(), 4);
    }

    /// A unit that cannot be read in an index this version made is damage: every file that
    /// shares it is left out and counted, whatever made it unreadable, and the first such
    /// file in database order says why.
    #[test]
    fn damaged_units_leave_out_every_file_that_shares_them() {
        let work = tempfile::tempdir().unwrap();
        let index = build(
            work.path(),
            &[
                ("a.c", "int Alpha;\n"),
                ("b.c", "int Beta;\n"),
                ("c.c", "int Alpha;\n"),
                ("d.c", "int Delta;\n"),
            ],
            &["d.c", "c.c", "b.c", "a.c"],
        );
        let connection = rusqlite::Connection::open(&index).unwrap();
        let unit_of = |path: &str| -> i64 {
            connection
                .query_row(
                    "SELECT unit FROM files WHERE path = CAST(?1 AS BLOB)",
                    [path],
                    |row| row.get(0),
                )
                .unwrap()
        };
        // a.c and c.c share one unit, damaged; d.c's claims another image version.
        assert_eq!(unit_of("a.c"), unit_of("c.c"));
        connection
            .execute(
                "UPDATE blocks SET image = x'00' WHERE id >> 20 = ?1",
                [unit_of("a.c")],
            )
            .unwrap();
        let mut image: Vec<u8> = connection
            .query_row(
                "SELECT image FROM blocks WHERE id = ?1 << 20",
                [unit_of("d.c")],
                |row| row.get(0),
            )
            .unwrap();
        image[4] = 0xff;
        connection
            .execute(
                "UPDATE blocks SET image = ?1 WHERE id = ?2 << 20",
                rusqlite::params![image, unit_of("d.c")],
            )
            .unwrap();
        let revision = every(&index).unwrap();
        assert_eq!(paths(&revision), [&b"a.c"[..], b"b.c", b"c.c", b"d.c"]);
        let files = revision.files();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, b"b.c");
        assert_eq!(
            revision.left_out(),
            LeftOut {
                other_version: 0,
                damaged: 3
            }
        );
        assert_eq!(
            revision.failure().unwrap(),
            StoreError::Unit(UnitError::Damaged).to_string()
        );
    }

    /// A name whose postings cannot be read is looked for in every file, whether it is
    /// asked for or matched by a pattern: the answer is whole, and says the index is
    /// damaged. Another name's damaged list does not matter.
    #[test]
    fn damaged_postings_read_every_file() {
        use crate::engine::{Field, Semantics};
        use crate::query::{IdentifierCase, StoredQuery};
        let work = tempfile::tempdir().unwrap();
        let index = build(
            work.path(),
            &[("a.c", "int Alpha;\n"), ("b.c", "int Beta;\n")],
            &["a.c", "b.c"],
        );
        rusqlite::Connection::open(&index)
            .unwrap()
            .execute(
                "UPDATE names SET postings = x'ff' WHERE name = CAST('Alpha' AS BLOB)",
                [],
            )
            .unwrap();
        for (pattern, damaged) in [("Alpha", true), ("Al.*", true), ("Be.*", false)] {
            let query = StoredQuery::compile(
                Field::Definitions,
                pattern.as_bytes(),
                IdentifierCase::Sensitive,
            )
            .unwrap();
            let revision =
                Revision::read(&index, &query.reads(Semantics::Own), &mut Units::none()).unwrap();
            assert_eq!(revision.postings_damaged(), damaged, "{pattern}");
            if damaged {
                assert_eq!(paths(&revision), [&b"a.c"[..], b"b.c"]);
                let rows: Vec<&[u8]> = query
                    .answer(Semantics::Own, &revision)
                    .map(|row| row.path)
                    .collect();
                assert_eq!(rows, [&b"a.c"[..]]);
            } else {
                assert_eq!(paths(&revision), [&b"b.c"[..]]);
            }
        }
    }

    /// A `names` table SQLite cannot read is damaged postings too: an exact name and a
    /// pattern are looked for in every file, with the warning, rather than failing.
    #[test]
    fn a_names_table_sqlite_cannot_read_reads_every_file() {
        use crate::engine::{Field, Semantics};
        use crate::query::{IdentifierCase, StoredQuery};
        let work = tempfile::tempdir().unwrap();
        let index = build(
            work.path(),
            &[("a.c", "int Alpha;\n"), ("b.c", "int Beta;\n")],
            &["a.c", "b.c"],
        );
        let connection = rusqlite::Connection::open(&index).unwrap();
        connection
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
            .unwrap();
        let (page, root): (i64, i64) = connection
            .query_row(
                "SELECT (SELECT page_size FROM pragma_page_size), rootpage FROM sqlite_schema \
                 WHERE name = 'names'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        drop(connection);
        let mut bytes = std::fs::read(&index).unwrap();
        let start = usize::try_from((root - 1) * page).unwrap();
        assert!(start > 0, "the names table is not the first page");
        bytes[start..start + 64].fill(0xff);
        std::fs::write(&index, bytes).unwrap();
        for pattern in ["Alpha", "Al.*"] {
            let query = StoredQuery::compile(
                Field::Symbols,
                pattern.as_bytes(),
                IdentifierCase::Sensitive,
            )
            .unwrap();
            let revision =
                Revision::read(&index, &query.reads(Semantics::Own), &mut Units::none()).unwrap();
            assert!(revision.postings_damaged(), "{pattern}");
            assert_eq!(paths(&revision), [&b"a.c"[..], b"b.c"], "{pattern}");
        }
    }

    /// The units of an index another version made are recognised by its analyzer: every
    /// file is left out of the queries that read units, while field 7 and text searches
    /// still read its paths.
    #[test]
    fn another_versions_units_are_known_by_its_analyzer() {
        let work = tempfile::tempdir().unwrap();
        let index = build(
            work.path(),
            &[("a.c", "int Alpha;\n"), ("b.c", "int Beta;\n")],
            &["a.c", "b.c"],
        );
        let connection = rusqlite::Connection::open(&index).unwrap();
        connection
            .execute(
                "UPDATE meta SET value = CAST('ripscope-scanner-1/unit-99' AS BLOB)
                 WHERE key = 'analyzer'",
                [],
            )
            .unwrap();
        let revision = every(&index).unwrap();
        assert!(revision.files().is_empty());
        assert_eq!(
            revision.left_out(),
            LeftOut {
                other_version: 2,
                damaged: 0
            }
        );
        assert_eq!(
            revision.failure().unwrap(),
            "2 indexed files were analysed by another version of ripscope; rebuild the index \
             with -bu and the options it was built with"
        );
        let listed = Revision::read(&index, &Reads::Paths, &mut Units::none()).unwrap();
        assert_eq!(paths(&listed), [&b"a.c"[..], b"b.c"]);
    }

    /// Each query reads the index as it is now: what another process published, and no
    /// index where it was removed or is being made afresh.
    #[test]
    fn each_query_reads_the_current_revision() {
        let work = tempfile::tempdir().unwrap();
        let index = build(work.path(), &[("a.c", "int Alpha;\n")], &["a.c"]);
        assert_eq!(paths(&every(&index).unwrap()), [&b"a.c"[..]]);
        std::fs::remove_file(&index).unwrap();
        for log in ["i.db-wal", "i.db-shm"] {
            let _ = std::fs::remove_file(work.path().join(log));
        }
        let error = every(&index).map(|_| ()).unwrap_err();
        assert_eq!(error, format!("no index at {}", index.display()));
        drop(Store::open(&index, Access::Write).unwrap());
        assert!(nothing_published(&index));
        assert!(!nothing_published(&work.path().join("missing.db")));
        assert_eq!(every(&index).map(|_| ()).unwrap_err(), unpublished(&index));
        build(work.path(), &[("b.c", "int Beta;\n")], &["b.c"]);
        assert!(!nothing_published(&index));
        assert_eq!(paths(&every(&index).unwrap()), [&b"b.c"[..]]);
    }

    /// A session holds the index between its queries, and reads the current revision of
    /// the file the path names: one updated in place, or another moved over it.
    #[test]
    fn a_held_index_reads_the_file_its_path_names_now() {
        let work = tempfile::tempdir().unwrap();
        let index = build(work.path(), &[("a.c", "int Alpha;\n")], &["a.c"]);
        let held = HeldIndex::new(&index);
        let read = || paths_of(&Revision::read(&held, &Reads::Paths, &mut Units::none()).unwrap());
        assert_eq!(read(), [b"a.c".to_vec()]);
        build(work.path(), &[("b.c", "int Beta;\n")], &["b.c"]);
        assert_eq!(read(), [b"b.c".to_vec()]);
        let other = tempfile::tempdir().unwrap();
        let moved = build(other.path(), &[("c.c", "int Gamma;\n")], &["c.c"]);
        held.release();
        assert_eq!(read(), [b"b.c".to_vec()]);
        for log in ["i.db-wal", "i.db-shm"] {
            let _ = std::fs::remove_file(work.path().join(log));
        }
        // On Windows a virus scanner may still be reading the index just written, as an
        // index build's own replacement allows for.
        let mut waits = 0;
        while let Err(error) = std::fs::rename(&moved, &index) {
            waits += 1;
            assert!(waits < 40, "{error}");
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert_eq!(read(), [b"c.c".to_vec()]);
    }

    fn paths_of(revision: &Revision) -> Vec<Vec<u8>> {
        revision.paths().map(<[u8]>::to_vec).collect()
    }

    /// A session keeps the units its queries decoded, within its budget: kept by index
    /// identity and unit key, they serve the next query, and the least recently used go
    /// first once the budget is spent.
    #[test]
    fn a_session_keeps_decoded_units_within_its_budget() {
        let work = tempfile::tempdir().unwrap();
        let index = build(
            work.path(),
            &[("a.c", "int Alpha;\n"), ("b.c", "int Beta;\n")],
            &["a.c", "b.c"],
        );
        let mut units = Units::with_budget(1 << 20);
        assert_eq!(
            Revision::read(&index, &Reads::Every, &mut units)
                .unwrap()
                .files()
                .len(),
            2
        );
        assert_eq!(units.kept.len(), 2);
        // Kept units answer although the index's copy is damaged meanwhile.
        rusqlite::Connection::open(&index)
            .unwrap()
            .execute("UPDATE blocks SET image = x'00'", [])
            .unwrap();
        let kept = Revision::read(&index, &Reads::Every, &mut units).unwrap();
        assert_eq!(kept.files().len(), 2);
        assert_eq!(kept.left_out(), LeftOut::default());
        let fresh = every(&index).unwrap();
        assert_eq!(fresh.left_out().damaged, 2);
        // A budget smaller than both units keeps only the more recently used one: its image
        // and its one block's slot.
        let one = unit("int Alpha;\n").image().len() + std::mem::size_of::<Option<Arc<Block>>>();
        let mut small = Units::with_budget(one * 3 / 2);
        let other = tempfile::tempdir().unwrap();
        let index = build(
            other.path(),
            &[("a.c", "int Alpha;\n"), ("b.c", "int Beta;\n")],
            &["a.c", "b.c"],
        );
        Revision::read(&index, &Reads::Every, &mut small).unwrap();
        assert_eq!(small.kept.len(), 1);
        assert!(small.bytes <= one * 3 / 2);
    }

    /// An exact name reads only the blocks its postings list for it, so a damaged block
    /// leaves its file out of the queries that read it, and only those: an exact name in
    /// it, and patterns, which read every block.
    #[test]
    fn a_damaged_block_leaves_its_file_out_of_the_queries_that_read_it() {
        // A damaged header is found as the block is read, damaged lines once it is walked.
        for damage in ["x'00'", "substr(image, 1, length(image) - 1)"] {
            a_damaged_block_leaves_its_file_out(damage);
        }
    }

    /// An includer query answers from the rows its includes recorded and reads no block, so
    /// damage in a file's blocks leaves the file in its answer; `--verify` finds that damage.
    #[test]
    fn includer_rows_read_no_blocks() {
        use crate::engine::{Field, Semantics};
        use crate::query::{IdentifierCase, StoredQuery};
        let work = tempfile::tempdir().unwrap();
        let source = "#include \"x.h\"\nint a;\n#include <sys/x.h>\n";
        let index = build_in_blocks(work.path(), &[("a.c", source)], &["a.c"], Some(1));
        rusqlite::Connection::open(&index)
            .unwrap()
            .execute("UPDATE blocks SET image = x'00'", [])
            .unwrap();
        let query =
            StoredQuery::compile(Field::Includers, b"x.h", IdentifierCase::Sensitive).unwrap();
        for units in [&mut Units::none(), &mut Units::with_budget(Units::SESSION)] {
            let revision = Revision::read(&index, &query.reads(Semantics::Own), units).unwrap();
            let rows: Vec<String> = query
                .answer(Semantics::Own, &revision)
                .map(|row| format!("{} {}", row.line, String::from_utf8_lossy(row.text)))
                .collect();
            assert_eq!(rows, ["1 #include \"x.h\"", "3 #include <sys/x.h>"]);
            assert_eq!(revision.left_out().damaged, 0);
        }
    }

    fn a_damaged_block_leaves_its_file_out(damage: &str) {
        use crate::engine::{Field, Semantics};
        use crate::query::{IdentifierCase, StoredQuery};
        let work = tempfile::tempdir().unwrap();
        let index = build_in_blocks(
            work.path(),
            &[("a.c", "int Alpha;\nint Beta;\n"), ("b.c", "int Beta;\n")],
            &["a.c", "b.c"],
            Some(1),
        );
        // a.c's second block, which holds Beta.
        rusqlite::Connection::open(&index)
            .unwrap()
            .execute(
                &format!(
                    "UPDATE blocks SET image = {damage} WHERE id = (SELECT unit FROM files \
                     WHERE path = CAST('a.c' AS BLOB)) << 20 | 1"
                ),
                [],
            )
            .unwrap();
        let read = |pattern: &str| {
            let query = StoredQuery::compile(
                Field::Definitions,
                pattern.as_bytes(),
                IdentifierCase::Sensitive,
            )
            .unwrap();
            let revision =
                Revision::read(&index, &query.reads(Semantics::Own), &mut Units::none()).unwrap();
            let rows: Vec<Vec<u8>> = query
                .answer(Semantics::Own, &revision)
                .map(|row| row.path.to_vec())
                .collect();
            (rows, revision.left_out().damaged)
        };
        for (pattern, answered, damaged) in [
            ("Alpha", &[&b"a.c"[..]][..], 0),
            ("Beta", &[b"b.c"], 1),
            ("Al.*", &[], 1),
            ("Be.*", &[b"b.c"], 1),
        ] {
            let answered: Vec<Vec<u8>> = answered.iter().map(|path| path.to_vec()).collect();
            assert_eq!(read(pattern), (answered, damaged), "{pattern}");
        }
        // A count of blocks lower than the index holds hides no block from a pattern: a.c's
        // whole unit is damaged, while an exact name in its first block still answers.
        let connection = rusqlite::Connection::open(&index).unwrap();
        connection
            .execute(
                "UPDATE blocks SET image = (SELECT image FROM blocks WHERE id & 1048575 = 0 \
                 AND id >> 20 = (SELECT unit FROM files WHERE path = CAST('b.c' AS BLOB))) \
                 WHERE id = (SELECT unit FROM files WHERE path = CAST('a.c' AS BLOB)) << 20 | 1",
                [],
            )
            .unwrap();
        assert_eq!(read("Be.*"), (vec![b"a.c".to_vec(), b"b.c".to_vec()], 0));
        connection
            .execute(
                "UPDATE units SET blocks = 1 WHERE id = (SELECT unit FROM files \
                 WHERE path = CAST('a.c' AS BLOB))",
                [],
            )
            .unwrap();
        assert_eq!(read("Be.*"), (vec![b"b.c".to_vec()], 1));
        assert_eq!(read("Alpha"), (vec![b"a.c".to_vec()], 0));
        assert_eq!(read("Beta"), (vec![b"b.c".to_vec()], 1));
    }

    /// The oracle answers over an index of a block for each line as over whole units: it
    /// reads every block of every file, in order, and starts a file's state at its first
    /// block, even after a file of no lines.
    #[test]
    fn the_oracle_reads_blocks_as_whole_files() {
        use crate::engine::{Field, Semantics};
        use crate::query::{IdentifierCase, StoredQuery};
        let sources = [
            (
                "a.c",
                "#define WRAP(x) \\\n\tfirst(x)\nint f(void)\n{\n\tone();\n#define INNER() \\\n\
                 \tinner()\n\ttwo();\n}\n",
            ),
            ("b.c", "void g(void) {\n\tthree();\n"),
            ("c.c", "/* nothing */\n"),
            ("d.c", "void g(void) {\n\tfour();\n}\n"),
        ];
        let listed = ["a.c", "b.c", "c.c", "d.c"];
        let (whole, blocks) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let whole = build(whole.path(), &sources, &listed);
        let blocks = build_in_blocks(blocks.path(), &sources, &listed, Some(1));
        let names = [
            "WRAP", "first", "f", "one", "INNER", "inner", "two", "g", "three", "four", "x", ".*",
        ];
        let mut answered = 0;
        for name in names {
            for digit in *b"0239" {
                let field = Field::from_digit(digit).unwrap();
                let query = StoredQuery::compile(field, name.as_bytes(), IdentifierCase::Sensitive)
                    .unwrap();
                let reads = query.reads(Semantics::Sequential);
                assert!(matches!(reads, Reads::Every));
                let rows = |index: &Path| -> Vec<String> {
                    let revision = Revision::read(index, &reads, &mut Units::none()).unwrap();
                    query
                        .answer(Semantics::Sequential, &revision)
                        .map(|row| {
                            let (path, context) = (row.path.to_vec(), row.context.to_vec());
                            let text = String::from_utf8_lossy(row.text).into_owned();
                            format!(
                                "{} {} {} {text}",
                                String::from_utf8_lossy(&path),
                                String::from_utf8_lossy(&context),
                                row.line
                            )
                        })
                        .collect()
                };
                let want = rows(&whole);
                assert_eq!(rows(&blocks), want, "{name} {}", char::from(digit));
                answered += want.len();
            }
        }
        assert!(answered > 20, "{answered}");
    }

    /// A session keeps the blocks its queries read, each in its unit's slot: a query for a
    /// name keeps the blocks it read, and the first query that reads the whole unit checks
    /// its whole range of ids and fills the other slots. Kept blocks then answer although
    /// the index's are damaged meanwhile.
    #[test]
    fn a_session_keeps_the_blocks_its_queries_read() {
        use crate::engine::{Field, Semantics};
        use crate::query::{IdentifierCase, StoredQuery};
        let work = tempfile::tempdir().unwrap();
        let index = build_in_blocks(
            work.path(),
            &[("a.c", "int Alpha;\nint Beta;\nint Gamma;\n")],
            &["a.c"],
            Some(1),
        );
        let mut units = Units::with_budget(1 << 20);
        let query =
            StoredQuery::compile(Field::Definitions, b"Beta", IdentifierCase::Sensitive).unwrap();
        let beta = Revision::read(&index, &query.reads(Semantics::Own), &mut units).unwrap();
        assert_eq!(beta.files().len(), 1);
        let slots = |units: &Units| -> Vec<bool> {
            let kept = units.kept.values().next().unwrap();
            kept.blocks.iter().map(Option::is_some).collect()
        };
        assert_eq!(slots(&units), [false, true, false]);
        let kept_beta = Arc::clone(
            units.kept.values().next().unwrap().blocks[1]
                .as_ref()
                .unwrap(),
        );
        let whole = Revision::read(&index, &Reads::Every, &mut units).unwrap();
        assert_eq!(whole.files().len(), 3);
        assert_eq!(whole.left_out(), LeftOut::default());
        assert_eq!(slots(&units), [true, true, true]);
        let kept = units.kept.values().next().unwrap();
        assert!(kept.checked);
        assert!(Arc::ptr_eq(&kept_beta, kept.blocks[1].as_ref().unwrap()));
        assert_eq!(units.bytes, kept.bytes());
        let connection = rusqlite::Connection::open(&index).unwrap();
        connection
            .execute("UPDATE blocks SET image = x'00'", [])
            .unwrap();
        let kept = Revision::read(&index, &Reads::Every, &mut units).unwrap();
        assert_eq!(kept.files().len(), 3);
        assert_eq!(every(&index).unwrap().left_out().damaged, 1);
        // A read that fails keeps nothing.
        let mut fresh = Units::with_budget(1 << 20);
        let damaged = Revision::read(&index, &Reads::Every, &mut fresh).unwrap();
        assert_eq!(damaged.left_out().damaged, 1);
        assert!(fresh.kept.is_empty());
        assert_eq!(fresh.bytes, 0);
    }

    /// A session reads a unit's whole range of ids the first time a query reads every
    /// block of it, as a query without a session does, so blocks it kept for an exact
    /// name do not hide a block beyond the unit's count from a pattern.
    #[test]
    fn a_session_checks_a_units_range_on_its_first_whole_read() {
        use crate::engine::{Field, Semantics};
        use crate::query::{IdentifierCase, StoredQuery};
        let work = tempfile::tempdir().unwrap();
        let index = build_in_blocks(
            work.path(),
            &[("a.c", "int Alpha;\nint Beta;\n"), ("b.c", "int Beta;\n")],
            &["a.c", "b.c"],
            Some(1),
        );
        rusqlite::Connection::open(&index)
            .unwrap()
            .execute(
                "UPDATE units SET blocks = 1 WHERE id = (SELECT unit FROM files \
                 WHERE path = CAST('a.c' AS BLOB))",
                [],
            )
            .unwrap();
        let read = |pattern: &str, units: &mut Units| {
            let query = StoredQuery::compile(
                Field::Definitions,
                pattern.as_bytes(),
                IdentifierCase::Sensitive,
            )
            .unwrap();
            let revision = Revision::read(&index, &query.reads(Semantics::Own), units).unwrap();
            let rows: Vec<Vec<u8>> = query
                .answer(Semantics::Own, &revision)
                .map(|row| row.path.to_vec())
                .collect();
            (rows, revision.left_out().damaged)
        };
        let damaged = (vec![b"b.c".to_vec()], 1);
        assert_eq!(read("Be.*", &mut Units::none()), damaged);
        let mut session = Units::with_budget(1 << 20);
        assert_eq!(read("Alpha", &mut session), (vec![b"a.c".to_vec()], 0));
        assert_eq!(read("Be.*", &mut session), damaged);
        assert_eq!(read("Be.*", &mut session), damaged);
    }

    /// An update that must add a unit and cannot read the index's block size fails and
    /// changes nothing. An index build with `-u` records the size a new index gets, or
    /// keeps one it can read (here one changed by hand), and gives the index a new
    /// identity, so a session that kept blocks split by another size answers as a query
    /// without one, and `--verify` finds nothing.
    #[test]
    fn a_block_size_recorded_again_splits_every_unit_again() {
        use crate::engine::{Field, Semantics};
        use crate::query::{IdentifierCase, StoredQuery};
        use rusqlite::OptionalExtension;
        let long = |name: &str, fill: char, length: usize| {
            format!("char {name}[] = \"{}\";\n", fill.to_string().repeat(length))
        };
        let source = [
            long("s0", 'a', 15_900),
            long("s1", 'b', 900),
            long("s2", 'c', 20_000),
        ]
        .concat();
        let work = tempfile::tempdir().unwrap();
        let sources = [("a.c", source.as_str()), ("b.c", "int Beta;\n")];
        let index = build_in_blocks(work.path(), &sources, &["a.c", "b.c"], Some(4096));
        let ask = |pattern: &str, units: &mut Units| -> Vec<String> {
            let query = StoredQuery::compile(
                Field::Definitions,
                pattern.as_bytes(),
                IdentifierCase::Sensitive,
            )
            .unwrap();
            let revision = Revision::read(&index, &query.reads(Semantics::Own), units).unwrap();
            query
                .answer(Semantics::Own, &revision)
                .map(|row| format!("{} {}", String::from_utf8_lossy(row.path), row.line))
                .collect()
        };
        // The file holding `s1`, its count of blocks, and the blocks listed for `s1`.
        let s1 = || {
            let store = Store::open(&index, Access::Read).unwrap();
            let reader = store.reader().unwrap();
            let (files, listed) = reader.holding(b"s1", u32::MAX).unwrap();
            let [(path, id)] = &files[..] else {
                panic!("{files:?}")
            };
            (path.clone(), reader.key(*id).unwrap().1, listed[id].clone())
        };
        let size = |connection: &rusqlite::Connection| -> Option<i64> {
            connection
                .query_row(
                    "SELECT value FROM meta WHERE key = 'block size'",
                    [],
                    |row| row.get(0),
                )
                .optional()
                .unwrap()
        };
        // a.c is two blocks at 4096 bytes and at 16 KiB, s1 in the second, then the first:
        // only the identity tells a session's blocks of the one split from the other's.
        assert_eq!(s1(), (b"a.c".to_vec(), 2, vec![1]));
        let mut session = Units::with_budget(1 << 20);
        assert_eq!(ask("s1", &mut session), ["a.c 2"]);
        let connection = rusqlite::Connection::open(&index).unwrap();
        connection
            .execute("DELETE FROM meta WHERE key = 'block size'", [])
            .unwrap();
        let listed = ["a.c", "b.c", "c.c"];
        let refreshed = refresh_in(
            work.path(),
            &[("c.c", "int Gamma;\n")],
            &listed,
            None,
            Trust::Metadata,
        );
        assert!(
            matches!(
                refreshed,
                Err(refresh::RefreshError::Store(StoreError::BlockSize))
            ),
            "{refreshed:?}"
        );
        assert_eq!(size(&connection), None);
        assert_eq!(ask("Gamma", &mut session), Vec::<String>::new());
        refresh_in(work.path(), &[], &listed, None, Trust::Nothing).unwrap();
        assert_eq!(size(&connection), Some(16 << 10));
        assert_eq!(s1(), (b"a.c".to_vec(), 2, vec![0]));
        assert_eq!(ask("Gamma", &mut session), ["c.c 1"]);
        assert_eq!(ask("s1", &mut session), ["a.c 2"]);
        assert_eq!(ask("s.*", &mut session), ask("s.*", &mut Units::none()));
        connection
            .execute("UPDATE meta SET value = 4096 WHERE key = 'block size'", [])
            .unwrap();
        refresh_in(work.path(), &[], &listed, None, Trust::Nothing).unwrap();
        assert_eq!(size(&connection), Some(4096));
        assert_eq!(s1(), (b"a.c".to_vec(), 2, vec![1]));
        assert_eq!(ask("s1", &mut session), ["a.c 2"]);
        assert_eq!(ask("s.*", &mut session), ask("s.*", &mut Units::none()));
        let store = Store::open(&index, Access::Read).unwrap();
        assert_eq!(store.verify().unwrap(), Vec::<String>::new());
        let problems = store.snapshot().unwrap().check_analyses().unwrap();
        assert_eq!(problems, Vec::<String>::new());
    }

    /// Over random queries, a session answers as a query without one, and its accounting
    /// holds: the bytes it counts are those its kept units hold, within its budget.
    #[test]
    fn a_sessions_cache_keeps_its_accounting() {
        use crate::engine::{Field, Semantics};
        use crate::query::{IdentifierCase, StoredQuery};
        let mut seed = 0x1234_5678_9abc_def0u64;
        let mut next = move |n: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed % n
        };
        let names = ["Alpha", "Beta", "Gamma", "Delta", "Eps", "f", "g", "h"];
        for round in 0..40 {
            let work = tempfile::tempdir().unwrap();
            let mut sources = Vec::new();
            for file in 0..6 {
                let mut text = String::new();
                for _ in 0..1 + next(12) {
                    let name = names[next(names.len() as u64) as usize];
                    text.push_str(&match next(4) {
                        0 => format!("int {name};\n"),
                        1 => format!("void {name}{file}(void)\n{{\n\t{name}();\n}}\n"),
                        2 => format!("#define {name}{file}(x) \\\n\t{name}(x)\n"),
                        _ => format!("x = {name};\n"),
                    });
                }
                sources.push((format!("f{file}.c"), text));
            }
            let sources: Vec<(&str, &str)> = sources
                .iter()
                .map(|(path, text)| (path.as_str(), text.as_str()))
                .collect();
            let listed: Vec<&str> = sources.iter().map(|(path, _)| *path).collect();
            let size = [1, 20, 60, 16 << 10][round % 4];
            let index = build_in_blocks(work.path(), &sources, &listed, Some(size));
            let budget = [64, 200, 600, 1 << 20][next(4) as usize];
            let mut units = Units::with_budget(budget);
            for _ in 0..60 {
                let field = Field::from_digit(b"01239"[next(5) as usize]).unwrap();
                let name = names[next(names.len() as u64) as usize];
                let pattern = if next(3) == 0 {
                    format!("{}.*", &name[..1])
                } else {
                    name.to_owned()
                };
                let query =
                    StoredQuery::compile(field, pattern.as_bytes(), IdentifierCase::Sensitive)
                        .unwrap();
                let rows = |units: &mut Units| -> Vec<String> {
                    let revision =
                        Revision::read(&index, &query.reads(Semantics::Own), units).unwrap();
                    assert_eq!(revision.left_out(), LeftOut::default());
                    query
                        .answer(Semantics::Own, &revision)
                        .map(|row| {
                            let (path, context) = (row.path, row.context);
                            let text = String::from_utf8_lossy(row.text);
                            let path = String::from_utf8_lossy(path);
                            let context = String::from_utf8_lossy(context);
                            format!("{path} {context} {} {text}", row.line)
                        })
                        .collect()
                };
                let kept = rows(&mut units);
                assert_eq!(kept, rows(&mut Units::none()), "{pattern} {field:?}");
                let held: usize = units.kept.values().map(Kept::bytes).sum();
                assert_eq!(units.bytes, held);
                assert!(units.bytes <= budget, "{} > {budget}", units.bytes);
            }
        }
    }

    /// A revision whose selection rule cannot be read is refused, since text searches
    /// read relative paths below the directory the index is built in, which it names;
    /// `--store-query` refuses it too.
    #[test]
    fn a_selection_rule_that_cannot_be_read_is_refused() {
        let work = tempfile::tempdir().unwrap();
        let index = build(work.path(), &[("a.c", "int Alpha;\n")], &["a.c"]);
        rusqlite::Connection::open(&index)
            .unwrap()
            .execute("UPDATE meta SET value = x'00' WHERE key = 'selection'", [])
            .unwrap();
        let error = every(&index).map(|_| ()).unwrap_err();
        assert_eq!(
            error,
            "the index's selection rule cannot be read; rebuild it with -bu and the options \
             it was built with"
        );
    }

    /// A session decodes each revision's selection rule once, for its root: it keeps the
    /// root while the revision lasts (here past a rule damaged by hand that a query without
    /// a session refuses), an index build from another directory publishes a revision whose
    /// root later reads use, and a revision whose rule cannot be read is refused after one
    /// whose rule could.
    #[test]
    fn a_session_reads_the_root_of_each_revision() {
        let work = tempfile::tempdir().unwrap();
        let index = build(work.path(), &[("a.c", "int Alpha;\n")], &["a.c"]);
        let mut session = Units::with_budget(Units::SESSION);
        let root = |session: &mut Units| {
            Revision::read(&index, &Reads::Every, session).map(|read| read.root().to_owned())
        };
        assert_eq!(root(&mut session).unwrap(), work.path());
        let damage = |revisions: i64| {
            rusqlite::Connection::open(&index)
                .unwrap()
                .execute_batch(&format!(
                    "UPDATE meta SET value = x'00' WHERE key = 'selection';
                     UPDATE meta SET value = value + {revisions} WHERE key = 'revision';"
                ))
                .unwrap();
        };
        damage(0);
        assert_eq!(root(&mut session).unwrap(), work.path());
        let refused = root(&mut Units::none()).unwrap_err();
        assert!(refused.starts_with("the index's selection rule cannot be read"));
        let other = work.path().join("other");
        std::fs::create_dir(&other).unwrap();
        std::fs::write(other.join("a.c"), "int Other;\n").unwrap();
        std::fs::write(other.join("files"), "a.c").unwrap();
        let selection = refresh::Selection {
            root: other.clone(),
            sources: Sources::List(b"files".to_vec()),
            include_dirs: Vec::new(),
            environment_dirs: Vec::new(),
            system_includes: false,
            system_dir: Vec::new(),
        };
        let options = Options {
            unreadable: Unreadable::Drop,
            trust: Trust::Metadata,
            wait: refresh::WAIT,
            analyzer: Analyzer::PRODUCT,
        };
        let mut store = Store::open(&index, Access::Write).unwrap();
        let built = refresh::build(&mut store, &selection, options).unwrap();
        assert!(built.published);
        assert_eq!(root(&mut session).unwrap(), other);
        damage(1);
        assert_eq!(root(&mut session).unwrap_err(), refused);
    }

    /// A revision read for field 7 or a text search holds every path and reads no unit, so
    /// units that cannot be read are neither read nor counted.
    #[test]
    fn a_revision_read_for_its_paths_reads_no_unit() {
        let work = tempfile::tempdir().unwrap();
        let index = build(
            work.path(),
            &[("a.c", "int Alpha;\n"), ("b.c", "int Beta;\n")],
            &["b.c", "a.c"],
        );
        rusqlite::Connection::open(&index)
            .unwrap()
            .execute("UPDATE blocks SET image = x'00'", [])
            .unwrap();
        let revision = Revision::read(&index, &Reads::Paths, &mut Units::none()).unwrap();
        assert!(revision.units.is_none());
        let paths: Vec<&[u8]> = Source::paths(&revision).collect();
        assert_eq!(paths, [&b"a.c"[..], b"b.c"]);
        assert_eq!(revision.root(), work.path());
        assert_eq!(every(&index).unwrap().left_out().damaged, 2);
    }

    #[test]
    fn files_analysed_in_memory_are_read_in_the_order_given() {
        let revision = Revision::in_memory(vec![
            (b"z.c".to_vec(), unit("int Zeta;\n")),
            (b"a.c".to_vec(), unit("int Alpha;\n")),
        ]);
        assert_eq!(paths(&revision), [&b"z.c"[..], b"a.c"]);
        let files = revision.files();
        assert!(*files[1].unit == unit("int Alpha;\n"));
        assert_eq!(revision.left_out(), LeftOut::default());
    }

    /// An exact name's query answers from the rows its postings record exactly as from
    /// walking the blocks, for every name of the C fixtures in fields 0, 1, 2, 3 and 9,
    /// in blocks of the default size and of 1 KiB, and field 8 from where the includes
    /// sit as from walking, for each operand and its last part.
    #[test]
    fn recorded_rows_answer_as_the_walk_does() {
        use crate::engine::{Field, Semantics};
        use crate::query::{IdentifierCase, StoredQuery};
        let mut sources = Vec::new();
        let mut pending = vec![Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")];
        while let Some(directory) = pending.pop() {
            for entry in std::fs::read_dir(directory).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    pending.push(path);
                } else if path.extension().is_some_and(|it| it == "c" || it == "h")
                    && let Ok(text) = std::fs::read_to_string(&path)
                {
                    sources.push((format!("f{}.c", sources.len()), text));
                }
            }
        }
        sources.sort();
        let sources: Vec<(&str, &str)> = sources
            .iter()
            .map(|(name, text)| (name.as_str(), text.as_str()))
            .collect();
        let listed: Vec<&str> = sources.iter().map(|(name, _)| *name).collect();
        for block_size in [None, Some(1024)] {
            let work = tempfile::tempdir().unwrap();
            let index = build_in_blocks(work.path(), &sources, &listed, block_size);
            let names: Vec<Vec<u8>> = rusqlite::Connection::open(&index)
                .unwrap()
                .prepare("SELECT name FROM names")
                .unwrap()
                .query_map([], |row| row.get(0))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();
            assert!(names.len() > 500, "{}", names.len());
            let mut recorded = 0;
            for name in &names {
                for field in [
                    Field::Symbols,
                    Field::Definitions,
                    Field::Callees,
                    Field::Callers,
                    Field::Assignments,
                ] {
                    let Ok(query) = StoredQuery::compile(field, name, IdentifierCase::Sensitive)
                    else {
                        continue;
                    };
                    let reads = query.reads(Semantics::Own);
                    let answer = |revision: &Revision| -> Vec<String> {
                        query
                            .answer(Semantics::Own, revision)
                            .map(|row| {
                                format!(
                                    "{} {} {} {}",
                                    String::from_utf8_lossy(row.path),
                                    String::from_utf8_lossy(row.context),
                                    row.line,
                                    String::from_utf8_lossy(row.text)
                                )
                            })
                            .collect()
                    };
                    let rows = Revision::read(&index, &reads, &mut Units::none()).unwrap();
                    let mut walked = Revision::read(&index, &reads, &mut Units::none()).unwrap();
                    recorded += usize::from(rows.rows.is_some());
                    walked.rows = None;
                    assert_eq!(
                        answer(&rows),
                        answer(&walked),
                        "{field:?} {} {block_size:?}",
                        String::from_utf8_lossy(name)
                    );
                }
            }
            assert!(recorded > names.len(), "{recorded}");
            // Field 8 from the recorded includes, read for one query and for a session.
            let operands: Vec<Vec<u8>> = rusqlite::Connection::open(&index)
                .unwrap()
                .prepare("SELECT operand FROM operands")
                .unwrap()
                .query_map([], |row| row.get(0))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();
            assert!(operands.len() > 20, "{}", operands.len());
            let mut patterns: Vec<Vec<u8>> = vec![b".*".to_vec(), b"h$".to_vec()];
            for operand in &operands {
                patterns.push(operand.clone());
                let base = operand.rsplit(|&byte| byte == b'/').next().unwrap();
                patterns.push(base.to_vec());
            }
            let mut placed = 0;
            for pattern in &patterns {
                let Ok(query) =
                    StoredQuery::compile(Field::Includers, pattern, IdentifierCase::Sensitive)
                else {
                    continue;
                };
                let reads = query.reads(Semantics::Own);
                let answer = |revision: &Revision| -> Vec<String> {
                    query
                        .answer(Semantics::Own, revision)
                        .map(|row| {
                            let text = String::from_utf8_lossy(row.text);
                            let path = String::from_utf8_lossy(row.path);
                            format!("{path} {} {text}", row.line)
                        })
                        .collect()
                };
                let mut session = Units::with_budget(Units::SESSION);
                for units in [&mut Units::none(), &mut session] {
                    let rows = Revision::read(&index, &reads, units).unwrap();
                    // An includer read reads no blocks, so the walk reads every file.
                    let walked = Revision::read(&index, &Reads::Every, units).unwrap();
                    placed += usize::from(rows.included.is_some());
                    assert!(walked.included.is_none());
                    assert_eq!(
                        answer(&rows),
                        answer(&walked),
                        "{} {block_size:?}",
                        String::from_utf8_lossy(pattern)
                    );
                }
            }
            assert!(placed > patterns.len(), "{placed}");
        }
    }
}
