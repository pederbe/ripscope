//! Decode the pinned reference cscope's database into per-file Analysis units.
//!
//! Shared by the probe captures and the full-tree compass. A unit is the list of line
//! records cscope stores for one file; each record is `{"line", "fragments", "symbols":
//! [[mark, name, assigned], ...], "define_end"}`, with text decoded as cscope displays
//! it. Text is Latin-1: each database byte is one character.

use crate::json_text::Json;
use std::collections::VecDeque;
use std::io::{self, BufRead};

/// cscope's default (compressed) database: bytes above 0x7f are digraphs of these first
/// and second characters (main.c:72-73), and bytes below 0x20 are keyword codes that
/// display as the keyword plus its delimiter (lookup.c:54-86, find.c:919-940).
pub const DICHAR1: &[u8; 16] = b" teisaprnl(of)=c";
pub const DICHAR2: &[u8; 8] = b" tnerpla";
const KEYWORDS: [Option<(&str, &str)>; 32] = [
    None,
    Some(("#define", " ")),
    Some(("#include", " ")),
    Some(("break", "")),
    Some(("case", " ")),
    Some(("char", " ")),
    Some(("continue", "")),
    Some(("default", "")),
    Some(("double", " ")),
    None,
    None,
    Some(("else", " ")),
    Some(("enum", " ")),
    Some(("extern", " ")),
    Some(("float", " ")),
    Some(("for", "(")),
    Some(("goto", " ")),
    Some(("if", "(")),
    Some(("int", " ")),
    Some(("long", " ")),
    Some(("register", " ")),
    Some(("return", "")),
    Some(("short", " ")),
    Some(("sizeof", "")),
    Some(("static", " ")),
    Some(("struct", " ")),
    Some(("switch", "(")),
    Some(("typedef", " ")),
    Some(("union", " ")),
    Some(("unsigned", " ")),
    Some(("void", " ")),
    Some(("while", "(")),
];

/// One line record of a unit.
#[derive(Clone, Debug, PartialEq)]
pub struct Record {
    pub line: u64,
    pub fragments: Vec<String>,
    /// Mark, name and whether field 9's test takes the name as assigned.
    pub symbols: Vec<(String, String, bool)>,
    pub define_end: bool,
}

impl Record {
    pub fn to_json(&self) -> Json {
        Json::object([
            ("line", Json::Int(self.line as i64)),
            ("fragments", Json::from(self.fragments.clone())),
            (
                "symbols",
                Json::Array(
                    self.symbols
                        .iter()
                        .map(|(mark, name, assigned)| {
                            Json::Array(vec![
                                mark.as_str().into(),
                                name.as_str().into(),
                                (*assigned).into(),
                            ])
                        })
                        .collect(),
                ),
            ),
            ("define_end", self.define_end.into()),
        ])
    }

    /// A record as `ripscope --dump-units` prints it.
    pub fn from_json(value: &serde_json::Value) -> io::Result<Record> {
        let invalid = || io::Error::other(format!("malformed unit record: {value}"));
        let text = |item: &serde_json::Value| item.as_str().map(str::to_owned).ok_or_else(invalid);
        Ok(Record {
            line: value["line"].as_u64().ok_or_else(invalid)?,
            fragments: value["fragments"]
                .as_array()
                .ok_or_else(invalid)?
                .iter()
                .map(text)
                .collect::<io::Result<_>>()?,
            symbols: value["symbols"]
                .as_array()
                .ok_or_else(invalid)?
                .iter()
                .map(|symbol| {
                    Ok((
                        text(&symbol[0])?,
                        text(&symbol[1])?,
                        symbol[2].as_bool().unwrap_or(false),
                    ))
                })
                .collect::<io::Result<_>>()?,
            define_end: value["define_end"].as_bool().ok_or_else(invalid)?,
        })
    }
}

/// Each byte as the Latin-1 character with the same value.
pub fn latin1(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| char::from(*byte)).collect()
}

/// A compressed-database fragment or name as cscope displays it (find.c:912-944).
pub fn decode_display(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        let code = u32::from(ch);
        if code > 0x7f && code <= 0xff {
            let code = (code & 0x7f) as usize;
            out.push(char::from(DICHAR1[code / 8]));
            out.push(char::from(DICHAR2[code & 7]));
        } else if let Some(Some((word, delimiter))) = KEYWORDS.get(code as usize) {
            out.push_str(word);
            if !delimiter.is_empty() {
                out.push(' ');
            }
            if *delimiter == "(" {
                out.push('(');
            }
        } else {
            out.push(ch);
        }
    }
    out
}

/// Ripscope's raw bytes as the reference displays them: high bytes as digraph pairs and
/// control bytes as keyword codes (find.c:912-944). Ripscope keeps raw bytes, so
/// comparisons with the reference view them this way.
pub fn reference_view(text: &str) -> String {
    decode_display(text)
}

/// A ripscope unit's records under [`reference_view`].
pub fn view_records(records: &[Record]) -> Vec<Record> {
    records
        .iter()
        .map(|record| Record {
            line: record.line,
            fragments: record.fragments.iter().map(|f| reference_view(f)).collect(),
            symbols: record
                .symbols
                .iter()
                .map(|(mark, name, assigned)| (mark.clone(), reference_view(name), *assigned))
                .collect(),
            define_end: record.define_end,
        })
        .collect()
}

/// Per-file Analysis units from a whole database, in file order. A file named twice
/// keeps its first place and gains the later records.
pub fn parse_cscope_database(data: &[u8]) -> io::Result<Vec<(String, Vec<Record>)>> {
    let mut units: Vec<(String, Vec<Record>)> = Vec::new();
    for unit in Units::new(data)? {
        let (name, records) = unit?;
        match units.iter_mut().find(|(known, _)| *known == name) {
            Some((_, existing)) => existing.extend(records),
            None => units.push((name, records)),
        }
    }
    Ok(units)
}

fn identifier(text: &str, first_required: bool) -> bool {
    let mut chars = text.chars();
    if first_required {
        match chars.next() {
            Some(ch) if ch.is_ascii_alphabetic() || ch == '_' || ch == '$' => {}
            _ => return false,
        }
    }
    chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '$')
}

/// Decodes an open database one file at a time, including empty files.
///
/// A full kernel does not fit in memory as a dictionary of every record, so this keeps
/// one file and four raw lines of assignment look-ahead. File headers are recognized
/// only outside records: a continued literal can contain the same bytes. The body after
/// the header line is a sequence of `\t@<file>` headers, each followed by line records
/// `<lineno> <text>` in which every symbol sits on its own line as `\t<mark><name>` (or
/// `<name>` for an unmarked identifier) followed by the next text fragment. An empty line
/// ends a record, and `\t)` after a record marks a macro end (crossref.c:236-372,
/// build.c:550-568).
pub struct Units<R> {
    stream: R,
    position: u64,
    trailer: u64,
    compressed: bool,
    pending: VecDeque<String>,
    current: Option<(String, Vec<Record>)>,
    number: usize,
    done: bool,
}

impl<'a> Units<&'a [u8]> {
    pub fn new(data: &'a [u8]) -> io::Result<Self> {
        Units::from_reader(data)
    }
}

impl<R: BufRead> Units<R> {
    pub fn from_reader(mut stream: R) -> io::Result<Self> {
        let mut raw = Vec::new();
        let position = stream.read_until(b'\n', &mut raw)? as u64;
        let header = latin1(raw.strip_suffix(b"\n").unwrap_or(&raw));
        if !header.starts_with("cscope ") {
            let shown: String = header.chars().take(80).collect();
            return Err(io::Error::other(format!(
                "not a cscope database: {shown:?}"
            )));
        }
        let compressed = !format!("{header} ").contains(" -c ");
        let trailer = header
            .rsplit(' ')
            .next()
            .and_then(|value| value.trim().parse::<u64>().ok())
            .ok_or_else(|| io::Error::other("cscope database header has no trailer offset"))?;
        Ok(Units {
            stream,
            position,
            trailer,
            compressed,
            pending: VecDeque::new(),
            current: None,
            number: 0,
            done: false,
        })
    }

    fn decode(&self, text: &str) -> String {
        if self.compressed {
            decode_display(text)
        } else {
            text.to_owned()
        }
    }

    fn body_line(&mut self) -> io::Result<Option<String>> {
        if self.position >= self.trailer {
            return Ok(None);
        }
        let mut raw = Vec::new();
        let limit = self.trailer - self.position;
        let read = io::Read::take(&mut self.stream, limit).read_until(b'\n', &mut raw)?;
        if read == 0 {
            return Err(io::Error::other("cscope database ends before its trailer"));
        }
        self.position += read as u64;
        Ok(Some(latin1(raw.strip_suffix(b"\n").unwrap_or(&raw))))
    }

    fn next_unit(&mut self) -> io::Result<Option<(String, Vec<Record>)>> {
        let mut record: Option<Record> = None;
        loop {
            while self.pending.len() < 5 {
                match self.body_line()? {
                    Some(line) => self.pending.push_back(line),
                    None => break,
                }
            }
            let Some(line) = self.pending.pop_front() else {
                return Err(io::Error::other("cscope database has no final file marker"));
            };
            self.number += 1;
            if let Some(open) = record.as_mut() {
                if line.is_empty() {
                    let finished = record.take().expect("open record");
                    self.current
                        .as_mut()
                        .ok_or_else(|| io::Error::other("cscope database record before a file"))?
                        .1
                        .push(finished);
                    continue;
                }
                // A name at the line's first byte can have that byte overwritten by the
                // code of a `#define` or `#include` later on the line (fscanner.l:423-426).
                let coded_name = (line.starts_with('\u{1}') || line.starts_with('\u{2}'))
                    && identifier(&self.decode(&line[1..]), false);
                if !line.starts_with('\t') && !coded_name && !identifier(&self.decode(&line), true)
                {
                    // A continued string literal writes its backslash-newline raw into
                    // the database (fscanner.l:756-759), so this line continues the last
                    // fragment. cscope's display drops that raw line end, and so does
                    // the decoder.
                    let decoded = self.decode(&line);
                    open.fragments
                        .last_mut()
                        .expect("a record starts with a fragment")
                        .push_str(&decoded);
                    continue;
                }
                let (mark, name) = match line.strip_prefix('\t') {
                    Some(rest) => {
                        let mut chars = rest.chars();
                        let mark = chars.next().map(String::from).unwrap_or_default();
                        (mark, chars.as_str().to_owned())
                    }
                    None => (" ".to_owned(), line.clone()),
                };
                let mut following = String::from("\n");
                following.push_str(
                    &self
                        .pending
                        .iter()
                        .map(String::as_str)
                        .collect::<Vec<_>>()
                        .join("\n"),
                );
                let name = self.decode(&name);
                let fragment = match self.pending.pop_front() {
                    Some(next) => self.decode(&next),
                    None => String::new(),
                };
                let open = record.as_mut().expect("open record");
                open.symbols
                    .push((mark, name, assignment_follows(&following)));
                open.fragments.push(fragment);
                self.number += 1;
                continue;
            }
            if let Some(name) = line.strip_prefix("\t@") {
                let finished = self.current.take();
                if !name.is_empty() {
                    self.current = Some((name.to_owned(), Vec::new()));
                } else {
                    self.done = true;
                }
                match finished {
                    Some(unit) => return Ok(Some(unit)),
                    None if self.done => return Ok(None),
                    None => continue,
                }
            }
            if line == "\t)" {
                if let Some(last) = self.current.as_mut().and_then(|(_, r)| r.last_mut()) {
                    last.define_end = true;
                }
                continue;
            }
            if line.is_empty() {
                continue;
            }
            let (number, first) = line.split_once(' ').unwrap_or((&line, ""));
            if number.is_empty() || !number.bytes().all(|byte| byte.is_ascii_digit()) {
                let shown: String = line.chars().take(80).collect();
                return Err(io::Error::other(format!(
                    "unexpected database line {}: {shown:?}",
                    self.number
                )));
            }
            record = Some(Record {
                line: number
                    .parse()
                    .map_err(|_| io::Error::other("database line number out of range"))?,
                fragments: vec![self.decode(first)],
                symbols: Vec::new(),
                define_end: false,
            });
        }
    }
}

impl<R: BufRead> Iterator for Units<R> {
    type Item = io::Result<(String, Vec<Record>)>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done && self.current.is_none() {
            return None;
        }
        match self.next_unit() {
            Ok(Some(unit)) => Some(Ok(unit)),
            Ok(None) => None,
            Err(error) => {
                self.done = true;
                self.current = None;
                Some(Err(error))
            }
        }
    }
}

/// Whether field 9 treats the name as assigned: the reference's test on the database
/// bytes after the name (find.c:103-157). It skips C whitespace (which includes the codes
/// for `else`, `enum` and `extern`, bytes 11-13), then accepts `=` not followed by `=`,
/// an operator followed by `=`, or `<<=`/`>>=`. A digraph byte counts as its first
/// character only where the test looks at it that way.
pub fn assignment_follows(raw: &str) -> bool {
    let bytes: Vec<u32> = raw.chars().map(u32::from).collect();
    let mut at = 0;
    while at < bytes.len() && [0x20, 0x09, 0x0a, 0x0b, 0x0c, 0x0d].contains(&bytes[at]) {
        at += 1;
    }
    let byte = |offset: usize| bytes.get(at + offset).copied().unwrap_or(0);
    let pair_starts_with_equals =
        |value: u32| value & 0x80 != 0 && DICHAR1[((value & 0x7f) / 8) as usize] == b'=';
    let equals = u32::from(b'=');
    let (first, second, third) = (byte(0), byte(1), byte(2));
    if pair_starts_with_equals(first) {
        return true;
    }
    if first == equals
        && ((second != equals && second & 0x80 == 0)
            || (second & 0x80 != 0 && DICHAR1[((second & 0x7f) / 8) as usize] != b'='))
    {
        return true;
    }
    let operator = |value: u32, set: &str| value != 0 && set.chars().any(|c| u32::from(c) == value);
    if operator(first, "+-*/%&|^") && (second == equals || pair_starts_with_equals(second)) {
        return true;
    }
    operator(first, "<>") && second == first && (third == equals || pair_starts_with_equals(third))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn database(body: &[u8]) -> Vec<u8> {
        let header_length = format!("cscope 15 /tmp -c {:010}\n", 0).len();
        let mut data =
            format!("cscope 15 /tmp -c {:010}\n", header_length + body.len()).into_bytes();
        data.extend_from_slice(body);
        data.extend_from_slice(b"not a record\n");
        data
    }

    fn record(
        line: u64,
        fragments: &[&str],
        symbols: &[(&str, &str, bool)],
        define_end: bool,
    ) -> Record {
        Record {
            line,
            fragments: fragments.iter().map(|f| (*f).to_owned()).collect(),
            symbols: symbols
                .iter()
                .map(|(m, n, a)| ((*m).to_owned(), (*n).to_owned(), *a))
                .collect(),
            define_end,
        }
    }

    #[test]
    fn records_empty_files_and_continued_literal_file_marker() {
        let data = database(
            b"\t@one.c\n\n1 int \n\tgvalue\n = 1;\n\n\
              2 #define \n\t#MESSAGE\n \"first\\\n\t@literal\nlast\"\n\n\t)\n\
              \t@empty.h\n\n\t@\n",
        );
        let expected = vec![
            (
                "one.c".to_owned(),
                vec![
                    record(1, &["int ", " = 1;"], &[("g", "value", true)], false),
                    record(
                        2,
                        &["#define ", " \"first\\", "last\""],
                        &[("#", "MESSAGE", false), ("@", "literal", false)],
                        true,
                    ),
                ],
            ),
            ("empty.h".to_owned(), Vec::new()),
        ];
        let streamed = Units::new(&data)
            .unwrap()
            .collect::<io::Result<Vec<_>>>()
            .unwrap();
        assert_eq!(streamed, expected);
        assert_eq!(parse_cscope_database(&data).unwrap(), expected);
    }

    #[test]
    fn truncated_database_is_not_an_empty_success() {
        let data = database(b"\t@empty.h\n\n\t@\n");
        let truncated =
            Units::new(&data[..20]).and_then(|units| units.collect::<io::Result<Vec<_>>>());
        assert!(truncated.is_err());
        let unfinished = database(b"\t@empty.h\n\n");
        assert!(
            Units::new(&unfinished)
                .unwrap()
                .collect::<io::Result<Vec<_>>>()
                .is_err()
        );
        assert!(Units::new(b"not a database\n").is_err());
    }

    #[test]
    fn display_expands_digraphs_and_keyword_codes() {
        assert_eq!(
            decode_display("\u{1}\u{f}x\u{80}\t\n"),
            "#define for (x  \t\n"
        );
        assert_eq!(decode_display("\u{8f}\u{f0}"), "ta= ");
        assert_eq!(reference_view("\u{12}a"), "int a");
    }

    #[test]
    fn assignment_test_follows_the_reference() {
        for (raw, assigned) in [
            ("\n = 1", true),
            ("\n == 1", false),
            ("\n += 1", true),
            ("\n <<= 1", true),
            ("\n << 1", false),
            ("\u{b}\u{c}= 1", true),
            ("\n=\u{f0}", false),
            ("\n=\u{f8}", true),
            ("\n\u{f0}", true),
            ("\n&\u{f0}", true),
            ("\n&\u{f8}", false),
            ("", false),
        ] {
            assert_eq!(assignment_follows(raw), assigned, "{raw:?}");
        }
    }
}
