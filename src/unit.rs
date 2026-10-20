//! The Analysis unit: what the analyzer produces for one captured source, and what the
//! Store keeps and queries read (docs/architecture/reading-c.md and
//! docs/architecture/storage.md).
//!
//! A unit is an immutable image of the file's stored lines. It is built once from the
//! analyzer's records, kept as it is by the Store, and read line by line by queries; its
//! bytes never need the source file again.

use crate::scanner::{Record, Symbol};
use std::fmt;

/// The image format's version. Images from other versions are refused by
/// [`AnalysisUnit::from_image`]; the Store rebuilds them from their sources.
pub(crate) const IMAGE_VERSION: u8 = 1;
const MAGIC: &[u8; 4] = b"RSU\0";

/// One file's stored lines, as one image, checked whole before its lines are read.
#[derive(Clone)]
pub(crate) struct AnalysisUnit {
    image: Box<[u8]>,
    lines: u32,
    /// Whether the whole image is sound, and if so its last Include's line: worked out
    /// once, when made or when first asked ([`Self::checked`]).
    checked: std::sync::OnceLock<Result<Option<u32>, UnitError>>,
}

/// Why records or an image cannot form a unit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum UnitError {
    /// Line numbers go backwards.
    LineOrder,
    /// A name lies outside its line's text, or names go backwards.
    NameBounds,
    /// A line, text or name count does not fit the image.
    TooLarge,
    /// The image is from another format version.
    Version(u8),
    /// The image is damaged.
    Damaged,
}

impl fmt::Display for UnitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LineOrder => f.write_str("analysis unit lines are out of order"),
            Self::NameBounds => f.write_str("analysis unit name lies outside its line"),
            Self::TooLarge => f.write_str("analysis unit is too large"),
            Self::Version(version) => {
                write!(f, "analysis unit image version {version} is not supported")
            }
            Self::Damaged => f.write_str("analysis unit image is damaged"),
        }
    }
}

/// A stored line, borrowed from a unit's image.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Line<'u> {
    pub number: u32,
    /// The displayed text, before any transport escaping.
    pub text: &'u [u8],
    names: &'u [u8],
    pub define_end_after: bool,
}

/// A name on a stored line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Name<'u> {
    pub mark: Mark,
    /// Where the name starts in its line's text.
    pub at: u32,
    pub text: &'u [u8],
    /// Field 9's test finds an assignment after the name.
    pub assigned: bool,
}

impl<'u> Line<'u> {
    pub(crate) fn names(&self) -> impl Iterator<Item = Name<'u>> + 'u {
        let text = self.text;
        let mut reader = Reader::new(self.names);
        std::iter::from_fn(move || {
            if reader.is_empty() {
                return None;
            }
            let (flags, at, len) = (reader.byte()?, reader.number()?, reader.number()?);
            let mark = Mark::from_byte(reader.byte()?)?;
            let start = usize::try_from(at).ok()?;
            let end = start.checked_add(usize::try_from(len).ok()?)?;
            Some(Name {
                mark,
                at: u32::try_from(at).ok()?,
                text: text.get(start..end)?,
                assigned: flags & 1 == 1,
            })
        })
    }
}

impl AnalysisUnit {
    /// The last validated Include's physical line, so includers need not walk its tail;
    /// none for an image that is not sound.
    pub(crate) fn last_include_line(&self) -> Option<u32> {
        self.checked().ok().flatten()
    }

    /// Whether the whole image is sound, checked once, its last Include's line if so.
    pub(crate) fn checked(&self) -> Result<Option<u32>, UnitError> {
        *self.checked.get_or_init(|| check(&self.image))
    }

    /// Its include operands in order, each without its delimiters and with whether it was
    /// quoted; an empty operand is left out.
    pub(crate) fn includes(&self) -> Vec<(Vec<u8>, bool)> {
        self.lines()
            .flat_map(|line| line.names().collect::<Vec<_>>())
            .filter(|name| name.mark == Mark::Include)
            .filter_map(|name| {
                let (&open, operand) = name.text.split_first()?;
                (!operand.is_empty()).then(|| (operand.to_vec(), open == b'"'))
            })
            .collect()
    }

    /// [`Self::includes`], each with where it sits and the row an includer query gives for
    /// it.
    pub(crate) fn placed_includes(&self) -> Vec<PlacedInclude<'_>> {
        let mut includes = Vec::new();
        let mut names = Vec::new();
        for (record, line, _) in self.placed_records() {
            if !line.names().any(|name| name.mark == Mark::Include) {
                continue;
            }
            names.clear();
            names.extend(line.names());
            let cut = display_cut(&names);
            for (name, included) in (0u32..).zip(&names) {
                if included.mark != Mark::Include {
                    continue;
                }
                if let Some((&open, operand)) = included.text.split_first()
                    && !operand.is_empty()
                {
                    let index = name as usize;
                    includes.push(PlacedInclude {
                        operand,
                        quoted: open == b'"',
                        at: Placed {
                            line: line.number,
                            record,
                            name,
                        },
                        text: shown(&line, &names, cut, index),
                        after: resume_after(&names, index) as u32,
                    });
                }
            }
        }
        includes
    }

    /// Its name postings, as the index keeps them: each distinct name it holds, with the
    /// roles it has here ([`Mark::role`]), in byte order. Include operands are left out,
    /// since the index keeps them apart, and so are empty names (function and macro
    /// ends), which no pattern matches. The index keeps what this gives, so a change to it
    /// needs a new schema version.
    pub(crate) fn postings(&self) -> Vec<(&[u8], u32)> {
        let mut postings = Vec::new();
        for line in self.lines() {
            for name in line.names() {
                if name.mark != Mark::Include && !name.text.is_empty() {
                    postings.push((name.text, name.mark.role(name.assigned)));
                }
            }
        }
        postings.sort_unstable_by(|a: &(&[u8], u32), b| a.0.cmp(b.0));
        postings.dedup_by(|later, kept| {
            let same = later.0 == kept.0;
            if same {
                kept.1 |= later.1;
            }
            same
        });
        postings
    }

    /// Builds a unit from the analyzer's records, checking their order and bounds.
    pub(crate) fn from_records(records: &[Record]) -> Result<Self, UnitError> {
        let mut image = Vec::with_capacity(
            16 + records
                .iter()
                .map(|r| r.text.len() + 8 * r.symbols.len())
                .sum::<usize>(),
        );
        image.extend_from_slice(MAGIC);
        image.push(IMAGE_VERSION);
        let count = u32::try_from(records.len()).map_err(|_| UnitError::TooLarge)?;
        write_number(&mut image, u64::from(count));
        let mut previous = 0u32;
        let mut last_include_line = None;
        for record in records {
            if record.line < previous {
                return Err(UnitError::LineOrder);
            }
            write_number(&mut image, u64::from(record.line - previous));
            previous = record.line;
            image.push(u8::from(record.define_end_after));
            write_bytes(&mut image, &record.text);
            let mut names = Vec::with_capacity(8 * record.symbols.len());
            let mut last_at = 0;
            for Symbol {
                mark,
                at,
                len,
                assigned,
            } in &record.symbols
            {
                let end = at.checked_add(*len).ok_or(UnitError::NameBounds)?;
                if *at < last_at || end > record.text.len() {
                    return Err(UnitError::NameBounds);
                }
                last_at = *at;
                if *mark == Mark::Include {
                    last_include_line = Some(record.line);
                }
                names.push(u8::from(*assigned));
                write_number(&mut names, *at as u64);
                write_number(&mut names, *len as u64);
                names.push(mark.byte());
            }
            write_bytes(&mut image, &names);
        }
        Ok(Self {
            image: image.into_boxed_slice(),
            lines: count,
            checked: std::sync::OnceLock::from(Ok(last_include_line)),
        })
    }

    /// Checks an image read back from storage, completely, so reading it cannot fail.
    pub(crate) fn from_image(image: Box<[u8]>) -> Result<Self, UnitError> {
        let unit = Self::unchecked(image)?;
        unit.checked()?;
        Ok(unit)
    }

    /// An image whose format version and line count are read now, and whose lines are
    /// checked when it is first asked whether it is sound ([`Self::checked`]), on
    /// whichever thread reads it first.
    pub(crate) fn unchecked(image: Box<[u8]>) -> Result<Self, UnitError> {
        let mut reader = Reader::new(&image);
        if reader.take(4) != Some(&MAGIC[..]) {
            return Err(UnitError::Damaged);
        }
        let version = reader.byte().ok_or(UnitError::Damaged)?;
        if version != IMAGE_VERSION {
            return Err(UnitError::Version(version));
        }
        let count = reader
            .number()
            .and_then(|n| u32::try_from(n).ok())
            .ok_or(UnitError::Damaged)?;
        Ok(Self {
            image,
            lines: count,
            checked: std::sync::OnceLock::new(),
        })
    }

    /// The image the Store keeps.
    pub(crate) fn image(&self) -> &[u8] {
        &self.image
    }

    /// Its lines in blocks of about `size` bytes (ADR 0011), as the index keeps them: runs
    /// of consecutive lines in order, each ending once its lines take at least `size` bytes
    /// of image, with the scope each starts in, and its postings with the blocks a query for
    /// each name reads. There is at least one block, empty for a unit without lines. The
    /// index keeps what this gives, so a change to it needs a new schema version.
    pub(crate) fn split(&self, size: usize) -> Split<'_> {
        let mut splitter = Splitter::new(size, true);
        for (line, record) in self.records() {
            splitter.add(&line, record);
        }
        splitter.finish()
    }

    /// The postings [`AnalysisUnit::split`] gives for the unit whose lines `blocks` hold,
    /// in order, when they and the scopes they start in are the blocks it gives for
    /// `size`; none when they are not.
    pub(crate) fn check_split<'u>(
        blocks: &[(&'u AnalysisUnit, Scope<'u>)],
        size: usize,
    ) -> Option<Postings<'u>> {
        let mut splitter = Splitter::new(size, false);
        for (block, start) in blocks {
            let mut first = true;
            for (line, record) in block.records() {
                let starts = splitter.add(&line, record)?;
                if starts != first || (starts && splitter.start != *start) {
                    return None;
                }
                first = false;
            }
            // Only a unit without lines has an empty block: its only one, from the file
            // scope.
            if first && (blocks.len() > 1 || *start != Scope::default()) {
                return None;
            }
        }
        let made = splitter.started.max(1) as usize;
        (made == blocks.len()).then(|| splitter.finish().postings)
    }

    /// The unit whose lines are those of `blocks`, in order, as [`AnalysisUnit::split`]
    /// made them. Line numbers must not go back from one block to the next.
    pub(crate) fn join<'b>(blocks: impl IntoIterator<Item = &'b Self>) -> Result<Self, UnitError> {
        let (mut records, mut count, mut previous) = (Vec::new(), 0u32, 0u32);
        let mut last_include_line = None;
        for block in blocks {
            if block.last_include_line().is_some() {
                last_include_line = block.last_include_line();
            }
            for (line, record) in block.records() {
                if line.number < previous {
                    return Err(UnitError::LineOrder);
                }
                write_number(&mut records, u64::from(line.number - previous));
                records.extend_from_slice(record);
                count = count.checked_add(1).ok_or(UnitError::TooLarge)?;
                previous = line.number;
            }
        }
        Ok(image(&mut records, &mut count, &mut last_include_line))
    }

    /// The stored lines, in line order.
    pub(crate) fn lines(&self) -> impl Iterator<Item = Line<'_>> + '_ {
        self.records().map(|(line, _)| line)
    }

    /// The stored line numbered `number` whose record starts `offset` bytes into its
    /// records; none where the image holds no record there.
    pub(crate) fn line_at(&self, offset: u32, number: u32) -> Option<Line<'_>> {
        let mut reader = Reader::new(self.image.get(MAGIC.len() + 1..)?);
        reader.number()?;
        reader.at = reader.at.checked_add(usize::try_from(offset).ok()?)?;
        reader.number()?;
        let define_end_after = reader.byte()? == 1;
        let text = reader.bytes()?;
        let names = reader.bytes()?;
        Some(Line {
            number,
            text,
            names,
            define_end_after,
        })
    }

    /// The stored lines, each with its record's bytes after the line number.
    fn records(&self) -> impl Iterator<Item = (Line<'_>, &[u8])> + '_ {
        self.placed_records()
            .map(|(_, line, record)| (line, record))
    }

    /// [`Self::records`], each with the offset of its record in the records, as
    /// [`Self::line_at`] takes it.
    fn placed_records(&self) -> impl Iterator<Item = (u32, Line<'_>, &[u8])> + '_ {
        let mut reader = Reader::new(&self.image[MAGIC.len() + 1..]);
        let _count = reader.number();
        let start = reader.at;
        let mut remaining = self.lines;
        let mut number = 0u32;
        std::iter::from_fn(move || {
            if remaining == 0 {
                return None;
            }
            remaining -= 1;
            let offset = u32::try_from(reader.at - start).ok()?;
            number += u32::try_from(reader.number()?).ok()?;
            let from = reader.at;
            let define_end_after = reader.byte()? == 1;
            let text = reader.bytes()?;
            let names = reader.bytes()?;
            let line = Line {
                number,
                text,
                names,
                define_end_after,
            };
            Some((offset, line, &reader.bytes[from..reader.at]))
        })
    }
}

/// Checks an image whose header is sound: its lines, every name within its line's text
/// and in order, and nothing after them. Gives its last Include's line.
fn check(image: &[u8]) -> Result<Option<u32>, UnitError> {
    let mut reader = Reader::new(image);
    if reader.take(4) != Some(&MAGIC[..]) {
        return Err(UnitError::Damaged);
    }
    let version = reader.byte().ok_or(UnitError::Damaged)?;
    if version != IMAGE_VERSION {
        return Err(UnitError::Version(version));
    }
    let count = reader
        .number()
        .and_then(|n| u32::try_from(n).ok())
        .ok_or(UnitError::Damaged)?;
    let mut number = 0u32;
    let mut last_include_line = None;
    for _ in 0..count {
        let delta = reader
            .number()
            .and_then(|n| u32::try_from(n).ok())
            .ok_or(UnitError::Damaged)?;
        number = number.checked_add(delta).ok_or(UnitError::Damaged)?;
        let flag = reader.byte().ok_or(UnitError::Damaged)?;
        let text = reader.bytes().ok_or(UnitError::Damaged)?;
        let names = reader.bytes().ok_or(UnitError::Damaged)?;
        if flag > 1 {
            return Err(UnitError::Damaged);
        }
        let mut reader = Reader::new(names);
        let mut last_at = 0;
        while !reader.is_empty() {
            let flags = reader.byte().ok_or(UnitError::Damaged)?;
            let at = reader.number().ok_or(UnitError::Damaged)?;
            let len = reader.number().ok_or(UnitError::Damaged)?;
            let mark = reader
                .byte()
                .and_then(Mark::from_byte)
                .ok_or(UnitError::Damaged)?;
            if mark == Mark::Include {
                last_include_line = Some(number);
            }
            let end = at.checked_add(len).ok_or(UnitError::Damaged)?;
            if flags > 1 || at < last_at || end > text.len() as u64 {
                return Err(UnitError::Damaged);
            }
            last_at = at;
        }
    }
    if !reader.is_empty() {
        return Err(UnitError::Damaged);
    }
    Ok(last_include_line)
}

/// An image of `records`, the stored records of `count` lines, which it takes.
fn image(
    records: &mut Vec<u8>,
    count: &mut u32,
    last_include_line: &mut Option<u32>,
) -> AnalysisUnit {
    let mut image = Vec::with_capacity(MAGIC.len() + 6 + records.len());
    image.extend_from_slice(MAGIC);
    image.push(IMAGE_VERSION);
    write_number(&mut image, u64::from(*count));
    image.append(records);
    AnalysisUnit {
        image: image.into_boxed_slice(),
        lines: std::mem::take(count),
        checked: std::sync::OnceLock::from(Ok(std::mem::take(last_include_line))),
    }
}

/// A unit's lines in blocks, as [`AnalysisUnit::split`] gives them.
pub(crate) struct Split<'u> {
    /// Each block's lines as an image of their own, with the scope its first line starts in.
    pub blocks: Vec<(AnalysisUnit, Scope<'u>)>,
    pub postings: Postings<'u>,
}

/// Each name a unit holds, as [`AnalysisUnit::postings`] gives it, with the blocks of its
/// split a query for it reads, in order: those it occurs in, and those that start inside
/// its macro's or function's body.
pub(crate) struct Postings<'u> {
    /// Each name, its roles, and where its blocks end in `blocks` and its answer rows in
    /// `rows`.
    names: Vec<(&'u [u8], u32, usize, usize)>,
    blocks: Vec<u32>,
    rows: Vec<u8>,
}

impl<'u> Postings<'u> {
    /// Each name, in byte order, with its roles, blocks and encoded answer rows
    /// ([`Occurrences`]).
    pub(crate) fn iter(&self) -> impl Iterator<Item = (&'u [u8], u32, &[u32], &[u8])> + '_ {
        let (mut start, mut from) = (0, 0);
        self.names.iter().map(move |&(name, roles, end, to)| {
            let blocks = &self.blocks[start..end];
            let rows = &self.rows[from..to];
            (start, from) = (end, to);
            (name, roles, blocks, rows)
        })
    }
}

/// The rows an exact name's query in fields 0, 1, 2, 3 and 9 answers from one unit, as its
/// postings keep them: for each field with rows, the field and how many, then each row's
/// line, block, the offset of its line's record in the block and the index of its name
/// there, and its context, where it is not the field's own. A query for the name reads
/// these lines instead of walking the blocks.
///
/// Lines count up from the row before in the field, and an offset from the row before
/// when it is in the same block. A context is none (0), the last one given in the field
/// when that row is in the same block (1), the block's
/// starting function (2) or macro (3), or a name in the block's records (4, then the
/// record's offset and the name's index), so it costs a few bytes however long it is.
pub(crate) struct Occurrences<'b>(pub &'b [u8]);

/// Where a line's name sits in a unit's image: the line's number, the offset of its record
/// ([`AnalysisUnit::line_at`]) and the name's index in the line.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Placed {
    pub line: u32,
    pub record: u32,
    pub name: u32,
}

/// An include as the index records it: its operand without its delimiters, whether it was
/// quoted, where it sits, and its includer row: the text the row shows and the index of
/// the name reading resumes at after it, since the row consumes the rest of its line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PlacedInclude<'u> {
    pub operand: &'u [u8],
    pub quoted: bool,
    pub at: Placed,
    pub text: &'u [u8],
    pub after: u32,
}

/// The name after which cscope's display of a line stops: the first that another follows
/// directly, which its database separates by an empty fragment.
pub(crate) fn display_cut(names: &[Name<'_>]) -> Option<usize> {
    (0..names.len().saturating_sub(1)).find(|&k| follows_directly(names, k))
}

/// Whether the name after the one at `k` starts no later than that one ends.
fn follows_directly(names: &[Name<'_>], k: usize) -> bool {
    names[k + 1].at as usize <= names[k].at as usize + names[k].text.len()
}

/// The text a row for the name at `index` shows, given the line's [`display_cut`]. cscope
/// shows the line up to the cut; a match after the cut aborts its query, and ripscope shows
/// the whole line.
pub(crate) fn shown<'u>(
    line: &Line<'u>,
    names: &[Name<'u>],
    cut: Option<usize>,
    index: usize,
) -> &'u [u8] {
    match cut {
        Some(cut) if index <= cut => {
            let name = &names[cut];
            &line.text[..name.at as usize + name.text.len()]
        }
        _ => line.text,
    }
}

/// Where reading resumes after a row for the name at `index` that consumes the rest of the
/// line: after the next name another follows directly, where cscope's display stopped.
pub(crate) fn resume_after(names: &[Name<'_>], index: usize) -> usize {
    (index..names.len().saturating_sub(1))
        .find(|&k| follows_directly(names, k))
        .map_or(names.len(), |k| k + 1)
}

/// One row of [`Occurrences`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Occurrence {
    pub field: u8,
    pub line: u32,
    pub block: u32,
    pub offset: u32,
    pub index: u32,
    /// The macro or function it sits in, for fields 0, 3 and 9.
    pub context: Option<Within>,
}

/// Where a row's context is named, in the row's block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Within {
    /// The function the block starts inside.
    StartFunction,
    /// The macro the block starts inside.
    StartMacro,
    /// The name at `index` of the record `offset` bytes into the block's records.
    At { offset: u32, index: u32 },
}

impl Within {
    /// The name it gives in `block`, which starts in `start`; none where the block holds
    /// no such name.
    pub(crate) fn name<'b>(self, block: &'b AnalysisUnit, start: Scope<'b>) -> Option<&'b [u8]> {
        match self {
            Within::StartFunction => start.function,
            Within::StartMacro => start.macro_name,
            Within::At { offset, index } => block
                .line_at(offset, 0)?
                .names()
                .nth(usize::try_from(index).ok()?)
                .map(|name| name.text),
        }
    }
}

impl Occurrences<'_> {
    /// The rows, in order; `None` once the bytes are damaged.
    pub(crate) fn iter(&self) -> impl Iterator<Item = Option<Occurrence>> + '_ {
        let mut reader = Reader::new(self.0);
        let (mut field, mut left) = (0, 0u64);
        // The row before in the field, and the last context given there.
        let mut before: Option<Occurrence> = None;
        let mut last = None;
        std::iter::from_fn(move || {
            if left == 0 && reader.is_empty() {
                return None;
            }
            Some((|| {
                if left == 0 {
                    field = reader.byte()?;
                    left = reader.number().filter(|count| *count > 0)?;
                    (before, last) = (None, None);
                }
                left -= 1;
                let number = |reader: &mut Reader<'_>| u32::try_from(reader.number()?).ok();
                let line = before
                    .map_or(0, |before| before.line)
                    .checked_add(number(&mut reader)?)?;
                let block = number(&mut reader)?;
                let offset = match before.filter(|before| before.block == block) {
                    Some(before) => before.offset.checked_add(number(&mut reader)?)?,
                    None => number(&mut reader)?,
                };
                let index = number(&mut reader)?;
                let context = match reader.number()? {
                    0 => None,
                    1 => Some(last?),
                    2 => Some(Within::StartFunction),
                    3 => Some(Within::StartMacro),
                    4 => Some(Within::At {
                        offset: number(&mut reader)?,
                        index: number(&mut reader)?,
                    }),
                    _ => return None,
                };
                last = context.or(last);
                let occurrence = Occurrence {
                    field,
                    line,
                    block,
                    offset,
                    index,
                    context,
                };
                before = Some(occurrence);
                Some(occurrence)
            })())
        })
    }
}

/// Cuts a unit's lines into blocks as they come, as [`AnalysisUnit::split`] does, and lists
/// the blocks a query for each name reads.
struct Splitter<'u> {
    size: usize,
    /// Whether it makes the blocks' images, or only finds where blocks start.
    making: bool,
    blocks: Vec<(AnalysisUnit, Scope<'u>)>,
    /// The current block's records, when making them, the bytes they take, and how many
    /// lines they hold.
    records: Vec<u8>,
    bytes: usize,
    count: u32,
    last_include_line: Option<u32>,
    /// The last line's number.
    previous: u32,
    /// How many blocks it started.
    started: u32,
    /// The scope after the lines so far, and the one the current block started in.
    scope: Scope<'u>,
    start: Scope<'u>,
    /// Where in the current block the scope's function and macro were named: the record's
    /// offset and the name's index; none for those the block starts in.
    function_at: Option<(u32, u32)>,
    macro_at: Option<(u32, u32)>,
    /// Each name read, with its roles there and its block.
    listed: Vec<(&'u [u8], u32, u32)>,
    /// Each exact-name answer row, by the number of the name it answers in `ids` and its
    /// field's place in [`ROW_FIELDS`]: line, block, offset of the line's record in the
    /// block, name index and context.
    rows: Vec<(usize, Occurrence)>,
    /// A number for each name a row answers, in the order first met.
    ids: crate::hashing::HashMap<&'u [u8], usize>,
    /// The current line's names, and the names it gave a row in fields 0 and 9 or where a
    /// field 1 row lets reading resume, kept from line to line to spare allocations.
    names: Vec<Name<'u>>,
    symbols: Vec<&'u [u8]>,
    assigned: Vec<&'u [u8]>,
    next: Vec<(&'u [u8], usize)>,
}

impl<'u> Splitter<'u> {
    /// Adds an exact-name answer row for `name`.
    fn row(&mut self, name: &'u [u8], occurrence: Occurrence) {
        let next = self.ids.len();
        let id = *self.ids.entry(name).or_insert(next);
        let field = ROW_FIELDS
            .iter()
            .position(|&field| field == occurrence.field);
        self.rows
            .push((id * ROW_FIELDS.len() + field.unwrap_or(0), occurrence));
    }

    fn new(size: usize, making: bool) -> Self {
        Self {
            size,
            making,
            blocks: Vec::new(),
            records: Vec::new(),
            bytes: 0,
            count: 0,
            last_include_line: None,
            previous: 0,
            started: 0,
            scope: Scope::default(),
            start: Scope::default(),
            function_at: None,
            macro_at: None,
            listed: Vec::new(),
            rows: Vec::new(),
            ids: crate::hashing::HashMap::default(),
            names: Vec::new(),
            symbols: Vec::new(),
            assigned: Vec::new(),
            next: Vec::new(),
        }
    }

    /// Adds the next line, given with its record after the line number: whether it starts
    /// a block, which a line does once the block before holds `size` bytes; none when its
    /// number is before the last line's.
    fn add(&mut self, line: &Line<'u>, record: &[u8]) -> Option<bool> {
        if line.number < self.previous {
            return None;
        }
        let starts = self.started == 0 || self.bytes >= self.size;
        if starts {
            if self.making && self.started > 0 {
                let block = image(
                    &mut self.records,
                    &mut self.count,
                    &mut self.last_include_line,
                );
                self.blocks.push((block, self.start));
            }
            (self.start, self.bytes, self.count) = (self.scope, 0, 0);
            (self.function_at, self.macro_at) = (None, None);
            // A block is also read for the macro and the function it starts inside, whose
            // bodies go on into it (field 2).
            for name in [self.start.macro_name, self.start.function]
                .into_iter()
                .flatten()
            {
                if !name.is_empty() {
                    self.listed.push((name, 0, self.started));
                }
            }
            self.started += 1;
        }
        // A block's first line number is kept whole, so that the block reads on its own.
        let delta = line.number - if starts { 0 } else { self.previous };
        let offset = u32::try_from(self.bytes).unwrap_or(u32::MAX);
        self.bytes += number_bytes(u64::from(delta)) + record.len();
        if self.making {
            write_number(&mut self.records, u64::from(delta));
            self.records.extend_from_slice(record);
        }
        (self.count, self.previous) = (self.count + 1, line.number);
        let block = self.started - 1;
        let mut names = std::mem::take(&mut self.names);
        names.clear();
        names.extend(line.names());
        let row = |field, index: usize, context| Occurrence {
            field,
            line: line.number,
            block,
            offset,
            index: u32::try_from(index).unwrap_or(u32::MAX),
            context,
        };
        // Fields 0 and 9 answer a line once, for its first name that matches.
        let (mut symbols, mut assigned) = (
            std::mem::take(&mut self.symbols),
            std::mem::take(&mut self.assigned),
        );
        symbols.clear();
        assigned.clear();
        for (index, name) in names.iter().enumerate() {
            if name.mark == Mark::Include {
                self.last_include_line = Some(line.number);
            }
            self.scope.read(name);
            let at = Some((offset, u32::try_from(index).unwrap_or(u32::MAX)));
            match name.mark {
                Mark::Function => self.function_at = at,
                Mark::Define => self.macro_at = at,
                _ => {}
            }
            if name.mark != Mark::Include && !name.text.is_empty() {
                let roles = name.mark.role(name.assigned);
                self.listed.push((name.text, roles, block));
            }
            let within = |name: Option<&[u8]>, at: Option<(u32, u32)>, start| {
                name.map(|_| at.map_or(start, |(offset, index)| Within::At { offset, index }))
            };
            let function = within(self.scope.function, self.function_at, Within::StartFunction);
            let macro_name = within(self.scope.macro_name, self.macro_at, Within::StartMacro);
            let caller = self.scope.macro_name.or(self.scope.function);
            let context = macro_name.or(function);
            if !matches!(
                name.mark,
                Mark::FunctionEnd | Mark::DefineEnd | Mark::Include
            ) && !name.text.is_empty()
            {
                let context = if name.mark == Mark::Define {
                    function
                } else {
                    context
                };
                if !symbols.contains(&name.text) {
                    symbols.push(name.text);
                    self.row(name.text, row(0, index, context));
                }
                if name.assigned && !assigned.contains(&name.text) {
                    assigned.push(name.text);
                    self.row(name.text, row(9, index, context));
                }
            }
            if name.mark == Mark::Call
                && let Some(caller) = caller
            {
                if !name.text.is_empty() {
                    self.row(name.text, row(3, index, context));
                }
                if !caller.is_empty() {
                    self.row(caller, row(2, index, None));
                }
            }
        }
        // Field 1: a definition row consumes the rest of its line, to the next name that
        // directly follows its predecessor.
        let mut next = std::mem::take(&mut self.next);
        next.clear();
        for (index, name) in names.iter().enumerate() {
            let definition = matches!(
                name.mark,
                Mark::Define
                    | Mark::Function
                    | Mark::Class
                    | Mark::Enum
                    | Mark::Member
                    | Mark::Struct
                    | Mark::Typedef
                    | Mark::Union
                    | Mark::Global
            );
            if !definition || name.text.is_empty() {
                continue;
            }
            let from = next
                .iter()
                .find(|(text, _)| *text == name.text)
                .map(|(_, at)| *at);
            if from.is_some_and(|from| index < from) {
                continue;
            }
            self.row(name.text, row(1, index, None));
            let after = after_row(&names, index);
            match next.iter_mut().find(|(text, _)| *text == name.text) {
                Some(entry) => entry.1 = after,
                None => next.push((name.text, after)),
            }
        }
        (self.names, self.symbols, self.assigned, self.next) = (names, symbols, assigned, next);
        self.scope.leave(line);
        Some(starts)
    }

    /// The blocks made, the last one included (a unit without lines has one, empty), and
    /// the postings.
    fn finish(mut self) -> Split<'u> {
        if self.making {
            let block = image(
                &mut self.records,
                &mut self.count,
                &mut self.last_include_line,
            );
            self.blocks.push((block, self.start));
        }
        self.listed
            .sort_unstable_by(|a, b| a.0.cmp(b.0).then_with(|| a.2.cmp(&b.2)));
        // Each name's rows, in field order and within a field in the order added, which is
        // line order: counted into place by name and field.
        let mut starts = vec![0; self.ids.len() * ROW_FIELDS.len() + 1];
        for (group, _) in &self.rows {
            starts[group + 1] += 1;
        }
        for group in 1..starts.len() {
            starts[group] += starts[group - 1];
        }
        let mut ordered = vec![Occurrence::default(); self.rows.len()];
        let mut next = starts.clone();
        for (group, occurrence) in self.rows {
            ordered[next[group]] = occurrence;
            next[group] += 1;
        }
        let mut postings = Postings {
            names: Vec::new(),
            blocks: Vec::new(),
            rows: Vec::new(),
        };
        for (name, roles, block) in self.listed {
            match postings.names.last_mut() {
                Some(last) if last.0 == name => {
                    last.1 |= roles;
                    if postings.blocks.last() != Some(&block) {
                        postings.blocks.push(block);
                        last.2 = postings.blocks.len();
                    }
                }
                _ => {
                    postings.blocks.push(block);
                    let rows = self.ids.get(name).map_or(&[][..], |id| {
                        let first = id * ROW_FIELDS.len();
                        &ordered[starts[first]..starts[first + ROW_FIELDS.len()]]
                    });
                    for rows in rows.chunk_by(|a, b| a.field == b.field) {
                        write_rows(&mut postings.rows, rows);
                    }
                    postings
                        .names
                        .push((name, roles, postings.blocks.len(), postings.rows.len()));
                }
            }
        }
        Split {
            blocks: self.blocks,
            postings,
        }
    }
}

/// Writes one field's rows, in line order, as [`Occurrences`] reads them.
fn write_rows(out: &mut Vec<u8>, rows: &[Occurrence]) {
    out.push(rows[0].field);
    write_number(out, rows.len() as u64);
    let mut before: Option<&Occurrence> = None;
    let mut last: Option<(u32, Within)> = None;
    for row in rows {
        write_number(
            out,
            u64::from(row.line - before.map_or(0, |before| before.line)),
        );
        write_number(out, u64::from(row.block));
        match before.filter(|before| before.block == row.block) {
            Some(before) => write_number(out, u64::from(row.offset - before.offset)),
            None => write_number(out, u64::from(row.offset)),
        }
        write_number(out, u64::from(row.index));
        match row.context {
            None => out.push(0),
            Some(context) if last == Some((row.block, context)) => out.push(1),
            Some(context) => {
                match context {
                    Within::StartFunction => out.push(2),
                    Within::StartMacro => out.push(3),
                    Within::At { offset, index } => {
                        out.push(4);
                        write_number(out, u64::from(offset));
                        write_number(out, u64::from(index));
                    }
                }
                last = Some((row.block, context));
            }
        }
        before = Some(row);
    }
}

/// The fields whose exact-name answer rows the postings keep, in the order kept.
const ROW_FIELDS: [u8; 5] = [0, 1, 2, 3, 9];

/// Where reading resumes after a row for the name at `index` that consumes the rest of its
/// line: the next name that directly follows its predecessor, as the query engine reads it.
fn after_row(names: &[Name<'_>], index: usize) -> usize {
    let follows = |k: usize| names[k + 1].at as usize <= names[k].at as usize + names[k].text.len();
    (index..names.len().saturating_sub(1))
        .find(|&k| follows(k))
        .map_or(names.len(), |k| k + 1)
}

/// Where a name sits within its file under the own query semantics (ADR 0010): the macro
/// being defined and the function, all the own walk carries from one line to the next, so
/// a block of lines is walked from the scope it starts in (ADR 0011).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Scope<'u> {
    /// The macro being defined.
    pub macro_name: Option<&'u [u8]>,
    /// The function being defined: the latest head since the last function end.
    pub function: Option<&'u [u8]>,
}

impl<'u> Scope<'u> {
    /// Reads `name`: a head opens its scope, which the head sits in; an end closes it, and
    /// sits outside.
    pub(crate) fn read(&mut self, name: &Name<'u>) {
        match name.mark {
            Mark::Define => self.macro_name = Some(name.text),
            Mark::DefineEnd => self.macro_name = None,
            Mark::Function => self.function = Some(name.text),
            Mark::FunctionEnd => self.function = None,
            _ => {}
        }
    }

    /// Leaves `line`, whose macro definition may end with it.
    pub(crate) fn leave(&mut self, line: &Line<'_>) {
        if line.define_end_after {
            self.macro_name = None;
        }
    }
}

/// Units are equal when their images are: the rest is worked out from the image.
impl PartialEq for AnalysisUnit {
    fn eq(&self, other: &Self) -> bool {
        self.image == other.image
    }
}

impl Eq for AnalysisUnit {}

impl fmt::Debug for AnalysisUnit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AnalysisUnit")
            .field("lines", &self.lines)
            .field("image_bytes", &self.image.len())
            .finish()
    }
}

/// How many bytes [`write_number`] takes for `value`.
fn number_bytes(value: u64) -> usize {
    (64 - (value | 1).leading_zeros() as usize).div_ceil(7)
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

fn write_bytes(out: &mut Vec<u8>, bytes: &[u8]) {
    write_number(out, bytes.len() as u64);
    out.extend_from_slice(bytes);
}

/// Reads an image; every method returns `None` rather than reading out of bounds.
struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, at: 0 }
    }

    fn is_empty(&self) -> bool {
        self.at >= self.bytes.len()
    }

    fn byte(&mut self) -> Option<u8> {
        let byte = *self.bytes.get(self.at)?;
        self.at += 1;
        Some(byte)
    }

    fn take(&mut self, length: usize) -> Option<&'a [u8]> {
        let end = self.at.checked_add(length)?;
        let slice = self.bytes.get(self.at..end)?;
        self.at = end;
        Some(slice)
    }

    fn number(&mut self) -> Option<u64> {
        let mut value = 0u64;
        for shift in (0..64).step_by(7) {
            let byte = self.byte()?;
            value |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return Some(value);
            }
        }
        None
    }

    fn bytes(&mut self) -> Option<&'a [u8]> {
        let length = usize::try_from(self.number()?).ok()?;
        self.take(length)
    }
}

/// The role a name plays on its line. Each role has the one-byte mark the reference
/// cscope writes in its database, recorded as a compatibility fact.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Mark {
    Plain,
    Define,
    DefineEnd,
    Function,
    FunctionEnd,
    Call,
    Include,
    Global,
    Member,
    Struct,
    Union,
    Enum,
    Typedef,
    Class,
}

impl Mark {
    /// The reference database's mark byte; a blank for plain occurrences.
    pub(crate) fn byte(self) -> u8 {
        match self {
            Self::Plain => b' ',
            Self::Define => b'#',
            Self::DefineEnd => b')',
            Self::Function => b'$',
            Self::FunctionEnd => b'}',
            Self::Call => b'`',
            Self::Include => b'~',
            Self::Global => b'g',
            Self::Member => b'm',
            Self::Struct => b's',
            Self::Union => b'u',
            Self::Enum => b'e',
            Self::Typedef => b't',
            Self::Class => b'c',
        }
    }

    pub(crate) fn from_byte(byte: u8) -> Option<Self> {
        Some(match byte {
            b' ' => Self::Plain,
            b'#' => Self::Define,
            b')' => Self::DefineEnd,
            b'$' => Self::Function,
            b'}' => Self::FunctionEnd,
            b'`' => Self::Call,
            b'~' => Self::Include,
            b'g' => Self::Global,
            b'm' => Self::Member,
            b's' => Self::Struct,
            b'u' => Self::Union,
            b'e' => Self::Enum,
            b't' => Self::Typedef,
            b'c' => Self::Class,
            _ => return None,
        })
    }

    /// Its bit in a name posting's roles ([`AnalysisUnit::postings`]), for an occurrence
    /// that is assigned or not. The most frequent roles take the lowest bits, so that most
    /// postings' roles take one byte. The index keeps these bits, so they never change.
    pub(crate) fn role(self, assigned: bool) -> u32 {
        let bit = match (self, assigned) {
            (Self::Plain, false) => 0,
            (Self::Plain, true) => 1,
            (Self::Call, false) => 2,
            (Self::Function, false) => 3,
            (Self::Define, false) => 4,
            (Self::Global, false) => 5,
            (Self::Member, false) => 6,
            (Self::Struct, false) => 7,
            (Self::Typedef, false) => 8,
            (Self::Enum, false) => 9,
            (Self::Union, false) => 10,
            (Self::Class, false) => 11,
            (Self::DefineEnd, false) => 12,
            (Self::FunctionEnd, false) => 13,
            (Self::Include, false) => 14,
            (Self::Call, true) => 15,
            (Self::Function, true) => 16,
            (Self::Define, true) => 17,
            (Self::Global, true) => 18,
            (Self::Member, true) => 19,
            (Self::Struct, true) => 20,
            (Self::Typedef, true) => 21,
            (Self::Enum, true) => 22,
            (Self::Union, true) => 23,
            (Self::Class, true) => 24,
            (Self::DefineEnd, true) => 25,
            (Self::FunctionEnd, true) => 26,
            (Self::Include, true) => 27,
        };
        1 << bit
    }

    /// The aggregate-tag role for the first letter of `enum`, `struct` or `union`, or `c`
    /// for a class.
    pub(crate) fn tag(letter: u8) -> Option<Self> {
        match letter {
            b'e' => Some(Self::Enum),
            b's' => Some(Self::Struct),
            b'u' => Some(Self::Union),
            b'c' => Some(Self::Class),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{AnalysisUnit, IMAGE_VERSION, MAGIC, Mark, Scope, UnitError};
    use crate::scanner::{self, Mode};
    use std::collections::BTreeMap;

    /// A unit's postings: each distinct name once, with every role it has, assigned ones
    /// apart, and neither include operands nor function and macro ends.
    #[test]
    fn postings_hold_each_name_once_with_its_roles() {
        let source = b"#include \"a.h\"\n#define LIMIT 4\nint total;\n\
                       int add(int n) {\n\ttotal = n;\n\treturn add(LIMIT) + total;\n}\n";
        let unit = AnalysisUnit::from_records(&scanner::scan(source, Mode::C)).expect("records");
        let postings: Vec<(String, u32)> = unit
            .postings()
            .into_iter()
            .map(|(name, roles)| (String::from_utf8_lossy(name).into_owned(), roles))
            .collect();
        let role = Mark::role;
        assert_eq!(
            postings,
            [
                (
                    "LIMIT".into(),
                    role(Mark::Define, false) | role(Mark::Plain, false)
                ),
                (
                    "add".into(),
                    role(Mark::Function, false) | role(Mark::Call, false)
                ),
                ("n".into(), role(Mark::Plain, false)),
                (
                    "total".into(),
                    role(Mark::Global, false) | role(Mark::Plain, true) | role(Mark::Plain, false)
                ),
            ]
        );
        // The bits are part of the index format.
        assert_eq!(role(Mark::Plain, false), 1);
        assert_eq!(role(Mark::Plain, true), 1 << 1);
        assert_eq!(role(Mark::Member, false), 1 << 6);
        assert_eq!(role(Mark::Include, true), 1 << 27);
    }

    /// A split holds the unit's lines in order, in blocks that each read as a unit of their
    /// own, start in the scope the lines before them leave, and end once their lines take at
    /// least the block size. Each name has its roles and is listed in the blocks it occurs
    /// in and in those that start inside its macro's or function's body.
    #[test]
    fn a_split_holds_the_lines_in_blocks_with_their_postings() {
        let source = b"#define LIMIT 4\n#define WRAP(x) \\\n\tcall(x)\nint total;\n\n\
                       int add(int n) {\n#define INNER() target()\n\ttotal = n + LIMIT + \
                       some_rather_longer_name_to_make_this_line_long;\n\treturn add(n);\n}\n";
        let source = [&b"#include <first.h>\n"[..], source, b"#include <last.h>\n"].concat();
        let unit = AnalysisUnit::from_records(&scanner::scan(&source, Mode::C)).expect("records");
        let whole: Vec<_> = unit
            .lines()
            .map(|line| (line, line.names().count()))
            .collect();
        let records = unit.image().len() - (MAGIC.len() + 2);
        let mut inside = 0;
        for size in [0, 1, 12, 30, 80, records, records + 1] {
            let split = unit.split(size);
            let (mut lines, mut scope) = (Vec::new(), Scope::default());
            let mut listed = BTreeMap::new();
            for (index, (block, start)) in split.blocks.iter().enumerate() {
                let checked =
                    AnalysisUnit::from_image(block.image().into()).expect("a block reads");
                assert_eq!(&checked, block, "{size}");
                assert_eq!(*start, scope, "{size}: block {index}");
                let held = block.image().len() - (MAGIC.len() + 2);
                let last = block.lines().last().expect("lines");
                if index + 1 < split.blocks.len() {
                    let before_last = held - line_bytes(block, last);
                    let one = block.lines().count() == 1;
                    assert!(
                        held >= size && (one || before_last < size),
                        "{size}: {index}"
                    );
                }
                let number = u32::try_from(index).expect("block number");
                for name in [start.macro_name, start.function].into_iter().flatten() {
                    inside += 1;
                    list(&mut listed, name, 0, number);
                }
                for line in block.lines() {
                    for name in line.names() {
                        scope.read(&name);
                        if name.mark != Mark::Include && !name.text.is_empty() {
                            list(
                                &mut listed,
                                name.text,
                                name.mark.role(name.assigned),
                                number,
                            );
                        }
                    }
                    scope.leave(&line);
                    lines.push((line, line.names().count()));
                }
            }
            assert_eq!(lines, whole, "{size}");
            let roles: Vec<_> = split
                .postings
                .iter()
                .map(|(name, roles, _, _)| (name, roles))
                .collect();
            assert_eq!(roles, unit.postings(), "{size}");
            let postings: BTreeMap<_, _> = split
                .postings
                .iter()
                .map(|(name, roles, blocks, _)| (name, (roles, blocks.to_vec())))
                .collect();
            assert_eq!(postings, listed, "{size}");
            // Checking the blocks again gives the same postings.
            let blocks: Vec<_> = split
                .blocks
                .iter()
                .map(|(block, start)| (block, *start))
                .collect();
            let checked = AnalysisUnit::check_split(&blocks, size).expect("a split");
            assert!(checked.iter().eq(split.postings.iter()), "{size}");
        }
        assert!(inside > 0, "no block starts inside a body");
        for size in [0, 30, records + 1] {
            let split = unit.split(size);
            let joined = AnalysisUnit::join(split.blocks.iter().map(|(block, _)| block));
            assert_eq!(joined, Ok(unit.clone()), "{size}");
        }
        let split = unit.split(30);
        let backwards = AnalysisUnit::join([&split.blocks[1].0, &split.blocks[0].0]);
        assert_eq!(backwards, Err(UnitError::LineOrder));
        assert_eq!(unit.split(0).blocks.len(), whole.len());
        assert_eq!(unit.split(records + 1).blocks.len(), 1);
        let empty = AnalysisUnit::from_records(&[]).expect("no records");
        let split = empty.split(16);
        assert_eq!(split.blocks.len(), 1);
        assert_eq!(split.blocks[0].0.lines().count(), 0);
        assert_eq!(split.blocks[0].1, Scope::default());
        assert!(split.postings.iter().next().is_none());
        let blocks = [(&split.blocks[0].0, Scope::default())];
        assert!(AnalysisUnit::check_split(&blocks, 16).is_some());
    }

    /// Blocks are found not to be a unit's split when they would be split otherwise:
    /// with another size, in another order, with another start scope, or with an empty
    /// block. Blocks left out can still be a split of fewer lines: the store finds those
    /// by their numbers and the postings.
    #[test]
    fn blocks_split_otherwise_are_found() {
        let source = b"#define WRAP(x) \\\n\tcall(x)\nint f(void)\n{\n\tg();\n\th();\n}\nint k;\n";
        let unit = AnalysisUnit::from_records(&scanner::scan(source, Mode::C)).expect("records");
        let split = unit.split(12);
        let blocks: Vec<_> = split
            .blocks
            .iter()
            .map(|(block, start)| (block, *start))
            .collect();
        assert!(blocks.len() > 3);
        assert!(AnalysisUnit::check_split(&blocks, 12).is_some());
        // Another size splits them otherwise, unless it happens to cut them alike.
        let mut otherwise = 0;
        for size in [1, 6, 11, 13, 20, 40, 1000] {
            let alike = unit.split(size).blocks == split.blocks;
            otherwise += usize::from(!alike);
            let checked = AnalysisUnit::check_split(&blocks, size);
            assert_eq!(checked.is_some(), alike, "{size}");
        }
        assert!(otherwise >= 2, "{otherwise}");
        let mut swapped = blocks.clone();
        swapped.swap(1, 2);
        assert!(AnalysisUnit::check_split(&swapped, 12).is_none());
        for index in 0..blocks.len() {
            let mut scoped = blocks.clone();
            scoped[index].1.function = Some(b"other");
            assert!(AnalysisUnit::check_split(&scoped, 12).is_none(), "{index}");
        }
        let empty = AnalysisUnit::from_records(&[]).expect("no records");
        let mut padded = blocks.clone();
        padded.push((&empty, Scope::default()));
        assert!(AnalysisUnit::check_split(&padded, 12).is_none());
        let scope = Scope {
            macro_name: Some(&b"M"[..]),
            function: None,
        };
        assert!(AnalysisUnit::check_split(&[(&empty, scope)], 12).is_none());
        assert!(AnalysisUnit::check_split(&[], 12).is_none());
    }

    /// Lists `name` with `roles` in `block`, the latest block so far.
    fn list<'u>(
        listed: &mut BTreeMap<&'u [u8], (u32, Vec<u32>)>,
        name: &'u [u8],
        roles: u32,
        block: u32,
    ) {
        let (held, blocks) = listed.entry(name).or_default();
        *held |= roles;
        if blocks.last() != Some(&block) {
            blocks.push(block);
        }
    }

    /// The bytes a line takes in `block`'s image, its number written as a delta.
    fn line_bytes(block: &AnalysisUnit, line: super::Line<'_>) -> usize {
        let previous = block
            .lines()
            .take_while(|other| other.number != line.number || other.text != line.text)
            .last()
            .map_or(0, |other| other.number);
        let mut bytes = Vec::new();
        super::write_number(&mut bytes, u64::from(line.number - previous));
        bytes.push(u8::from(line.define_end_after));
        super::write_bytes(&mut bytes, line.text);
        super::write_bytes(&mut bytes, line.names);
        bytes.len()
    }

    #[test]
    fn units_round_trip_every_probe() {
        let probes =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/scanner/probes");
        for entry in std::fs::read_dir(probes).expect("probes") {
            let path = entry.expect("probe").path();
            let bytes = std::fs::read(&path).expect("probe bytes");
            let name = path
                .file_name()
                .expect("name")
                .to_string_lossy()
                .into_owned();
            let records = scanner::scan(&bytes, Mode::for_name(name.as_bytes()));
            let unit = AnalysisUnit::from_records(&records).expect("records form a unit");
            let reread = AnalysisUnit::from_image(unit.image().into()).expect("image reads back");
            assert_eq!(reread, unit, "{name}");
            let lines: Vec<_> = reread.lines().collect();
            assert_eq!(lines.len(), records.len(), "{name}");
            for (line, record) in lines.iter().zip(&records) {
                assert_eq!(line.number, record.line);
                assert_eq!(line.text, &record.text[..]);
                assert_eq!(line.define_end_after, record.define_end_after);
                let names: Vec<_> = line.names().collect();
                assert_eq!(
                    names.len(),
                    record.symbols.len(),
                    "{name} line {}",
                    record.line
                );
                for (name, symbol) in names.iter().zip(&record.symbols) {
                    assert_eq!(name.mark, symbol.mark);
                    assert_eq!(name.at as usize, symbol.at);
                    assert_eq!(name.text, &record.text[symbol.at..symbol.at + symbol.len]);
                    assert_eq!(name.assigned, symbol.assigned);
                }
            }
        }
    }

    #[test]
    fn damaged_and_foreign_images_are_refused() {
        let records = scanner::scan(
            b"int a;
int f(void) { return b(a); }
",
            Mode::C,
        );
        let unit = AnalysisUnit::from_records(&records).expect("unit");
        let image = unit.image().to_vec();
        let mut newer = image.clone();
        newer[4] = IMAGE_VERSION + 1;
        assert_eq!(
            AnalysisUnit::from_image(newer.into()),
            Err(UnitError::Version(IMAGE_VERSION + 1))
        );
        for cut in 0..image.len() {
            let result = AnalysisUnit::from_image(image[..cut].to_vec().into());
            assert!(result.is_err(), "a truncated image at {cut} was accepted");
        }
        let mut extended = image.clone();
        extended.push(0);
        assert!(AnalysisUnit::from_image(extended.into()).is_err());
        for flip in 5..image.len() {
            let mut damaged = image.clone();
            damaged[flip] ^= 0xff;
            // Damage is either refused or read without panicking.
            if let Ok(unit) = AnalysisUnit::from_image(damaged.into()) {
                for line in unit.lines() {
                    line.names().for_each(drop);
                }
            }
        }
    }

    #[test]
    fn mark_bytes_round_trip() {
        for mark in [
            Mark::Plain,
            Mark::Define,
            Mark::DefineEnd,
            Mark::Function,
            Mark::FunctionEnd,
            Mark::Call,
            Mark::Include,
            Mark::Global,
            Mark::Member,
            Mark::Struct,
            Mark::Union,
            Mark::Enum,
            Mark::Typedef,
            Mark::Class,
        ] {
            assert_eq!(Mark::from_byte(mark.byte()), Some(mark));
        }
    }
}
