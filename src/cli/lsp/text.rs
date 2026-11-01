//! What the Language server reads from text and URIs: `file:` URIs and paths, positions
//! in the encoding agreed with the editor, and C names in a line.

use std::path::{Path, PathBuf};

/// How a position's `character` counts within its line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Encoding {
    Utf8,
    /// The protocol's default.
    Utf16,
}

impl Encoding {
    /// The first encoding offered that the server speaks, else UTF-16.
    pub(super) fn chosen<'a>(offered: impl IntoIterator<Item = &'a str>) -> Self {
        offered
            .into_iter()
            .find_map(|name| match name {
                "utf-8" => Some(Self::Utf8),
                "utf-16" => Some(Self::Utf16),
                _ => None,
            })
            .unwrap_or(Self::Utf16)
    }

    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Utf8 => "utf-8",
            Self::Utf16 => "utf-16",
        }
    }

    /// The byte offset in LINE of CHARACTER, clamped to the line and to a character
    /// boundary.
    pub(super) fn byte(self, line: &str, character: u32) -> usize {
        let character = character as usize;
        match self {
            Self::Utf8 => {
                let mut byte = character.min(line.len());
                while !line.is_char_boundary(byte) {
                    byte -= 1;
                }
                byte
            }
            Self::Utf16 => {
                let mut units = 0;
                for (byte, c) in line.char_indices() {
                    if units + c.len_utf16() > character {
                        return byte;
                    }
                    units += c.len_utf16();
                }
                line.len()
            }
        }
    }

    /// The character of byte offset BYTE in LINE.
    pub(super) fn character(self, line: &str, byte: usize) -> u32 {
        let count = match self {
            Self::Utf8 => byte,
            Self::Utf16 => line[..byte].encode_utf16().count(),
        };
        u32::try_from(count).unwrap_or(u32::MAX)
    }
}

fn identifier_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// The C name at byte offset BYTE in LINE, or just before it, as when the cursor is
/// at a name's end.
pub(super) fn name_at(line: &str, byte: usize) -> Option<&str> {
    let bytes = line.as_bytes();
    let inside = |at: usize| bytes.get(at).copied().is_some_and(identifier_byte);
    let at = if inside(byte) {
        byte
    } else if byte > 0 && inside(byte - 1) {
        byte - 1
    } else {
        return None;
    };
    let start = (0..=at).rev().take_while(|&i| inside(i)).last()?;
    let end = (at..bytes.len()).take_while(|&i| inside(i)).last()? + 1;
    let name = &line[start..end];
    (!name.as_bytes()[0].is_ascii_digit()).then_some(name)
}

/// The byte range of the first whole-name NAME in LINE.
pub(super) fn find_name(line: &str, name: &str) -> Option<(usize, usize)> {
    let bytes = line.as_bytes();
    line.match_indices(name)
        .map(|(start, _)| start)
        .find_map(|start| {
            let end = start + name.len();
            let before = start > 0 && identifier_byte(bytes[start - 1]);
            let after = bytes.get(end).copied().is_some_and(identifier_byte);
            (!before && !after).then_some((start, end))
        })
}

/// Whether TEXT is a C name or part of one.
pub(super) fn name_like(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(identifier_byte)
}

/// Line NUMBER (from 0) of TEXT, without its line ending.
pub(super) fn line(text: &str, number: usize) -> Option<&str> {
    text.split('\n')
        .nth(number)
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
}

/// The path a `file:` URI names.
pub(super) fn path_of(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let slash = rest.find('/')?;
    let (authority, path) = rest.split_at(slash);
    let path = path.split(['?', '#']).next().unwrap_or(path);
    let bytes = decode(path)?;
    let local = authority.is_empty() || authority.eq_ignore_ascii_case("localhost");
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        local.then(|| PathBuf::from(std::ffi::OsString::from_vec(bytes)))
    }
    #[cfg(windows)]
    {
        let text = String::from_utf8(bytes).ok()?;
        if !local {
            return Some(PathBuf::from(format!(
                "\\\\{authority}{}",
                text.replace('/', "\\")
            )));
        }
        // `/C:/dir` names `C:\dir`.
        let drive = text
            .strip_prefix('/')
            .filter(|rest| rest.as_bytes().get(1) == Some(&b':'))
            .unwrap_or(&text);
        Some(PathBuf::from(drive.replace('/', "\\")))
    }
}

/// The `file:` URI of PATH.
pub(super) fn uri_of(path: &Path) -> String {
    #[cfg(unix)]
    let (authority, bytes) = {
        use std::os::unix::ffi::OsStrExt;
        (String::new(), path.as_os_str().as_bytes().to_vec())
    };
    #[cfg(windows)]
    let (authority, bytes) = {
        let text = crate::project::text(path)
            .unwrap_or_else(|_| path.to_string_lossy().replace('\\', "/"));
        match text.strip_prefix("//") {
            Some(unc) => {
                let (host, rest) = unc.split_at(unc.find('/').unwrap_or(unc.len()));
                (host.to_owned(), rest.as_bytes().to_vec())
            }
            None => (String::new(), format!("/{text}").into_bytes()),
        }
    };
    let mut uri = format!("file://{authority}");
    for byte in bytes {
        if byte.is_ascii_alphanumeric() || b"/-._~".contains(&byte) {
            uri.push(char::from(byte));
        } else {
            uri.push_str(&format!("%{byte:02X}"));
        }
    }
    uri
}

fn decode(text: &str) -> Option<Vec<u8>> {
    let bytes = text.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'%' {
            let hex = std::str::from_utf8(bytes.get(at + 1..at + 3)?).ok()?;
            decoded.push(u8::from_str_radix(hex, 16).ok()?);
            at += 3;
        } else {
            decoded.push(bytes[at]);
            at += 1;
        }
    }
    Some(decoded)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positions_count_in_the_agreed_encoding() {
        let line = "/* é😀 */ greet";
        let byte = line.find("greet").unwrap();
        assert_eq!(Encoding::Utf8.character(line, byte), 13);
        assert_eq!(Encoding::Utf16.character(line, byte), 10);
        assert_eq!(Encoding::Utf8.byte(line, 13), byte);
        assert_eq!(Encoding::Utf16.byte(line, 10), byte);
        // Inside a character, or past the line's end.
        assert_eq!(Encoding::Utf8.byte(line, 4), 3);
        assert_eq!(Encoding::Utf16.byte(line, 5), 5);
        assert_eq!(Encoding::Utf16.byte(line, 99), line.len());
    }

    #[test]
    fn the_name_under_the_cursor_is_a_whole_c_name() {
        let line = "\tx = greet_all(9, b);";
        assert_eq!(name_at(line, 6), Some("greet_all"));
        assert_eq!(name_at(line, 14), Some("greet_all"));
        assert_eq!(name_at(line, 2), Some("x"));
        assert_eq!(name_at(line, 3), None);
        assert_eq!(name_at(line, 15), None);
        assert_eq!(name_at(line, 16), None);
        assert_eq!(name_at(line, 99), None);
    }

    #[test]
    fn a_name_is_found_whole() {
        assert_eq!(find_name("greeting(greet)", "greet"), Some((9, 14)));
        assert_eq!(find_name("greeting", "greet"), None);
        assert_eq!(line("a\r\nb\n", 0), Some("a"));
        assert_eq!(line("a\r\nb\n", 1), Some("b"));
        assert_eq!(line("a\r\nb", 2), None);
    }

    #[test]
    fn uris_and_paths_round_trip() {
        let path = if cfg!(windows) {
            PathBuf::from(r"C:\work\café dir\a#b.c")
        } else {
            PathBuf::from("/work/café dir/a#b.c")
        };
        let uri = uri_of(&path);
        assert!(uri.starts_with("file:///"), "{uri}");
        assert!(uri.ends_with("/caf%C3%A9%20dir/a%23b.c"), "{uri}");
        assert_eq!(path_of(&uri), Some(path));
        assert_eq!(path_of("http://example.com/a.c"), None);
        assert_eq!(path_of("file:///a%2"), None);
    }
}
