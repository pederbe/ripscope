//! Direct named written mutations, using bounded original source spans (ADR 0012).
//! Their work is independent of function-definition recognition.

pub(super) const WINDOW: usize = 256;

pub(super) fn named(before: &super::input::Before<'_>, after: &[u8]) -> bool {
    // Reading the whole window before every name took a third of an index build. A
    // mutation needs `++` or `--` as the token next to the name or its parentheses,
    // which the bytes beside it rule out for most names.
    (prefix_mutation_possible(&before.bytes) || postfix_mutation_follows(after))
        && named_in_full(before, after)
}

/// Whether the last token before the `(`s that end `before` could be `++` or `--`:
/// `false` only when the bytes rule it out. Only spacing, comments, line splices and
/// `(`s can follow that token, and on the name's own line they end in a space, a tab,
/// a form feed, a `(` or a comment's `/`, so the last other byte there is the token's
/// own. Earlier lines are passed only when they hold no `/`, quote or splice, so no
/// comment or literal can hide a token in them.
fn prefix_mutation_possible(before: &[u8]) -> bool {
    let mut lines = before.split(|byte| matches!(byte, b'\n' | b'\r')).rev();
    let mut own = true;
    for line in lines.by_ref() {
        if !own && (line.ends_with(b"\\") || line.iter().any(|b| matches!(b, b'/' | b'"' | b'\'')))
        {
            return true;
        }
        own = false;
        if let Some(last) = line
            .iter()
            .rev()
            .find(|byte| !matches!(byte, b' ' | b'\t' | b'\x0c' | b'('))
        {
            return matches!(last, b'+' | b'-' | b'/');
        }
    }
    // The window starts inside the token or its spacing.
    true
}

/// Whether `after` starts with `)`s and then `++` or `--`.
fn postfix_mutation_follows(after: &[u8]) -> bool {
    let mut suffix = Tokens::new(after);
    loop {
        match kind(suffix.next().ok().flatten()) {
            Some(Kind::Close) => {}
            Some(Kind::Mutation) => return true,
            _ => return false,
        }
    }
}

fn named_in_full(before: &super::input::Before<'_>, after: &[u8]) -> bool {
    let mut source = Tokens::new(&before.bytes);
    let mut prefix = Vec::new();
    loop {
        match source.next() {
            Ok(Some(token)) => prefix.push(token),
            Ok(None) => break,
            Err(()) => return false,
        }
    }
    let mut start = prefix.len();
    while start > 0 && prefix[start - 1].kind == Kind::Open {
        start -= 1;
    }
    let groups = prefix.len() - start;
    if start > 0
        && prefix[start - 1].kind == Kind::Mutation
        // A truncated leading sign could be the tail of a longest token such
        // as `other+++name`. Do not invent its missing token boundary.
        && !(before.bytes.len() == WINDOW && prefix[start - 1].start == 0)
    {
        let mut suffix = Tokens::new(after);
        if (0..groups).all(|_| {
            suffix
                .next()
                .is_ok_and(|next| kind(next) == Some(Kind::Close))
        }) && suffix.next().is_ok_and(|next| match kind(next) {
            Some(Kind::Member | Kind::Subscript | Kind::Open) => false,
            None => after.len() < WINDOW,
            _ => true,
        }) {
            return true;
        }
    }
    let mut suffix = Tokens::new(after);
    let mut closes = 0;
    loop {
        match suffix.next() {
            Ok(Some(Token {
                kind: Kind::Close, ..
            })) => closes += 1,
            Ok(Some(Token {
                kind: Kind::Mutation,
                ..
            })) => break,
            _ => return false,
        }
    }
    if closes > groups {
        return false;
    }
    if before.bytes.len() == WINDOW
        && (prefix.is_empty() || (closes > 0 && prefix[prefix.len() - closes].start == 0))
    {
        return false;
    }
    if before.clipped && prefix.len() == closes {
        return false;
    }
    let preceding = prefix
        .len()
        .checked_sub(closes + 1)
        .and_then(|at| prefix.get(at));
    !preceding.is_some_and(|token| {
        token.kind == Kind::Member || (closes > 0 && token.kind == Kind::CallWord)
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Mutation,
    Open,
    Close,
    Member,
    Subscript,
    CallWord,
    Other,
}

struct Token {
    kind: Kind,
    start: usize,
}

fn kind(token: Option<Token>) -> Option<Kind> {
    token.map(|token| token.kind)
}

/// Only the operand boundary is recognized, not a C expression. Longest sign
/// tokens distinguish `other++ +name` from a prefix mutation of `name`.
struct Tokens<'s> {
    bytes: &'s [u8],
    at: usize,
}

impl<'s> Tokens<'s> {
    fn new(bytes: &'s [u8]) -> Self {
        Self { bytes, at: 0 }
    }

    fn next(&mut self) -> Result<Option<Token>, ()> {
        use super::assignments::{gap, logical_byte, quoted};
        if !gap(self.bytes, &mut self.at) {
            return Err(());
        }
        let start = self.at;
        let Some((first, end)) = logical_byte(self.bytes, start) else {
            return Ok(None);
        };
        self.at = end;
        let next = logical_byte(self.bytes, end);
        let kind = match (first, next) {
            (b'+' | b'-', Some((second, end))) if first == second => {
                self.at = end;
                Kind::Mutation
            }
            (b'-', Some((b'>', end))) => {
                self.at = end;
                Kind::Member
            }
            (b'*', Some((b'/', _))) => return Err(()),
            (b'.', _) => Kind::Member,
            (b'(', _) => Kind::Open,
            (b')', _) => Kind::Close,
            (b'[', _) => Kind::Subscript,
            (b'\'' | b'"', _) => {
                self.at = start;
                if !quoted(self.bytes, &mut self.at) {
                    return Err(());
                }
                Kind::Other
            }
            (byte, _) if super::shapes::is_word(byte) => {
                while let Some((byte, end)) = logical_byte(self.bytes, self.at) {
                    if !super::shapes::is_word(byte) {
                        break;
                    }
                    self.at = end;
                }
                if matches!(&self.bytes[start..self.at], b"return" | b"else" | b"do") {
                    Kind::Other
                } else {
                    Kind::CallWord
                }
            }
            _ => Kind::Other,
        };
        Ok(Some(Token { kind, start }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scanner::input::Before;
    use std::borrow::Cow;

    #[test]
    fn the_quick_check_agrees_with_reading_in_full() {
        // Windows made of the pieces that decide tokens, comments, literals and splices,
        // and of stray bytes that break them.
        const PIECES: &[&[u8]] = &[
            b"++", b"--", b"+", b"-", b"(", b")", b"[", b"]", b".", b"->", b"=", b" ", b"\t",
            b"\x0b", b"\x0c", b"\n", b"\r\n", b"\\\n", b"\\", b"/* c */", b"/*", b"*/", b"// c\n",
            b"//", b"/", b"*", b"\"s\"", b"\"", b"'c'", b"'", b"x", b"a1", b"return", b";",
        ];
        let mut state = 0x9e37_79b9_7f4a_7c15_u64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let window = |next: &mut dyn FnMut() -> u64| -> Vec<u8> {
            let pieces = (next() % 7) as usize;
            (0..pieces)
                .flat_map(|_| {
                    PIECES[(next() % PIECES.len() as u64) as usize]
                        .iter()
                        .copied()
                })
                .collect()
        };
        let mut mutations = 0;
        for _ in 0..200_000 {
            let before = window(&mut next);
            let after = window(&mut next);
            let before = Before {
                bytes: Cow::Owned(before),
                clipped: next() % 4 == 0,
            };
            let full = named_in_full(&before, &after);
            assert_eq!(
                named(&before, &after),
                full,
                "{:?} name {:?}",
                String::from_utf8_lossy(&before.bytes),
                String::from_utf8_lossy(&after)
            );
            mutations += usize::from(full);
        }
        // The windows include many mutations, not only names next to nothing.
        assert!(mutations > 1_000, "{mutations}");
    }
}
