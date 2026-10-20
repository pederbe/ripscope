//! Source-labelled definition and call assessment through the stored CLI.
//! See scripts/navigation-assessment.md for the manifest and counting rules.

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[path = "support/capture.rs"]
mod capture;
#[path = "support/source_inventory.rs"]
mod source_inventory;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn string(value: &Value) -> Result<&str> {
    value
        .as_str()
        .ok_or_else(|| invalid("expected string").into())
}

fn array(value: &Value) -> Result<&Vec<Value>> {
    value
        .as_array()
        .ok_or_else(|| invalid("expected array").into())
}

fn number(value: &Value) -> Result<u64> {
    value
        .as_u64()
        .ok_or_else(|| invalid("expected unsigned number").into())
}

fn read(path: &Path) -> Result<Value> {
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}

fn write(path: &Path, value: &Value) -> Result<()> {
    fs::write(path, serde_json::to_vec_pretty(value)?)?;
    Ok(())
}

fn digest(path: &Path) -> Result<String> {
    Ok(format!("{:x}", Sha256::digest(fs::read(path)?)))
}

fn source_path(root: &Path, name: &str) -> Result<PathBuf> {
    let path = Path::new(name);
    let relative = if path.is_absolute() {
        path.strip_prefix(root)
            .map_err(|_| invalid("source escapes the pinned tree"))?
    } else {
        path
    };
    if relative
        .components()
        .any(|component| !matches!(component, std::path::Component::Normal(_)))
        || name.contains(['\n', '\r', '"'])
        || name.is_empty()
    {
        return Err(invalid("source paths must be relative without parent components").into());
    }
    Ok(root.join(path))
}

/// Freeze hand-written labels and all selected bytes before querying the product.
fn seal(root: &Path, draft: &Path, destination: &Path) -> Result<()> {
    let mut manifest = read(draft)?;
    let mut sources = Vec::new();
    let mut lengths = BTreeMap::new();
    let assessed = array(&manifest["files"])?;
    let index_files = manifest.get("index_files").unwrap_or(&manifest["files"]);
    if assessed
        .iter()
        .any(|file| !array(index_files).is_ok_and(|items| items.contains(file)))
    {
        return Err(invalid("assessed files must belong to the indexed selection").into());
    }
    let names = array(index_files)?
        .iter()
        .chain(assessed)
        .chain(
            manifest
                .get("closure_files")
                .and_then(Value::as_array)
                .into_iter()
                .flatten(),
        )
        .map(string)
        .collect::<Result<BTreeSet<_>>>()?;
    for file in names {
        let path = source_path(root, file)?;
        if !fs::canonicalize(&path)?.starts_with(fs::canonicalize(root)?) {
            return Err(invalid("source escapes the pinned tree").into());
        }
        let bytes = fs::read(&path)?;
        let lines = bytes.split(|byte| *byte == b'\n').count() as u64;
        if assessed.iter().any(|value| value.as_str() == Some(file)) {
            lengths.insert(file.to_owned(), lines);
        }
        sources.push(source_inventory::freeze(root, file)?);
    }
    manifest["source_inventory_version"] = json!(3);
    manifest["unsupported_inputs"] = json!(source_inventory::preflight(
        root,
        &sources
            .iter()
            .map(|source| string(&source["path"]))
            .collect::<Result<Vec<_>>>()?,
        &manifest["include_directories"]
            .as_array()
            .into_iter()
            .flatten()
            .map(string)
            .collect::<Result<Vec<_>>>()?
    )?);
    let whole = manifest["whole_files"].as_bool().unwrap_or(false);
    let mut scopes = BTreeMap::<u64, Vec<Value>>::new();
    let mut queries = BTreeMap::<(u64, String), Value>::new();
    for (kind, fields) in [("definitions", &[1][..]), ("calls", &[2, 3][..])] {
        for fact in array(&manifest[kind])? {
            let fact = array(fact)?;
            let file = string(&fact[0])?;
            let line = number(&fact[1])?;
            let length = *lengths
                .get(file)
                .ok_or_else(|| invalid("unselected label file"))?;
            if line == 0 || line > length {
                return Err(invalid("label line outside source").into());
            }
            let scope = if whole {
                json!([file, 1, length])
            } else {
                json!([file, line, line])
            };
            for field in fields {
                let (pattern, context, construct) = match field {
                    1 => (string(&fact[2])?, string(&fact[2])?, string(&fact[3])?),
                    2 => (string(&fact[2])?, string(&fact[3])?, string(&fact[4])?),
                    _ => (string(&fact[3])?, string(&fact[2])?, string(&fact[4])?),
                };
                scopes.entry(*field).or_default().push(scope.clone());
                let query = queries.entry((*field, pattern.to_owned())).or_insert_with(|| {
                    json!({"field": field, "pattern": pattern, "expected": [], "construct": construct})
                });
                let expected = query["expected"].as_array_mut().expect("created array");
                let row = json!([file, context, line, construct]);
                if !expected
                    .iter()
                    .any(|other| (0..3).all(|at| other[at] == row[at]))
                {
                    expected.push(row);
                }
            }
        }
    }
    // Empty-answer controls use the same independently labelled scope as positives.
    for control in array(&manifest["controls"])? {
        let field = number(&control[0])?;
        let pattern = string(&control[1])?;
        if queries.contains_key(&(field, pattern.to_owned())) {
            return Err(invalid("control conflicts with positive query").into());
        }
        queries.insert((field, pattern.to_owned()), json!({
            "field": field, "pattern": pattern, "construct": string(&control[2])?, "expected": []
        }));
    }
    for item in manifest
        .get("query_inventory")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let field = number(&item[0])?;
        if field > 9 {
            return Err(invalid("invalid inventory field").into());
        }
        let pattern = string(&item[1])?;
        queries
            .entry((field, pattern.to_owned()))
            .or_insert_with(|| {
                json!({
                    "field":field,"pattern":pattern,"construct":item[2],"expected":[]
                })
            });
    }
    for query in manifest
        .get("extra_queries")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let field = number(&query["field"])?;
        let pattern = string(&query["pattern"])?;
        if field > 9 || !query["expected"].is_array() {
            return Err(invalid("invalid explicit query").into());
        }
        if queries
            .insert((field, pattern.to_owned()), query.clone())
            .is_some()
        {
            return Err(invalid("explicit query duplicates another query").into());
        }
    }
    let unscored = manifest.get("unscored_queries").and_then(Value::as_array);
    if manifest["unresolved"]
        .as_array()
        .is_some_and(|items| !items.is_empty())
        && unscored.is_none()
    {
        return Err(invalid("unresolved labels require explicit unscored_queries").into());
    }
    for item in unscored.into_iter().flatten() {
        let key = (number(&item[0])?, string(&item[1])?.to_owned());
        if key.0 > 9 || string(&item[2])?.is_empty() {
            return Err(invalid("invalid unscored query key or reason").into());
        }
        let query = queries.entry(key.clone()).or_insert_with(|| {
            json!({
                "field":key.0,"pattern":key.1,"construct":"unresolved","expected":[]
            })
        });
        query["unscored_reason"] = json!(string(&item[2])?);
    }
    for unresolved in manifest["unresolved"].as_array().into_iter().flatten() {
        let keys = array(&unresolved["queries"])?;
        if keys.is_empty()
            || keys.iter().any(|key| {
                !queries
                    .get(&(
                        key[0].as_u64().unwrap_or(u64::MAX),
                        key[1].as_str().unwrap_or("").into(),
                    ))
                    .is_some_and(|query| query["unscored_reason"].is_string())
            })
        {
            return Err(invalid("each unresolved label must map to an unscored query").into());
        }
    }
    if whole {
        for field in 0..=9 {
            scopes.insert(
                field,
                lengths
                    .iter()
                    .map(|(file, length)| json!([file, 1, length]))
                    .collect(),
            );
        }
    }
    for ((field, _), query) in &mut queries {
        query["expected"].as_array_mut().unwrap().sort_by(|a, b| {
            a[0].as_str()
                .cmp(&b[0].as_str())
                .then_with(|| a[2].as_u64().cmp(&b[2].as_u64()))
                .then_with(|| a[1].as_str().cmp(&b[1].as_str()))
        });
        let ranges = scopes.entry(*field).or_default();
        ranges.sort_by_key(Value::to_string);
        ranges.dedup();
        if !query["scopes"].is_array() {
            query["scopes"] = json!(ranges);
        }
    }
    manifest["sources"] = json!(sources);
    manifest["queries"] = json!(queries.into_values().collect::<Vec<_>>());
    write(destination, &manifest)
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Row {
    path: String,
    context: String,
    line: u64,
}

impl Row {
    fn parse(value: &Value) -> Result<Self> {
        if array(value)?.len() < 3 {
            return Err(invalid("short row").into());
        }
        Ok(Self {
            path: string(&value[0])?.replace('\\', "/"),
            context: string(&value[1])?.to_owned(),
            line: number(&value[2])?,
        })
    }
}

fn in_scope(row: &Row, scopes: &[Value]) -> bool {
    scopes.iter().any(|scope| {
        scope[0].as_str() == Some(row.path.as_str())
            && scope[1].as_u64().is_some_and(|start| row.line >= start)
            && scope[2].as_u64().is_some_and(|end| row.line <= end)
    })
}

/// Classify only file/line-labelled answers; context never controls admission.
fn score(query: &Value, actual: &[Value]) -> Result<Value> {
    if query["unscored_reason"].is_string() {
        return Ok(json!({"unscored":actual,"reason":query["unscored_reason"]}));
    }
    let scopes = array(&query["scopes"])?;
    let mut assessed = Vec::new();
    let mut unassessed = Vec::new();
    for value in actual {
        let row = Row::parse(value)?;
        if query["field"] == 7 || in_scope(&row, scopes) {
            assessed.push((row, value.clone()));
        } else {
            unassessed.push(value.clone());
        }
    }
    let mut remaining = Vec::new();
    let mut outcomes = Vec::new();
    // Exact matches first: an erroneous answer must not consume a correct one.
    for value in array(&query["expected"])? {
        let expected = Row::parse(value)?;
        if !in_scope(&expected, scopes) {
            return Err(invalid("expected answer outside labelled scope").into());
        }
        if let Some(at) = assessed.iter().position(|(row, _)| *row == expected) {
            let (_, actual) = assessed.remove(at);
            let outcome = if query["match_text"] == true && value.get(4) != actual.get(3) {
                "wrong_text"
            } else {
                "correct"
            };
            outcomes.push(json!({"outcome": outcome, "expected": value, "actual": actual}));
        } else {
            remaining.push((expected, value));
        }
    }
    let unmatched = remaining
        .iter()
        .map(|(row, _)| row.clone())
        .collect::<Vec<_>>();
    let known = array(&query["expected"])?
        .iter()
        .map(Row::parse)
        .collect::<Result<Vec<_>>>()?;
    let candidate = |expected: &Row, actual: &Row| {
        expected.path == actual.path
            && (expected.line == actual.line || expected.context == actual.context)
            && !known.contains(actual)
    };
    let mut paired = vec![false; assessed.len()];
    for (expected, value) in remaining {
        // Keep the candidate set fixed: removing a pair must not resolve another
        // row's original ambiguity between a location and a context error.
        let candidates = assessed
            .iter()
            .enumerate()
            .filter_map(|(at, (row, _))| candidate(&expected, row).then_some(at))
            .collect::<Vec<_>>();
        let pair = if let [at] = candidates.as_slice() {
            (unmatched
                .iter()
                .filter(|row| candidate(row, &assessed[*at].0))
                .count()
                == 1)
                .then_some(*at)
        } else {
            None
        };
        let (outcome, actual) = if let Some(at) = pair {
            paired[at] = true;
            let (row, actual) = &assessed[at];
            (
                if row.line == expected.line {
                    "wrong_context"
                } else {
                    "wrong_location"
                },
                Some(actual.clone()),
            )
        } else {
            ("miss", None)
        };
        outcomes.push(json!({"outcome": outcome, "expected": value, "actual": actual}));
    }
    Ok(
        json!({"outcomes": outcomes, "spurious": assessed.into_iter().enumerate().filter_map(|(at, (_, value))| (!paired[at]).then_some(value)).collect::<Vec<_>>(), "unassessed": unassessed}),
    )
}

fn command(
    binary: &Path,
    root: &Path,
    args: &[String],
    prefix: &Path,
    seconds: u64,
) -> Result<Value> {
    let mut result = capture::run(binary, args, root, prefix, seconds, false)?;
    result["timed_out"] = json!(result["stopped"] == "timeout");
    result["stdout"] = json!(fs::read_to_string(string(&result["stdout_path"])?)?);
    result["stderr"] = json!(fs::read_to_string(string(&result["stderr_path"])?)?);
    Ok(result)
}

fn rows(output: &Value) -> Result<Vec<Value>> {
    if output["status"] != 0 || output["timed_out"] == true || output["stopped"].is_string() {
        return Err(invalid("CLI command failed; see raw capture").into());
    }
    string(&output["stdout"])?
        .lines()
        .map(|line| {
            let value: Value = serde_json::from_str(line)?;
            if array(&value)?.len() != 4 || !value[3].is_string() {
                return Err(invalid("malformed JSON result row").into());
            }
            Row::parse(&value)?;
            Ok(value)
        })
        .collect()
}

fn assess(binary: &Path, root: &Path, manifest_path: &Path, destination: &Path) -> Result<()> {
    if destination.exists() {
        return Err(invalid("assessment destination already exists").into());
    }
    let result = assess_inner(binary, root, manifest_path, destination);
    if let Err(error) = &result
        && destination.is_dir()
    {
        write(
            &destination.join("incomplete.json"),
            &json!({"complete":false,"error":error.to_string()}),
        )?;
    }
    result
}

fn assess_inner(
    binary: &Path,
    root: &Path,
    manifest_path: &Path,
    destination: &Path,
) -> Result<()> {
    // A fresh directory prevents accidentally reusing an older index or report.
    fs::create_dir(destination)?;
    let binary = fs::canonicalize(binary)?;
    let binary_sha256 = digest(&binary)?;
    let root = fs::canonicalize(root)?;
    let manifest = read(manifest_path)?;
    for source in array(&manifest["sources"])? {
        source_path(&root, string(&source["path"])?)?;
        source_inventory::verify(
            &root,
            source,
            manifest["source_inventory_version"].as_u64().unwrap_or(2),
        )?;
    }
    write(&destination.join("manifest.json"), &manifest)?;
    let mut unsupported = source_inventory::preflight(
        &root,
        &array(&manifest["sources"])?
            .iter()
            .map(|source| string(&source["path"]))
            .collect::<Result<Vec<_>>>()?,
        &manifest["include_directories"]
            .as_array()
            .into_iter()
            .flatten()
            .map(string)
            .collect::<Result<Vec<_>>>()?,
    )?;
    for item in manifest["unsupported_inputs"]
        .as_array()
        .into_iter()
        .flatten()
    {
        if !unsupported.contains(item) {
            unsupported.push(item.clone());
        }
    }
    if !unsupported.is_empty()
        || manifest["unsupported_inputs"]
            .as_array()
            .is_some_and(|items| !items.is_empty())
    {
        write(
            &destination.join("report.json"),
            &json!({"complete":false,"run_finished":false,
            "comparison_eligible":false,"status":"unsupported_inputs","unsupported_inputs":unsupported,
            "error":"external inputs are outside the frozen project; no index command was launched",
            "binary_sha256":binary_sha256,"manifest_sha256":digest(manifest_path)?,"queries":[],"totals":{}}),
        )?;
        return Err(invalid("unsupported inputs outside the frozen project").into());
    }
    let destination = fs::canonicalize(destination)?;
    let selection = destination.join("selection.files");
    let index_files = manifest.get("index_files").unwrap_or(&manifest["files"]);
    let files = array(index_files)?
        .iter()
        .map(|file| Ok(format!("\"{}\"\n", string(file)?)))
        .collect::<Result<String>>()?;
    fs::write(&selection, files)?;
    let index = destination.join("index.db").to_string_lossy().into_owned();
    let mut build_args = vec![
        "-bk".into(),
        "-f".into(),
        index.clone(),
        "-i".into(),
        selection.to_string_lossy().into_owned(),
    ];
    for directory in manifest
        .get("include_directories")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let directory = string(directory)?;
        if directory != "." && !fs::canonicalize(source_path(&root, directory)?)?.starts_with(&root)
        {
            return Err(invalid("include directory escapes pinned source tree").into());
        }
        build_args.extend(["-I".into(), directory.into()]);
    }
    let build = command(
        &binary,
        &root,
        &build_args,
        &destination.join("build"),
        1800,
    )?;
    write(&destination.join("build.json"), &build)?;
    if build["complete"] != true {
        return Err(invalid("index build failed; see build.json").into());
    }
    let discovery = command(
        &binary,
        &root,
        &[
            "-dL".into(),
            "--json".into(),
            "-f".into(),
            index.clone(),
            "-7".into(),
            ".*".into(),
        ],
        &destination.join("discovery"),
        900,
    )?;
    write(&destination.join("discovery.json"), &discovery)?;
    let discovered = rows(&discovery)?;
    let pinned = array(&manifest["sources"])?
        .iter()
        .map(|source| string(&source["path"]))
        .collect::<Result<BTreeSet<_>>>()?;
    let mut closure = Vec::new();
    for row in &discovered {
        let name = string(&row[0])?;
        let path = source_path(&root, name)?;
        if !pinned.contains(name) {
            return Err(invalid(&format!(
                "discovered unpinned source {name}; seal closure_files before querying"
            ))
            .into());
        }
        closure.push(json!({"path":name,"sha256":digest(&path)?}));
    }
    write(&destination.join("closure-before.json"), &json!(closure))?;
    let mut captured = Vec::new();
    let limit = manifest["query_limit"].as_u64().unwrap_or(u64::MAX) as usize;
    for (at, query) in array(&manifest["queries"])?.iter().enumerate() {
        if at >= limit {
            captured.push(json!({"query":query,"unmeasured_reason":"query_limit"}));
            continue;
        }
        let raw = command(
            &binary,
            &root,
            &[
                "-dL".into(),
                "--json".into(),
                "-f".into(),
                index.clone(),
                format!("-{}", number(&query["field"])?),
                string(&query["pattern"])?.into(),
            ],
            &destination.join(format!("query-{at:03}")),
            120,
        )?;
        write(&destination.join(format!("query-{at:03}.json")), &raw)?;
        let capture = match rows(&raw) {
            Ok(actual) => json!({"query": query, "result": score(query, &actual)?}),
            Err(error) => json!({"query":query,"unmeasured_reason":error.to_string()}),
        };
        {
            use std::io::Write;
            let mut partial = fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(destination.join("queries.jsonl"))?;
            writeln!(partial, "{}", serde_json::to_string(&capture)?)?;
        }
        captured.push(capture);
    }
    for source in array(&manifest["sources"])? {
        source_inventory::verify(
            &root,
            source,
            manifest["source_inventory_version"].as_u64().unwrap_or(2),
        )?;
    }
    let mut totals = BTreeMap::<String, BTreeMap<String, u64>>::new();
    let mut constructs = BTreeMap::<String, BTreeMap<String, u64>>::new();
    for capture in &captured {
        let field = capture["query"]["field"].to_string();
        let counts = totals.entry(field).or_default();
        if capture["unmeasured_reason"].is_string() {
            *counts.entry("unmeasured_queries".into()).or_default() += 1;
            continue;
        }
        if capture["result"]["reason"].is_string() {
            *counts.entry("unscored_queries".into()).or_default() += 1;
            *counts.entry("unscored_rows".into()).or_default() +=
                array(&capture["result"]["unscored"])?.len() as u64;
            continue;
        }
        for kind in [
            "expected",
            "correct",
            "miss",
            "wrong_context",
            "wrong_location",
            "wrong_text",
            "spurious",
            "unassessed",
        ] {
            counts.entry(kind.into()).or_default();
        }
        for outcome in array(&capture["result"]["outcomes"])? {
            *counts.entry("expected".into()).or_default() += 1;
            let kind = string(&outcome["outcome"])?;
            *counts.entry(kind.into()).or_default() += 1;
            let construct = string(&outcome["expected"][3])?;
            let counts = constructs
                .entry(format!("{}:{construct}", capture["query"]["field"]))
                .or_default();
            *counts.entry("expected".into()).or_default() += 1;
            *counts.entry(kind.into()).or_default() += 1;
        }
        for kind in ["spurious", "unassessed"] {
            let count = array(&capture["result"][kind])?.len() as u64;
            *counts.entry(kind.into()).or_default() += count;
            let construct = string(&capture["query"]["construct"])?;
            *constructs
                .entry(format!("{}:{construct}", capture["query"]["field"]))
                .or_default()
                .entry(kind.into())
                .or_default() += count;
        }
    }
    if digest(&binary)? != binary_sha256 {
        return Err(invalid("binary changed during assessment").into());
    }
    let report = json!({"binary": binary, "binary_sha256": binary_sha256, "manifest_sha256": digest(manifest_path)?,
        "selected_files": array(index_files)?.len(), "assessed_files":array(&manifest["files"])?.len(),
        "complete":captured.iter().all(|item|!item["unmeasured_reason"].is_string()),"query_limit":manifest["query_limit"],
        "discovered_files": discovered.len(), "discovered": discovered,
        "totals": totals, "constructs": constructs, "queries": captured,
        "unresolved": manifest["unresolved"], "build": build, "discovery": discovery});
    write(&destination.join("report.json"), &report)?;
    println!("{}", serde_json::to_string(&report["totals"])?);
    Ok(())
}

fn main() -> Result<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["seal", root, draft, destination] => seal(Path::new(root), Path::new(draft), Path::new(destination)),
        ["run", binary, root, manifest, destination] => assess(Path::new(binary), Path::new(root), Path::new(manifest), Path::new(destination)),
        _ => Err(invalid("usage: assess-navigation seal ROOT DRAFT MANIFEST | run BINARY ROOT MANIFEST NEW_OUTPUT_DIR").into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assessment_counts_duplicates_context_errors_and_unassessed_answers() {
        let query = json!({"scopes": [["a.c", 1, 5]], "expected": [
            ["a.c", "f", 2, "call"], ["a.c", "f", 3, "call"], ["a.c", "f", 4, "call"], ["a.c", "f", 5, "call"]
        ]});
        let actual = vec![
            json!(["a.c", "f", 2, "text"]),
            json!(["a.c", "f", 2, "text"]),
            json!(["a.c", "wrong", 3, "text"]),
            json!(["a.c", "other", 1, "text"]),
            json!(["a.c", "f", 9, "text"]),
        ];
        let scored = score(&query, &actual).unwrap();
        assert_eq!(scored["outcomes"][0]["outcome"], "correct");
        assert_eq!(scored["outcomes"][1]["outcome"], "wrong_context");
        assert_eq!(scored["outcomes"][2]["outcome"], "miss");
        assert_eq!(scored["outcomes"][3]["outcome"], "miss");
        assert_eq!(
            scored["spurious"],
            json!([["a.c", "f", 2, "text"], ["a.c", "other", 1, "text"]])
        );
        assert_eq!(scored["unassessed"], json!([["a.c", "f", 9, "text"]]));
    }

    #[test]
    fn wrong_locations_are_paired_only_inside_labelled_scope() {
        let query = json!({"scopes": [["a.c", 1, 5]], "expected": [["a.c", "f", 2, "call"]]});
        let scored = score(&query, &[json!(["a.c", "f", 3, "text"])]).unwrap();
        assert_eq!(scored["outcomes"][0]["outcome"], "wrong_location");
        let scored = score(&query, &[json!(["a.c", "f", 6, "text"])]).unwrap();
        assert_eq!(scored["outcomes"][0]["outcome"], "miss");
        assert_eq!(array(&scored["unassessed"]).unwrap().len(), 1);
    }

    #[test]
    fn ambiguous_callees_are_not_assigned_to_an_arbitrary_fact() {
        let query = json!({"scopes": [["a.c", 1, 1]], "expected": [
            ["a.c", "first", 1, "call"], ["a.c", "second", 1, "call"]
        ]});
        let scored = score(&query, &[json!(["a.c", "unknown", 1, "text"])]).unwrap();
        assert_eq!(scored["outcomes"][0]["outcome"], "miss");
        assert_eq!(scored["outcomes"][1]["outcome"], "miss");
        assert_eq!(scored["spurious"], json!([["a.c", "unknown", 1, "text"]]));
        let scored = score(
            &query,
            &[
                json!(["a.c", "first", 1, "text"]),
                json!(["a.c", "first", 1, "text"]),
            ],
        )
        .unwrap();
        assert_eq!(scored["outcomes"][0]["outcome"], "correct");
        assert_eq!(scored["outcomes"][1]["outcome"], "miss");
        assert_eq!(scored["spurious"], json!([["a.c", "first", 1, "text"]]));
        let query = json!({"scopes": [["a.c", 1, 2]], "expected": [
            ["a.c", "first", 1, "call"], ["a.c", "second", 2, "call"]
        ]});
        let scored = score(&query, &[json!(["a.c", "first", 2, "text"])]).unwrap();
        assert_eq!(scored["outcomes"][0]["outcome"], "miss");
        assert_eq!(scored["outcomes"][1]["outcome"], "miss");
        assert_eq!(scored["spurious"], json!([["a.c", "first", 2, "text"]]));
    }

    #[test]
    fn operational_failures_are_not_navigation_misses() {
        assert!(rows(&json!({"status": 1, "stdout": ""})).is_err());
        assert!(rows(&json!({"status": 0, "stdout": "not json"})).is_err());
        assert!(rows(&json!({"status": 0, "stdout": "[\"a.c\",\"f\",1]"})).is_err());
        assert!(rows(&json!({"status": 0, "stopped":"output_limit", "stdout": ""})).is_err());
    }

    #[test]
    fn unresolved_queries_and_live_text_have_separate_counting_rules() {
        let unresolved = json!({"unscored_reason":"ambiguous macro","scopes":[],"expected":[]});
        assert_eq!(
            score(&unresolved, &[json!(["a.c", "f", 1, "x"])]).unwrap()["unscored"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        let query = json!({"field":4,"scopes":[["a.c",1,1]],"match_text":true,"expected":[["a.c","<unknown>",1,"text","\tx;\r"]]});
        assert_eq!(
            score(&query, &[json!(["a.c", "<unknown>", 1, "x;"])]).unwrap()["outcomes"][0]["outcome"],
            "wrong_text"
        );
        let query = json!({"field":7,"scopes":[["a.c",1,1]],"expected":[["a.c","<unknown>",1,"inventory"]]});
        assert_eq!(
            score(
                &query,
                &[
                    json!(["a.c", "<unknown>", 1, "x"]),
                    json!(["extra.h", "<unknown>", 1, "x"])
                ]
            )
            .unwrap()["spurious"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn missing_selected_headers_cannot_be_sealed() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.c"), "int x;\n").unwrap();
        let draft = dir.path().join("draft.json");
        write(&draft,&json!({"files":["a.c"],"index_files":["a.c","missing.h"],"definitions":[],"calls":[],"controls":[],"unresolved":[]})).unwrap();
        assert!(seal(dir.path(), &draft, &dir.path().join("sealed.json")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn changed_sources_stop_before_any_build_or_query() {
        let directory = tempfile::tempdir().unwrap();
        fs::write(directory.path().join("a.c"), "int x;\n").unwrap();
        let draft = directory.path().join("draft.json");
        write(
            &draft,
            &json!({"files":["a.c"],"definitions":[],"calls":[],"controls":[],"unresolved":[]}),
        )
        .unwrap();
        let sealed = directory.path().join("sealed.json");
        seal(directory.path(), &draft, &sealed).unwrap();
        fs::write(directory.path().join("a.c"), "int changed;\n").unwrap();
        let output = directory.path().join("run");
        assert!(assess(Path::new("/bin/false"), directory.path(), &sealed, &output).is_err());
        assert_eq!(
            read(&output.join("incomplete.json")).unwrap()["complete"],
            false
        );
        assert!(!output.join("build.stdout").exists());
    }

    #[cfg(unix)]
    #[test]
    fn sealed_aliases_reject_equal_byte_retargeting_before_building() {
        let directory = tempfile::tempdir().unwrap();
        fs::write(directory.path().join("one.h"), "int same;\n").unwrap();
        fs::write(directory.path().join("two.h"), "int same;\n").unwrap();
        std::os::unix::fs::symlink("one.h", directory.path().join("alias.h")).unwrap();
        let draft = directory.path().join("draft.json");
        write(
            &draft,
            &json!({"files":["alias.h"],"definitions":[],"calls":[],"controls":[],"unresolved":[]}),
        )
        .unwrap();
        let sealed = directory.path().join("sealed.json");
        seal(directory.path(), &draft, &sealed).unwrap();
        fs::remove_file(directory.path().join("alias.h")).unwrap();
        std::os::unix::fs::symlink("two.h", directory.path().join("alias.h")).unwrap();
        let output = directory.path().join("run");
        assert!(assess(Path::new("/bin/false"), directory.path(), &sealed, &output).is_err());
        assert!(!output.join("build.stdout").exists());
    }

    #[cfg(unix)]
    #[test]
    fn legacy_manifest_absolute_operands_stop_before_building() {
        let directory = tempfile::tempdir().unwrap();
        fs::write(directory.path().join("a.c"), "#include \"alias.h\"\n").unwrap();
        fs::write(
            directory.path().join("real.h"),
            "#include \"/missing-host/input.h\"\n",
        )
        .unwrap();
        std::os::unix::fs::symlink("real.h", directory.path().join("alias.h")).unwrap();
        let manifest = directory.path().join("legacy.json");
        write(
            &manifest,
            &json!({"files":["a.c"],"sources":[{"path":"a.c",
            "sha256":digest(&directory.path().join("a.c")).unwrap()}],"queries":[]}),
        )
        .unwrap();
        let output = directory.path().join("run");
        assert!(
            assess(
                Path::new("/bin/false"),
                directory.path(),
                &manifest,
                &output
            )
            .is_err()
        );
        assert!(!output.join("build.stdout").exists());
        let report = read(&output.join("report.json")).unwrap();
        assert_eq!(report["status"], "unsupported_inputs");
        assert_eq!(report["comparison_eligible"], false);
        assert_eq!(
            report["unsupported_inputs"][0]["operand"],
            "/missing-host/input.h"
        );
    }

    #[cfg(unix)]
    #[test]
    fn sealed_aliases_reject_link_spelling_drift_and_missing_targets() {
        let directory = tempfile::tempdir().unwrap();
        fs::create_dir(directory.path().join("include")).unwrap();
        fs::write(directory.path().join("include/real.h"), "int same;\n").unwrap();
        std::os::unix::fs::symlink("include", directory.path().join("view")).unwrap();
        let alias = directory.path().join("include/alias.h");
        std::os::unix::fs::symlink("real.h", &alias).unwrap();
        let draft = directory.path().join("draft.json");
        write(&draft, &json!({"files":["view/alias.h"],"definitions":[],"calls":[],"controls":[],"unresolved":[]})).unwrap();
        let manifest = directory.path().join("manifest.json");
        seal(directory.path(), &draft, &manifest).unwrap();
        fs::remove_file(&alias).unwrap();
        std::os::unix::fs::symlink("./real.h", &alias).unwrap();
        let output = directory.path().join("link-drift");
        assert!(
            assess(
                Path::new("/bin/false"),
                directory.path(),
                &manifest,
                &output
            )
            .is_err()
        );
        assert!(!output.join("build.stdout").exists());
        fs::remove_file(&alias).unwrap();
        std::os::unix::fs::symlink("real.h", &alias).unwrap();
        fs::remove_file(directory.path().join("include/real.h")).unwrap();
        let output = directory.path().join("target-missing");
        assert!(
            assess(
                Path::new("/bin/false"),
                directory.path(),
                &manifest,
                &output
            )
            .is_err()
        );
        assert!(!output.join("build.stdout").exists());
    }

    #[test]
    fn sealing_keeps_the_full_index_but_scores_only_the_audited_files() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.c"), "int known;\n").unwrap();
        fs::write(dir.path().join("b.c"), "int other;\n").unwrap();
        let draft = dir.path().join("draft.json");
        write(
            &draft,
            &json!({"files":["a.c"],"index_files":["a.c","b.c"],"whole_files":true,
            "definitions":[["a.c",1,"known","object"]],"calls":[],"controls":[],"unresolved":[],
            "query_inventory":[[1,"other","negative"],[2,"empty","empty context"]],
            "unscored_queries":[[1,"other","macro ambiguity"]]}),
        )
        .unwrap();
        let manifest = dir.path().join("manifest.json");
        seal(dir.path(), &draft, &manifest).unwrap();
        let sealed = read(&manifest).unwrap();
        assert_eq!(array(&sealed["sources"]).unwrap().len(), 2);
        let queries = array(&sealed["queries"]).unwrap();
        let other = queries
            .iter()
            .find(|query| query["pattern"] == "other")
            .unwrap();
        assert_eq!(other["unscored_reason"], "macro ambiguity");
        assert_eq!(other["scopes"], json!([["a.c", 1, 2]]));
        assert!(queries.iter().any(|query| query["field"] == 2
            && query["pattern"] == "empty"
            && query["expected"] == json!([])));
    }
}
