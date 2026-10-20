//! Name postings as the index keeps them: one row for each distinct name, holding a list
//! of the units that hold it, in unit order, each with the name's roles there and the
//! blocks of its lines that a query for the name reads
//! ([`AnalysisUnit::split`](crate::unit::AnalysisUnit::split)). A row for each name is
//! smaller, and quicker to build, than a row for each name and unit.
//!
//! An update collects the postings of the units it adds and drops, and stores them in
//! batches: in each batch, each name's list is read, changed and written back once, in
//! name order.

use super::{BLOCK_BITS, StoreError};
use crate::hashing::{HashMap, HashSet};
use rusqlite::{OptionalExtension, Transaction, params};

/// How many postings an update collects before it stores them, which bounds its memory
/// (about 400 MB on the Linux kernel); an index build afresh of it stores them in four
/// batches.
const BATCH: usize = 8_000_000;

/// A unit in a name's list: its id, the name's roles there ([`crate::unit::Mark::role`]),
/// and the blocks listed for it, as the row keeps them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Posting<'b> {
    pub unit: i64,
    pub roles: u32,
    /// How many blocks, then each block's number as its difference from the one before
    /// (the first from 0), as variable-length integers; checked when decoded.
    blocks: &'b [u8],
    /// The length of its answer rows, then the rows ([`crate::unit::Occurrences`]).
    rows: &'b [u8],
}

impl<'b> Posting<'b> {
    /// The unit's answer rows for the name ([`crate::unit::Occurrences`]).
    pub(crate) fn rows(&self) -> &'b [u8] {
        let mut at = 0;
        match read_number(self.rows, &mut at) {
            Some(_) => &self.rows[at..],
            None => &[],
        }
    }

    /// The blocks listed, in order.
    pub(crate) fn blocks(&self) -> impl Iterator<Item = u32> + '_ {
        let mut at = 0;
        let count = read_number(self.blocks, &mut at).unwrap_or_default();
        let mut number = 0u32;
        (0..count).map(move |_| {
            let delta = read_number(self.blocks, &mut at).unwrap_or_default();
            number += u32::try_from(delta).unwrap_or_default();
            number
        })
    }
}

/// A list as a row keeps it: for each unit, its id as its difference from the one before,
/// the roles, and its blocks, as variable-length integers.
pub(super) fn encode(list: &[Posting<'_>]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(4 * list.len());
    let mut previous = 0;
    for posting in list {
        debug_assert!(
            posting.unit > previous,
            "a list holds its units in order, each once"
        );
        write_number(
            &mut bytes,
            u64::try_from(posting.unit - previous).unwrap_or_default(),
        );
        write_number(&mut bytes, u64::from(posting.roles));
        bytes.extend_from_slice(posting.blocks);
        bytes.extend_from_slice(posting.rows);
        previous = posting.unit;
    }
    bytes
}

/// A row's list; `None` for bytes no list encodes to: a unit twice, a unit without
/// blocks, or a block twice or beyond those a unit can have.
pub(super) fn decode(bytes: &[u8]) -> Option<Vec<Posting<'_>>> {
    let mut list = Vec::new();
    let (mut at, mut unit) = (0, 0i64);
    while at < bytes.len() {
        let delta = read_number(bytes, &mut at)?;
        let roles = u32::try_from(read_number(bytes, &mut at)?).ok()?;
        unit = unit.checked_add(i64::try_from(delta).ok()?)?;
        if delta == 0 && !list.is_empty() {
            return None;
        }
        let from = at;
        let count = read_number(bytes, &mut at)?;
        if count == 0 {
            return None;
        }
        let mut number = 0u64;
        for index in 0..count {
            let delta = read_number(bytes, &mut at)?;
            if delta == 0 && index > 0 {
                return None;
            }
            number = number.checked_add(delta)?;
            if number >= 1 << BLOCK_BITS {
                return None;
            }
        }
        let blocks = &bytes[from..at];
        let from = at;
        let length = usize::try_from(read_number(bytes, &mut at)?).ok()?;
        at = at.checked_add(length).filter(|end| *end <= bytes.len())?;
        list.push(Posting {
            unit,
            roles,
            blocks,
            rows: &bytes[from..at],
        });
    }
    Some(list)
}

fn write_number(out: &mut Vec<u8>, mut value: u64) {
    loop {
        let byte = (value & 0x7f) as u8;
        value >>= 7;
        if value == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

fn read_number(bytes: &[u8], at: &mut usize) -> Option<u64> {
    let mut value = 0u64;
    for shift in (0..64).step_by(7) {
        let byte = *bytes.get(*at)?;
        *at += 1;
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Some(value);
        }
    }
    None
}

/// Postings an update has yet to store.
pub(super) struct Pending {
    /// Units to add to each name's list, with the name's roles there.
    added: HashMap<Box<[u8]>, Added>,
    /// Units to take out of each name's list.
    removed: HashMap<Box<[u8]>, Vec<i64>>,
    count: usize,
    /// How many to collect before storing them.
    batch: usize,
}

impl Default for Pending {
    fn default() -> Self {
        Self::with_batch(BATCH)
    }
}

/// Units to add to a name's list, encoded as a row keeps a list: an update adds units in
/// the order it makes them, so each follows the one before.
#[derive(Default)]
struct Added {
    last: i64,
    bytes: Vec<u8>,
}

impl Pending {
    pub(super) fn with_batch(batch: usize) -> Self {
        Self {
            added: HashMap::default(),
            removed: HashMap::default(),
            count: 0,
            batch,
        }
    }

    /// Adds `unit` to `name`'s list, with the name's roles there and the blocks listed for
    /// it, in order.
    pub(super) fn add(&mut self, name: &[u8], unit: i64, roles: u32, blocks: &[u32], rows: &[u8]) {
        let added = match self.added.get_mut(name) {
            Some(added) => added,
            None => self.added.entry(name.into()).or_default(),
        };
        debug_assert!(
            unit > added.last,
            "an update adds units in the order it makes them"
        );
        debug_assert!(
            !blocks.is_empty() && blocks.is_sorted_by(|a, b| a < b),
            "a name is listed in some blocks, in order"
        );
        write_number(
            &mut added.bytes,
            u64::try_from(unit - added.last).unwrap_or_default(),
        );
        write_number(&mut added.bytes, u64::from(roles));
        write_number(&mut added.bytes, blocks.len() as u64);
        let mut previous = 0;
        for &block in blocks {
            write_number(&mut added.bytes, u64::from(block - previous));
            previous = block;
        }
        write_number(&mut added.bytes, rows.len() as u64);
        added.bytes.extend_from_slice(rows);
        added.last = unit;
        self.count += 1;
    }

    pub(super) fn remove(&mut self, name: &[u8], unit: i64) {
        self.removed.entry(name.into()).or_default().push(unit);
        self.count += 1;
    }

    pub(super) fn batch(&self) -> usize {
        self.batch
    }

    /// Whether nothing is collected.
    #[cfg(test)]
    pub(super) fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Whether enough are collected to store them now.
    pub(super) fn full(&self) -> bool {
        self.count >= self.batch
    }

    /// Stores what is collected, in name order: each list read (unless `fresh`: the
    /// index holds no postings yet), changed and written back, or deleted once empty. A
    /// unit added again replaces what the list held for it.
    pub(super) fn store(
        &mut self,
        transaction: &Transaction<'_>,
        fresh: bool,
    ) -> Result<(), StoreError> {
        let added = std::mem::take(&mut self.added);
        let removed = std::mem::take(&mut self.removed);
        self.count = 0;
        let mut names: Vec<(&[u8], Option<&Added>)> = added
            .iter()
            .map(|(name, added)| (&name[..], Some(added)))
            .chain(
                removed
                    .keys()
                    .filter(|name| !added.contains_key(&name[..]))
                    .map(|name| (&name[..], None)),
            )
            .collect();
        names.sort_unstable_by(|a, b| a.0.cmp(b.0));
        let mut read = transaction.prepare_cached("SELECT postings FROM names WHERE name = ?1")?;
        let mut write = transaction
            .prepare_cached("INSERT OR REPLACE INTO names (name, postings) VALUES (?1, ?2)")?;
        let mut insert =
            transaction.prepare_cached("INSERT INTO names (name, postings) VALUES (?1, ?2)")?;
        let mut delete = transaction.prepare_cached("DELETE FROM names WHERE name = ?1")?;
        for (name, new) in names {
            let gone = removed.get(name).map_or(&[][..], Vec::as_slice);
            if fresh && gone.is_empty() {
                if let Some(new) = new {
                    insert.execute(params![name, new.bytes])?;
                }
                continue;
            }
            let stored: Option<Vec<u8>> = read.query_row([name], |row| row.get(0)).optional()?;
            let stored = stored.unwrap_or_default();
            let mut list = decode(&stored).ok_or_else(damaged)?;
            // A unit dropped goes, whether the list held it or it was added meanwhile; a
            // unit added replaces what the list held for it.
            let gone: HashSet<i64> = gone.iter().copied().collect();
            let mut new = match new {
                Some(new) => decode(&new.bytes).ok_or_else(damaged)?,
                None => Vec::new(),
            };
            new.retain(|posting| !gone.contains(&posting.unit));
            let added: HashSet<i64> = new.iter().map(|posting| posting.unit).collect();
            list.retain(|posting| !gone.contains(&posting.unit) && !added.contains(&posting.unit));
            list.extend(new);
            list.sort_unstable_by_key(|posting| posting.unit);
            if list.is_empty() {
                delete.execute([name])?;
            } else {
                write.execute(params![name, encode(&list)])?;
            }
        }
        Ok(())
    }
}

/// Takes `units` out of every list, for units whose images no longer tell which names
/// they hold: one pass over every list.
pub(super) fn purge(transaction: &Transaction<'_>, units: &HashSet<i64>) -> Result<(), StoreError> {
    let mut changed = Vec::new();
    {
        let mut rows = transaction.prepare("SELECT name, postings FROM names")?;
        let mut rows = rows.query([])?;
        while let Some(row) = rows.next()? {
            let bytes: Vec<u8> = row.get(1)?;
            let mut list = decode(&bytes).ok_or_else(damaged)?;
            let before = list.len();
            list.retain(|posting| !units.contains(&posting.unit));
            if list.len() != before {
                changed.push((row.get::<_, Vec<u8>>(0)?, encode(&list)));
            }
        }
    }
    for (name, list) in changed {
        if list.is_empty() {
            transaction.execute("DELETE FROM names WHERE name = ?1", [&name])?;
        } else {
            transaction.execute(
                "UPDATE names SET postings = ?2 WHERE name = ?1",
                params![name, list],
            )?;
        }
    }
    Ok(())
}

/// A stored list that does not decode: the index is damaged.
pub(super) fn damaged() -> StoreError {
    StoreError::Postings
}

/// What stopped a query reading the name postings, as it takes it: damage it reads
/// around, unless the index was only busy.
pub(super) fn unread(error: StoreError) -> StoreError {
    match error {
        StoreError::Busy => StoreError::Busy,
        _ => StoreError::Postings,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A posting as the tests write it: its unit, roles, blocks and answer rows.
    type Written = (i64, u32, Vec<u32>, Vec<u8>);

    /// What a list as `Pending::add` writes it decodes to: each unit with its roles,
    /// blocks and answer rows.
    fn listed(bytes: &[u8]) -> Option<Vec<Written>> {
        let list = decode(bytes)?;
        Some(
            list.iter()
                .map(|posting| {
                    let blocks = posting.blocks().collect();
                    (posting.unit, posting.roles, blocks, posting.rows().to_vec())
                })
                .collect(),
        )
    }

    #[test]
    fn lists_round_trip_and_damage_is_refused() {
        let list: Vec<Written> = vec![
            (1, 1, vec![0], vec![]),
            (2, 1 << 16, vec![3, 4, 900], vec![7; 200]),
            (300, 0x2a, vec![0, 1], vec![1, 2, 3]),
            (70_000, 1 << 13, vec![(1 << BLOCK_BITS) - 1], vec![]),
        ];
        let mut pending = Pending::default();
        for (unit, roles, blocks, rows) in &list {
            pending.add(b"name", *unit, *roles, blocks, rows);
        }
        let bytes = std::mem::take(&mut pending.added.get_mut(&b"name"[..]).unwrap().bytes);
        assert_eq!(listed(&bytes), Some(list));
        assert_eq!(encode(&decode(&bytes).unwrap()), bytes);
        assert_eq!(listed(&[]), Some(Vec::new()));
        assert_eq!(decode(&bytes[..bytes.len() - 1]), None);
        // Unit 5 with roles 1 in blocks 0 and 2 and rows 9 is a list; then the same unit
        // again, a unit without blocks, a block twice, a block beyond 2^20 and rows longer
        // than the list are none.
        assert_eq!(
            listed(&[5, 1, 2, 0, 2, 1, 9]),
            Some(vec![(5, 1, vec![0, 2], vec![9])])
        );
        assert_eq!(decode(&[5, 1, 1, 0, 0, 0, 1, 1, 0, 0]), None);
        assert_eq!(decode(&[5, 1, 0, 0]), None);
        assert_eq!(decode(&[5, 1, 2, 3, 0, 0]), None);
        assert_eq!(decode(&[5, 1, 1, 0x80, 0x80, 0x40, 0]), None);
        assert_eq!(decode(&[5, 1, 1, 0, 2, 9]), None);
        assert_eq!(
            listed(&[5, 1, 1, 0xff, 0xff, 0x3f, 0]),
            Some(vec![(5, 1, vec![(1 << 20) - 1], vec![])])
        );
    }
}
