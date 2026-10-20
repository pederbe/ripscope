//! Answers ripscope gives by design where a recorded answer (a cscope capture or an approved
//! reading) differs: ripscope's own query semantics (ADR 0010, "Ripscope's own query
//! semantics give each name the context it sits in").
//!
//! `tests/fixtures/intended-answers.json` holds, per fixture and query, the recorded answer
//! and the intended one, each row that differs with the rules of cscope's sequential
//! search that explain it (`examples/support/query_semantics.rs`). A test compares the product
//! with [`expected`]. The recorded answer is kept beside the intended one, so a recapture
//! that changes it is noticed.
//! Scanner Corrections also have source-labelled added rows here, named explicitly
//! in their change rules, beside the untouched reference captures.
//!
//! With `RIPSCOPE_RECORD_INTENDED=FILE`, [`check`] appends each new difference to FILE as
//! a JSON line instead of failing, for the `intended-answers` example to check and add; a
//! query that already has an intended answer still fails when the product departs from it.

#![allow(dead_code)]

use serde_json::Value;
use std::io::Write;
use std::path::Path;
use std::sync::OnceLock;

fn answers() -> &'static Value {
    static ANSWERS: OnceLock<Value> = OnceLock::new();
    ANSWERS.get_or_init(|| {
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/intended-answers.json");
        serde_json::from_slice(&std::fs::read(path).expect("intended answers")).expect("JSON")
    })
}

/// What the product should answer for query `key` of `fixture`, whose recorded answer is
/// `recorded`.
pub fn expected(fixture: &str, key: &str, recorded: &str) -> String {
    let Some(entry) = answers()["fixtures"][fixture].get(key) else {
        return recorded.to_owned();
    };
    assert_eq!(
        entry["recorded"].as_str(),
        Some(recorded),
        "{fixture} {key}: the recorded answer changed; review its intended answer"
    );
    let intended = entry["intended"].as_str().expect("intended");
    assert_ne!(intended, recorded, "{fixture} {key}: no longer differs");
    intended.to_owned()
}

/// Asserts that `actual` is the expected answer for query `key` of `fixture`.
pub fn check(fixture: &str, key: &str, recorded: &str, actual: &str) {
    let want = expected(fixture, key, recorded);
    if actual == want {
        return;
    }
    let new = answers()["fixtures"][fixture].get(key).is_none();
    if let Some(file) = std::env::var_os("RIPSCOPE_RECORD_INTENDED").filter(|_| new) {
        let line = serde_json::json!({
            "fixture": fixture, "key": key, "recorded": recorded, "actual": actual
        });
        let mut out = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(file)
            .expect("record file");
        writeln!(out, "{line}").expect("record");
        return;
    }
    assert_eq!(actual, want, "{fixture} {key}");
}
