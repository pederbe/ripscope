//! Conservative evidence for opaque words at a declaration boundary.
//!
//! One word cannot name a written C declaration by itself. Two words can be a
//! typedef-name and its declarator, even on separate lines, so that shape stays
//! unchanged. Longer runs can continue a global declaration through a directive.
//! At a proved struct/union close, the opaque-member-tails reading also covers
//! longer word runs and a numeric invocation before one trailing word. That bound
//! is deliberately ambiguous for macro-expanded types missing a final semicolon.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Tail {
    words: u8,
    invocation: bool,
    blocked: bool,
}

impl Tail {
    pub(super) fn word(&mut self, declaration: bool) {
        self.words = self.words.saturating_add(1);
        self.blocked |= declaration;
    }

    pub(super) fn block(&mut self) {
        self.blocked = true;
    }

    pub(super) fn invocation(&mut self) {
        self.blocked |= self.words == 0;
        self.invocation = true;
    }

    pub(super) fn before_directive(self) -> bool {
        !self.blocked && !self.invocation && self.words == 1
    }

    pub(super) fn before_member_end(self) -> bool {
        !self.blocked
            && (self.words == 1
                || (!self.invocation && self.words >= 3)
                || (self.invocation && self.words == 2))
    }

    pub(super) fn merge(self, other: Self) -> Self {
        if self == other {
            self
        } else {
            Self {
                blocked: true,
                ..Self::default()
            }
        }
    }
}

/// Declaration evidence is broader than the reference's keyword/display table.
/// Reserved implementation words are left alone: an unfamiliar extension can be
/// a type or qualifier, and its spelling does not prove that it is a macro tail.
pub(super) fn declaration_word(word: &[u8]) -> bool {
    super::keywords::keyword(word).is_some()
        || word.starts_with(b"__")
        || matches!(
            word,
            b"_Alignas"
                | b"_Alignof"
                | b"_Atomic"
                | b"_BitInt"
                | b"_Bool"
                | b"_Complex"
                | b"_Decimal32"
                | b"_Decimal64"
                | b"_Decimal128"
                | b"_Imaginary"
                | b"_Noreturn"
                | b"_Thread_local"
                | b"alignas"
                | b"alignof"
                | b"bool"
                | b"constexpr"
                | b"inline"
                | b"restrict"
                | b"thread_local"
                | b"typeof"
                | b"typeof_unqual"
        )
}
