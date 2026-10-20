//! Written element stores, independent of declaration look-ahead and its work budget.
//! Each occurrence examines at most 256 original bytes on either side of its name.

pub(super) const WINDOW: usize = 256;

use super::input::Before;

/// A simple written declaration prefix, for local arrays whose names have no
/// definition mark. No expression keyword or expression punctuation proves a type.
pub(super) fn declaration_prefix(before: &[u8]) -> bool {
    let from = before
        .iter()
        .rposition(|byte| matches!(byte, b';' | b'{' | b'}' | b'#'))
        .map_or(0, |at| at + 1);
    let bytes = &before[from..];
    let mut at = 0;
    let mut words = 0;
    while gap(bytes, &mut at) && at < bytes.len() {
        if bytes[at] == b'*' && words > 0 {
            at += 1;
            continue;
        }
        let start = at;
        if !super::shapes::is_word_start(bytes[at]) {
            return false;
        }
        at += 1;
        while bytes
            .get(at)
            .is_some_and(|byte| super::shapes::is_word(*byte))
        {
            at += 1;
        }
        let word = &bytes[start..at];
        if super::keywords::keyword(word).is_some()
            && !super::keywords::keyword_type(word)
            && !matches!(
                word,
                b"const" | b"volatile" | b"static" | b"extern" | b"register" | b"restrict"
            )
        {
            return false;
        }
        words += 1;
    }
    words > 0
}

pub(super) fn subscript_store(before: &Before<'_>, after: &[u8], declarator: bool) -> bool {
    let mut at = 0;
    if !gap(after, &mut at) || after.get(at) != Some(&b'[') {
        return false;
    }
    let declarator = declarator || declaration_prefix(&before.bytes);
    let Some(prefix) = trim_gap(&before.bytes) else {
        return false;
    };
    if before.clipped && prefix.is_empty() {
        return false;
    }
    if ends_token(prefix, b".")
        || ends_token(prefix, b"->")
        || (!declarator && (ends_token(prefix, b"*") || ends_token(prefix, b"&")))
    {
        return false;
    }
    while after.get(at) == Some(&b'[') {
        at += 1;
        if !gap(after, &mut at) || (after.get(at) == Some(&b']') && !declarator) {
            return false;
        }
        let mut closes = vec![b']'];
        while let Some(close) = closes.last().copied() {
            if !gap(after, &mut at) {
                return false;
            }
            match after.get(at).copied() {
                Some(b'[') => closes.push(b']'),
                Some(b'(') => closes.push(b')'),
                Some(b']' | b')') if after[at] == close => {
                    closes.pop();
                }
                Some(b']' | b')' | b';' | b'{' | b'}' | b'#') | None => return false,
                Some(b'\'' | b'"') => {
                    if !quoted(after, &mut at) {
                        return false;
                    }
                    continue;
                }
                _ => {}
            }
            at += 1;
        }
        if !gap(after, &mut at) {
            return false;
        }
    }
    operator(&after[at..], after.len() < WINDOW)
}

/// Preserve an independently written scalar assignment inside a subscript even if
/// its name appeared earlier on the line. Ordinary scalar deduplication stays intact.
pub(super) fn scalar_store(before: &Before<'_>, after: &[u8]) -> bool {
    let mut at = 0;
    if !gap(after, &mut at) || !operator(&after[at..], after.len() < WINDOW) {
        return false;
    }
    trim_gap(&before.bytes).is_some_and(|prefix| {
        !(before.clipped && prefix.is_empty())
            && !ends_token(prefix, b".")
            && !ends_token(prefix, b"->")
            && !ends_token(prefix, b"*")
            && !ends_token(prefix, b"&")
    })
}

fn operator(bytes: &[u8], source_ends: bool) -> bool {
    // At the window edge a lone '=' could be the first half of '=='. It is only
    // proven when its following byte fits too, or the original source ends here.
    let Some((first, at)) = logical_byte(bytes, 0) else {
        return false;
    };
    let next = logical_byte(bytes, at);
    match first {
        b'=' => next.is_some_and(|(byte, _)| byte != b'=') || (next.is_none() && source_ends),
        b'+' | b'-' | b'*' | b'/' | b'%' | b'&' | b'|' | b'^' => {
            next.is_some_and(|(byte, _)| byte == b'=')
        }
        b'<' | b'>' => next.is_some_and(|(byte, at)| {
            byte == first && logical_byte(bytes, at).is_some_and(|(byte, _)| byte == b'=')
        }),
        _ => false,
    }
}

/// Phase-two line splices join punctuation; whitespace and comments never do.
pub(super) fn logical_byte(bytes: &[u8], mut at: usize) -> Option<(u8, usize)> {
    loop {
        let tail = bytes.get(at..)?;
        if tail.starts_with(b"\\\r\n") {
            at += 3;
        } else if tail.starts_with(b"\\\n") {
            at += 2;
        } else {
            return bytes.get(at).copied().map(|byte| (byte, at + 1));
        }
    }
}

fn ends_token(mut bytes: &[u8], token: &[u8]) -> bool {
    for expected in token.iter().rev() {
        loop {
            if bytes.ends_with(b"\\\r\n") {
                bytes = &bytes[..bytes.len() - 3];
            } else if bytes.ends_with(b"\\\n") {
                bytes = &bytes[..bytes.len() - 2];
            } else {
                break;
            }
        }
        if bytes.last() != Some(expected) {
            return false;
        }
        bytes = &bytes[..bytes.len() - 1];
    }
    true
}

pub(super) fn quoted(bytes: &[u8], at: &mut usize) -> bool {
    let quote = bytes[*at];
    *at += 1;
    while let Some(byte) = bytes.get(*at) {
        *at += 1;
        if *byte == quote {
            return true;
        }
        if *byte == b'\\' {
            if bytes.get(*at).is_none() {
                return false;
            }
            *at += if bytes[*at..].starts_with(b"\r\n") {
                2
            } else {
                1
            };
        } else if *byte == b'\n' || *byte == b'\r' {
            return false;
        }
    }
    false
}

pub(super) fn gap(bytes: &[u8], at: &mut usize) -> bool {
    loop {
        match bytes.get(*at..) {
            Some(tail) if tail.starts_with(b"\\\r\n") => *at += 3,
            Some(tail) if tail.starts_with(b"\\\n") => *at += 2,
            _ if bytes.get(*at).is_some_and(u8::is_ascii_whitespace) => *at += 1,
            _ => {
                let Some((b'/', second)) = logical_byte(bytes, *at) else {
                    return true;
                };
                match logical_byte(bytes, second) {
                    Some((b'*', mut end)) => loop {
                        let Some((byte, next)) = logical_byte(bytes, end) else {
                            return false;
                        };
                        if byte == b'*'
                            && let Some((b'/', close)) = logical_byte(bytes, next)
                        {
                            *at = close;
                            break;
                        }
                        end = next;
                    },
                    Some((b'/', mut end)) => loop {
                        let Some((byte, next)) = logical_byte(bytes, end) else {
                            return false;
                        };
                        if byte == b'\n' {
                            *at = next;
                            break;
                        }
                        end = next;
                    },
                    _ => return true,
                }
            }
        }
    }
}

fn trim_gap(bytes: &[u8]) -> Option<&[u8]> {
    let mut at = 0;
    let mut end = 0;
    while at < bytes.len() {
        if !gap(bytes, &mut at) {
            return None;
        }
        if at == bytes.len() {
            break;
        }
        if logical_byte(bytes, at).is_some_and(|(byte, next)| {
            byte == b'*' && logical_byte(bytes, next).is_some_and(|(byte, _)| byte == b'/')
        }) {
            return None;
        }
        if matches!(bytes[at], b'\'' | b'"') {
            if !quoted(bytes, &mut at) {
                return None;
            }
        } else {
            at += 1;
        }
        end = at;
    }
    Some(&bytes[..end])
}
