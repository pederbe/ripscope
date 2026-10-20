//! Queries: a query field's pattern compiled with the product's limits, as a stored query
//! answered from the index revision or a live text query, which reads the files as they
//! are now. Patterns are a plain identifier, or cscope's bounded C-locale regular
//! expressions (`pattern`).
pub(crate) mod pattern;

use crate::engine::{self, Context, Field, Reads, Semantics, Source};
use pattern::{CompiledPattern, PatternError, PatternLimits};

/// The product's limits on a pattern: 4,096 bytes, as a session request's length allows.
pub(crate) const LIMITS: PatternLimits = PatternLimits {
    input_bytes: 4096,
    group_depth: 64,
    compiled_bytes: 1024 * 1024,
    dfa_cache_bytes: 2 * 1024 * 1024,
};

/// A query ready to answer.
pub(crate) enum Query<'p> {
    /// A stored query: fields 0 to 3, 7, 8 and 9.
    Stored(StoredQuery<'p>),
    /// A live text query: fields 4, 5 and 6.
    Text(TextQuery<'p>),
}

/// Why a query cannot be answered at all. The wording, and any limit of a transport, are
/// the transport's.
#[derive(Debug)]
pub(crate) enum Refusal {
    /// The pattern holds a NUL byte.
    Nul,
    /// The field is not one of cscope's.
    NoField,
    /// The pattern cannot be compiled for its field.
    Pattern(PatternError),
}

impl<'p> Query<'p> {
    /// Compiles `pattern` for query field `field` (0 to 9). A NUL byte is refused before the
    /// field is looked at.
    pub(crate) fn compile(
        field: u8,
        pattern: &'p [u8],
        case: IdentifierCase,
    ) -> Result<Self, Refusal> {
        if pattern.contains(&0) {
            return Err(Refusal::Nul);
        }
        Ok(match field {
            // In line mode cscope answers field 5, "change this text", as a text search.
            4 | 5 => Self::Text(TextQuery(
                CompiledPattern::compile_text(pattern, case, LIMITS).map_err(Refusal::Pattern)?,
            )),
            6 => Self::Text(TextQuery(
                CompiledPattern::compile_regex_text(pattern, case, LIMITS)
                    .map_err(Refusal::Pattern)?,
            )),
            _ => {
                let field = Field::from_digit(b'0' + field).ok_or(Refusal::NoField)?;
                Self::Stored(StoredQuery::compile(field, pattern, case)?)
            }
        })
    }
}

/// A stored query: a field answered from the Analysis units.
pub(crate) struct StoredQuery<'p> {
    field: Field,
    request: CompiledPattern<'p>,
}

impl<'p> StoredQuery<'p> {
    /// Compiles `pattern` for the stored field `field`. A pattern holding a NUL is
    /// refused before, by [`Query::compile`].
    pub(crate) fn compile(
        field: Field,
        pattern: &'p [u8],
        case: IdentifierCase,
    ) -> Result<Self, Refusal> {
        let request = match field {
            // Paths print raw, so their patterns may hold any byte but NUL, a line break or a
            // carriage return, as cscope's do; includes share the file-name grammar.
            Field::Files | Field::Includers => CompiledPattern::compile_path(pattern, case, LIMITS),
            _ => CompiledPattern::compile(pattern, case, LIMITS),
        }
        .map_err(Refusal::Pattern)?;
        Ok(Self { field, request })
    }

    pub(crate) fn field(&self) -> Field {
        self.field
    }

    /// The pattern as field 1 shows it in each row's context column, as cscope shows it:
    /// trailing blanks trimmed and, in a regular expression matched with case, its `^` and
    /// `$` anchors dropped (`^Alph.$` shows `Alph.`, with `-C` `^Alph.$`).
    pub(crate) fn shown_pattern(&self) -> &[u8] {
        self.request.context()
    }

    /// What the query reads with `semantics`: a source need give it no other file.
    pub(crate) fn reads(&self, semantics: Semantics) -> Reads<'_> {
        engine::reads(semantics, self.field, &self.request)
    }

    /// The query's Result rows from `source` with `semantics`, in cscope's order.
    pub(crate) fn answer<'a>(
        &'a self,
        semantics: Semantics,
        source: &'a impl Source,
    ) -> impl Iterator<Item = ResultRow<'a>> + 'a {
        engine::answer(semantics, self.field, &self.request, source)
            .into_iter()
            .map(move |row| ResultRow {
                at_file_scope: row.at_file_scope(),
                path: row.path,
                context: match row.context {
                    Context::Global => b"<global>",
                    Context::Name(name) => name,
                    Context::Pattern => self.request.context(),
                    Context::Unknown => engine::UNKNOWN,
                },
                line: row.line,
                text: row.text,
            })
    }
}

/// A live text query: a line pattern for the files as they are now.
pub(crate) struct TextQuery<'p>(CompiledPattern<'p>);

impl TextQuery<'_> {
    /// The lines of `content`, each with its line end, that match, in order, with their
    /// numbers from 1.
    pub(crate) fn matching_lines<E>(
        &self,
        content: &[u8],
        scratch: &mut Vec<u8>,
        folded: &mut Vec<u8>,
        each: impl FnMut(usize, &[u8]) -> Result<(), E>,
    ) -> Result<(), E> {
        self.0.matching_lines(content, scratch, folded, each)
    }
}

/// One answer of a stored query, before any transport writes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ResultRow<'a> {
    /// Whether the row is at the file scope (`<global>`): an answer gives those rows
    /// first, each group in file order.
    pub at_file_scope: bool,
    /// The indexed path, as displayed.
    pub path: &'a [u8],
    /// The context column: `<global>`, the enclosing function or macro (for field 2 the
    /// function called), field 1's pattern, or field 7's `<unknown>`.
    pub context: &'a [u8],
    pub line: u32,
    /// The displayed text, before any transport escaping.
    pub text: &'a [u8],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum IdentifierCase {
    Sensitive,
    AsciiInsensitive,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum IdentifierRequestError {
    Empty,
    OutsideIdentifierSubset,
}

/// A pattern that is a plain identifier (its blanks already trimmed): matched as a whole
/// name.
#[derive(Debug)]
pub(crate) struct IdentifierPattern<'request> {
    context: &'request [u8],
    case: IdentifierCase,
}

impl<'request> IdentifierPattern<'request> {
    pub fn new(name: &'request [u8], case: IdentifierCase) -> Result<Self, IdentifierRequestError> {
        let Some(first) = name.first() else {
            return Err(IdentifierRequestError::Empty);
        };
        if !(first.is_ascii_alphabetic() || *first == b'_')
            || !name
                .iter()
                .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
        {
            return Err(IdentifierRequestError::OutsideIdentifierSubset);
        }
        Ok(Self {
            context: name,
            case,
        })
    }

    pub(crate) fn matches_bytes(&self, name: &[u8]) -> bool {
        match self.case {
            IdentifierCase::Sensitive => name == self.context,
            IdentifierCase::AsciiInsensitive => name.eq_ignore_ascii_case(self.context),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifier_requests_take_only_identifiers() {
        let request =
            IdentifierPattern::new(b"alpha", IdentifierCase::Sensitive).expect("identifier");
        assert!(request.matches_bytes(b"alpha"));
        assert!(!request.matches_bytes(b"Alpha"));
        let caseless =
            IdentifierPattern::new(b"alpha", IdentifierCase::AsciiInsensitive).expect("identifier");
        assert!(caseless.matches_bytes(b"ALPHA"));
        for (pattern, error) in [
            (&b""[..], IdentifierRequestError::Empty),
            (b"alpha ", IdentifierRequestError::OutsideIdentifierSubset),
            (b"9lives", IdentifierRequestError::OutsideIdentifierSubset),
            (b"a-b", IdentifierRequestError::OutsideIdentifierSubset),
        ] {
            assert_eq!(
                IdentifierPattern::new(pattern, IdentifierCase::Sensitive).unwrap_err(),
                error,
                "{pattern:?}"
            );
        }
    }

    /// A pattern holding a NUL is refused whatever its field, before the field is looked
    /// at: even field 10, which there is none of.
    #[test]
    fn a_nul_in_any_pattern_is_refused_before_its_field_is_looked_at() {
        for field in 0..=10 {
            for pattern in [&b"\0"[..], b"Alpha\0", b"\0Alpha", b"Al\0pha", b"a.c\0"] {
                for case in [IdentifierCase::Sensitive, IdentifierCase::AsciiInsensitive] {
                    assert!(
                        matches!(Query::compile(field, pattern, case), Err(Refusal::Nul)),
                        "{field}, {pattern:?}, {case:?}"
                    );
                }
            }
        }
    }
}
