//! Text splitting and trimming as the existing captures and answer files were made, so
//! a recapture reads them the same way.

/// Lines split at every line boundary: newline, carriage return, vertical tab, form
/// feed, the information separators U+001C to U+001E, and U+0085, U+2028 and U+2029. A
/// final boundary does not start another line.
pub fn lines(text: &str) -> Vec<&str> {
    let mut lines = Vec::new();
    let mut start = 0;
    let mut chars = text.char_indices().peekable();
    while let Some((at, ch)) = chars.next() {
        if matches!(
            ch,
            '\n' | '\r'
                | '\u{b}'
                | '\u{c}'
                | '\u{1c}'
                | '\u{1d}'
                | '\u{1e}'
                | '\u{85}'
                | '\u{2028}'
                | '\u{2029}'
        ) {
            lines.push(&text[start..at]);
            start = at + ch.len_utf8();
            if ch == '\r' && chars.peek().is_some_and(|(_, next)| *next == '\n') {
                chars.next();
                start += 1;
            }
        }
    }
    if start < text.len() {
        lines.push(&text[start..]);
    }
    lines
}

/// Text without surrounding whitespace, which includes U+001C to U+001F.
pub fn strip(text: &str) -> &str {
    text.trim_matches(|ch: char| ch.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&ch))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_and_trimming_keep_the_captured_shapes() {
        assert_eq!(lines("0a\r\nC 1b\u{c}2c\n"), ["0a", "C 1b", "2c"]);
        assert_eq!(lines("0a\r"), ["0a"]);
        assert_eq!(lines("a\n\nb"), ["a", "", "b"]);
        assert!(lines("").is_empty());
        assert_eq!(strip("\u{1f} warning \n"), "warning");
    }
}
