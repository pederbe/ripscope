//! The result sink: rows written in the line protocol or as JSON.
//!
//! In the line protocol a row is `path context line text`. The path is written raw,
//! spaces and non-ASCII included, as cscope writes it; a row whose path holds a control
//! character is skipped, with one warning per answer. The displayed text is written raw
//! except for control bytes and DEL, which appear as `\xHH` so no row can break the
//! protocol. A line a text search found keeps its tabs, and the carriage return of a
//! CR LF ending, raw, as cscope prints such lines. JSON rows keep every byte: a text
//! that is not UTF-8 is written as an array of byte values.

use crate::query::ResultRow;
use std::borrow::Cow;
use std::io::{self, Read, Seek, SeekFrom, Write};
use tempfile::SpooledTempFile;

/// How rows reach the reader.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Transport {
    /// cscope's line protocol.
    Line,
    /// One JSON array per row: `[path, context, line, text]`.
    Json,
}

/// Spools an answer, with rows at file scope before the other rows. Each stream keeps
/// up to 8 MiB in memory; a walking thread's part keeps up to 1 MiB per stream. Larger
/// answers spill to temporary files, which are removed when the answer is dropped.
pub(crate) struct Sink {
    first: Rows<SpooledTempFile>,
    rest: Rows<SpooledTempFile>,
}

impl Sink {
    pub(crate) fn new(transport: Transport) -> Self {
        Self::with_capacity(transport, 8 << 20)
    }

    pub(crate) fn part(transport: Transport) -> Self {
        Self::with_capacity(transport, 1 << 20)
    }

    pub(crate) fn with_capacity(transport: Transport, bytes: usize) -> Self {
        Self {
            first: Rows::new(SpooledTempFile::new(bytes), transport),
            rest: Rows::new(SpooledTempFile::new(bytes), transport),
        }
    }

    pub(crate) fn transport(&self) -> Transport {
        self.first.transport
    }

    pub(crate) fn row(&mut self, row: ResultRow<'_>) -> io::Result<()> {
        let rows = if row.at_file_scope {
            &mut self.first
        } else {
            &mut self.rest
        };
        rows.row(row.path, row.context, row.line, row.text)
    }

    pub(crate) fn text_row(
        &mut self,
        path: &[u8],
        context: &[u8],
        line: u32,
        text: &[u8],
    ) -> io::Result<()> {
        self.first.text_row(path, context, line, text)
    }

    /// Appends a part in file order, retaining its two groups and skipped-row count.
    pub(crate) fn absorb(&mut self, part: Self) -> io::Result<()> {
        self.first.absorb(part.first)?;
        self.rest.absorb(part.rest)
    }

    /// Finishes both streams before an answer can be delivered. Query and spool-write
    /// errors therefore leave no partial answer on the transport.
    pub(crate) fn finish(self, mut warnings: Vec<String>) -> io::Result<Answer> {
        let skipped = self.first.skipped + self.rest.skipped;
        if skipped == 1 {
            warnings.push("1 row skipped: its path holds control characters; use --json".into());
        } else if skipped > 1 {
            warnings.push(format!(
                "{skipped} rows skipped: their paths hold control characters; use --json"
            ));
        }
        Ok(Answer {
            rows: self.first.written + self.rest.written,
            warnings,
            first: self.first.finish()?,
            rest: self.rest.finish()?,
        })
    }
}

/// How a reader receives warnings about a completed answer. One-shot JSON uses stderr
/// too: only a JSON session carries warning frames after its rows.
#[derive(Clone, Copy)]
pub(crate) enum WarningStyle {
    Stderr,
    Vim,
    JsonSession,
}

/// A completed answer owns its encoded rows, count and warnings, independent of the
/// index and the threads that found them. Transports supply only their result header.
pub(crate) struct Answer {
    rows: usize,
    warnings: Vec<String>,
    first: SpooledTempFile,
    rest: SpooledTempFile,
}

impl Answer {
    pub(crate) fn rows(&self) -> usize {
        self.rows
    }

    pub(crate) fn has_warnings(&self) -> bool {
        !self.warnings.is_empty()
    }

    pub(crate) fn add_warnings(&mut self, warnings: Vec<String>) {
        self.warnings.extend(warnings);
    }

    #[cfg(test)]
    pub(crate) fn warnings(&self) -> &[String] {
        &self.warnings
    }

    /// Places warnings around the framed rows: stderr before a line answer, or warning
    /// frames after a JSON session's rows. Stderr errors stay best effort; an error in
    /// the header, body or JSON frames fails delivery. The caller owns prompts and Vim's
    /// acknowledgement handshake.
    pub(crate) fn deliver<O: Write>(
        &mut self,
        out: &mut O,
        errors: &mut impl Write,
        warnings: WarningStyle,
        header: impl FnOnce(&mut O, usize) -> io::Result<()>,
    ) -> io::Result<()> {
        if !matches!(warnings, WarningStyle::JsonSession) {
            for warning in &self.warnings {
                let text = one_line(warning);
                let text = if matches!(warnings, WarningStyle::Vim) {
                    Cow::Owned(text.replace('>', "?"))
                } else {
                    text
                };
                let _ = writeln!(errors, "ripscope: {text}");
            }
            let _ = errors.flush();
        }
        header(out, self.rows())?;
        for spool in [&mut self.first, &mut self.rest] {
            spool.seek(SeekFrom::Start(0))?;
            io::copy(spool, out)?;
        }
        if matches!(warnings, WarningStyle::JsonSession) {
            for warning in &self.warnings {
                let frame = serde_json::json!({"type": "warning", "message": warning});
                writeln!(out, "{frame}")?;
            }
        }
        Ok(())
    }
}

/// A diagnostic on one line, including paths containing control characters.
pub(crate) fn one_line(message: &str) -> Cow<'_, str> {
    if !message.chars().any(char::is_control) {
        return Cow::Borrowed(message);
    }
    let mut line = String::with_capacity(message.len() + 8);
    for c in message.chars() {
        if c.is_control() {
            line.push_str(&format!("\\x{:02x}", u32::from(c)));
        } else {
            line.push(c);
        }
    }
    Cow::Owned(line)
}

/// Encodes one stream of rows and counts those it had to skip.
struct Rows<W> {
    out: W,
    transport: Transport,
    written: usize,
    skipped: usize,
    buffer: Vec<u8>,
}

impl<W: Write> Rows<W> {
    fn new(out: W, transport: Transport) -> Self {
        Self {
            out,
            transport,
            written: 0,
            skipped: 0,
            buffer: Vec::new(),
        }
    }

    /// Writes one row of displayed text, or skips it when the line protocol cannot carry
    /// its path.
    fn row(&mut self, path: &[u8], context: &[u8], line: u32, text: &[u8]) -> io::Result<()> {
        self.write(path, context, line, text, Spacing::Escaped)
    }

    /// Writes one row of a line a text search found, whose tabs and CR LF ending stay as
    /// they are.
    fn text_row(&mut self, path: &[u8], context: &[u8], line: u32, text: &[u8]) -> io::Result<()> {
        self.write(path, context, line, text, Spacing::Kept)
    }

    fn write(
        &mut self,
        path: &[u8],
        context: &[u8],
        line: u32,
        text: &[u8],
        spacing: Spacing,
    ) -> io::Result<()> {
        self.buffer.clear();
        match self.transport {
            Transport::Line => {
                if path.is_empty() || path.iter().any(|byte| is_control(*byte)) {
                    self.skipped += 1;
                    return Ok(());
                }
                self.buffer.extend_from_slice(path);
                self.buffer.push(b' ');
                escape_into(&mut self.buffer, context, Spacing::Escaped);
                self.buffer.push(b' ');
                decimal_into(&mut self.buffer, line);
                self.buffer.push(b' ');
                escape_into(&mut self.buffer, text, spacing);
                self.buffer.push(b'\n');
            }
            Transport::Json => {
                let value = serde_json::Value::Array(vec![
                    json_bytes(path),
                    json_bytes(context),
                    line.into(),
                    json_bytes(text),
                ]);
                serde_json::to_writer(&mut self.buffer, &value).map_err(io::Error::other)?;
                self.buffer.push(b'\n');
            }
        }
        self.out.write_all(&self.buffer)?;
        self.written += 1;
        Ok(())
    }

    /// Writes after its own rows those another sink wrote for the same transport.
    fn absorb<R: Read + Seek>(&mut self, other: Rows<R>) -> io::Result<()> {
        debug_assert_eq!(self.transport, other.transport);
        let mut rows = other.out;
        rows.seek(SeekFrom::Start(0))?;
        io::copy(&mut rows, &mut self.out)?;
        self.written += other.written;
        self.skipped += other.skipped;
        Ok(())
    }

    fn finish(mut self) -> io::Result<W> {
        self.out.flush()?;
        Ok(self.out)
    }
}

/// Whether the line protocol can carry `text` as a space-separated column (field 1
/// shows the pattern as the context column).
pub(crate) fn fits_column(text: &[u8]) -> bool {
    !text.iter().any(|byte| *byte == b' ' || is_control(*byte))
}

fn is_control(byte: u8) -> bool {
    byte < 0x20 || byte == 0x7f
}

/// How a row's text treats tabs and a CR LF ending's carriage return.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Spacing {
    Escaped,
    Kept,
}

/// Text with control bytes and DEL written as `\xHH`; with `Spacing::Kept`, tabs and a
/// final carriage return stay raw. The raw runs between escaped bytes are copied whole.
fn escape_into(out: &mut Vec<u8>, text: &[u8], spacing: Spacing) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut raw = 0;
    for (at, &byte) in text.iter().enumerate() {
        if !is_control(byte)
            || (spacing == Spacing::Kept
                && (byte == b'\t' || (byte == b'\r' && at + 1 == text.len())))
        {
            continue;
        }
        out.extend_from_slice(&text[raw..at]);
        out.extend_from_slice(&[
            b'\\',
            b'x',
            HEX[usize::from(byte >> 4)],
            HEX[usize::from(byte & 0xf)],
        ]);
        raw = at + 1;
    }
    out.extend_from_slice(&text[raw..]);
}

/// A line number's decimal digits.
fn decimal_into(out: &mut Vec<u8>, mut value: u32) {
    let mut digits = [0; 10];
    let mut start = digits.len();
    loop {
        start -= 1;
        digits[start] = b'0' + (value % 10) as u8;
        value /= 10;
        if value == 0 {
            break;
        }
    }
    out.extend_from_slice(&digits[start..]);
}

fn json_bytes(bytes: &[u8]) -> serde_json::Value {
    match std::str::from_utf8(bytes) {
        Ok(text) => text.into(),
        Err(_) => bytes.iter().map(|byte| u64::from(*byte)).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    #[derive(Clone, Default)]
    struct Shared(Rc<RefCell<Vec<u8>>>);

    impl Write for Shared {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.borrow_mut().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn delivery_places_warnings_around_framing_and_keeps_json_messages() {
        for (transport, warnings, framed, expected) in [
            (
                Transport::Line,
                WarningStyle::Stderr,
                true,
                "ripscope: a > b\\x0aline\\x7f\nresult 1\na.c <unknown> 2 \tx;\r\n",
            ),
            (
                Transport::Line,
                WarningStyle::Vim,
                true,
                "ripscope: a ? b\\x0aline\\x7f\nresult 1\na.c <unknown> 2 \tx;\r\n",
            ),
            (
                Transport::Json,
                WarningStyle::JsonSession,
                true,
                "result 1\n[\"a.c\",\"<unknown>\",2,\"\\tx;\\r\"]\n\
                 {\"message\":\"a > b\\nline\u{7f}\",\"type\":\"warning\"}\n",
            ),
            (
                Transport::Json,
                WarningStyle::Stderr,
                false,
                "ripscope: a > b\\x0aline\\x7f\n[\"a.c\",\"<unknown>\",2,\"\\tx;\\r\"]\n",
            ),
        ] {
            let mut sink = Sink::new(transport);
            sink.text_row(b"a.c", b"<unknown>", 2, b"\tx;\r").unwrap();
            let mut answer = sink.finish(vec!["a > b\nline\u{7f}".into()]).unwrap();
            let mut out = Shared::default();
            // Both streams reach the same reader, as Vim and xcscope arrange them.
            // Buffering stderr makes the flush before the header observable.
            let mut errors = io::BufWriter::new(out.clone());
            answer
                .deliver(&mut out, &mut errors, warnings, |out, rows| {
                    if framed {
                        writeln!(out, "result {rows}")?;
                    }
                    Ok(())
                })
                .unwrap();
            assert_eq!(&*out.0.borrow(), expected.as_bytes());
        }
    }

    struct FailsAfter {
        bytes: Vec<u8>,
        remaining: usize,
    }

    impl Write for FailsAfter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if self.remaining == 0 {
                return Err(io::ErrorKind::BrokenPipe.into());
            }
            let taken = self.remaining.min(bytes.len());
            self.bytes.extend_from_slice(&bytes[..taken]);
            self.remaining -= taken;
            Ok(taken)
        }

        fn flush(&mut self) -> io::Result<()> {
            Err(io::ErrorKind::BrokenPipe.into())
        }
    }

    #[test]
    fn delivery_ignores_stderr_failure_but_stops_at_output_failure() {
        let mut sink = Sink::new(Transport::Json);
        sink.text_row(b"a.c", b"f", 1, b"x").unwrap();
        let mut answer = sink.finish(vec!["warning".into()]).unwrap();
        let mut closed = FailsAfter {
            bytes: Vec::new(),
            remaining: 0,
        };
        let mut out = Vec::new();
        answer
            .deliver(&mut out, &mut closed, WarningStyle::Stderr, |_, _| Ok(()))
            .unwrap();
        assert_eq!(out, b"[\"a.c\",\"f\",1,\"x\"]\n");

        let complete =
            b"header\n[\"a.c\",\"f\",1,\"x\"]\n{\"message\":\"warning\",\"type\":\"warning\"}\n";
        // Failure in the header, the body or the warning frame must stop delivery.
        for remaining in [0, 3, 12, complete.len() - 1] {
            let mut out = FailsAfter {
                bytes: Vec::new(),
                remaining,
            };
            let error = answer
                .deliver(
                    &mut out,
                    &mut io::sink(),
                    WarningStyle::JsonSession,
                    |out, _| out.write_all(b"header\n"),
                )
                .unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
            assert_eq!(out.bytes, complete[..remaining]);
        }
    }

    #[test]
    fn spooled_parts_keep_row_order_counts_and_one_skipped_path_warning() {
        for spill in [0, 1024] {
            let mut sink = Sink::with_capacity(Transport::Line, spill);
            for path in [b"a.c".as_slice(), b"b.c"] {
                let mut part = Sink::with_capacity(Transport::Line, spill);
                for (context, at_file_scope) in [(b"f".as_slice(), false), (b"<global>", true)] {
                    part.row(crate::query::ResultRow {
                        path,
                        context,
                        line: 1,
                        text: b"x",
                        at_file_scope,
                    })
                    .unwrap();
                }
                part.text_row(b"bad\npath.c", b"<unknown>", 1, b"x")
                    .unwrap();
                sink.absorb(part).unwrap();
            }
            let mut answer = sink.finish(Vec::new()).unwrap();
            assert_eq!(answer.rows(), 4);
            let mut out = Vec::new();
            let mut errors = Vec::new();
            answer
                .deliver(&mut out, &mut errors, WarningStyle::Stderr, |out, rows| {
                    writeln!(out, "cscope: {rows} lines")
                })
                .unwrap();
            assert_eq!(
                out,
                b"cscope: 4 lines\na.c <global> 1 x\nb.c <global> 1 x\na.c f 1 x\nb.c f 1 x\n"
            );
            assert_eq!(
                errors,
                b"ripscope: 2 rows skipped: their paths hold control characters; use --json\n"
            );
        }
    }

    /// A row as the sink takes it: path, context, line and text.
    type TestRow<'a> = (&'a [u8], &'a [u8], u32, &'a [u8]);

    fn line_rows(rows: &[TestRow<'_>]) -> (String, Vec<String>) {
        let mut sink = Sink::new(Transport::Line);
        for (path, context, line, text) in rows {
            sink.row(ResultRow {
                path,
                context,
                line: *line,
                text,
                at_file_scope: true,
            })
            .expect("row");
        }
        sink.text_row(b"b.c", b"<unknown>", 2, b"\tint\ry;\r")
            .expect("text row");
        let mut answer = sink.finish(Vec::new()).expect("finish");
        let mut out = Vec::new();
        answer
            .deliver(&mut out, &mut io::sink(), WarningStyle::Stderr, |_, _| {
                Ok(())
            })
            .unwrap();
        (String::from_utf8(out).unwrap(), answer.warnings().to_vec())
    }

    #[test]
    fn the_line_protocol_writes_paths_raw_and_escapes_control_bytes() {
        let (out, skipped) = line_rows(&[
            (
                b"dir with space/caf\xc3\xa9.c",
                b"<global>",
                3,
                b"int caf\xc3\xa9;",
            ),
            (b"a.c", b"f", 7, b"x = 1;\x0c y\x7f"),
            (b"bad\npath.c", b"f", 1, b"int x;"),
            (b"c.c", b"f", 4, b"\tx;\r"),
        ]);
        assert_eq!(
            out,
            "dir with space/café.c <global> 3 int café;\na.c f 7 x = 1;\\x0c y\\x7f\n\
             c.c f 4 \\x09x;\\x0d\nb.c <unknown> 2 \tint\\x0dy;\r\n"
        );
        assert_eq!(
            skipped,
            ["1 row skipped: its path holds control characters; use --json"]
        );
    }

    /// The delivered line-protocol answer of rows at line 7 of `a.c` in `f`, written as
    /// found lines or as displayed text.
    fn texts_at_line_7(texts: &[Vec<u8>], found: bool) -> Vec<u8> {
        let mut sink = Sink::new(Transport::Line);
        for text in texts {
            if found {
                sink.text_row(b"a.c", b"f", 7, text).unwrap();
            } else {
                sink.row(ResultRow {
                    path: b"a.c",
                    context: b"f",
                    line: 7,
                    text,
                    at_file_scope: true,
                })
                .unwrap();
            }
        }
        let mut out = Vec::new();
        sink.finish(Vec::new())
            .unwrap()
            .deliver(&mut out, &mut io::sink(), WarningStyle::Stderr, |_, _| {
                Ok(())
            })
            .unwrap();
        out
    }

    #[test]
    fn rows_escape_each_control_byte_and_found_lines_keep_their_spacing() {
        // Every text of up to three bytes drawn from those escaping treats differently.
        let bytes = [
            b'a', b' ', b'\t', b'\r', b'\n', 0x00, 0x1f, 0x7f, 0x80, 0xff,
        ];
        let mut texts = vec![Vec::new()];
        let mut longest = texts.clone();
        for _ in 0..3 {
            longest = longest
                .iter()
                .flat_map(|text: &Vec<u8>| {
                    bytes
                        .iter()
                        .map(move |byte| [text.as_slice(), &[*byte]].concat())
                })
                .collect();
            texts.extend(longest.iter().cloned());
        }
        for found in [false, true] {
            let mut expected = Vec::new();
            for text in &texts {
                expected.extend_from_slice(b"a.c f 7 ");
                for (at, &byte) in text.iter().enumerate() {
                    let kept = found && (byte == b'\t' || (byte == b'\r' && at == text.len() - 1));
                    if (byte < 0x20 && !kept) || byte == 0x7f {
                        expected.extend_from_slice(format!("\\x{byte:02x}").as_bytes());
                    } else {
                        expected.push(byte);
                    }
                }
                expected.push(b'\n');
            }
            assert_eq!(texts_at_line_7(&texts, found), expected, "found: {found}");
        }
    }

    #[test]
    fn line_numbers_are_written_in_decimal() {
        let mut lines: Vec<u32> = (0..=1000).collect();
        for digits in 4..=9 {
            let power = 10_u32.pow(digits);
            lines.extend([power - 1, power, power + 1]);
        }
        lines.extend([u32::MAX - 1, u32::MAX]);
        let mut sink = Sink::new(Transport::Line);
        let mut expected = String::new();
        for &line in &lines {
            sink.text_row(b"a.c", b"f", line, b"x").unwrap();
            expected.push_str(&format!("a.c f {line} x\n"));
        }
        let mut out = Vec::new();
        sink.finish(Vec::new())
            .unwrap()
            .deliver(&mut out, &mut io::sink(), WarningStyle::Stderr, |_, _| {
                Ok(())
            })
            .unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), expected);
    }

    #[test]
    fn json_rows_keep_every_byte() {
        let mut sink = Sink::new(Transport::Json);
        sink.row(ResultRow {
            path: b"bad\npath.c",
            context: b"<global>",
            line: 2,
            text: b"\xff\x01x",
            at_file_scope: true,
        })
        .expect("row");
        let mut answer = sink.finish(Vec::new()).expect("finish");
        assert_eq!(answer.rows(), 1);
        assert!(!answer.has_warnings());
        let mut out = Vec::new();
        answer
            .deliver(&mut out, &mut io::sink(), WarningStyle::Stderr, |_, _| {
                Ok(())
            })
            .unwrap();
        let row: serde_json::Value = serde_json::from_slice(&out).expect("json");
        assert_eq!(row[0], "bad\npath.c");
        assert_eq!(row[2], 2);
        assert_eq!(row[3], serde_json::json!([255, 1, 120]));
    }
}
