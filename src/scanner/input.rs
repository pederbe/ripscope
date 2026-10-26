//! The scanner's input: the file's bytes plus bytes the scanner has pushed back.

use std::{borrow::Cow, ops::Range};

use super::assignments::{logical_byte, quoted};

/// Original bounded look-behind, with only a clipped, completed lexical span hidden.
pub(super) struct Before<'s> {
    pub(super) bytes: Cow<'s, [u8]>,
    pub(super) clipped: bool,
}

/// A forward lexical cursor shared by successive assignment windows. No source or
/// token list is retained; the cursor only advances, keeping its total work linear.
#[derive(Default)]
struct AssignmentBoundary {
    at: usize,
    opaque: Range<usize>,
}

impl AssignmentBoundary {
    fn clipped_end(&mut self, source: &[u8], from: usize) -> usize {
        while self.at < from {
            let start = self.at;
            let Some((byte, next)) = logical_byte(source, start) else {
                self.at = source.len();
                break;
            };
            self.at = next;
            let complete = match (byte, logical_byte(source, next)) {
                (b'/', Some((b'*', end))) => {
                    self.at = end;
                    let mut closed = false;
                    while let Some((byte, next)) = logical_byte(source, self.at) {
                        self.at = next;
                        if byte == b'*'
                            && let Some((b'/', end)) = logical_byte(source, next)
                        {
                            self.at = end;
                            closed = true;
                            break;
                        }
                    }
                    closed
                }
                (b'/', Some((b'/', end))) => {
                    self.at = end;
                    while let Some((byte, next)) = logical_byte(source, self.at) {
                        self.at = next;
                        if byte == b'\n' {
                            break;
                        }
                    }
                    true
                }
                (b'\'' | b'"', _) => {
                    self.at = next - 1;
                    quoted(source, &mut self.at)
                }
                _ => false,
            };
            if complete {
                self.opaque = start..self.at;
            }
        }
        if self.opaque.start < from && from < self.opaque.end {
            self.opaque.end
        } else {
            from
        }
    }
}

pub(super) struct Input<'s> {
    source: &'s [u8],
    position: usize,
    /// Pushed-back bytes; the last element is read next.
    pushed: Vec<u8>,
    assignment_boundary: AssignmentBoundary,
}

impl<'s> Input<'s> {
    pub(super) fn new(source: &'s [u8]) -> Self {
        Self {
            source,
            position: 0,
            pushed: Vec::new(),
            assignment_boundary: AssignmentBoundary::default(),
        }
    }

    /// The length of the file's bytes.
    pub(super) fn source_len(&self) -> usize {
        self.source.len()
    }

    /// Original bytes around a name just consumed. Pushed-back input has no reliable
    /// source span. Each side is independently bounded; no compacted trivia is used.
    pub(super) fn around_name(&self, length: usize, bound: usize) -> Option<(&[u8], &[u8])> {
        let end = self.source_position(0)?;
        let start = end.checked_sub(length)?;
        Some((
            &self.source[start.saturating_sub(bound)..start],
            &self.source[end..end.saturating_add(bound).min(self.source.len())],
        ))
    }

    /// Assignment windows keep the original byte budget and unknown left boundary.
    /// Mask only a completed comment/literal that the left edge cuts, leaving all
    /// nearby target syntax intact. A name inside that span has no assignment proof.
    pub(super) fn around_assignment(
        &mut self,
        length: usize,
        bound: usize,
    ) -> Option<(Before<'_>, &[u8])> {
        let end = self.source_position(0)?;
        let start = end.checked_sub(length)?;
        let from = start.saturating_sub(bound);
        let clipped_end = self.assignment_boundary.clipped_end(self.source, from);
        if clipped_end > start {
            return None;
        }
        let mut bytes = Cow::Borrowed(&self.source[from..start]);
        let clipped = clipped_end > from;
        if clipped {
            bytes.to_mut()[..clipped_end - from].fill(b' ');
        }
        Some((
            Before { bytes, clipped },
            &self.source[end..end.saturating_add(bound).min(self.source.len())],
        ))
    }

    /// The byte `offset` places ahead of the read position.
    pub(super) fn at(&self, offset: usize) -> Option<u8> {
        let pushed = self.pushed.len();
        if offset < pushed {
            Some(self.pushed[pushed - 1 - offset])
        } else {
            self.source.get(self.position + offset - pushed).copied()
        }
    }

    pub(super) fn starts_with(&self, word: &[u8]) -> bool {
        word.iter()
            .enumerate()
            .all(|(offset, byte)| self.at(offset) == Some(*byte))
    }

    /// The `length` bytes ahead of the read position.
    pub(super) fn peek(&self, length: usize) -> Cow<'s, [u8]> {
        self.peek_at(0, length)
    }

    /// The `length` bytes starting `offset` places ahead, borrowed from the source
    /// unless pushed-back bytes are among them. Ends early at the end of input.
    pub(super) fn peek_at(&self, offset: usize, length: usize) -> Cow<'s, [u8]> {
        match self.source_position(offset) {
            Some(start) => {
                let start = start.min(self.source.len());
                let end = start.saturating_add(length).min(self.source.len());
                Cow::Borrowed(&self.source[start..end])
            }
            None => Cow::Owned(
                (offset..offset + length)
                    .filter_map(|offset| self.at(offset))
                    .collect(),
            ),
        }
    }

    pub(super) fn advance(&mut self, mut length: usize) {
        while length > 0 && !self.pushed.is_empty() {
            self.pushed.pop();
            length -= 1;
        }
        self.position = (self.position + length).min(self.source.len());
    }

    /// Makes `bytes` the next bytes read, in order.
    pub(super) fn push_back(&mut self, bytes: &[u8]) {
        self.pushed.extend(bytes.iter().rev());
    }

    /// The source position `offset` places ahead, unless that is a pushed-back byte.
    pub(super) fn source_position(&self, offset: usize) -> Option<usize> {
        let pushed = self.pushed.len();
        (offset >= pushed).then(|| self.position + offset - pushed)
    }

    /// How far ahead a source position at or after the read position lies.
    pub(super) fn offset_of(&self, position: usize) -> usize {
        position - self.position + self.pushed.len()
    }

    /// How many bytes ahead satisfy `accept`, contiguously.
    pub(super) fn run(&self, from: usize, accept: impl Fn(u8) -> bool) -> usize {
        self.run_within(from, usize::MAX, accept)
    }

    /// How many bytes ahead, before offset `limit`, satisfy `accept`, contiguously.
    pub(super) fn run_within(
        &self,
        from: usize,
        limit: usize,
        accept: impl Fn(u8) -> bool,
    ) -> usize {
        let mut length = 0;
        while from + length < limit && self.at(from + length).is_some_and(&accept) {
            length += 1;
        }
        length
    }
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use super::Input;

    #[test]
    fn peeks_borrow_source_spans() {
        let mut input = Input::new(b"abcdef");
        input.advance(1);
        assert!(matches!(input.peek(3), Cow::Borrowed(b"bcd")));
        assert!(matches!(input.peek_at(2, 9), Cow::Borrowed(b"def")));
        assert!(matches!(input.peek_at(7, 2), Cow::Borrowed(b"")));
    }

    #[test]
    fn peeks_read_pushed_back_bytes_first() {
        let mut input = Input::new(b"abcdef");
        input.advance(4);
        input.push_back(b"xy");
        assert_eq!(&input.peek(3)[..], b"xye");
        assert_eq!(&input.peek(9)[..], b"xyef");
        assert_eq!(&input.peek_at(1, 2)[..], b"ye");
        assert!(matches!(input.peek_at(2, 2), Cow::Borrowed(b"ef")));
    }
}
