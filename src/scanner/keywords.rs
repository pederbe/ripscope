//! The reference's C keywords, recorded as compatibility facts (reading-c.md §7-§8).
//!
//! A keyword is never a symbol. Keywords with a display code are respaced in displayed
//! text; the code values match the reference's database so the per-file oracle can be
//! compared byte for byte.

/// How a respaced keyword is followed in displayed text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Follow {
    /// Displayed unchanged.
    Nothing,
    /// Displayed with exactly one blank after it.
    Blank,
    /// Displayed as the keyword, a blank and `(`.
    Parenthesis,
}

/// What a keyword says about the declaration being read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Role {
    /// `typedef`: names that follow the type become typedef names.
    Typedef,
    /// `extern`: nothing declared until the `;` is a definition.
    Extern,
    /// `enum`, `struct` and `union`, whose tags are read separately.
    Tag,
    /// Every other keyword: a type or statement word.
    Other,
}

pub(super) struct Keyword {
    pub word: &'static [u8],
    /// The reference's display code, for keywords it respaces.
    pub code: Option<u8>,
    pub follow: Follow,
    pub role: Role,
}

const fn respaced(word: &'static [u8], code: u8, follow: Follow) -> Keyword {
    Keyword {
        word,
        code: Some(code),
        follow,
        role: role_of(word),
    }
}

const fn plain(word: &'static [u8], follow: Follow) -> Keyword {
    Keyword {
        word,
        code: None,
        follow,
        role: role_of(word),
    }
}

const fn role_of(word: &[u8]) -> Role {
    match word {
        b"typedef" => Role::Typedef,
        b"extern" => Role::Extern,
        b"enum" | b"struct" | b"union" => Role::Tag,
        _ => Role::Other,
    }
}

use Follow::{Blank, Nothing, Parenthesis};

/// Every keyword, with the code and follow rule observed in the reference.
pub(super) const KEYWORDS: &[Keyword] = &[
    respaced(b"#define", 1, Blank),
    respaced(b"#include", 2, Blank),
    respaced(b"break", 3, Nothing),
    respaced(b"case", 4, Blank),
    respaced(b"char", 5, Blank),
    respaced(b"continue", 6, Nothing),
    respaced(b"default", 7, Nothing),
    respaced(b"double", 8, Blank),
    respaced(b"else", 11, Blank),
    respaced(b"enum", 12, Blank),
    respaced(b"extern", 13, Blank),
    respaced(b"float", 14, Blank),
    respaced(b"for", 15, Parenthesis),
    respaced(b"goto", 16, Blank),
    respaced(b"if", 17, Parenthesis),
    respaced(b"int", 18, Blank),
    respaced(b"long", 19, Blank),
    respaced(b"register", 20, Blank),
    respaced(b"return", 21, Nothing),
    respaced(b"short", 22, Blank),
    respaced(b"sizeof", 23, Nothing),
    respaced(b"static", 24, Blank),
    respaced(b"struct", 25, Blank),
    respaced(b"switch", 26, Parenthesis),
    respaced(b"typedef", 27, Blank),
    respaced(b"union", 28, Blank),
    respaced(b"unsigned", 29, Blank),
    respaced(b"void", 30, Blank),
    respaced(b"while", 31, Parenthesis),
    plain(b"do", Nothing),
    plain(b"auto", Blank),
    plain(b"fortran", Blank),
    plain(b"const", Blank),
    plain(b"signed", Blank),
    plain(b"volatile", Blank),
];

/// The keyword spelled `word`, if any. `#define` and `#include` are never identifiers.
pub(super) fn keyword(word: &[u8]) -> Option<&'static Keyword> {
    KEYWORDS.iter().find(|keyword| keyword.word == word)
}

/// A written builtin type, without storage classes or statement keywords.
pub(super) fn keyword_type(word: &[u8]) -> bool {
    matches!(
        word,
        b"void"
            | b"char"
            | b"short"
            | b"int"
            | b"long"
            | b"float"
            | b"double"
            | b"signed"
            | b"unsigned"
    )
}

/// The keyword with display `code`.
pub(super) fn by_code(code: u8) -> Option<&'static Keyword> {
    KEYWORDS.iter().find(|keyword| keyword.code == Some(code))
}

pub(super) const DEFINE_CODE: u8 = 1;
pub(super) const INCLUDE_CODE: u8 = 2;
