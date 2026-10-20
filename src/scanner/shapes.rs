//! The look-ahead layer (reading-c.md §5, §6, §9).
//!
//! Where the reference decides a role from the text ahead, several candidate shapes
//! compete and the longest match wins. Each shape function returns how far its shape
//! reaches from the read position, or `None`.
//!
//! A shape is read as a walk through phases. Within a phase the text allows at most one
//! step at each position, except at a block comment, which can end at several places
//! (a `*/` after three or more stars may end it or be read through). There the walk
//! takes the best value over all of the comment's ends from per-file tables, built once
//! per phase from the last comment end backwards. So every shape costs time linear in
//! the text it walks, and a file costs linear time overall, with no window: shapes reach
//! as far as the reference's do.

use super::input::Input;
use std::collections::HashMap;

pub(super) fn is_blank(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\r' | 0x0b | 0x0c)
}

pub(super) fn is_word_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_' || byte == b'$'
}

pub(super) fn is_word(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'$'
}

/// Bytes of words and numbers: letters, digits, `_`, `$` and `.`.
fn is_word_or_number(byte: u8) -> bool {
    is_word(byte) || byte == b'.'
}

/// A stretch of a shape. Each allows gaps (blanks, line ends and comments) and some
/// phase-specific steps, and scores positions it reaches.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// Gaps; every position reached scores.
    Any,
    /// Gaps, then `{`.
    Brace,
    /// Gaps, then `(`.
    Paren,
    /// Gaps, then `#`, blanks, `if`, `ifdef` or `ifndef`, and blanks.
    Elif,
    /// Gaps, then a tag name and [`Phase::Brace`].
    TagName,
    /// Gaps, then a tag name and [`Phase::Any`].
    UseName,
    /// Stars and gaps, then a name.
    Declarator,
    /// Argument-like text and gaps; every position reached scores.
    Arguments,
    /// Parameter-like text and gaps, then `)` and [`Phase::Tail`].
    Parameters,
    /// Parentheses and gaps, then a byte that can start a body or a declaration.
    Tail,
    /// Gaps, then a class name and [`Phase::ClassBody`].
    ClassName,
    /// Names, parentheses, colons and gaps, then `{`.
    ClassBody,
}

const PHASES: usize = 12;

impl Phase {
    fn index(self) -> usize {
        self as usize
    }
}

/// Where a `*/` sits, and how many stars run into it.
struct CommentEnd {
    /// The `/`.
    slash: usize,
    stars: usize,
}

/// A phase's table: for each comment end, in file order, the best score reachable from
/// just after it, and the best over the ends a comment reaching it may still take.
#[derive(Default)]
struct Table {
    state: TableState,
    after: Vec<Option<usize>>,
    onward: Vec<Option<usize>>,
}

#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum TableState {
    #[default]
    Missing,
    Building,
    Built,
}

/// The per-file state of the look-ahead layer.
pub(super) struct LookAhead<'s> {
    source: &'s [u8],
    ends: Option<Vec<CommentEnd>>,
    tables: [Table; PHASES],
    /// Scores of [`Phase::ClassBody`] walks by source position: a class head's walk
    /// passes names, so heads in a row would otherwise walk the same text again.
    class_bodies: HashMap<usize, Option<usize>>,
    /// Walk steps taken, for the cost tests.
    #[cfg(test)]
    steps: u64,
}

/// Text a walk reads: the input ahead of the read position, or the whole source.
trait Text {
    fn byte(&self, at: usize) -> Option<u8>;
    /// The source position of `at`, if it is not a pushed-back byte.
    fn position(&self, at: usize) -> Option<usize>;
    /// Where a source position lies in this text.
    fn offset_at(&self, position: usize) -> usize;

    fn run(&self, from: usize, accept: impl Fn(u8) -> bool) -> usize {
        let mut length = 0;
        while self.byte(from + length).is_some_and(&accept) {
            length += 1;
        }
        length
    }

    fn word_end(&self, at: usize) -> Option<usize> {
        self.byte(at)
            .is_some_and(is_word_start)
            .then(|| at + self.run(at, is_word))
    }
}

impl Text for Input<'_> {
    fn byte(&self, at: usize) -> Option<u8> {
        self.at(at)
    }

    fn position(&self, at: usize) -> Option<usize> {
        self.source_position(at)
    }

    fn offset_at(&self, position: usize) -> usize {
        self.offset_of(position)
    }
}

struct Source<'s>(&'s [u8]);

impl Text for Source<'_> {
    fn byte(&self, at: usize) -> Option<u8> {
        self.0.get(at).copied()
    }

    fn position(&self, at: usize) -> Option<usize> {
        Some(at)
    }

    fn offset_at(&self, position: usize) -> usize {
        position
    }
}

fn better(best: Option<usize>, candidate: Option<usize>) -> Option<usize> {
    best.max(candidate)
}

impl<'s> LookAhead<'s> {
    pub(super) fn new(source: &'s [u8]) -> Self {
        Self {
            source,
            ends: None,
            tables: Default::default(),
            class_bodies: HashMap::new(),
            #[cfg(test)]
            steps: 0,
        }
    }

    // Shapes, from the read position.

    /// After a name: a parameter list and the start of a body (a function definition).
    pub(super) fn definition(&mut self, input: &Input) -> Option<usize> {
        let open = input.run(0, is_blank);
        (input.at(open) == Some(b'(')).then_some(())?;
        self.walk(Phase::Parameters, input, open + 1)
    }

    /// After a name: `(` and argument-like text (a call).
    pub(super) fn call(&mut self, input: &Input) -> Option<usize> {
        let open = input.run(0, is_blank);
        (input.at(open) == Some(b'(')).then_some(())?;
        self.walk(Phase::Arguments, input, open + 1)
    }

    /// After a name: stars or gaps, then another name (a declarator after a type word).
    pub(super) fn declarator(&mut self, input: &Input) -> Option<usize> {
        if input.at(0) == Some(b'*') {
            return self.walk(Phase::Declarator, input, 1);
        }
        self.after_gap(Phase::Declarator, input, 0)
    }

    /// After `enum`, `struct` or `union`: a tag name and `{` (a tag definition).
    pub(super) fn tag_definition(&mut self, input: &Input) -> Option<usize> {
        self.after_gap(Phase::TagName, input, 0)
    }

    /// After `enum`, `struct` or `union`: `{` without a tag.
    pub(super) fn tagless_body(&mut self, input: &Input) -> Option<usize> {
        self.walk(Phase::Brace, input, 0)
    }

    /// After `enum`, `struct` or `union`: an optional tag name and gaps (a use).
    pub(super) fn tag_use(&mut self, input: &Input) -> Option<usize> {
        let plain = self.walk(Phase::Any, input, 0);
        let named = self.after_gap(Phase::UseName, input, 0);
        better(plain, named).filter(|end| *end > 0)
    }

    /// `class`, a name, and anything name-like up to `{` (a class definition).
    pub(super) fn class_definition(&mut self, input: &Input) -> Option<usize> {
        if !input.starts_with(b"class") {
            return None;
        }
        self.after_gap(Phase::ClassName, input, 5)
    }

    /// `if`, gaps and `(`.
    pub(super) fn if_parenthesis(&mut self, input: &Input) -> Option<usize> {
        if !input.starts_with(b"if") {
            return None;
        }
        self.walk(Phase::Paren, input, 2)
    }

    /// After an `#endif` line: its line end, gaps, and another `#if`, `#ifdef` or
    /// `#ifndef` followed by blanks (a pseudo-elif).
    pub(super) fn continued_conditional(&mut self, input: &Input) -> Option<usize> {
        (input.at(0) == Some(b'\n')).then_some(())?;
        self.walk(Phase::Elif, input, 1)
    }

    /// After an `#endif` line: its line end and gaps.
    pub(super) fn endif_gaps(&mut self, input: &Input) -> Option<usize> {
        (input.at(0) == Some(b'\n')).then_some(())?;
        self.walk(Phase::Any, input, 1)
    }

    // Walks.

    /// The best score of `phase` after at least one gap item from `at`.
    fn after_gap<T: Text>(&mut self, phase: Phase, text: &T, at: usize) -> Option<usize> {
        match text.byte(at)? {
            byte if is_blank(byte) || byte == b'\n' => self.walk(phase, text, at + 1),
            b'/' if text.byte(at + 1) == Some(b'/') => {
                let end = line_comment_end(text, at)?;
                self.walk(phase, text, end)
            }
            b'/' if text.byte(at + 1) == Some(b'*') => {
                let position = text.position(at)?;
                self.comment(phase, position)
                    .map(|reach| text.offset_at(reach))
            }
            _ => None,
        }
    }

    /// The best score of `phase` from `at`.
    fn walk<T: Text>(&mut self, phase: Phase, text: &T, mut at: usize) -> Option<usize> {
        let mut best = None;
        let mut passed = Vec::new();
        let result = loop {
            #[cfg(test)]
            {
                self.steps += 1;
            }
            if phase == Phase::ClassBody
                && let Some(position) = text.position(at)
            {
                if let Some(known) = self.class_bodies.get(&position) {
                    break better(best, known.map(|reach| text.offset_at(reach)));
                }
                passed.push(position);
            }
            best = better(best, self.score(phase, text, at));
            let Some(byte) = text.byte(at) else {
                break best;
            };
            if phase == Phase::Parameters && byte == b')' {
                break better(best, self.walk(Phase::Tail, text, at + 1));
            }
            let next = match byte {
                b'/' if text.byte(at + 1) == Some(b'*') => {
                    // Every place the comment can end is covered by the tables.
                    let reach = text
                        .position(at)
                        .and_then(|position| self.comment(phase, position))
                        .map(|reach| text.offset_at(reach));
                    break better(best, reach);
                }
                b'/' if text.byte(at + 1) == Some(b'/') => line_comment_end(text, at),
                _ if is_blank(byte) || byte == b'\n' => Some(at + 1),
                _ => step(phase, text, at, byte),
            };
            match next {
                Some(next) => at = next,
                None => break best,
            }
        };
        if phase == Phase::ClassBody {
            let known = result.and_then(|reach| text.position(reach));
            for position in passed {
                self.class_bodies.insert(position, known);
            }
        }
        result
    }

    /// What reaching `at` scores in `phase`.
    fn score<T: Text>(&mut self, phase: Phase, text: &T, at: usize) -> Option<usize> {
        let byte = text.byte(at);
        match phase {
            Phase::Any | Phase::Arguments => Some(at),
            Phase::Brace | Phase::ClassBody => (byte == Some(b'{')).then_some(at + 1),
            Phase::Paren => (byte == Some(b'(')).then_some(at + 1),
            Phase::Elif => (byte == Some(b'#'))
                .then(|| conditional(text, at))
                .flatten(),
            Phase::Tail => byte
                .is_some_and(|byte| byte.is_ascii_alphabetic() || b"{#:_".contains(&byte))
                .then_some(at + 1),
            Phase::Declarator => text.word_end(at),
            Phase::TagName => {
                let end = text.word_end(at)?;
                self.walk(Phase::Brace, text, end)
            }
            Phase::UseName => {
                let end = text.word_end(at)?;
                self.walk(Phase::Any, text, end)
            }
            Phase::ClassName => {
                let end = text.word_end(at)?;
                self.walk(Phase::ClassBody, text, end)
            }
            Phase::Parameters => None,
        }
    }

    /// The best score of `phase` over every place the block comment opening at source
    /// `position` can end. A `*/` whose stars number exactly one must end the comment,
    /// exactly two never end it, and three or more may end it or be read through. The
    /// comment's own opening star does not count.
    fn comment(&mut self, phase: Phase, position: usize) -> Option<usize> {
        self.build(phase);
        let source = self.source;
        let ends = self.ends.as_ref().expect("comment ends are indexed");
        let first = position + 2;
        let stars = source[first.min(source.len())..]
            .iter()
            .take_while(|byte| **byte == b'*')
            .count();
        let slash = first + stars;
        let mut best = None;
        let from = if source.get(slash) == Some(&b'/') {
            let own = ends.partition_point(|end| end.slash < slash);
            match stars {
                1 => return self.tables[phase.index()].after[own],
                0 | 2 => {}
                _ => best = self.tables[phase.index()].after[own],
            }
            slash + 1
        } else {
            slash
        };
        let next = ends.partition_point(|end| end.slash < from);
        if let Some(onward) = self.tables[phase.index()].onward.get(next) {
            best = better(best, *onward);
        }
        best
    }

    /// Builds `phase`'s table, from the last comment end backwards, so a walk from one
    /// end finds every later comment's value already in place.
    fn build(&mut self, phase: Phase) {
        let index = phase.index();
        if self.tables[index].state != TableState::Missing {
            return;
        }
        let ends = self.ends.get_or_insert_with(|| comment_ends(self.source));
        let count = ends.len();
        let slashes: Vec<(usize, usize)> = ends.iter().map(|end| (end.slash, end.stars)).collect();
        self.tables[index] = Table {
            state: TableState::Building,
            after: vec![None; count],
            onward: vec![None; count],
        };
        let source = Source(self.source);
        for (at, &(slash, stars)) in slashes.iter().enumerate().rev() {
            let after = self.walk(phase, &source, slash + 1);
            let table = &mut self.tables[index];
            table.after[at] = after;
            let ends_here = stars != 2;
            let own = if ends_here { after } else { None };
            // A comment reading through this end may still take a later one, up to the
            // first end it must take.
            table.onward[at] = if stars == 1 {
                own
            } else {
                better(own, table.onward.get(at + 1).copied().flatten())
            };
        }
        self.tables[index].state = TableState::Built;
    }
}

/// The step `phase` takes from `at`, other than a gap.
fn step<T: Text>(phase: Phase, text: &T, at: usize, byte: u8) -> Option<usize> {
    let single = match phase {
        Phase::Declarator => byte == b'*',
        Phase::Arguments => is_word_or_number(byte) || b",=[]*&".contains(&byte),
        Phase::Parameters => {
            if byte == b'(' {
                return function_pointer_parameter(text, at);
            }
            is_word_or_number(byte) || b",=[]*&:".contains(&byte)
        }
        Phase::Tail => byte == b'(' || byte == b')',
        Phase::ClassBody => {
            if is_word_start(byte) {
                return text.word_end(at);
            }
            b"():".contains(&byte)
        }
        _ => false,
    };
    single.then_some(at + 1)
}

/// Just past the line end that closes a line comment starting at `at`; a line comment
/// needs one.
fn line_comment_end<T: Text>(text: &T, at: usize) -> Option<usize> {
    let rest = text.run(at + 2, |byte| byte != b'\n');
    text.byte(at + 2 + rest).map(|_| at + 3 + rest)
}

/// `#` at `at`, blanks, `if`, `ifdef` or `ifndef`, and at least one blank: its end.
fn conditional<T: Text>(text: &T, at: usize) -> Option<usize> {
    let word = at + 1 + text.run(at + 1, is_blank);
    [&b"ifndef"[..], b"ifdef", b"if"]
        .iter()
        .filter(|keyword| {
            keyword
                .iter()
                .enumerate()
                .all(|(offset, byte)| text.byte(word + offset) == Some(*byte))
        })
        .filter_map(|keyword| {
            let after = word + keyword.len();
            let blanks = text.run(after, is_blank);
            (blanks > 0).then_some(after + blanks)
        })
        .max()
}

/// The one nested form the reference's definition shape accepts inside a parameter list
/// (its Debian rule, reading-c.md §5): `(`, `*` and a name, each separated only by blanks,
/// `)`, blanks, then `(` and any text without parentheses up to `)`. Line ends may appear
/// only in that last part [22-nested-parameters, 23-nested-parameters-blanks].
fn function_pointer_parameter<T: Text>(text: &T, open: usize) -> Option<usize> {
    let mut at = open + 1;
    at += text.run(at, is_blank);
    (text.byte(at) == Some(b'*')).then_some(())?;
    at += 1;
    at += text.run(at, is_blank);
    at = text.word_end(at)?;
    at += text.run(at, is_blank);
    (text.byte(at) == Some(b')')).then_some(())?;
    at += 1;
    at += text.run(at, is_blank);
    (text.byte(at) == Some(b'(')).then_some(())?;
    at += 1;
    at += text.run(at, |byte| byte != b'(' && byte != b')');
    (text.byte(at) == Some(b')')).then_some(at + 1)
}

/// Every `*/` in the source, in order, with the stars that run into it.
fn comment_ends(source: &[u8]) -> Vec<CommentEnd> {
    let mut ends = Vec::new();
    let mut stars = 0;
    for (at, byte) in source.iter().enumerate() {
        match byte {
            b'*' => stars += 1,
            b'/' if stars > 0 => {
                ends.push(CommentEnd { slash: at, stars });
                stars = 0;
            }
            _ => stars = 0,
        }
    }
    ends
}

#[cfg(test)]
mod tests;
