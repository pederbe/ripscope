//! Print a query's rows under the second implementation of the stored query semantics
//! (ADR 0010), with any of the sequential search's rules switched on.
//!
//! usage: cargo run --example query-semantics -- UNITS.jsonl FIELD PATTERN [-C] [RULE...]
//!
//! UNITS.jsonl holds units as `ripscope --dump-units` prints them. With no rules the rows
//! follow ripscope's own semantics; with every rule, cscope's sequential search.

use std::process::ExitCode;

#[path = "support/query_semantics.rs"]
mod query_semantics;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("query semantics: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let caseless = arguments.iter().any(|arg| arg == "-C");
    let arguments: Vec<&str> = arguments
        .iter()
        .map(String::as_str)
        .filter(|arg| *arg != "-C")
        .collect();
    let [units_path, field, pattern, rules @ ..] = arguments.as_slice() else {
        return Err("usage: query-semantics UNITS.jsonl FIELD PATTERN [-C] [RULE...]".into());
    };
    let units = std::fs::read_to_string(units_path)?
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(serde_json::from_str)
        .collect::<std::result::Result<Vec<serde_json::Value>, _>>()?;
    let pattern = query_semantics::Pattern::new(pattern, caseless)?;
    for row in query_semantics::shown(&query_semantics::rows(&units, field, &pattern, rules)?) {
        println!("{row}");
    }
    Ok(())
}
