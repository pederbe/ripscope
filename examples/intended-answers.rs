//! Adds intended answers under ripscope's own query semantics (ADR 0010) to
//! `tests/fixtures/intended-answers.json`, from the differences a test run recorded.
//!
//! Run the tests with `RIPSCOPE_RECORD_INTENDED=FILE`: each query whose answer differs
//! from its recorded one (a cscope capture or an approved reading) is appended to FILE
//! instead of failing. Then:
//!
//! ```sh
//! cargo run --example intended-answers -- target/debug/ripscope FILE
//! ```
//!
//! For each difference it rebuilds the query's files, reads them with `--dump-units`, and
//! asks the second implementation of the query rules (`examples/support/
//! query_semantics.rs`) for the own answer over the product's reading, with its enabled
//! Corrections, which must equal the product's answer. The recorded answer must be cscope's
//! sequential search over the baseline reading, without Corrections, or for an approved
//! reading (`*-approved.json`) over the product's reading. Each changed row is explained
//! by the rules of the sequential search, or by the Correction whose absence alone
//! (`--corrections enabled,-NAME`) restores the baseline's reading of that row, labelled
//! `correction:NAME`. The Corrections tried are those already named in the file and any
//! given with `--correction NAME`. An entry the second implementation does not reproduce
//! is refused. Existing entries are kept: review the diff before committing. `--check` in
//! place of FILE rechecks and re-attributes every entry and reports every failure.

use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

#[path = "support/captured_text.rs"]
#[allow(dead_code)]
mod captured_text;
#[path = "support/json_text.rs"]
#[allow(dead_code)]
mod json_text;
#[path = "support/query_semantics.rs"]
mod query_semantics;

use json_text::{FIXTURE, Json};
use query_semantics::{Pattern, RULES};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn main() -> ExitCode {
    match run() {
        Ok(added) => {
            println!("{added} entries added");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("intended answers: {error}");
            ExitCode::FAILURE
        }
    }
}

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// One recorded difference: the product's answer and the recorded one.
struct Difference {
    fixture: String,
    key: String,
    recorded: String,
    actual: String,
}

const USAGE: &str = "usage: intended-answers BINARY (RECORDS | --check) [--correction NAME]...";

fn run() -> Result<usize> {
    let mut arguments = std::env::args().skip(1);
    let mut positional = Vec::new();
    let mut named = Vec::new();
    while let Some(argument) = arguments.next() {
        if argument == "--correction" {
            named.push(arguments.next().ok_or(USAGE)?);
        } else {
            positional.push(argument);
        }
    }
    let [binary, records] = positional.as_slice() else {
        return Err(USAGE.into());
    };
    let binary = std::path::absolute(binary)?.canonicalize()?;
    let path = fixtures().join("intended-answers.json");
    let answers: Value = serde_json::from_slice(&fs::read(&path)?)?;
    let semantics = answers.get("semantics").cloned();
    if let Some(unknown) = answers
        .as_object()
        .ok_or("intended answers are not an object")?
        .keys()
        .find(|key| !["semantics", "fixtures"].contains(&key.as_str()))
    {
        return Err(format!("unexpected member {unknown:?} in intended answers").into());
    }
    let mut entries: BTreeMap<String, BTreeMap<String, Json>> = BTreeMap::new();
    for (fixture, keyed) in answers["fixtures"].as_object().ok_or("no fixtures")? {
        let slot = entries.entry(fixture.clone()).or_default();
        for (key, entry) in keyed
            .as_object()
            .ok_or("fixture entries are not an object")?
        {
            slot.insert(key.clone(), stored_entry(entry)?);
        }
    }
    // The Corrections that may explain a changed row: those already named in the file,
    // and any the caller names for a new difference.
    let mut corrections: Vec<String> = named;
    for change in answers["fixtures"]
        .as_object()
        .into_iter()
        .flat_map(|fixtures| fixtures.values())
        .filter_map(Value::as_object)
        .flat_map(|keyed| keyed.values())
        .filter_map(|entry| entry["changes"].as_array())
        .flatten()
    {
        for rule in change["rules"].as_array().into_iter().flatten() {
            if let Some(name) = rule
                .as_str()
                .and_then(|rule| rule.strip_prefix("correction:"))
                && !corrections.iter().any(|known| known == name)
            {
                corrections.push(name.to_owned());
            }
        }
    }
    corrections.sort();
    let mut added = 0;
    if records == "--check" {
        // Every entry is rechecked, and every failure is reported, before anything is written.
        let mut failures = Vec::new();
        for (fixture, keyed) in entries.iter_mut() {
            for (key, entry) in keyed.iter_mut() {
                let text = |name: &str| -> Result<String> {
                    Ok(entry
                        .get(name)
                        .and_then(|value| match value {
                            Json::Str(text) => Some(text.clone()),
                            _ => None,
                        })
                        .ok_or_else(|| format!("{fixture} {key}: no {name} answer"))?)
                };
                let difference = Difference {
                    fixture: fixture.clone(),
                    key: key.clone(),
                    recorded: text("recorded")?,
                    actual: text("intended")?,
                };
                match make_entry(&binary, &difference, &corrections) {
                    Ok(checked) => *entry = checked,
                    Err(error) => failures.push(error.to_string()),
                }
            }
        }
        if !failures.is_empty() {
            return Err(
                format!("{} entries fail:\n{}", failures.len(), failures.join("\n")).into(),
            );
        }
    } else {
        for line in fs::read_to_string(records)?.lines() {
            let record: Value = serde_json::from_str(line)?;
            let text = |name: &str| -> Result<String> {
                Ok(record[name]
                    .as_str()
                    .ok_or_else(|| format!("record without {name}: {line}"))?
                    .to_owned())
            };
            let difference = Difference {
                fixture: text("fixture")?,
                key: text("key")?,
                recorded: text("recorded")?,
                actual: text("actual")?,
            };
            let keyed = entries.entry(difference.fixture.clone()).or_default();
            if let Some(existing) = keyed.get(&difference.key) {
                if existing.get("intended") != Some(&Json::from(difference.actual.as_str())) {
                    return Err(format!(
                        "{} {}: the product no longer gives the intended answer",
                        difference.fixture, difference.key
                    )
                    .into());
                }
                continue;
            }
            let entry = make_entry(&binary, &difference, &corrections)?;
            keyed.insert(difference.key.clone(), entry);
            added += 1;
        }
    }
    let mut written = Json::object(Vec::<(String, Json)>::new());
    if let Some(semantics) = semantics {
        written.set("semantics", Json::from(semantics));
    }
    written.set(
        "fixtures",
        Json::object(
            entries
                .into_iter()
                .map(|(fixture, keyed)| (fixture, Json::object(keyed))),
        ),
    );
    fs::write(&path, written.text(FIXTURE) + "\n")?;
    Ok(added)
}

/// A kept entry, with its members in the order entries are written.
fn stored_entry(entry: &Value) -> Result<Json> {
    let members = entry.as_object().ok_or("an entry is not an object")?;
    let mut out = Json::object(Vec::<(String, Json)>::new());
    for name in ["recorded", "intended"] {
        if let Some(value) = members.get(name) {
            out.set(name, Json::from(value.clone()));
        }
    }
    if let Some(changes) = members.get("changes") {
        let changes = changes.as_array().ok_or("changes are not a list")?;
        let mut items = Vec::new();
        for change in changes {
            let fields = change.as_object().ok_or("a change is not an object")?;
            let mut item = Json::object(Vec::<(String, Json)>::new());
            for name in ["row", "change", "rules"] {
                if let Some(value) = fields.get(name) {
                    item.set(name, Json::from(value.clone()));
                }
            }
            for (name, value) in fields {
                if item.get(name).is_none() {
                    item.set(name, Json::from(value.clone()));
                }
            }
            items.push(item);
        }
        out.set("changes", Json::Array(items));
    }
    for (name, value) in members {
        if out.get(name).is_none() {
            out.set(name, Json::from(value.clone()));
        }
    }
    Ok(out)
}

fn json_file(path: &Path) -> Result<Value> {
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}

/// Each source file a query's index was built from, by name.
type Files = Vec<(String, Vec<u8>)>;

/// Latin-1 text as the bytes it stands for.
fn bytes_of(text: &str) -> Result<Vec<u8>> {
    text.chars()
        .map(|ch| u8::try_from(u32::from(ch)).map_err(|_| "text outside Latin-1".into()))
        .collect()
}

/// The files a query's index was built from, in database order, and the rest of the
/// key (the query).
fn sources(fixture: &str, key: &str) -> Result<(Files, String)> {
    let split = |key: &str| -> Result<(String, String)> {
        let (first, rest) = key
            .split_once(' ')
            .ok_or_else(|| format!("malformed key {key:?}"))?;
        Ok((first.to_owned(), rest.to_owned()))
    };
    if fixture == "queries" {
        let (probe, rest) = split(key)?;
        let directory = fixtures().join("queries").join(&probe);
        let expected = json_file(&directory.join("expected.json"))?;
        let mut files = Vec::new();
        for name in expected["files"].as_array().ok_or("probe without files")? {
            let name = name.as_str().ok_or("file name is not text")?;
            files.push((name.to_owned(), fs::read(directory.join(name))?));
        }
        return Ok((files, rest));
    }
    let data = json_file(&fixtures().join(fixture))?;
    let (capture, rest) = if data.get("cases").is_some() {
        let (case, rest) = split(key)?;
        let entry = &data["cases"][&case];
        if let Some(source) = entry.get("source") {
            let source = source.as_str().ok_or("case source is not text")?;
            return Ok((vec![("case.c".to_owned(), bytes_of(source)?)], rest));
        }
        (entry["0"].clone(), rest)
    } else if data.get("fields").is_some() {
        (data["fields"]["0"].clone(), key.to_owned())
    } else {
        (data, key.to_owned())
    };
    let files = capture["files"]
        .as_object()
        .ok_or("capture without files")?;
    let mut seeds: Vec<String> = match &capture["seeds"] {
        Value::Array(items) => items
            .iter()
            .filter_map(|item| item.as_str().map(str::to_owned))
            .collect(),
        Value::Object(members) => members.keys().cloned().collect(),
        _ => return Err("capture without seeds".into()),
    };
    seeds.sort();
    let mut rest_names: Vec<String> = files
        .keys()
        .filter(|name| !seeds.contains(name))
        .cloned()
        .collect();
    rest_names.sort();
    let mut out = Vec::new();
    for name in seeds.into_iter().chain(rest_names) {
        let text = files
            .get(&name)
            .and_then(Value::as_str)
            .ok_or_else(|| format!("missing capture file {name}"))?;
        out.push((name, bytes_of(text)?));
    }
    Ok((out, rest))
}

/// (field, pattern, caseless) from `-dL [-C] -N PATTERN` or `[-C ]NPATTERN`.
fn query_of(rest: &str) -> Result<(String, String, bool)> {
    let tokens: Vec<&str> = rest.split(' ').collect();
    for (at, token) in tokens.iter().enumerate() {
        let mut chars = token.chars();
        if chars.next() == Some('-')
            && chars.next().is_some_and(|digit| digit.is_ascii_digit())
            && chars.next().is_none()
        {
            return Ok((
                token[1..].to_owned(),
                tokens[at + 1..].join(" "),
                tokens[..at].contains(&"-C"),
            ));
        }
    }
    let caseless = rest.starts_with("-C ");
    let rest = if caseless { &rest[3..] } else { rest };
    let mut chars = rest.chars();
    let field = chars.next().ok_or("empty query")?.to_string();
    Ok((field, chars.as_str().to_owned(), caseless))
}

/// The units `--dump-units` reads from `files`, with the named Corrections or none.
fn units(
    binary: &Path,
    files: &[(String, Vec<u8>)],
    corrections: Option<&str>,
) -> Result<Vec<Value>> {
    let work = tempfile::tempdir()?;
    for (name, data) in files {
        let path = work.path().join(name);
        fs::create_dir_all(path.parent().ok_or("file without a directory")?)?;
        fs::write(path, data)?;
    }
    let mut listing = Vec::new();
    for (name, _) in files {
        listing.extend(bytes_of(name)?);
        listing.push(b'\n');
    }
    fs::write(work.path().join(".names"), listing)?;
    let mut command = Command::new(binary);
    command.arg("--dump-units");
    if let Some(names) = corrections {
        command.args(["--corrections", names]);
    }
    let done = command
        .arg("-i")
        .arg(work.path().join(".names"))
        .current_dir(work.path())
        .output()?;
    if !done.status.success() {
        return Err(format!(
            "{} --dump-units failed: {}",
            binary.display(),
            String::from_utf8_lossy(&done.stderr)
        )
        .into());
    }
    Ok(std::str::from_utf8(&done.stdout)?
        .lines()
        .map(serde_json::from_str)
        .collect::<std::result::Result<_, _>>()?)
}

/// Each row of an answer as (path, context, line).
fn located(answer: &str) -> Result<Vec<(String, String, u64)>> {
    let mut out = Vec::new();
    for row in captured_text::lines(answer) {
        let padded = format!("{row} ");
        let parts: Vec<&str> = padded.splitn(4, ' ').collect();
        let [path, context, line, _] = parts[..] else {
            return Err(format!("malformed row {row:?}").into());
        };
        out.push((path.to_owned(), context.to_owned(), line.parse()?));
    }
    Ok(out)
}

/// A row's place: path, context and line number.
type Located = (String, String, u64);

fn make_entry(binary: &Path, difference: &Difference, corrections: &[String]) -> Result<Json> {
    let (files, rest) = sources(&difference.fixture, &difference.key)?;
    let (field, pattern, caseless) = query_of(&rest)?;
    let pattern = Pattern::new(&pattern, caseless)?;
    // The product reads with its enabled Corrections. A recorded cscope capture comes
    // from the baseline reading, which has none. An approved reading (`*-approved.json`)
    // is a corrected one, so the product's reading reproduces it.
    let product = units(binary, &files, Some("enabled"))?;
    // The reading the recorded answer came from.
    let reference = if difference.fixture.ends_with("-approved.json") {
        product.clone()
    } else {
        units(binary, &files, None)?
    };
    let (intended, recorded) = (&difference.actual, &difference.recorded);
    // The second implementation must give the intended answer under the own semantics,
    // and the recorded one under the sequential search, row for row and text included.
    for (read, rules, want, name) in [
        (&product, &[][..], intended, "product"),
        (&reference, &RULES[..], recorded, "recorded answer"),
    ] {
        let shown = query_semantics::shown(&query_semantics::rows(read, &field, &pattern, rules)?);
        if shown != captured_text::lines(want) {
            return Err(format!(
                "{} {}: the second implementation gives {shown:?}, the {name} {:?}",
                difference.fixture,
                difference.key,
                captured_text::lines(want)
            )
            .into());
        }
    }
    let located_rows = |read: &[Value], rules: &[&str]| -> Result<Vec<Located>> {
        Ok(query_semantics::rows(read, &field, &pattern, rules)?
            .into_iter()
            .map(|(path, context, line, _)| (path, context, line))
            .collect())
    };
    let mut answers: BTreeMap<Vec<&str>, Vec<Located>> = BTreeMap::new();
    let mut under = |rules: Vec<&'static str>| -> Result<Vec<Located>> {
        if let Some(rows) = answers.get(&rules) {
            return Ok(rows.clone());
        }
        let rows = located_rows(&reference, &rules)?;
        answers.insert(rules, rows.clone());
        Ok(rows)
    };
    let mut combinations: Vec<Vec<Vec<&'static str>>> = vec![Vec::new(), Vec::new()];
    for (first, rule) in RULES.iter().enumerate() {
        combinations[0].push(vec![*rule]);
        for other in &RULES[first + 1..] {
            combinations[1].push(vec![*rule, *other]);
        }
    }
    // The single rules that explain the change, else the pairs that do together.
    let mut explain = |row: &Located, present: bool| -> Result<Vec<String>> {
        for size in &combinations {
            let mut found = Vec::new();
            for rules in size {
                if under(rules.clone())?.contains(row) == present {
                    found.push(rules.join("+"));
                }
            }
            if !found.is_empty() {
                return Ok(found);
            }
        }
        Ok(Vec::new())
    };
    // A row only a Correction adds or removes, which cannot happen for an approved reading: the
    // Corrections whose absence alone (`--corrections enabled,-NAME`) restores the
    // baseline's reading of it.
    let own_reference = located_rows(&reference, &[])?;
    let mut without: BTreeMap<String, Vec<Located>> = BTreeMap::new();
    let mut corrected = |row: &Located, present: bool| -> Result<Vec<String>> {
        let mut found = Vec::new();
        for name in corrections {
            if !without.contains_key(name) {
                let read = units(binary, &files, Some(&format!("enabled,-{name}")))?;
                without.insert(name.clone(), located_rows(&read, &[])?);
            }
            if without[name].contains(row) == present {
                found.push(format!("correction:{name}"));
            }
        }
        if found.is_empty() {
            return Err(format!(
                "{} {}: a Correction changes {row:?}; name it with --correction NAME",
                difference.fixture, difference.key
            )
            .into());
        }
        Ok(found)
    };
    let (rows_now, rows_then) = (located(intended)?, located(recorded)?);
    let mut changes = Vec::new();
    for (row, text) in rows_now.iter().zip(captured_text::lines(intended)) {
        if !rows_then.contains(row) {
            let rules = if own_reference.contains(row) {
                explain(row, false)?
            } else {
                corrected(row, false)?
            };
            changes.push(change(Json::from(text), "added", rules));
        }
    }
    for (row, text) in rows_then.iter().zip(captured_text::lines(recorded)) {
        if !rows_now.contains(row) {
            let rules = if own_reference.contains(row) {
                corrected(row, true)?
            } else {
                explain(row, true)?
            };
            changes.push(change(Json::from(text), "removed", rules));
        }
    }
    if changes.is_empty() {
        // Only the order changed: file-scope rows come first under both semantics, so a
        // row that changes context between file scope and a function moves.
        changes.push(change(Json::Null, "order", Vec::new()));
    }
    Ok(Json::object([
        ("recorded", Json::from(recorded.as_str())),
        ("intended", Json::from(intended.as_str())),
        ("changes", Json::Array(changes)),
    ]))
}

fn change(row: Json, kind: &str, rules: Vec<String>) -> Json {
    Json::object([
        ("row", row),
        ("change", Json::from(kind)),
        ("rules", Json::from(rules)),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queries_and_rows_are_read_as_the_keys_write_them() {
        assert_eq!(
            query_of("-dL -C -9 a b").unwrap(),
            ("9".to_owned(), "a b".to_owned(), true)
        );
        assert_eq!(
            query_of("-C 0x.*").unwrap(),
            ("0".to_owned(), "x.*".to_owned(), true)
        );
        assert_eq!(
            query_of("1main").unwrap(),
            ("1".to_owned(), "main".to_owned(), false)
        );
        assert_eq!(
            located("a.c f 3 int x;\nb.c <global> 4").unwrap(),
            [
                ("a.c".to_owned(), "f".to_owned(), 3),
                ("b.c".to_owned(), "<global>".to_owned(), 4)
            ]
        );
    }

    #[test]
    fn kept_entries_keep_their_written_order() {
        let entry = serde_json::json!({
            "changes": [{"rules": [], "row": null, "change": "order"}],
            "intended": "b", "recorded": "a"
        });
        assert_eq!(
            stored_entry(&entry).unwrap().text(json_text::LINE),
            "{\"recorded\": \"a\", \"intended\": \"b\", \"changes\": [{\"row\": null, \"change\": \"order\", \"rules\": []}]}"
        );
    }
}
