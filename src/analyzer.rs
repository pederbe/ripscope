//! The Analyzer (GLOSSARY.md): reads a captured source into its Analysis unit, with the
//! Corrections it is given, and names itself, so an index knows which analyses it holds
//! and a change to the reading analyses every file again.

use crate::scanner::{self, Corrections, Mode, Record};
use crate::unit::{AnalysisUnit, UnitError};
use sha2::{Digest, Sha256};

/// The scanner, with the Corrections it reads with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Analyzer {
    corrections: Corrections,
}

impl Analyzer {
    /// The product's: every enabled Correction.
    pub(crate) const PRODUCT: Self = Self::with(Corrections::ENABLED);

    /// Reading with `corrections`: none is the reference reading the compass compares with
    /// cscope.
    pub(crate) const fn with(corrections: Corrections) -> Self {
        Self { corrections }
    }

    /// Its identity, stored with the index and part of each unit's key: a different
    /// scanner version, Correction set or image version analyses every file again.
    pub(crate) fn identity(self) -> Vec<u8> {
        format!(
            "ripscope-scanner-{}+{}/unit-{}",
            scanner::VERSION,
            self.corrections.names(),
            crate::unit::IMAGE_VERSION
        )
        .into_bytes()
    }

    /// Whether this version reads the units of the analyzer named `identity`: its name ends
    /// in the unit image's version.
    pub(crate) fn reads_units_of(identity: &[u8]) -> bool {
        identity.ends_with(format!("/unit-{}", crate::unit::IMAGE_VERSION).as_bytes())
    }

    /// The scanner's records for the source `bytes` of the file named `name`, whose
    /// suffix chooses how it is read.
    pub(crate) fn records(self, name: &[u8], bytes: &[u8]) -> Vec<Record> {
        scanner::scan_with(bytes, Mode::for_name(name), self.corrections)
    }

    /// The Analysis unit of the source `bytes` of the file named `name`.
    pub(crate) fn analyse(self, name: &[u8], bytes: &[u8]) -> Result<AnalysisUnit, UnitError> {
        AnalysisUnit::from_records(&self.records(name, bytes))
    }

    /// The key an index stores the unit of a file named `name`, whose bytes hash to
    /// `hash`, under: files whose bytes, reading and analyzer match share one unit.
    pub(crate) fn key(self, name: &[u8], hash: &[u8; 32]) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(self.identity());
        hasher.update(format!("{:?}", Mode::for_name(name)).as_bytes());
        hasher.update(hash);
        hasher.finalize().into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A change to the identity or to a key makes every index analyse every file again,
    /// so both are pinned; a new scanner version or Correction set updates them here.
    #[test]
    fn the_identity_and_a_unit_key_are_pinned() {
        assert_eq!(
            String::from_utf8(Analyzer::PRODUCT.identity()).unwrap(),
            "ripscope-scanner-38+linkage-blocks,definition-bodies,directive-continuations,\
             crlf-continuations,unspaced-conditionals,macro-bodies,initializer-entries,\
             definition-heads,macro-line-comments,pointer-declarators,parenthesised-typedefs,\
             typedef-qualifiers,extern-definitions,typedef-parameters,attribute-expressions,\
             conditional-comments,keyword-macros,bare-directive-words,opaque-member-tails,\
             typedef-decorations,dereferenced-calls,c11-expression-keywords,atomic-types,\
             preprocessor-defined,macro-function-heads,subscript-assignments,named-mutations,split-calls,closing-arms,written-head-names,conditional-lists,alternative-bodies/unit-1"
        );
        assert!(Analyzer::reads_units_of(&Analyzer::PRODUCT.identity()));
        assert!(Analyzer::reads_units_of(
            &Analyzer::with(Corrections::default()).identity()
        ));
        assert!(!Analyzer::reads_units_of(b"ripscope-scanner-9+/unit-99"));
        let hash: [u8; 32] = Sha256::digest(b"int Alpha;\n").into();
        let key: String = Analyzer::PRODUCT
            .key(b"a.c", &hash)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        assert_eq!(
            key,
            "b2fb1e44f3d4fb0be01ed15df28025f5d83308ed461c6a96787e222bf78403fb"
        );
    }

    #[test]
    fn the_name_chooses_the_reading_and_the_key() {
        let hash = [0; 32];
        assert_ne!(
            Analyzer::PRODUCT.key(b"a.c", &hash),
            Analyzer::PRODUCT.key(b"a.y", &hash)
        );
        assert_eq!(
            Analyzer::PRODUCT.key(b"a.c", &hash),
            Analyzer::PRODUCT.key(b"b.h", &hash)
        );
        assert_ne!(
            Analyzer::PRODUCT.identity(),
            Analyzer::with(Corrections::default()).identity()
        );
        let unit = Analyzer::PRODUCT.analyse(b"a.c", b"int Alpha;\n").unwrap();
        assert!(
            unit == AnalysisUnit::from_records(&Analyzer::PRODUCT.records(b"a.c", b"int Alpha;\n"))
                .unwrap()
        );
    }
}
