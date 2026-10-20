//! cscope's query patterns compiled for C-locale byte matching, within bounds.
use super::{IdentifierCase, IdentifierPattern};
use regex::bytes::{Regex, RegexBuilder};
use std::{borrow::Cow, fmt, fmt::Write};

#[path = "text_pattern.rs"]
mod text_pattern;

#[derive(Clone, Copy, Debug)]
pub(crate) struct PatternLimits {
    pub input_bytes: usize,
    pub group_depth: usize,
    pub compiled_bytes: usize,
    pub dfa_cache_bytes: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum UnsupportedPattern {
    Encoding,
    FinalDollarEscape,
    TrailingEscape,
    Backreference,
    BoundaryEscape,
    CountedRepetition,
    RepeatedQuantifier,
    EmptyBranch,
    SpecialGroup,
    ClosingParenthesis,
    BracketForm,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ResourceLimit {
    InputBytes,
    GroupDepth,
    CompiledBytes,
}

#[derive(Debug)]
pub(crate) enum PatternError {
    Empty,
    InvalidIdentifier,
    InvalidSyntax,
    Unsupported(UnsupportedPattern),
    Resource(ResourceLimit),
    Backend(regex::Error),
}

impl fmt::Display for PatternError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("the pattern is empty"),
            Self::InvalidIdentifier => f.write_str("the pattern is not an identifier"),
            Self::InvalidSyntax => f.write_str("invalid syntax"),
            Self::Unsupported(UnsupportedPattern::Encoding) => f.write_str(
                "a symbol pattern is printable ASCII, a file or include pattern holds no \
                 carriage return, and no pattern holds a NUL or a line break",
            ),
            Self::Unsupported(feature) => write!(f, "not supported: {feature:?}"),
            Self::Resource(limit) => write!(f, "over the limit: {limit:?}"),
            Self::Backend(error) => write!(f, "rejected by the regular expression engine: {error}"),
        }
    }
}

impl std::error::Error for PatternError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Backend(error) => Some(error),
            _ => None,
        }
    }
}

#[derive(Debug)]
enum Matcher<'request> {
    Identifier(IdentifierPattern<'request>),
    Regex {
        compiled: Regex,
        case: IdentifierCase,
    },
    TextRegex {
        compiled: Regex,
        case: IdentifierCase,
    },
}

/// A compiled pattern; one serves every file a query reads. Patterns are bytes, matched
/// byte for byte as cscope's are in the C locale.
#[derive(Debug)]
pub(crate) struct CompiledPattern<'request> {
    context: Cow<'request, [u8]>,
    matcher: Matcher<'request>,
    /// For a name pattern, bytes one of which every name it matches starts with, when
    /// there are a few: names it may match are then looked up by these prefixes.
    prefixes: Option<Vec<Vec<u8>>>,
}

impl<'request> CompiledPattern<'request> {
    pub fn compile_regex_text(
        pattern: &'request (impl AsRef<[u8]> + ?Sized),
        case: IdentifierCase,
        limits: PatternLimits,
    ) -> Result<Self, PatternError> {
        let pattern = within(pattern.as_ref(), limits)?;
        if pattern.is_empty() {
            return Err(PatternError::Empty);
        }
        if pattern.contains(&b'\n') {
            return Err(PatternError::Unsupported(UnsupportedPattern::Encoding));
        }
        let body = folded(pattern, case);
        let translated = text_pattern::translate(&body, limits.group_depth)?;
        let compiled = build_regex(&translated, pattern.len(), limits)?;
        Ok(Self {
            context: Cow::Borrowed(pattern),
            matcher: Matcher::TextRegex { compiled, case },
            prefixes: None,
        })
    }

    /// The initial LF is a boundary; a final LF exists only in terminated lines.
    pub(crate) fn matches_line(&self, line: &[u8], scratch: &mut Vec<u8>) -> bool {
        if let Matcher::TextRegex { compiled, case } = &self.matcher {
            scratch.clear();
            scratch.push(b'\n');
            if *case == IdentifierCase::AsciiInsensitive {
                scratch.extend(line.iter().map(u8::to_ascii_lowercase));
            } else {
                scratch.extend_from_slice(line);
            }
            compiled.is_match(scratch)
        } else {
            self.matches_name(line, scratch)
        }
    }
    /// The lines of `content` that [`Self::matches_line`] accepts, in order, each with its
    /// number from 1, given to `each`. The whole content is searched at once, and only
    /// the lines a match starts in are matched line by line.
    ///
    /// A text regex matches a line with a line break before it (its window), so each
    /// window but the first, whose break is not in `content`, is a part of the content
    /// that overlaps the next window by a line break. The first line is matched on its
    /// own. A literal matches within a line. Any match in a line's window is a match in
    /// the content, and the leftmost match from the first window not yet looked at starts
    /// in a window no later than the first line that matches: so the lines it starts in
    /// are matched one by one, and the search goes on from the window after them.
    pub(crate) fn matching_lines<E>(
        &self,
        content: &[u8],
        scratch: &mut Vec<u8>,
        folded: &mut Vec<u8>,
        mut each: impl FnMut(usize, &[u8]) -> Result<(), E>,
    ) -> Result<(), E> {
        let (compiled, case, windowed) = match &self.matcher {
            Matcher::TextRegex { compiled, case } => (compiled, *case, true),
            Matcher::Regex { compiled, case } => (compiled, *case, false),
            Matcher::Identifier(_) => {
                for (number, line) in content.split_inclusive(|b| *b == b'\n').enumerate() {
                    if self.matches_line(line, scratch) {
                        each(number + 1, line)?;
                    }
                }
                return Ok(());
            }
        };
        let haystack: &[u8] = if case == IdentifierCase::AsciiInsensitive {
            folded.clear();
            folded.extend(content.iter().map(u8::to_ascii_lowercase));
            folded
        } else {
            content
        };
        let line_end =
            |at: usize| memchr::memchr(b'\n', &content[at..]).map_or(content.len(), |n| at + n + 1);
        // The first line not yet looked at, and its number.
        let (mut start, mut number) = (0, 1);
        if windowed && !content.is_empty() {
            let end = line_end(0);
            if self.matches_line(&content[..end], scratch) {
                each(1, &content[..end])?;
            }
            (start, number) = (end, 2);
        }
        while start < content.len() {
            let from = if windowed { start - 1 } else { start };
            let Some(found) = compiled.find_at(haystack, from) else {
                break;
            };
            let at = found.start();
            // The line whose window the match starts in: the first line not yet looked at,
            // for the line break before it, else the line holding `at`. A match from the
            // line break that ends a line is found again from the next line's window.
            let line = if at < start {
                start
            } else {
                let line = memchr::memrchr(b'\n', &content[..at]).map_or(0, |n| n + 1);
                number += memchr::memchr_iter(b'\n', &content[start..line]).count();
                line
            };
            let end = line_end(line);
            if self.matches_line(&content[line..end], scratch) {
                each(number, &content[line..end])?;
            }
            number += 1;
            let line = end;
            start = line;
        }
        Ok(())
    }

    /// Text requests preserve whitespace and match their bytes literally anywhere.
    pub fn compile_text(
        pattern: &'request (impl AsRef<[u8]> + ?Sized),
        case: IdentifierCase,
        limits: PatternLimits,
    ) -> Result<Self, PatternError> {
        let pattern = within(pattern.as_ref(), limits)?;
        if pattern.is_empty() {
            return Err(PatternError::Empty);
        }
        if pattern.contains(&b'\n') {
            return Err(PatternError::Unsupported(UnsupportedPattern::Encoding));
        }
        let body = folded(pattern, case);
        let mut literal = String::with_capacity(4 * body.len());
        for &byte in body.iter() {
            emit_byte(&mut literal, byte);
        }
        let compiled = build_regex(&literal, pattern.len(), limits)?;
        Ok(Self {
            context: Cow::Borrowed(pattern),
            matcher: Matcher::Regex { compiled, case },
            prefixes: None,
        })
    }

    /// A file-name or include pattern (fields 7 and 8) that may hold any byte but a line
    /// break or a carriage return, since paths print as they are (a query refuses NUL
    /// before).
    pub(crate) fn compile_path(
        pattern: &'request (impl AsRef<[u8]> + ?Sized),
        case: IdentifierCase,
        limits: PatternLimits,
    ) -> Result<Self, PatternError> {
        let pattern = within(pattern.as_ref(), limits)?;
        if pattern.is_empty() {
            return Err(PatternError::Empty);
        }
        // A file-name ERE searches anywhere in the path, keeping its anchors.
        let context = trim_blanks(pattern);
        if context.iter().any(|byte| matches!(byte, b'\n' | b'\r')) {
            return Err(PatternError::Unsupported(UnsupportedPattern::Encoding));
        }
        let body = folded(context, case);
        let translated = if body.is_empty() {
            String::new()
        } else {
            translate(&body, limits.group_depth)?
        };
        let compiled = build_regex(&translated, pattern.len(), limits)?;
        Ok(Self {
            context: Cow::Borrowed(context),
            matcher: Matcher::Regex { compiled, case },
            prefixes: None,
        })
    }

    /// A pattern for the identifier fields: printable ASCII, an identifier or a regular
    /// expression over names.
    pub fn compile(
        pattern: &'request (impl AsRef<[u8]> + ?Sized),
        case: IdentifierCase,
        limits: PatternLimits,
    ) -> Result<Self, PatternError> {
        let pattern = within(pattern.as_ref(), limits)?;
        let context = trim_blanks(pattern);
        if context.is_empty() {
            return Err(PatternError::Empty);
        }
        if !context.iter().all(|byte| (b' '..=b'~').contains(byte)) {
            return Err(PatternError::Unsupported(UnsupportedPattern::Encoding));
        }
        if !context.iter().any(|byte| b"^.[{*+$|(".contains(byte)) {
            let identifier = IdentifierPattern::new(context, case)
                .map_err(|_| PatternError::InvalidIdentifier)?;
            return Ok(Self {
                context: Cow::Borrowed(context),
                matcher: Matcher::Identifier(identifier),
                prefixes: (case == IdentifierCase::AsciiInsensitive)
                    .then(|| caseless_identifier_prefixes(context)),
            });
        }
        let mut body = folded(context, case).into_owned();
        if body.ends_with(b"\\$") {
            return Err(PatternError::Unsupported(
                UnsupportedPattern::FinalDollarEscape,
            ));
        }
        if body.starts_with(b"^") {
            body.remove(0);
        }
        if body.ends_with(b"$") {
            body.pop();
        }
        let translated = translate(&body, limits.group_depth)?;
        let whole = format!("\\A(?:{translated})\\z");
        let compiled = build_regex(&whole, pattern.len(), limits)?;
        // Names are folded before a caseless pattern matches them, so its prefixes are
        // not those of the names stored.
        let prefixes = match case {
            IdentifierCase::Sensitive => name_prefixes(&whole),
            IdentifierCase::AsciiInsensitive => None,
        };
        Ok(Self {
            context: match case {
                IdentifierCase::Sensitive => Cow::Owned(body),
                IdentifierCase::AsciiInsensitive => Cow::Borrowed(context),
            },
            matcher: Matcher::Regex { compiled, case },
            prefixes,
        })
    }

    /// For a name pattern, bytes one of which every name it matches starts with, when that
    /// set is known and small.
    pub(crate) fn name_prefixes(&self) -> Option<&[Vec<u8>]> {
        self.prefixes.as_deref()
    }

    /// The pattern as field 1 shows it in each row's context column.
    pub fn context(&self) -> &[u8] {
        &self.context
    }

    pub(crate) fn is_case_insensitive(&self) -> bool {
        match &self.matcher {
            Matcher::Identifier(request) => request.case == IdentifierCase::AsciiInsensitive,
            Matcher::Regex { case, .. } | Matcher::TextRegex { case, .. } => {
                *case == IdentifierCase::AsciiInsensitive
            }
        }
    }

    /// The only name a case-sensitive identifier request matches.
    pub(crate) fn exact_name(&self) -> Option<&[u8]> {
        match &self.matcher {
            Matcher::Identifier(request) if request.case == IdentifierCase::Sensitive => {
                Some(&self.context)
            }
            _ => None,
        }
    }

    pub(crate) fn matches_name(&self, name: &[u8], folded: &mut Vec<u8>) -> bool {
        match &self.matcher {
            Matcher::TextRegex { .. } => unreachable!("text regex requires physical lines"),
            Matcher::Identifier(request) => request.matches_bytes(name),
            Matcher::Regex { compiled, case } => {
                if *case == IdentifierCase::AsciiInsensitive {
                    folded.clear();
                    folded.extend(name.iter().map(u8::to_ascii_lowercase));
                    compiled.is_match(folded)
                } else {
                    compiled.is_match(name)
                }
            }
        }
    }
}

/// At most eight disjoint raw prefixes cover every ASCII casing of an identifier.
/// Matching still checks the whole name. Keeping three bytes bounds the range seeks.
fn caseless_identifier_prefixes(name: &[u8]) -> Vec<Vec<u8>> {
    let mut prefixes = vec![Vec::new()];
    for &byte in name.iter().take(3) {
        let count = prefixes.len();
        for at in 0..count {
            if byte.is_ascii_alphabetic() {
                let mut upper = prefixes[at].clone();
                upper.push(byte.to_ascii_uppercase());
                prefixes.push(upper);
            }
            prefixes[at].push(byte.to_ascii_lowercase());
        }
    }
    prefixes
}

/// The bytes one of which every name the anchored regex `whole` matches starts with,
/// none of them empty, or `None` when there is no such finite set.
fn name_prefixes(whole: &str) -> Option<Vec<Vec<u8>>> {
    use regex_syntax::hir::literal::{ExtractKind, Extractor};
    let hir = regex_syntax::ParserBuilder::new()
        .unicode(false)
        .utf8(false)
        .dot_matches_new_line(true)
        .build()
        .parse(whole)
        .ok()?;
    let found = Extractor::new().kind(ExtractKind::Prefix).extract(&hir);
    let mut prefixes: Vec<Vec<u8>> = found
        .literals()?
        .iter()
        .map(|literal| literal.as_bytes().to_vec())
        .collect();
    if prefixes.iter().any(Vec::is_empty) {
        return None;
    }
    // A prefix that starts with another adds no names.
    prefixes.sort();
    prefixes.dedup_by(|longer, shorter| longer.starts_with(shorter));
    Some(prefixes)
}

/// `pattern`, unless it is longer than the limits allow.
fn within(pattern: &[u8], limits: PatternLimits) -> Result<&[u8], PatternError> {
    if pattern.len() > limits.input_bytes {
        return Err(PatternError::Resource(ResourceLimit::InputBytes));
    }
    Ok(pattern)
}

/// `pattern` as it is matched: with `-C` its ASCII letters in lower case, as names and
/// lines are folded before they are matched.
fn folded(pattern: &[u8], case: IdentifierCase) -> Cow<'_, [u8]> {
    match case {
        IdentifierCase::Sensitive => Cow::Borrowed(pattern),
        IdentifierCase::AsciiInsensitive => Cow::Owned(pattern.to_ascii_lowercase()),
    }
}

/// The pattern without the blanks cscope trims from its end (the C locale's `isspace`,
/// which includes the vertical tab and form feed).
fn trim_blanks(pattern: &[u8]) -> &[u8] {
    let end = pattern
        .iter()
        .rposition(|byte| !matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c))
        .map_or(0, |last| last + 1);
    &pattern[..end]
}

fn build_regex(body: &str, input_len: usize, limits: PatternLimits) -> Result<Regex, PatternError> {
    // Bound the generated AST without inheriting the backend's default depth.
    let backend_depth = input_len
        .checked_mul(2)
        .and_then(|n| n.checked_add(16))
        .and_then(|n| u32::try_from(n).ok())
        .ok_or(PatternError::Resource(ResourceLimit::InputBytes))?;
    RegexBuilder::new(body)
        .unicode(false)
        .case_insensitive(false)
        .multi_line(false)
        .crlf(false)
        .dot_matches_new_line(true)
        .nest_limit(backend_depth)
        .size_limit(limits.compiled_bytes)
        .dfa_size_limit(limits.dfa_cache_bytes)
        .build()
        .map_err(|error| match error {
            regex::Error::CompiledTooBig(_) => PatternError::Resource(ResourceLimit::CompiledBytes),
            error => PatternError::Backend(error),
        })
}

#[derive(Default)]
struct Frame {
    has_atom: bool,
    repeatable: bool,
    repeated: bool,
}

impl Frame {
    fn atom(&mut self, repeatable: bool) {
        self.has_atom = true;
        self.repeatable = repeatable;
        self.repeated = false;
    }
}

fn translate(input: &[u8], depth_limit: usize) -> Result<String, PatternError> {
    let mut out = String::new();
    let mut frames = vec![Frame::default()];
    let mut p = 0;
    while p < input.len() {
        let byte = input[p];
        p += 1;
        match byte {
            b'(' => {
                if input.get(p) == Some(&b'?') {
                    return Err(PatternError::Unsupported(UnsupportedPattern::SpecialGroup));
                }
                if frames.len() > depth_limit {
                    return Err(PatternError::Resource(ResourceLimit::GroupDepth));
                }
                frames.push(Frame::default());
                out.push_str("(?:");
            }
            b')' => {
                if frames.len() == 1 {
                    return Err(PatternError::Unsupported(
                        UnsupportedPattern::ClosingParenthesis,
                    ));
                }
                if !frames.pop().unwrap().has_atom {
                    return Err(PatternError::Unsupported(UnsupportedPattern::EmptyBranch));
                }
                out.push(')');
                frames.last_mut().unwrap().atom(true);
            }
            b'|' => {
                let frame = frames.last_mut().unwrap();
                if !frame.has_atom {
                    return Err(PatternError::Unsupported(UnsupportedPattern::EmptyBranch));
                }
                *frame = Frame::default();
                out.push('|');
            }
            b'*' | b'+' | b'?' => {
                let frame = frames.last_mut().unwrap();
                if !frame.repeatable {
                    return Err(PatternError::InvalidSyntax);
                }
                if frame.repeated {
                    return Err(PatternError::Unsupported(
                        UnsupportedPattern::RepeatedQuantifier,
                    ));
                }
                frame.repeated = true;
                out.push(char::from(byte));
            }
            b'{' => {
                let (lower, upper) = interval(input, &mut p)?;
                let frame = frames.last_mut().unwrap();
                if !frame.repeatable {
                    return Err(PatternError::InvalidSyntax);
                }
                if frame.repeated {
                    return Err(PatternError::Unsupported(
                        UnsupportedPattern::RepeatedQuantifier,
                    ));
                }
                frame.repeated = true;
                match upper {
                    Some(upper) if upper == lower => write!(out, "{{{lower}}}").unwrap(),
                    Some(upper) => write!(out, "{{{lower},{upper}}}").unwrap(),
                    None => write!(out, "{{{lower},}}").unwrap(),
                }
            }
            b'^' | b'$' => {
                out.push_str(if byte == b'^' { "\\A" } else { "\\z" });
                frames.last_mut().unwrap().atom(false);
            }
            b'.' => {
                out.push_str("[^\\x00]");
                frames.last_mut().unwrap().atom(true);
            }
            b'[' => {
                let set = bracket(input, &mut p)?;
                emit_set(&mut out, &set);
                frames.last_mut().unwrap().atom(true);
            }
            b'\\' => {
                let escaped = *input.get(p).ok_or(PatternError::Unsupported(
                    UnsupportedPattern::TrailingEscape,
                ))?;
                p += 1;
                match escaped {
                    b'1'..=b'9' => {
                        return Err(PatternError::Unsupported(UnsupportedPattern::Backreference));
                    }
                    b'b' | b'B' | b'<' | b'>' | b'`' | b'\'' => {
                        return Err(PatternError::Unsupported(
                            UnsupportedPattern::BoundaryEscape,
                        ));
                    }
                    b'w' | b'W' | b's' | b'S' => {
                        let mut set = [false; 256];
                        for (n, included) in set.iter_mut().enumerate() {
                            let byte = n as u8;
                            *included = match escaped {
                                b'w' | b'W' => byte.is_ascii_alphanumeric() || byte == b'_',
                                _ => matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | 11 | 12),
                            } ^ escaped.is_ascii_uppercase();
                        }
                        emit_set(&mut out, &set);
                    }
                    byte => emit_byte(&mut out, byte),
                }
                frames.last_mut().unwrap().atom(true);
            }
            byte => {
                emit_byte(&mut out, byte);
                frames.last_mut().unwrap().atom(true);
            }
        }
    }
    if frames.len() != 1 {
        return Err(PatternError::InvalidSyntax);
    }
    if !frames[0].has_atom {
        return Err(PatternError::Unsupported(UnsupportedPattern::EmptyBranch));
    }
    Ok(out)
}

/// Parse only qualified intervals; other brace forms retain an explicit gap.
fn interval(input: &[u8], p: &mut usize) -> Result<(u16, Option<u16>), PatternError> {
    let omitted_lower = input.get(*p) == Some(&b',');
    let lower = if omitted_lower {
        0
    } else {
        repetition_count(input, p)?
    };
    let upper = if input.get(*p) == Some(&b',') {
        *p += 1;
        if !omitted_lower && input.get(*p) == Some(&b'}') {
            None
        } else {
            Some(repetition_count(input, p)?)
        }
    } else {
        Some(lower)
    };
    if input.get(*p) != Some(&b'}') {
        return Err(PatternError::Unsupported(
            UnsupportedPattern::CountedRepetition,
        ));
    }
    *p += 1;
    if upper.is_some_and(|upper| lower > upper) {
        return Err(PatternError::InvalidSyntax);
    }
    Ok((lower, upper))
}

fn repetition_count(input: &[u8], p: &mut usize) -> Result<u16, PatternError> {
    let start = *p;
    let mut count = 0u16;
    while let Some(&byte @ b'0'..=b'9') = input.get(*p) {
        // The inspected C-locale runtime defines RE_DUP_MAX as 32767.
        count = count
            .checked_mul(10)
            .and_then(|count| count.checked_add(u16::from(byte - b'0')))
            .filter(|&count| count <= 32767)
            .ok_or(PatternError::Unsupported(
                UnsupportedPattern::CountedRepetition,
            ))?;
        *p += 1;
    }
    if start == *p {
        return Err(PatternError::Unsupported(
            UnsupportedPattern::CountedRepetition,
        ));
    }
    Ok(count)
}

type ByteSet = [bool; 256];

fn bracket(input: &[u8], p: &mut usize) -> Result<ByteSet, PatternError> {
    let negated = input.get(*p) == Some(&b'^');
    *p += usize::from(negated);
    let mut first = true;
    let mut set = [false; 256];
    loop {
        let byte = *input.get(*p).ok_or(PatternError::InvalidSyntax)?;
        // A valid middle hyphen is consumed with its preceding range endpoint.
        if byte == b'-' && !first && input.get(*p + 1) != Some(&b']') {
            return Err(PatternError::InvalidSyntax);
        }
        if byte == b']' && !first {
            *p += 1;
            if negated {
                set.iter_mut().for_each(|value| *value = !*value);
            }
            return Ok(set);
        }
        let (mut added, single) = bracket_item(input, p)?;
        if input.get(*p) == Some(&b'-') && input.get(*p + 1) != Some(&b']') {
            *p += 1;
            let (_, end) = bracket_item(input, p)?;
            let (Some(start), Some(end)) = (single, end) else {
                return Err(PatternError::Unsupported(UnsupportedPattern::BracketForm));
            };
            if start > end {
                return Err(PatternError::InvalidSyntax);
            }
            added[usize::from(start)..=usize::from(end)].fill(true);
        }
        for (included, added) in set.iter_mut().zip(added) {
            *included |= added;
        }
        first = false;
    }
}

fn bracket_item(input: &[u8], p: &mut usize) -> Result<(ByteSet, Option<u8>), PatternError> {
    let byte = *input.get(*p).ok_or(PatternError::InvalidSyntax)?;
    *p += 1;
    if byte == b'[' && matches!(input.get(*p), Some(b'.' | b'=')) {
        return Err(PatternError::Unsupported(UnsupportedPattern::BracketForm));
    }
    let mut set = [false; 256];
    if byte == b'[' && input.get(*p) == Some(&b':') {
        *p += 1;
        let start = *p;
        while *p + 1 < input.len() && &input[*p..*p + 2] != b":]" {
            *p += 1;
        }
        if *p + 1 >= input.len() {
            return Err(PatternError::InvalidSyntax);
        }
        let name = &input[start..*p];
        *p += 2;
        for (n, included) in set.iter_mut().enumerate() {
            let byte = n as u8;
            *included = match name {
                b"alnum" => byte.is_ascii_alphanumeric(),
                b"alpha" => byte.is_ascii_alphabetic(),
                b"blank" => matches!(byte, b' ' | b'\t'),
                b"cntrl" => byte.is_ascii_control(),
                b"digit" => byte.is_ascii_digit(),
                b"graph" => byte.is_ascii_graphic(),
                b"lower" => byte.is_ascii_lowercase(),
                b"print" => (b' '..=b'~').contains(&byte),
                b"punct" => byte.is_ascii_punctuation(),
                b"space" => matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | 11 | 12),
                b"upper" => byte.is_ascii_uppercase(),
                b"xdigit" => byte.is_ascii_hexdigit(),
                _ => return Err(PatternError::Unsupported(UnsupportedPattern::BracketForm)),
            };
        }
        Ok((set, None))
    } else {
        // Unlike an outside escape, backslash is an ordinary GNU bracket member.
        set[usize::from(byte)] = true;
        Ok((set, Some(byte)))
    }
}

fn emit_byte(out: &mut String, byte: u8) {
    write!(out, "\\x{byte:02X}").expect("writing to a String cannot fail");
}

fn emit_set(out: &mut String, set: &ByteSet) {
    // `[]` is no class to the engine; a bracket expression that excludes every byte
    // (`[^[:cntrl:] -\xff]`) matches nothing, as in the reference.
    if !set.contains(&true) {
        out.push_str("[^\\x00-\\xFF]");
        return;
    }
    out.push('[');
    let mut p = 0;
    while p < set.len() {
        if !set[p] {
            p += 1;
            continue;
        }
        let start = p;
        while p + 1 < set.len() && set[p + 1] {
            p += 1;
        }
        emit_byte(out, start as u8);
        if p > start {
            if p > start + 1 {
                out.push('-');
            }
            emit_byte(out, p as u8);
        }
        p += 1;
    }
    out.push(']');
}

#[cfg(test)]
mod tests {
    /// A name regex knows the prefixes every name it matches starts with, when there are
    /// a few, and a caseless one or one that can start anywhere knows none.
    #[test]
    fn name_regexes_know_their_prefixes() {
        use super::{CompiledPattern, IdentifierCase};
        let prefixes = |pattern: &str, case| {
            CompiledPattern::compile(pattern, case, crate::query::LIMITS)
                .expect("a pattern")
                .name_prefixes()
                .map(<[Vec<u8>]>::to_vec)
        };
        let sensitive = IdentifierCase::Sensitive;
        let bytes = |names: &[&str]| Some(names.iter().map(|n| n.as_bytes().to_vec()).collect());
        assert_eq!(
            prefixes("kmem_cache_.*", sensitive),
            bytes(&["kmem_cache_"])
        );
        assert_eq!(prefixes("^get|set.*$", sensitive), bytes(&["get", "set"]));
        assert_eq!(
            prefixes("(un)?lock.*", sensitive),
            bytes(&["lock", "unlock"])
        );
        assert_eq!(prefixes("ab|abc.*", sensitive), bytes(&["ab"]));
        assert_eq!(prefixes(".*_lock", sensitive), None);
        assert_eq!(prefixes("x*y", sensitive), bytes(&["x", "y"]));
        assert_eq!(prefixes("kmem.*", IdentifierCase::AsciiInsensitive), None);
        assert_eq!(prefixes("kmem", sensitive), None);
    }

    /// Searching a whole file finds exactly the lines that match one by one: with
    /// anchors, empty lines, matches that would run across line breaks, a set that holds
    /// the line feed, carriage returns, a last line without its line feed, and `-C`.
    #[test]
    fn a_whole_file_search_finds_the_lines_that_match_one_by_one() {
        use super::{CompiledPattern, IdentifierCase};
        let patterns: &[&[u8]] = &[
            b"^",
            b"$",
            b"^$",
            b"a$",
            b"^b",
            b"a*",
            b"x",
            b"ab",
            b"b$|^a",
            b"[\t-\r]a",
            b"a[\t-\r]",
            b"[^a]*",
            b"a.b",
            b"$a",
            b"b^",
            b"(a|b)+$",
            b"\r$",
            b"^ *$",
        ];
        let literals: &[&[u8]] = &[b"a", b"ab", b"b\r", b"A", b" "];
        // A small deterministic generator over the bytes that matter.
        let mut state = 0x2545_f491_u32;
        let mut contents: Vec<Vec<u8>> = vec![Vec::new(), b"\n".to_vec(), b"a".to_vec()];
        for _ in 0..400 {
            let length = (state % 24) as usize;
            let mut content = Vec::new();
            for _ in 0..length {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                content.push(b"ab\n\r xA"[(state % 7) as usize]);
            }
            contents.push(content);
        }
        for case in [IdentifierCase::Sensitive, IdentifierCase::AsciiInsensitive] {
            let compiled = patterns
                .iter()
                .map(|p| CompiledPattern::compile_regex_text(*p, case, crate::query::LIMITS))
                .chain(
                    literals
                        .iter()
                        .map(|p| CompiledPattern::compile_text(*p, case, crate::query::LIMITS)),
                );
            for pattern in compiled {
                let pattern = pattern.expect("a text pattern");
                for content in &contents {
                    let mut scratch = Vec::new();
                    let one_by_one: Vec<(usize, Vec<u8>)> = content
                        .split_inclusive(|b| *b == b'\n')
                        .enumerate()
                        .filter(|(_, line)| pattern.matches_line(line, &mut scratch))
                        .map(|(number, line)| (number + 1, line.to_vec()))
                        .collect();
                    let mut whole = Vec::new();
                    let Ok(()) = pattern.matching_lines(
                        content,
                        &mut scratch,
                        &mut Vec::new(),
                        |number, line| {
                            whole.push((number, line.to_vec()));
                            Ok::<(), std::convert::Infallible>(())
                        },
                    );
                    assert_eq!(whole, one_by_one, "{pattern:?} in {content:?}");
                }
            }
        }
    }

    /// An identifier pattern's trailing blanks, the C locale's (vertical tab and form feed
    /// included), are trimmed once; a blank inside is no identifier.
    #[test]
    fn identifier_patterns_lose_their_trailing_blanks() {
        let request = super::CompiledPattern::compile(
            "alpha \t\u{b}\u{c}",
            super::IdentifierCase::Sensitive,
            crate::query::LIMITS,
        )
        .expect("identifier");
        assert_eq!(request.exact_name(), Some(&b"alpha"[..]));
        assert_eq!(request.context(), b"alpha");
        assert!(matches!(
            super::CompiledPattern::compile(
                "al pha",
                super::IdentifierCase::Sensitive,
                crate::query::LIMITS
            ),
            Err(super::PatternError::InvalidIdentifier)
        ));
    }

    #[test]
    fn regex_text_has_qualified_boundaries_and_bounded_compilation() {
        let mut scratch = Vec::new();
        for (pattern, bytes, expected) in [
            ("(.a|$z)", b"a\n".as_slice(), false),
            ("(.a|$z)", b"z\n", true),
            ("(.a|$z)", b"ba\n", true),
            ("([^x]a|$z)", b"a", false),
            ("([^x]a|$z)", b"xa\n", false),
            ("([^x]a|$z)", b"ba\n", true),
            ("^$", b"\n", true),
            ("^$", b"a\n", false),
            ("a$", b"a", false),
            ("a$", b"a\r\n", false),
            ("a\r$", b"a\r\n", true),
            (".", b"\0\n", true),
            ("[^a]", b"\xff\n", true),
            ("café", "café\n".as_bytes(), true),
            ("É", "é\n".as_bytes(), false),
            ("[A-Z]", b"UPPER\n", true),
            ("a+?", b"b\n", true),
        ] {
            for case in [IdentifierCase::Sensitive, IdentifierCase::AsciiInsensitive] {
                let request = CompiledPattern::compile_regex_text(pattern, case, LIMITS).unwrap();
                assert_eq!(
                    request.matches_line(bytes, &mut scratch),
                    expected,
                    "{pattern:?}, {bytes:?}, {case:?}"
                );
            }
        }
        for pattern in [
            "", "a\n", "(", ")", "()", "a||b", "||a", "a||", "[a-]", "[", "\\", "*a",
        ] {
            assert!(
                CompiledPattern::compile_regex_text(pattern, IdentifierCase::Sensitive, LIMITS)
                    .is_err(),
                "{pattern:?}"
            );
        }
        let compile = |pattern: &str, limits| {
            CompiledPattern::compile_regex_text(pattern, IdentifierCase::Sensitive, limits)
                .map(|_| ())
        };
        assert!(compile(&"x".repeat(LIMITS.input_bytes), LIMITS).is_ok());
        for quantifier in ["*", "?", "+"] {
            assert!(matches!(
                compile(
                    &format!("a{}", quantifier.repeat(LIMITS.input_bytes - 1)),
                    LIMITS
                ),
                Err(PatternError::Resource(ResourceLimit::GroupDepth))
            ));
        }
        let shallow = PatternLimits {
            group_depth: 3,
            ..LIMITS
        };
        assert!(compile("(a*|b)+", shallow).is_ok());
        assert!(compile("a***b***", shallow).is_ok());
        for pattern in ["(a*|b)++", "((a**))", "(a***|b)", "(a***b)"] {
            assert!(
                matches!(
                    compile(pattern, shallow),
                    Err(PatternError::Resource(ResourceLimit::GroupDepth))
                ),
                "{pattern}"
            );
        }
        assert!(matches!(
            compile(&"x".repeat(LIMITS.input_bytes + 1), LIMITS),
            Err(PatternError::Resource(ResourceLimit::InputBytes))
        ));
        assert!(
            compile(
                "((a))",
                PatternLimits {
                    group_depth: 2,
                    ..LIMITS
                }
            )
            .is_ok()
        );
        assert!(matches!(
            compile(
                "(((a)))",
                PatternLimits {
                    group_depth: 2,
                    ..LIMITS
                }
            ),
            Err(PatternError::Resource(ResourceLimit::GroupDepth))
        ));
        assert!(matches!(
            compile(
                "(ab|cd)+",
                PatternLimits {
                    compiled_bytes: 1,
                    ..LIMITS
                }
            ),
            Err(PatternError::Resource(ResourceLimit::CompiledBytes))
        ));
    }

    /// Patterns are bytes, matched as cscope matches them in the C locale: any byte
    /// but NUL and a line break in fields 4 to 8, printable ASCII in the others.
    #[test]
    fn patterns_that_are_not_utf8_match_byte_for_byte() {
        let mut scratch = Vec::new();
        let path =
            CompiledPattern::compile_path(b"caf\xe9", IdentifierCase::Sensitive, LIMITS).unwrap();
        assert!(path.matches_name(b"src/caf\xe9.c", &mut scratch));
        assert!(!path.matches_name("src/café.c".as_bytes(), &mut scratch));
        // `-C` folds ASCII letters only.
        let folded =
            CompiledPattern::compile_path(b"CAF\xe9", IdentifierCase::AsciiInsensitive, LIMITS)
                .unwrap();
        assert!(folded.matches_name(b"caf\xe9.c", &mut scratch));
        assert!(!folded.matches_name(b"caf\xc9.c", &mut scratch));
        let text =
            CompiledPattern::compile_text(b"na\xefve", IdentifierCase::Sensitive, LIMITS).unwrap();
        assert!(text.matches_name(b"/* na\xefve */", &mut scratch));
        let egrep =
            CompiledPattern::compile_regex_text(b"na\xef(ve)+", IdentifierCase::Sensitive, LIMITS)
                .unwrap();
        assert!(egrep.matches_line(b"/* na\xefve */\n", &mut scratch));
        assert!(!egrep.matches_line(b"/* naive */\n", &mut scratch));
        // Bracket ranges are ranges of unsigned bytes, across 0x7F too, as the pinned
        // cscope reads them in field 6.
        for (pattern, line, expected) in [
            (&b"[\xe0-\xff]"[..], &b"caf\xe9\n"[..], true),
            (b"[\xe0-\xff]", b"cafe\n", false),
            (b"[\x7e-\x81]", b"\x80\n", true),
            (b"[\x7e-\x81]", b"~\n", true),
            (b"[\x7e-\x81]", b"\x82\n", false),
            ("[a-é]".as_bytes(), b"\xc3\n", true),
        ] {
            let request =
                CompiledPattern::compile_regex_text(pattern, IdentifierCase::Sensitive, LIMITS)
                    .unwrap();
            assert_eq!(
                request.matches_line(line, &mut scratch),
                expected,
                "{pattern:?}"
            );
        }
        for pattern in [&b"caf\xe9\nx"[..], b"caf\r\xe9"] {
            assert!(
                CompiledPattern::compile_path(pattern, IdentifierCase::Sensitive, LIMITS).is_err(),
                "{pattern:?}"
            );
        }
        assert!(matches!(
            CompiledPattern::compile(b"Al\xe9", IdentifierCase::Sensitive, LIMITS),
            Err(PatternError::Unsupported(UnsupportedPattern::Encoding))
        ));
        // The refusal names what the pattern may not hold.
        let refused = CompiledPattern::compile_path(b"a\rb", IdentifierCase::Sensitive, LIMITS)
            .map(|_| ())
            .expect_err("refused")
            .to_string();
        assert!(refused.contains("carriage return"), "{refused}");
    }

    /// A bracket expression that excludes every byte matches nothing, as in cscope, and
    /// leaves the rest of the pattern usable.
    #[test]
    fn a_bracket_of_no_byte_matches_nothing() {
        let mut scratch = Vec::new();
        let none =
            CompiledPattern::compile_path(b"[^[:cntrl:] -\xff]", IdentifierCase::Sensitive, LIMITS)
                .unwrap();
        assert!(!none.matches_name(b"a.c", &mut scratch));
        assert!(!none.matches_name(b"caf\xe9.c", &mut scratch));
        let either = CompiledPattern::compile_path(
            b"a|[^[:cntrl:] -\xff]",
            IdentifierCase::Sensitive,
            LIMITS,
        )
        .unwrap();
        assert!(either.matches_name(b"a.c", &mut scratch));
        let prefix = CompiledPattern::compile_path(
            b"x[^[:cntrl:] -\xff]*",
            IdentifierCase::Sensitive,
            LIMITS,
        )
        .unwrap();
        assert!(prefix.matches_name(b"x\xff.c", &mut scratch));
    }

    #[test]
    fn literal_text_preserves_bytes_and_only_folds_ascii() {
        let mut folded = Vec::new();
        let raw = CompiledPattern::compile_text("Needle", IdentifierCase::AsciiInsensitive, LIMITS)
            .unwrap();
        assert!(raw.matches_name(b"\xff\0NEEDLE\xfe", &mut folded));
        for (pattern, source, sensitive, insensitive) in [
            ("Needle", "needle", false, true),
            ("needle ", "needle", false, false),
            ("\t", "x\ty", true, true),
            ("\r", "x\r", true, true),
            (".*[\\^$+?|(){2}", "x.*[\\^$+?|(){2}y", true, true),
            (".*", "other", false, false),
            ("café", "xcafé", true, true),
            ("É", "é", false, false),
        ] {
            for (case, expected) in [
                (IdentifierCase::Sensitive, sensitive),
                (IdentifierCase::AsciiInsensitive, insensitive),
            ] {
                let request = CompiledPattern::compile_text(pattern, case, LIMITS).unwrap();
                assert_eq!(
                    request.matches_name(source.as_bytes(), &mut folded),
                    expected,
                    "{pattern:?}, {case:?}"
                );
            }
        }
        for pattern in ["", "a\nb"] {
            assert!(
                CompiledPattern::compile_text(pattern, IdentifierCase::Sensitive, LIMITS).is_err()
            );
        }
        let pattern = "x".repeat(LIMITS.input_bytes);
        assert!(CompiledPattern::compile_text(&pattern, IdentifierCase::Sensitive, LIMITS).is_ok());
        assert!(matches!(
            CompiledPattern::compile_text(&(pattern + "x"), IdentifierCase::Sensitive, LIMITS),
            Err(PatternError::Resource(ResourceLimit::InputBytes))
        ));
    }

    use super::*;

    const LIMITS: PatternLimits = PatternLimits {
        input_bytes: 4096,
        group_depth: 64,
        compiled_bytes: 1024 * 1024,
        dfa_cache_bytes: 2 * 1024 * 1024,
    };

    fn compile(pattern: &str) -> Result<CompiledPattern<'_>, PatternError> {
        CompiledPattern::compile(pattern, IdentifierCase::Sensitive, LIMITS)
    }

    #[test]
    fn counted_atoms_keep_zero_bounds_literals_and_resource_failures_distinct() {
        let cases: &[(&str, &[u8], &[u8])] = &[
            ("a{0}", b"", b"a"),
            ("a{0,0}", b"", b"a"),
            ("a{0}b", b"b", b"ab"),
            ("(a{0})b", b"b", b"ab"),
            ("a{0,}b", b"aaaab", b"ba"),
            ("a{1,3}", b"aaa", b"aaaa"),
            ("a{,2}", b"", b"aaa"),
            ("a{,2}", b"aa", b"aaa"),
            ("a{,0002}", b"aa", b"aaa"),
            ("a{,0}b", b"b", b"ab"),
            ("(a{,2})b", b"aab", b"aaab"),
            ("(a{2}){2}", b"aaaa", b"aa"),
            (r"\{2}", b"{2}", b""),
            ("[{a}]{2}", b"{}", b"a"),
        ];
        for (pattern, included, excluded) in cases {
            let request = compile(pattern).unwrap();
            let Matcher::Regex { compiled, .. } = request.matcher else {
                panic!("expected regex")
            };
            assert!(compiled.is_match(included), "{pattern}");
            assert!(!compiled.is_match(excluded), "{pattern}");
        }
        for pattern in [
            "a{32768}",
            "a{1,32768}",
            "a{65536}",
            "a{999999999999999999999}",
            "a{1",
            "a{1,",
            "a{1,2",
            "a{-1}",
            "a{1, 2}",
            "a{,}",
            "a{,32768}",
            "a{,999999999999999999999}",
            "a{,2",
        ] {
            assert!(
                matches!(
                    compile(pattern),
                    Err(PatternError::Unsupported(
                        UnsupportedPattern::CountedRepetition
                    ))
                ),
                "{pattern}"
            );
        }
        for pattern in ["a{2}*", "a{2}?", "a+{2}", "a{2}{3}", "a{,2}+", "a+{,2}"] {
            assert!(
                matches!(
                    compile(pattern),
                    Err(PatternError::Unsupported(
                        UnsupportedPattern::RepeatedQuantifier
                    ))
                ),
                "{pattern}"
            );
        }
        for pattern in ["a{2,1}", "({2}a)", "^{2}a", "{,2}a", "(^{,2}a)"] {
            assert!(
                matches!(compile(pattern), Err(PatternError::InvalidSyntax)),
                "{pattern}"
            );
        }
        let zeroes = format!("a{{{}2}}", "0".repeat(100));
        assert!(compile(&zeroes).is_ok());
        assert!(matches!(
            CompiledPattern::compile(
                &zeroes,
                IdentifierCase::Sensitive,
                PatternLimits {
                    input_bytes: 16,
                    ..LIMITS
                }
            ),
            Err(PatternError::Resource(ResourceLimit::InputBytes))
        ));
        assert!(matches!(
            CompiledPattern::compile(
                "(a{100}){100}",
                IdentifierCase::Sensitive,
                PatternLimits {
                    compiled_bytes: 1024,
                    ..LIMITS
                }
            ),
            Err(PatternError::Resource(ResourceLimit::CompiledBytes))
        ));
        let request = CompiledPattern::compile(
            "a{32767}",
            IdentifierCase::Sensitive,
            PatternLimits {
                compiled_bytes: 8 * 1024 * 1024,
                ..LIMITS
            },
        )
        .unwrap();
        let Matcher::Regex { compiled, .. } = request.matcher else {
            panic!("expected regex")
        };
        assert!(compiled.is_match(&vec![b'a'; 32767]));
        assert!(!compiled.is_match(&vec![b'a'; 32766]));
        assert!(!compiled.is_match(&vec![b'a'; 32768]));
    }

    #[test]
    fn compiler_distinguishes_profile_syntax_and_resource_failures() {
        for pattern in ["", " \t\u{b}\r\n"] {
            assert!(matches!(compile(pattern), Err(PatternError::Empty)));
        }
        for pattern in ["café", "(a\0)", "a\nb", "a\u{a0}"] {
            assert!(matches!(
                compile(pattern),
                Err(PatternError::Unsupported(UnsupportedPattern::Encoding))
            ));
        }
        for pattern in [
            "[",
            "[abc",
            "(a",
            "*a",
            "[z-a]",
            "(^*)",
            "[a-b-c]",
            "[a-z-0-9]",
        ] {
            assert!(
                matches!(compile(pattern), Err(PatternError::InvalidSyntax)),
                "{pattern:?}"
            );
        }
        for (pattern, expected) in [
            ("a{,}", UnsupportedPattern::CountedRepetition),
            ("a*?", UnsupportedPattern::RepeatedQuantifier),
            ("a|", UnsupportedPattern::EmptyBranch),
            ("()", UnsupportedPattern::EmptyBranch),
            ("(?i:a)", UnsupportedPattern::SpecialGroup),
            (r"(a)\1", UnsupportedPattern::Backreference),
            (r"a\b+", UnsupportedPattern::BoundaryEscape),
            ("[[.a.]]", UnsupportedPattern::BracketForm),
            ("[[=a=]]", UnsupportedPattern::BracketForm),
            ("[[:unknown:]]", UnsupportedPattern::BracketForm),
            ("[a-[:digit:]]", UnsupportedPattern::BracketForm),
            ("(a))|aa", UnsupportedPattern::ClosingParenthesis),
            ("aa|(a)\\", UnsupportedPattern::TrailingEscape),
            (r"Alpha\$", UnsupportedPattern::FinalDollarEscape),
            (r"Alpha|aa\\$", UnsupportedPattern::FinalDollarEscape),
        ] {
            assert!(
                matches!(compile(pattern), Err(PatternError::Unsupported(actual)) if actual == expected),
                "{pattern:?}"
            );
        }
        let limited = PatternLimits {
            input_bytes: 3,
            ..LIMITS
        };
        assert!(matches!(
            CompiledPattern::compile("a   ", IdentifierCase::Sensitive, limited),
            Err(PatternError::Resource(ResourceLimit::InputBytes))
        ));
        let limited = PatternLimits {
            group_depth: 1,
            ..LIMITS
        };
        assert!(CompiledPattern::compile("(a)", IdentifierCase::Sensitive, limited).is_ok());
        assert!(matches!(
            CompiledPattern::compile("((a))", IdentifierCase::Sensitive, limited),
            Err(PatternError::Resource(ResourceLimit::GroupDepth))
        ));
        let limited = PatternLimits {
            compiled_bytes: 0,
            ..LIMITS
        };
        assert!(matches!(
            CompiledPattern::compile("(a|b)+", IdentifierCase::Sensitive, limited),
            Err(PatternError::Resource(ResourceLimit::CompiledBytes))
        ));
        // The identifier branch does not consume regex compilation resources.
        let identifier =
            CompiledPattern::compile("Alpha", IdentifierCase::Sensitive, limited).unwrap();
        assert!(matches!(identifier.matcher, Matcher::Identifier(_)));
    }

    #[test]
    fn accepted_deep_groups_do_not_hit_an_unrelated_backend_default() {
        let pattern = format!("{}a{}", "(".repeat(300), ")".repeat(300));
        let request = CompiledPattern::compile(
            &pattern,
            IdentifierCase::Sensitive,
            PatternLimits {
                group_depth: 300,
                ..LIMITS
            },
        )
        .unwrap();
        assert!(request.matches_name(b"a", &mut Vec::new()));
        assert!(matches!(
            CompiledPattern::compile(
                &pattern,
                IdentifierCase::Sensitive,
                PatternLimits {
                    group_depth: 299,
                    ..LIMITS
                }
            ),
            Err(PatternError::Resource(ResourceLimit::GroupDepth))
        ));
    }

    #[test]
    fn byte_sets_and_generated_syntax_keep_c_locale_semantics() {
        // Test byte membership directly, including values no C identifier contains.
        let cases: &[(&str, &[u8], &[u8])] = &[
            ("([[:space:]])", b" \t\r\n\x0B\x0C", b"a\0\xA0"),
            ("([[:blank:]])", b" \t", b"\n\x0B"),
            ("([[:cntrl:]])", b"\0\n\x7F", b" \x80"),
            ("([[:print:]])", b" A~", b"\0\x7F\x80"),
            ("([[:graph:]])", b"!A~", b" \n\x80"),
            ("([[:punct:]])", b"!_~", b"A1 \x80"),
            ("([[:alpha:]])", b"AZaz", b"_0\xFF"),
            ("([[:alnum:]])", b"AZaz09", b"_ \xFF"),
            ("([[:lower:]])", b"az", b"AZ0"),
            ("([[:upper:]])", b"AZ", b"az0"),
            ("([[:digit:]])", b"09", b"aA_"),
            ("([[:xdigit:]])", b"09aAfF", b"gG_"),
            (r"(\w)", b"aA0_", b"! \xFF"),
            (r"(\W)", b"! \xFF", b"aA0_"),
            (r"(\s)", b" \x0B\n", b"a\xA0"),
            (r"(\S)", b"a\xA0", b" \x0B\n"),
            ("(.)", b"a\n\xFF", b"\0"),
            ("([^a])", b"b\xFF", b"a"),
            (r"([a\d])", b"a\\d", b"1"),
            ("([a&&b])", b"a&b", b"c"),
            ("([]a])", b"]a", b"b"),
            ("(])", b"]", b"a"),
            ("([a-b-])", b"ab-", b"c"),
            ("([-a-c])", b"abc-", b"d"),
            ("([--a])", b"-.09AZ_a", b"bz"),
        ];
        for (pattern, included, excluded) in cases {
            let request = compile(pattern).unwrap();
            let Matcher::Regex { compiled, .. } = request.matcher else {
                panic!("expected regex")
            };
            for byte in *included {
                assert!(compiled.is_match(&[*byte]), "{pattern}/{byte}");
            }
            for byte in *excluded {
                assert!(!compiled.is_match(&[*byte]), "{pattern}/{byte}");
            }
        }
    }
}
