//! Include discovery for one Refresh, after file-list options have been applied: where
//! each operand leads, searched as cscope searches, with what the file system answered.

use super::{Selection, below, canonical_path, path_of};
use crate::hashing::{HashMap, HashSet};
use std::path::PathBuf;

pub(super) struct IncludeResolver {
    root: PathBuf,
    directories: Vec<Vec<u8>>,
    /// Directory listings for discovery's exact Windows spelling.
    listings: HashMap<PathBuf, HashSet<std::ffi::OsString>>,
    existing: HashMap<Vec<u8>, bool>,
    /// Discovery's candidates for an operand, for angle and quoted delimiters, in search
    /// order up to the first that exists, and whether one does: the files do not change
    /// during a Refresh.
    candidates: HashMap<Vec<u8>, [Option<Candidates>; 2]>,
}

/// Discovery's candidates for an operand up to the first that exists, and whether one does.
type Candidates = (Vec<Vec<u8>>, bool);

/// A recorded question to the file system: the name it asks about, and whether it was a
/// regular file spelled exactly.
pub(super) type Question<'a> = (&'a [u8], bool);

impl IncludeResolver {
    pub(super) fn new(selection: &Selection, list_dirs: &[Vec<u8>], system_includes: bool) -> Self {
        // cscope searches command-line, file-list, environment, then system directories.
        let mut directories: Vec<_> = selection
            .include_dirs
            .iter()
            .chain(list_dirs)
            .chain(&selection.environment_dirs)
            .cloned()
            .collect();
        if system_includes {
            directories.push(selection.system_dir.clone());
        }
        Self {
            root: selection.root.clone(),
            directories,
            listings: HashMap::default(),
            existing: HashMap::default(),
            candidates: HashMap::default(),
        }
    }

    /// The next indexed spelling, using quoted build-root lookup before directories.
    /// An already-listed candidate stops discovery, even for an angle operand at root.
    pub(super) fn discover(
        &mut self,
        operand: &[u8],
        quoted: bool,
        listed: &HashSet<Vec<u8>>,
    ) -> Option<Vec<u8>> {
        let known = self
            .candidates
            .get(operand)
            .is_some_and(|known| known[usize::from(quoted)].is_some());
        if !known {
            let found = self.search(operand, quoted);
            self.candidates.entry(operand.to_vec()).or_default()[usize::from(quoted)] = Some(found);
        }
        let (candidates, found) = self.candidates[operand][usize::from(quoted)]
            .as_ref()
            .expect("searched above");
        if candidates
            .iter()
            .any(|candidate| listed.contains(candidate))
        {
            return None;
        }
        found.then(|| candidates.last().expect("a found candidate").clone())
    }

    /// The candidates discovery tries for an operand, in order, up to the first that
    /// exists, and whether one does. The operand as written is tried first, but only a
    /// quoted one is taken from there. A listed candidate before the found one stops
    /// discovery.
    fn search(&mut self, operand: &[u8], quoted: bool) -> Candidates {
        let operand_path = canonical_path(operand);
        let exists = quoted && self.exists(&operand_path);
        let mut candidates = vec![operand_path];
        if exists {
            return (candidates, true);
        }
        for at in 0..self.directories.len() {
            let mut candidate = self.directories[at].clone();
            candidate.push(b'/');
            candidate.extend_from_slice(operand);
            let candidate = canonical_path(&candidate);
            let exists = self.exists(&candidate);
            candidates.push(candidate);
            if exists {
                return (candidates, true);
            }
        }
        (candidates, false)
    }

    /// A regular file spelled exactly, including on case-insensitive filesystems.
    fn exists(&mut self, name: &[u8]) -> bool {
        if let Some(known) = self.existing.get(name) {
            return *known;
        }
        let exists = self.look_for(name);
        self.existing.insert(name.to_vec(), exists);
        exists
    }

    /// Everything this resolver learned from the file system, as bytes in a fixed
    /// order: which names exist as regular files spelled exactly. A Refresh that observes
    /// the same resolves every include the same way.
    pub(super) fn observations(&self) -> Vec<u8> {
        let mut out = Vec::new();
        let mut existing: Vec<_> = self.existing.iter().collect();
        existing.sort();
        for (name, exists) in existing {
            out.push(EXISTS);
            out.extend_from_slice(&(name.len() as u64).to_le_bytes());
            out.extend_from_slice(name);
            out.push(u8::from(*exists));
        }
        out
    }

    /// Whether the file system still answers every question `observations` records as
    /// it did then, asked on several threads.
    pub(super) fn observes(&mut self, observations: &[u8]) -> bool {
        let Some(questions) = questions(observations) else {
            return false;
        };
        // On Windows an exact spelling is found through directory listings, which this
        // resolver keeps: the questions are asked here.
        if cfg!(windows) {
            return questions
                .iter()
                .all(|&(name, exists)| self.exists(name) == exists);
        }
        let root = &self.root;
        let same = |&(name, exists): &Question<'_>| below(root, name).is_file() == exists;
        let threads = super::checking_threads(questions.len()).min(8);
        let chunk = questions.len().div_ceil(threads).max(1);
        std::thread::scope(|scope| {
            let others: Vec<_> = questions
                .chunks(chunk)
                .skip(1)
                .map(|questions| scope.spawn(move || questions.iter().all(same)))
                .collect();
            let first = questions
                .chunks(chunk)
                .next()
                .is_none_or(|q| q.iter().all(same));
            others
                .into_iter()
                .fold(first, |all, other| other.join().unwrap_or(false) && all)
        })
    }

    /// The search configuration, as bytes: what else decides how includes resolve.
    pub(super) fn configuration(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&(self.directories.len() as u64).to_le_bytes());
        for directory in &self.directories {
            out.extend_from_slice(&(directory.len() as u64).to_le_bytes());
            out.extend_from_slice(directory);
        }
        out
    }

    fn look_for(&mut self, name: &[u8]) -> bool {
        let path = below(&self.root, name);
        if !path.is_file() {
            return false;
        }
        if !cfg!(windows) {
            return true;
        }
        let mut parent = if path_of(name).is_absolute() {
            PathBuf::new()
        } else {
            self.root.clone()
        };
        for component in path_of(name).components() {
            let std::path::Component::Normal(part) = component else {
                parent.push(component.as_os_str());
                continue;
            };
            let listing = self.listings.entry(parent.clone()).or_insert_with(|| {
                std::fs::read_dir(&parent)
                    .map(|entries| entries.flatten().map(|entry| entry.file_name()).collect())
                    .unwrap_or_default()
            });
            if !listing.contains(part) {
                return false;
            }
            parent.push(part);
        }
        true
    }
}

/// What begins each question [`IncludeResolver::observations`] records: whether a name is
/// a regular file spelled exactly.
const EXISTS: u8 = b'e';

/// The questions `observations` ([`IncludeResolver::observations`]) records, in order;
/// `None` for a record that cannot be read, such as one from an earlier version that asked
/// other questions.
pub(super) fn questions(observations: &[u8]) -> Option<Vec<Question<'_>>> {
    fn take<'a>(at: &mut &'a [u8], length: usize) -> Option<&'a [u8]> {
        let (taken, rest) = (at.get(..length)?, at.get(length..)?);
        *at = rest;
        Some(taken)
    }
    let mut questions = Vec::new();
    let mut at = observations;
    while let Some(kind) = take(&mut at, 1) {
        if kind != [EXISTS] {
            return None;
        }
        let length = u64::from_le_bytes(take(&mut at, 8)?.try_into().ok()?);
        let name = take(&mut at, usize::try_from(length).ok()?)?;
        let exists = match take(&mut at, 1)? {
            [0] => false,
            [1] => true,
            _ => return None,
        };
        questions.push((name, exists));
    }
    Some(questions)
}

#[cfg(test)]
mod tests;
