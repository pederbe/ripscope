//! The line being collected: its text as the reference collects it, the keyword codes
//! written into that text, and the names found on it (reading-c.md §3, §7).

use super::Record;
use super::render::render;
use crate::hashing::HashSet;
use crate::unit::Mark;

/// A name found on the line, before rendering: `text[start..end]`.
pub(super) struct Named {
    pub mark: Mark,
    pub start: usize,
    pub end: usize,
    pub source_assigned: bool,
    /// How many function definitions came before it on the line; a name repeats only
    /// within the same count.
    level: u32,
}

#[derive(Default)]
pub(super) struct Line {
    text: Vec<u8>,
    /// Keyword codes written into the text, in order; a later code at the same position
    /// replaces an earlier one.
    codes: Vec<(usize, u8)>,
    /// The code written over the text's first byte, by a `#define` or `#include`.
    first_code: Option<u8>,
    names: Vec<Named>,
    /// Names already found away from the first byte, for de-duplication.
    found: HashSet<(Mark, u32, Vec<u8>)>,
    /// The mark, level and end of a name at the first byte, whose bytes a code can
    /// still change.
    first_name: Option<(Mark, u32, usize)>,
    definitions: u32,
    /// Where the next name or marker is placed. Added text moves it to the end. Text
    /// written in place of a marker (a brace or `%%` that closes something, a tag's
    /// opening brace) leaves it behind, and a name leaves it at its own start, so the
    /// next name or marker can start before text already collected, as the reference's
    /// own position does.
    pub anchor: usize,
}

impl Line {
    pub(super) fn text(&self) -> &[u8] {
        &self.text
    }

    pub(super) fn len(&self) -> usize {
        self.text.len()
    }

    /// Adds collected text; the anchor moves to its end.
    pub(super) fn add(&mut self, bytes: &[u8]) {
        self.text.extend_from_slice(bytes);
        self.anchor = self.text.len();
    }

    /// Adds text that stands in for a marker; the anchor stays.
    pub(super) fn put(&mut self, bytes: &[u8]) {
        self.text.extend_from_slice(bytes);
    }

    pub(super) fn write_code(&mut self, at: usize, code: u8) {
        if at == 0 {
            self.first_code = Some(code);
        }
        self.codes.push((at, code));
    }

    /// The line starts as a directive, as the reference checks it: `#` first, not yet
    /// overwritten by a keyword code.
    pub(super) fn is_directive(&self) -> bool {
        self.text.first() == Some(&b'#') && self.first_code.is_none()
    }

    pub(super) fn has_names(&self) -> bool {
        !self.names.is_empty()
    }

    /// An empty-named marker at the anchor.
    pub(super) fn marker(&mut self, mark: Mark) {
        let at = self.anchor;
        self.name(mark, at, at);
    }

    /// Records a name, unless the line already has it with the same mark and level.
    pub(super) fn name(&mut self, mark: Mark, start: usize, end: usize) {
        self.name_with_assignment(mark, start, end, false);
    }

    /// A written assignment keeps its actual occurrence, even after an earlier read
    /// with the same mark. Its position determines the stored query's context.
    pub(super) fn name_with_assignment(
        &mut self,
        mark: Mark,
        start: usize,
        end: usize,
        source_assigned: bool,
    ) {
        if start == end {
            self.names.push(Named {
                mark,
                start,
                end,
                source_assigned,
                level: self.definitions,
            });
            return;
        }
        if mark == Mark::Function {
            self.definitions = self.definitions.saturating_add(1);
        }
        let level = self.definitions;
        let Some(bytes) = self.text.get(start..end) else {
            return;
        };
        // Names compare as the line's bytes are now: a code written over the first byte
        // makes a name there differ from a later occurrence [27-define-after-name].
        let repeats_first = self
            .first_name
            .is_some_and(|(first_mark, first_level, first_end)| {
                first_mark == mark
                    && first_level == level
                    && first_end == bytes.len()
                    && self.first_code.or(self.text.first().copied()) == Some(bytes[0])
                    && self.text.get(1..first_end) == Some(&bytes[1..])
            });
        if repeats_first && !source_assigned {
            return;
        }
        if start == 0 {
            self.first_name = Some((mark, level, end));
        } else if !self.found.insert((mark, level, bytes.to_vec())) && !source_assigned {
            return;
        }
        self.names.push(Named {
            mark,
            start,
            end,
            source_assigned,
            level,
        });
    }

    /// Replaces the text from `from` on, as an include operand is rewritten. Names and
    /// codes from `from` on no longer point at their bytes: the reference shows stale
    /// bytes there, and ripscope drops them. A name that starts before `from` keeps its
    /// place and reads the new bytes, as the reference's does, unless it would reach past
    /// the new text.
    pub(super) fn rewrite_from(&mut self, from: usize, bytes: &[u8]) {
        self.text.truncate(from);
        self.text.extend_from_slice(bytes);
        let length = self.text.len();
        let intact = |named: &Named| named.start < from && named.end <= length;
        if !self.names.iter().all(intact) {
            self.names.retain(intact);
            let text = &self.text;
            self.found = self
                .names
                .iter()
                .filter(|named| named.start > 0 && named.end > named.start)
                .map(|named| {
                    (
                        named.mark,
                        named.level,
                        text[named.start..named.end].to_vec(),
                    )
                })
                .collect();
        }
        self.codes.retain(|(at, _)| *at < from);
    }

    /// Ends the line: a record if it has names, and a fresh line either way.
    pub(super) fn finish(&mut self, number: u32) -> Option<Record> {
        let record = self
            .has_names()
            .then(|| render(number, &self.text, &self.codes, &self.names));
        self.text.clear();
        self.codes.clear();
        self.first_code = None;
        self.names.clear();
        self.found.clear();
        self.first_name = None;
        self.definitions = 0;
        self.anchor = 0;
        record
    }
}
