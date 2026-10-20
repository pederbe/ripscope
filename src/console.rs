//! Output to a Windows console. Rust's standard streams refuse to write bytes that are not
//! UTF-8 to a Windows console, so a row holding one (a Latin-1 `é` in a found line or a
//! stored path) would stop the output with an error. There such bytes are written as
//! `\xHH`, as the line protocol writes control bytes; pipes, files and `--json` get every
//! byte.

use std::io::{self, Write};

/// A writer that passes UTF-8 through and writes every other byte as `\xHH`. A sequence
/// cut at the end of one write waits for the next.
pub(crate) struct ConsoleSafe<W: Write> {
    inner: W,
    pending: Vec<u8>,
}

impl<W: Write> ConsoleSafe<W> {
    pub(crate) fn new(inner: W) -> Self {
        Self {
            inner,
            pending: Vec::new(),
        }
    }

    /// Writes what `pending` holds, keeping a sequence cut at its end unless `last`.
    fn drain(&mut self, last: bool) -> io::Result<()> {
        let mut out = Vec::with_capacity(self.pending.len());
        let mut rest: &[u8] = &self.pending;
        loop {
            match std::str::from_utf8(rest) {
                Ok(text) => {
                    out.extend_from_slice(text.as_bytes());
                    rest = &[];
                    break;
                }
                Err(error) => {
                    let (valid, after) = rest.split_at(error.valid_up_to());
                    out.extend_from_slice(valid);
                    let invalid = match error.error_len() {
                        Some(length) => length,
                        // A sequence cut at the end: complete it with the next write.
                        None if !last => {
                            rest = after;
                            break;
                        }
                        None => after.len(),
                    };
                    for byte in &after[..invalid] {
                        write!(out, "\\x{byte:02x}")?;
                    }
                    rest = &after[invalid..];
                }
            }
        }
        let kept = rest.len();
        self.inner.write_all(&out)?;
        let start = self.pending.len() - kept;
        self.pending.drain(..start);
        Ok(())
    }
}

impl<W: Write> Write for ConsoleSafe<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.pending.extend_from_slice(buf);
        self.drain(false)?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.drain(true)?;
        self.inner.flush()
    }
}

impl<W: Write> Drop for ConsoleSafe<W> {
    fn drop(&mut self) {
        let _ = self.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn through(writes: &[&[u8]]) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut console = ConsoleSafe::new(&mut out);
            for bytes in writes {
                console.write_all(bytes).expect("write");
            }
            console.flush().expect("flush");
        }
        out
    }

    #[test]
    fn utf8_passes_and_other_bytes_are_escaped() {
        assert_eq!(through(&["café".as_bytes()]), "café".as_bytes());
        assert_eq!(through(&[b"caf\xe9.c 1\n"]), b"caf\\xe9.c 1\n");
        assert_eq!(through(&[b"\xff\xfe"]), b"\\xff\\xfe");
    }

    #[test]
    fn a_sequence_cut_between_writes_is_kept_whole() {
        let e_acute = "é".as_bytes();
        assert_eq!(
            through(&[b"caf", &e_acute[..1], &e_acute[1..], b"\n"]),
            "café\n".as_bytes()
        );
        // Cut at the very end, it is escaped when the output ends.
        assert_eq!(through(&[b"caf", &e_acute[..1]]), b"caf\\xc3");
    }
}
