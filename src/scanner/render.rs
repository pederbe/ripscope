//! Displayed text for one record (reading-c.md §7).
//!
//! The scanner collects a line's text as the reference collects it: blank runs already
//! mostly collapsed, keyword spellings marked with their display codes. Rendering then
//! produces what the reference's database displays:
//! - blank and tab runs become one blank, and trailing blanks go;
//! - respaced keywords take their standard spacing;
//! - symbols keep their exact bytes.
//!
//! One subtlety comes from the database's pair packing. While names are still to come on
//! the line, a byte from a small set followed by a blank is stored as one pair, so a
//! blank run after it is not merged into that blank and a second blank survives.
//!
//! Rendering also follows the stored bytes one unit at a time, because field 9's test for
//! an assignment looks at the units after each name, not at the displayed text.

use super::keywords::{self, Follow};
use super::line::Named;
use super::{Record, Symbol};
use crate::unit::Mark;

/// Bytes the database packs as the first half of a pair.
const PAIR_FIRST: &[u8] = b" teisaprnl(of)=c";
/// Bytes the database packs as the second half of a pair.
const PAIR_SECOND: &[u8] = b" tnerpla";

/// One stored unit of a line's text, as field 9's test sees it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Unit {
    /// A byte the test skips as whitespace: blanks, raw line ends, carriage returns, and
    /// the codes of `else`, `enum` and `extern`, which share those byte values.
    Space,
    /// A packed pair; the test sees only its first byte, and only in some positions.
    Pair(u8),
    Byte(u8),
}

fn unit_of(byte: u8) -> Unit {
    if matches!(byte, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r') {
        Unit::Space
    } else {
        Unit::Byte(byte)
    }
}

pub(super) fn render(line: u32, text: &[u8], codes: &[(usize, u8)], names: &[Named]) -> Record {
    // The last code written at a position wins.
    let mut code_of = vec![None; text.len() + 1];
    for &(at, code) in codes {
        if let Some(slot) = code_of.get_mut(at) {
            *slot = Some(code);
        }
    }
    let code_at = |at: usize| code_of.get(at).copied().flatten();
    let mut out = Vec::with_capacity(text.len());
    let mut units = Vec::with_capacity(text.len());
    let mut symbols = Vec::with_capacity(names.len());
    // Where each written name's following units start.
    let mut after_name = Vec::with_capacity(names.len());
    let mut blank = false;
    let mut next = 0;
    let mut at = 0;
    while at < text.len() {
        let code = code_at(at);
        let byte = text[at];
        if code.is_none() && (byte == b' ' || byte == b'\t') {
            blank = true;
            at += 1;
            continue;
        }
        if let Some(named) = names
            .get(next)
            .filter(|named| named.start == at && named.end <= text.len())
        {
            if blank {
                out.push(b' ');
                units.push(Unit::Space);
                blank = false;
            }
            symbols.push(Symbol {
                mark: named.mark,
                at: out.len(),
                len: named.end - named.start,
                assigned: named.source_assigned,
            });
            // A keyword code written over the line's first byte (by `#define` or `#include`
            // later on the line) lands inside a name that starts there, as in the
            // reference's database.
            out.extend((named.start..named.end).map(|at| code_at(at).unwrap_or(text[at])));
            // The name ends its database line; the test skips that line end.
            units.push(Unit::Space);
            after_name.push(units.len());
            // Keyword-spelled macros are symbols with their exact name, while
            // their surrounding displayed text retains keyword respacing.
            if let Some(keyword) = (named.mark == Mark::Define)
                .then(|| keywords::keyword(&text[named.start..named.end]))
                .flatten()
                .filter(|keyword| keyword.code.is_some())
            {
                keyword_suffix(&mut out, keyword.follow);
                at = skip_after_keyword(keyword.follow, text, named.start, |at| {
                    code_at(at).is_some()
                });
            } else {
                at = named.end;
            }
            next += 1;
            continue;
        }
        if blank {
            out.push(b' ');
            blank = false;
            if code.is_none() && PAIR_SECOND.contains(&byte) {
                // The blank and this byte are stored as one pair.
                out.push(byte);
                units.push(Unit::Pair(b' '));
                at += 1;
                continue;
            }
            units.push(Unit::Space);
        } else if code.is_none()
            && next < names.len()
            && names[next].start != at + 1
            && PAIR_FIRST.contains(&byte)
            && text
                .get(at + 1)
                .is_some_and(|second| PAIR_SECOND.contains(second))
            && code_at(at + 1).is_none()
        {
            out.push(byte);
            out.push(text[at + 1]);
            units.push(Unit::Pair(byte));
            at += 2;
            continue;
        }
        if let Some(keyword) = code.and_then(keywords::by_code) {
            out.extend_from_slice(keyword.word);
            keyword_suffix(&mut out, keyword.follow);
            units.push(unit_of(keyword.code.unwrap_or(0)));
            at = skip_after_keyword(keyword.follow, text, at, |at| code_at(at).is_some());
        } else if byte < 0x20 && byte != b'\t' {
            // The reference's database cannot tell a raw control byte from a keyword
            // code, so it skips the same text after one. The byte stays raw (it displays
            // as that keyword); a raw line end is not displayed at all.
            if byte != b'\n' {
                out.push(byte);
            }
            units.push(unit_of(byte));
            let follow = keywords::by_code(byte).map_or(Follow::Nothing, |keyword| keyword.follow);
            at = skip_after_keyword(follow, text, at, |at| code_at(at).is_some());
        } else {
            out.push(byte);
            units.push(Unit::Byte(byte));
            at += 1;
        }
    }
    for (index, symbol) in symbols.iter_mut().enumerate() {
        let from = after_name[index];
        let to = after_name
            .get(index + 1)
            .map_or(units.len(), |next_from| next_from - 1);
        symbol.assigned |= assignment_follows(&units[from..to.max(from)]);
    }
    let define_end_after = names
        .get(next)
        .is_some_and(|named| named.mark == Mark::DefineEnd);
    Record {
        line,
        text: out,
        symbols,
        define_end_after,
    }
}

fn keyword_suffix(out: &mut Vec<u8>, follow: Follow) {
    match follow {
        Follow::Nothing => {}
        Follow::Blank => out.push(b' '),
        Follow::Parenthesis => out.extend_from_slice(b" ("),
    }
}

/// Field 9's test on the units after a name (reading-c.md §7): after whitespace, `=` not
/// followed by `=`, an operator followed by `=`, or `<<=` / `>>=`. A pair counts where the
/// test reads its first byte as `=`. Units beyond the name's fragment are never part of an
/// assignment, since the next database line starts with a mark, a name or a line number.
fn assignment_follows(units: &[Unit]) -> bool {
    let start = units.iter().position(|unit| *unit != Unit::Space);
    let Some(start) = start else { return false };
    let unit = |offset: usize| units.get(start + offset).copied();
    let equals_pair = |unit: Option<Unit>| unit == Some(Unit::Pair(b'='));
    match unit(0) {
        Some(Unit::Pair(b'=')) => true,
        Some(Unit::Byte(b'=')) => match unit(1) {
            Some(Unit::Byte(b'=')) => false,
            Some(Unit::Pair(first)) => first != b'=',
            _ => true,
        },
        Some(Unit::Byte(operator)) if b"+-*/%&|^".contains(&operator) => {
            unit(1) == Some(Unit::Byte(b'=')) || equals_pair(unit(1))
        }
        Some(Unit::Byte(shift)) if shift == b'<' || shift == b'>' => {
            unit(1) == Some(Unit::Byte(shift))
                && (unit(2) == Some(Unit::Byte(b'=')) || equals_pair(unit(2)))
        }
        _ => false,
    }
}

/// Where the text continues after a keyword code at `at`: past blanks and letters and,
/// for keywords followed by a blank or `(`, past the source's own spacing. A keyword that
/// already holds its own code is not letters to skip [31-hash-lines].
fn skip_after_keyword(
    follow: Follow,
    text: &[u8],
    at: usize,
    coded: impl Fn(usize) -> bool,
) -> usize {
    let is_blank = |byte: &u8| *byte == b' ' || *byte == b'\t';
    let mut next = at + 1;
    while text.get(next).is_some_and(is_blank) {
        next += 1;
    }
    while text
        .get(next)
        .is_some_and(|byte| byte.is_ascii_alphabetic())
        && !coded(next)
    {
        next += 1;
    }
    if follow != Follow::Nothing {
        while text.get(next).is_some_and(is_blank) {
            next += 1;
        }
    }
    if follow == Follow::Parenthesis && text.get(next) == Some(&b'(') {
        next += 1;
    }
    next
}

/// Bytes as the reference displays them: its database stores high bytes as pairs and
/// control bytes as keyword codes, and displays them that way. Ripscope keeps the raw
/// bytes, so comparisons view ripscope's bytes the reference's way.
#[cfg(test)]
pub(crate) fn reference_view(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    for &byte in bytes {
        if byte > 0x7f {
            let code = usize::from(byte & 0x7f);
            out.push(PAIR_FIRST[code / 8]);
            out.push(PAIR_SECOND[code & 7]);
        } else if byte < 0x20 && byte != b'\n' {
            match keywords::by_code(byte) {
                Some(keyword) => {
                    out.extend_from_slice(keyword.word);
                    match keyword.follow {
                        Follow::Nothing => {}
                        Follow::Blank => out.push(b' '),
                        Follow::Parenthesis => out.extend_from_slice(b" ("),
                    }
                }
                None => out.push(byte),
            }
        } else {
            out.push(byte);
        }
    }
    out
}
