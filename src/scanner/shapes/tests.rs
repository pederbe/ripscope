//! The look-ahead layer against the windowed layer it replaced (kept here as the test
//! oracle), on random text where the old window never matters, and its cost on text
//! built to be expensive.

use super::super::input::Input;
use super::{LookAhead, is_word};

/// The previous implementation: every shape explored by closure within a 64 KiB window.
mod windowed {
    use super::super::super::input::Input;
    use super::super::{is_blank, is_word, is_word_start};
    use std::collections::BTreeSet;

    /// How far ahead of the read position a shape may look.
    const WINDOW: usize = 1 << 16;

    /// Bytes of words and numbers: letters, digits, `_`, `$` and `.`.
    fn is_word_or_number(byte: u8) -> bool {
        is_word(byte) || byte == b'.'
    }

    /// Offsets just past one blank, line end or comment starting at `at`.
    fn gap_item(input: &Input, at: usize, ends: &mut Vec<usize>) {
        match input.at(at) {
            Some(byte) if is_blank(byte) || byte == b'\n' => ends.push(at + 1),
            Some(b'/') => match input.at(at + 1) {
                Some(b'*') => block_comment_ends(input, at, ends),
                Some(b'/') => {
                    // A line comment reaches through its line end, and needs one.
                    let mut probe = at + 2;
                    while probe < WINDOW {
                        match input.at(probe) {
                            Some(b'\n') => {
                                ends.push(probe + 1);
                                break;
                            }
                            Some(_) => probe += 1,
                            None => break,
                        }
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }

    /// Every place a block comment starting at `at` can end, as the reference's comment
    /// shape reads it. A `*/` preceded by exactly one star must end the comment; exactly
    /// two stars can never end it; three or more stars can end it or be read through.
    fn block_comment_ends(input: &Input, at: usize, ends: &mut Vec<usize>) {
        let mut probe = at + 2;
        let mut stars = 0usize;
        while probe < WINDOW {
            match input.at(probe) {
                None => return,
                Some(b'*') => stars += 1,
                Some(b'/') if stars > 0 => {
                    if stars == 1 {
                        ends.push(probe + 1);
                        return;
                    }
                    if stars >= 3 {
                        ends.push(probe + 1);
                    }
                    stars = 0;
                }
                Some(_) => stars = 0,
            }
            probe += 1;
        }
    }

    /// Every offset reachable from `starts` by zero or more steps of `step`.
    fn closure(
        input: &Input,
        starts: impl IntoIterator<Item = usize>,
        step: impl Fn(&Input, usize, &mut Vec<usize>),
    ) -> BTreeSet<usize> {
        let mut seen = BTreeSet::new();
        let mut pending: Vec<usize> = starts.into_iter().filter(|at| *at < WINDOW).collect();
        let mut next = Vec::new();
        while let Some(at) = pending.pop() {
            if !seen.insert(at) {
                continue;
            }
            next.clear();
            step(input, at, &mut next);
            pending.extend(
                next.iter()
                    .copied()
                    .filter(|end| *end < WINDOW && !seen.contains(end)),
            );
        }
        seen
    }

    fn gaps(input: &Input, starts: impl IntoIterator<Item = usize>) -> BTreeSet<usize> {
        closure(input, starts, gap_item)
    }

    /// Offsets reachable by one or more gap items.
    fn gaps_plus(input: &Input, at: usize) -> BTreeSet<usize> {
        let mut first = Vec::new();
        gap_item(input, at, &mut first);
        gaps(input, first)
    }

    /// The end of the word starting at `at`.
    fn word_end(input: &Input, at: usize) -> Option<usize> {
        input
            .at(at)
            .is_some_and(is_word_start)
            .then(|| at + input.run(at, is_word))
    }

    /// After a name: a parameter list and the start of a body (a function definition).
    pub(super) fn definition(input: &Input) -> Option<usize> {
        let open = input.run(0, is_blank);
        if input.at(open) != Some(b'(') {
            return None;
        }
        let is_item = |byte: u8| is_word_or_number(byte) || b"*&[]=,:".contains(&byte);
        let mut closed = BTreeSet::new();
        closure(input, [open + 1], |input, at, ends| {
            match input.at(at) {
                Some(byte) if is_item(byte) => ends.push(at + 1),
                Some(b'(') => ends.extend(function_pointer_parameter(input, at)),
                _ => {}
            }
            gap_item(input, at, ends);
        })
        .into_iter()
        // A closing parenthesis ends the parameter list.
        .filter(|at| input.at(*at) == Some(b')'))
        .for_each(|at| {
            closed.insert(at + 1);
        });
        let tail = closure(input, closed, |input, at, ends| {
            if matches!(input.at(at), Some(b'(' | b')')) {
                ends.push(at + 1);
            }
            gap_item(input, at, ends);
        });
        tail.into_iter()
            .filter(|at| {
                input
                    .at(*at)
                    .is_some_and(|byte| byte.is_ascii_alphabetic() || b":_#{".contains(&byte))
            })
            .map(|at| at + 1)
            .max()
    }

    /// The one nested form the reference's definition shape accepts inside a parameter list
    /// (its Debian rule, reading-c.md §5): `(`, `*` and a name, each separated only by blanks,
    /// `)`, blanks, then `(` and any text without parentheses up to `)`. Line ends may appear
    /// only in that last part [22-nested-parameters, 23-nested-parameters-blanks].
    fn function_pointer_parameter(input: &Input, open: usize) -> Option<usize> {
        let mut at = open + 1;
        at += input.run(at, is_blank);
        (input.at(at) == Some(b'*')).then_some(())?;
        at += 1;
        at += input.run(at, is_blank);
        at = word_end(input, at)?;
        at += input.run(at, is_blank);
        (input.at(at) == Some(b')')).then_some(())?;
        at += 1;
        at += input.run(at, is_blank);
        (input.at(at) == Some(b'(')).then_some(())?;
        at += 1;
        let inside = input.run(at, |byte| byte != b'(' && byte != b')');
        at += inside;
        (at < WINDOW && input.at(at) == Some(b')')).then_some(at + 1)
    }

    /// After a name: `(` and argument-like text (a call).
    pub(super) fn call(input: &Input) -> Option<usize> {
        let open = input.run(0, is_blank);
        if input.at(open) != Some(b'(') {
            return None;
        }
        let reach = closure(input, [open + 1], |input, at, ends| {
            if input
                .at(at)
                .is_some_and(|byte| is_word_or_number(byte) || b"*&[]=,".contains(&byte))
            {
                ends.push(at + 1);
            }
            gap_item(input, at, ends);
        });
        reach.into_iter().max()
    }

    /// After a name: stars or gaps, then another name (a declarator after a type word).
    pub(super) fn declarator(input: &Input) -> Option<usize> {
        let step = |input: &Input, at: usize, ends: &mut Vec<usize>| {
            if input.at(at) == Some(b'*') {
                ends.push(at + 1);
            }
            gap_item(input, at, ends);
        };
        let mut first = Vec::new();
        step(input, 0, &mut first);
        closure(input, first, step)
            .into_iter()
            .filter_map(|at| word_end(input, at))
            .max()
    }

    /// After `enum`, `struct` or `union`: a tag name and `{` (a tag definition).
    pub(super) fn tag_definition(input: &Input) -> Option<usize> {
        gaps_plus(input, 0)
            .into_iter()
            .filter_map(|at| word_end(input, at))
            .flat_map(|end| gaps(input, [end]))
            .filter(|at| input.at(*at) == Some(b'{'))
            .map(|at| at + 1)
            .max()
    }

    /// After `enum`, `struct` or `union`: `{` without a tag.
    pub(super) fn tagless_body(input: &Input) -> Option<usize> {
        gaps(input, [0])
            .into_iter()
            .filter(|at| input.at(*at) == Some(b'{'))
            .map(|at| at + 1)
            .max()
    }

    /// After `enum`, `struct` or `union`: an optional tag name and gaps (a use).
    pub(super) fn tag_use(input: &Input) -> Option<usize> {
        let named = gaps_plus(input, 0)
            .into_iter()
            .filter_map(|at| word_end(input, at));
        gaps(input, std::iter::once(0).chain(named))
            .into_iter()
            .max()
            .filter(|end| *end > 0)
    }

    /// `class`, a name, and anything name-like up to `{` (a class definition).
    pub(super) fn class_definition(input: &Input) -> Option<usize> {
        if !input.starts_with(b"class") {
            return None;
        }
        let named: Vec<usize> = gaps_plus(input, 5)
            .into_iter()
            .filter_map(|at| word_end(input, at))
            .collect();
        closure(input, named, |input, at, ends| {
            match input.at(at) {
                Some(b'(' | b')' | b':') => ends.push(at + 1),
                Some(byte) if is_word_start(byte) => ends.push(at + input.run(at, is_word)),
                _ => {}
            }
            gap_item(input, at, ends);
        })
        .into_iter()
        .filter(|at| input.at(*at) == Some(b'{'))
        .map(|at| at + 1)
        .max()
    }

    /// `if`, gaps and `(`.
    pub(super) fn if_parenthesis(input: &Input) -> Option<usize> {
        if !input.starts_with(b"if") {
            return None;
        }
        gaps(input, [2])
            .into_iter()
            .filter(|at| input.at(*at) == Some(b'('))
            .map(|at| at + 1)
            .max()
    }

    /// After an `#endif` line: its line end, gaps, and another `#if`, `#ifdef` or `#ifndef`
    /// followed by blanks (a pseudo-elif).
    pub(super) fn continued_conditional(input: &Input) -> Option<usize> {
        if input.at(0) != Some(b'\n') {
            return None;
        }
        gaps(input, [1])
            .into_iter()
            .filter(|at| input.at(*at) == Some(b'#'))
            .filter_map(|hash| {
                let word = hash + 1 + input.run(hash + 1, is_blank);
                [&b"ifndef"[..], b"ifdef", b"if"]
                    .iter()
                    .filter(|keyword| {
                        keyword
                            .iter()
                            .enumerate()
                            .all(|(offset, byte)| input.at(word + offset) == Some(*byte))
                    })
                    .filter_map(|keyword| {
                        let after = word + keyword.len();
                        let blanks = input.run(after, is_blank);
                        (blanks > 0).then_some(after + blanks)
                    })
                    .max()
            })
            .max()
    }

    /// After an `#endif` line: its line end and gaps.
    pub(super) fn endif_gaps(input: &Input) -> Option<usize> {
        if input.at(0) != Some(b'\n') {
            return None;
        }
        gaps(input, [1]).into_iter().max()
    }
}

/// A small deterministic generator (xorshift), so failures reproduce.
struct Random(u64);

impl Random {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, bound: usize) -> usize {
        (self.next() % bound as u64) as usize
    }
}

const PIECES: &[&str] = &[
    "a", "b", "_x", "if", "class", "struct", "enum", " ", " ", "	", "
", "(", ")", "(", ")", "*",
    "/", "/*", "*/", "**/", "***/", "/**/", "/***/", "//", "{", "}", "#", "#if ", "#ifdef ", "[",
    "]", ",", "=", ":", "&", ".", "1", ";", "\"",
];

fn random_text(random: &mut Random) -> Vec<u8> {
    let pieces = 1 + random.below(40);
    (0..pieces)
        .flat_map(|_| PIECES[random.below(PIECES.len())].bytes())
        .collect()
}

#[test]
fn shapes_agree_with_the_windowed_layer_on_random_text() {
    let mut random = Random(0x5eed_1234_abcd_ef01);
    for _ in 0..20_000 {
        let text = random_text(&mut random);
        let mut look = LookAhead::new(&text);
        for start in 0..=text.len() {
            for pushed in [&b""[..], b" "] {
                let mut input = Input::new(&text);
                input.advance(start);
                input.push_back(pushed);
                let cases = [
                    (
                        "definition",
                        look.definition(&input),
                        windowed::definition(&input),
                    ),
                    ("call", look.call(&input), windowed::call(&input)),
                    (
                        "declarator",
                        look.declarator(&input),
                        windowed::declarator(&input),
                    ),
                    (
                        "tag_definition",
                        look.tag_definition(&input),
                        windowed::tag_definition(&input),
                    ),
                    (
                        "tagless_body",
                        look.tagless_body(&input),
                        windowed::tagless_body(&input),
                    ),
                    ("tag_use", look.tag_use(&input), windowed::tag_use(&input)),
                    (
                        "class_definition",
                        look.class_definition(&input),
                        windowed::class_definition(&input),
                    ),
                    (
                        "if_parenthesis",
                        look.if_parenthesis(&input),
                        windowed::if_parenthesis(&input),
                    ),
                    (
                        "continued_conditional",
                        look.continued_conditional(&input),
                        windowed::continued_conditional(&input),
                    ),
                    (
                        "endif_gaps",
                        look.endif_gaps(&input),
                        windowed::endif_gaps(&input),
                    ),
                ];
                for (shape, new, old) in cases {
                    assert_eq!(
                        new,
                        old,
                        "{shape} at {start} (pushed {pushed:?}) in {:?}",
                        String::from_utf8_lossy(&text)
                    );
                }
            }
        }
    }
}

/// Runs every shape after every name, as the scanner would, and returns the walk steps.
fn cost(text: &[u8]) -> u64 {
    let mut look = LookAhead::new(text);
    for at in 1..=text.len() {
        if !is_word(text[at - 1]) || text.get(at).is_some_and(|byte| is_word(*byte)) {
            continue;
        }
        let mut input = Input::new(text);
        input.advance(at);
        look.definition(&input);
        look.call(&input);
        look.declarator(&input);
        look.tag_definition(&input);
        look.tagless_body(&input);
        look.tag_use(&input);
        look.class_definition(&input);
        look.if_parenthesis(&input);
        look.continued_conditional(&input);
        look.endif_gaps(&input);
    }
    look.steps
}

#[test]
fn shapes_cost_linear_time_on_text_built_to_be_expensive() {
    type Maker = fn(usize) -> String;
    let cases: [(&str, Maker); 6] = [
        ("comments ending in four stars", |n| {
            "x /****/
"
            .repeat(n)
        }),
        ("doc comments ending in two stars", |n| {
            "A, /** doc **/
"
            .repeat(n)
        }),
        ("conditionals between star comments", |n| {
            "#if A
#endif
/****/
/****/
"
            .repeat(n)
        }),
        ("class heads in a row", |n| "class a ".repeat(n)),
        ("long argument lists", |n| format!("f({})", "a, ".repeat(n))),
        ("names before one long comment", |n| {
            format!("{}/*{}*/", "x ".repeat(n), " ".repeat(n))
        }),
    ];
    for (name, make) in cases {
        let small = make(2_000);
        let large = make(8_000);
        let (small_cost, large_cost) = (cost(small.as_bytes()), cost(large.as_bytes()));
        // Four times the text may cost at most about four times the work.
        assert!(
            large_cost <= 5 * small_cost.max(1),
            "{name}: {small_cost} steps for {} bytes, {large_cost} for {}",
            small.len(),
            large.len()
        );
        assert!(
            large_cost <= 64 * large.len() as u64,
            "{name}: {large_cost} steps for {} bytes",
            large.len()
        );
    }
}
