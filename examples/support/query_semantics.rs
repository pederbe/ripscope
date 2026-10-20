//! A second implementation of the stored query fields 0, 2, 3 and 9 (ADR 0010), written
//! from the rules rather than from the Rust engine, with one switch per rule of cscope's
//! sequential search.
//!
//! With every rule off it gives ripscope's own semantics; with every rule on, cscope's
//! sequential search (the engine's oracle). Switching rules on one at a time attributes
//! each difference between the two to a rule. It reads units as `ripscope --dump-units`
//! prints them: `{"file", "records": [{"line", "fragments", "symbols": [[mark, name,
//! assigned], ...], "define_end"}]}`.
//!
//! Rules:
//! - consumption (fields 0, 9): a row consumes the rest of its line up to where cscope's
//!   display stops, context changes there included; otherwise a line gives one row, for
//!   its first matching name, and every context change applies.
//! - carry (0, 2, 3, 9): the macro (and for field 3 the open functions, for field 2 the
//!   listing state and its path) carries from one file to the next.
//! - heads (3): a call is attributed to every distinct head opened since the last
//!   function end (at most nine); otherwise to the latest.
//! - caseless-own-row (0, 9): under -C, a macro definition's own row takes the macro's
//!   name; otherwise the function it sits in.
//! - listing (2): the calls after a matching definition up to its end, passing over
//!   macros defined in the body; otherwise the calls whose macro, else function, matches.

use regex::Regex;
use std::io;

pub const RULES: [&str; 5] = [
    "consumption",
    "carry",
    "heads",
    "caseless-own-row",
    "listing",
];

/// One answer row: path, context, line number and displayed text.
pub type Row = (String, String, u64, String);

/// A pattern as the engine matches names: an exact name, or (any pattern with -C) a
/// regular expression matched against the whole name.
pub struct Pattern {
    pub caseless: bool,
    exact: Option<String>,
    regex: Option<Regex>,
}

impl Pattern {
    pub fn new(pattern: &str, caseless: bool) -> io::Result<Pattern> {
        // cscope trims the C locale's blanks from the pattern's end before matching.
        let pattern = pattern.trim_end_matches([' ', '\t', '\n', '\r', '\u{b}', '\u{c}']);
        let identifier = {
            let mut chars = pattern.chars();
            chars
                .next()
                .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
                && chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
        };
        if !caseless && identifier {
            return Ok(Pattern {
                caseless,
                exact: Some(pattern.to_owned()),
                regex: None,
            });
        }
        let flags = if caseless { "(?i)" } else { "" };
        let regex = Regex::new(&format!("{flags}^(?:{pattern})$"))
            .map_err(|error| io::Error::other(format!("invalid pattern {pattern:?}: {error}")))?;
        Ok(Pattern {
            caseless,
            exact: None,
            regex: Some(regex),
        })
    }

    pub fn marked(&self, name: &str) -> bool {
        if name.is_empty() {
            return false;
        }
        match (&self.exact, &self.regex) {
            (Some(exact), _) => name == exact,
            (None, Some(regex)) => regex.is_match(name),
            (None, None) => false,
        }
    }

    pub fn plain(&self, name: &str) -> bool {
        if self.exact.is_none()
            && !name
                .chars()
                .next()
                .is_some_and(|first| first.is_alphabetic() || first == '_')
        {
            return false;
        }
        self.marked(name)
    }
}

struct Name {
    mark: char,
    name: String,
    assigned: bool,
    at: usize,
}

/// A stored line: its names with their offsets, and where cscope's display stops.
struct Line {
    number: u64,
    define_end: bool,
    names: Vec<Name>,
    text: Vec<char>,
    cut: Option<usize>,
}

impl Line {
    fn new(record: &serde_json::Value) -> io::Result<Line> {
        let invalid = || io::Error::other(format!("malformed unit record: {record}"));
        let fragments = record["fragments"].as_array().ok_or_else(invalid)?;
        let mut text: Vec<char> = fragments
            .first()
            .and_then(|first| first.as_str())
            .ok_or_else(invalid)?
            .chars()
            .collect();
        let mut names = Vec::new();
        for (symbol, fragment) in record["symbols"]
            .as_array()
            .ok_or_else(invalid)?
            .iter()
            .zip(fragments.iter().skip(1))
        {
            let name = symbol[0]
                .as_str()
                .zip(symbol[1].as_str())
                .ok_or_else(invalid)?;
            names.push(Name {
                mark: name.0.chars().next().unwrap_or('\0'),
                name: name.1.to_owned(),
                assigned: symbol.get(2).is_some_and(truthy),
                at: text.len(),
            });
            text.extend(name.1.chars());
            text.extend(fragment.as_str().ok_or_else(invalid)?.chars());
        }
        let mut line = Line {
            number: record["line"].as_u64().ok_or_else(invalid)?,
            define_end: record["define_end"].as_bool().ok_or_else(invalid)?,
            names,
            text,
            cut: None,
        };
        line.cut = (0..line.names.len().saturating_sub(1)).find(|k| line.follows(*k));
        Ok(line)
    }

    fn follows(&self, k: usize) -> bool {
        let (name, after) = (&self.names[k], &self.names[k + 1]);
        after.at <= name.at + name.name.chars().count()
    }

    fn shown(&self, index: usize) -> String {
        match self.cut {
            Some(cut) if index <= cut => {
                let name = &self.names[cut];
                self.text[..name.at + name.name.chars().count()]
                    .iter()
                    .collect()
            }
            _ => self.text.iter().collect(),
        }
    }

    fn after_row(&self, index: usize) -> usize {
        (index..self.names.len().saturating_sub(1))
            .find(|k| self.follows(*k))
            .map_or(self.names.len(), |k| k + 1)
    }
}

fn truthy(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Null => false,
        serde_json::Value::Bool(value) => *value,
        serde_json::Value::Number(number) => number.as_f64() != Some(0.0),
        serde_json::Value::String(text) => !text.is_empty(),
        serde_json::Value::Array(items) => !items.is_empty(),
        serde_json::Value::Object(members) => !members.is_empty(),
    }
}

/// A context name, when it is set and not empty.
fn named(name: &Option<String>) -> Option<&String> {
    name.as_ref().filter(|name| !name.is_empty())
}

type Files = Vec<(String, Vec<Line>)>;

/// Rows as (path, context, line, text), file-scope rows first.
pub fn rows(
    units: &[serde_json::Value],
    field: &str,
    pattern: &Pattern,
    rules: &[&str],
) -> io::Result<Vec<Row>> {
    if let Some(unknown) = rules.iter().find(|rule| !RULES.contains(rule)) {
        return Err(io::Error::other(format!("unknown rule {unknown:?}")));
    }
    let on = |rule: &str| rules.contains(&rule);
    let mut files: Files = Vec::new();
    for unit in units {
        let path = unit["file"]
            .as_str()
            .ok_or_else(|| io::Error::other("unit without a file"))?
            .to_owned();
        let lines = unit["records"]
            .as_array()
            .ok_or_else(|| io::Error::other("unit without records"))?
            .iter()
            .map(Line::new)
            .collect::<io::Result<_>>()?;
        files.push((path, lines));
    }
    let found = match field {
        "0" | "9" => symbols(&files, field, pattern, &on),
        "3" => callers(&files, pattern, &on),
        "2" if on("listing") => listed_callees(&files, pattern, &on),
        "2" => callees(&files, pattern, &on),
        _ => return Err(io::Error::other(format!("no semantics for field {field}"))),
    };
    let (global, rest): (Vec<Row>, Vec<Row>) =
        found.into_iter().partition(|row| row.1 == "<global>");
    Ok(global.into_iter().chain(rest).collect())
}

fn symbols(files: &Files, field: &str, pattern: &Pattern, on: &dyn Fn(&str) -> bool) -> Vec<Row> {
    let mut out = Vec::new();
    let mut macro_name: Option<String> = None;
    for (path, lines) in files {
        let mut function: Option<String> = None;
        if !on("carry") {
            macro_name = None;
        }
        for line in lines {
            let (mut index, mut answered) = (0, false);
            while index < line.names.len() {
                let name = &line.names[index];
                match name.mark {
                    '}' => function = None,
                    ')' => macro_name = None,
                    '$' => function = Some(name.name.clone()),
                    '#' => macro_name = Some(name.name.clone()),
                    _ => {}
                }
                let matched = if "})~".contains(name.mark) || answered {
                    false
                } else if name.mark == ' ' {
                    pattern.plain(&name.name)
                } else {
                    pattern.marked(&name.name)
                };
                if !matched || (field == "9" && !name.assigned) {
                    index += 1;
                    continue;
                }
                let context = if name.mark == '#' && !(on("caseless-own-row") && pattern.caseless) {
                    named(&function)
                } else {
                    named(&macro_name).or(named(&function))
                };
                out.push((
                    path.clone(),
                    context.cloned().unwrap_or_else(|| "<global>".to_owned()),
                    line.number,
                    line.shown(index),
                ));
                if on("consumption") {
                    index = line.after_row(index);
                } else {
                    answered = true;
                    index += 1;
                }
            }
            if line.define_end {
                macro_name = None;
            }
        }
    }
    out
}

fn callers(files: &Files, pattern: &Pattern, on: &dyn Fn(&str) -> bool) -> Vec<Row> {
    let mut out = Vec::new();
    let mut macro_name: Option<String> = None;
    let mut heads: Vec<String> = Vec::new();
    for (path, lines) in files {
        if !on("carry") {
            macro_name = None;
            heads.clear();
        }
        for line in lines {
            for (index, name) in line.names.iter().enumerate() {
                match name.mark {
                    '#' => macro_name = Some(name.name.clone()),
                    ')' => macro_name = None,
                    '$' => {
                        if on("heads") {
                            if !heads.contains(&name.name) && heads.len() < 9 {
                                heads.push(name.name.clone());
                            }
                        } else {
                            heads = vec![name.name.clone()];
                        }
                    }
                    '}' => heads.clear(),
                    '`' if pattern.marked(&name.name) => {
                        let callers = match named(&macro_name) {
                            Some(owner) => vec![owner.clone()],
                            None => heads.clone(),
                        };
                        for caller in callers {
                            out.push((path.clone(), caller, line.number, line.shown(index)));
                        }
                    }
                    _ => {}
                }
            }
            if line.define_end {
                macro_name = None;
            }
        }
    }
    out
}

fn callees(files: &Files, pattern: &Pattern, on: &dyn Fn(&str) -> bool) -> Vec<Row> {
    let mut out = Vec::new();
    let mut macro_name: Option<String> = None;
    let mut function: Option<String> = None;
    for (path, lines) in files {
        if !on("carry") {
            macro_name = None;
            function = None;
        }
        for line in lines {
            for (index, name) in line.names.iter().enumerate() {
                match name.mark {
                    '#' => macro_name = Some(name.name.clone()),
                    ')' => macro_name = None,
                    '$' => function = Some(name.name.clone()),
                    '}' => function = None,
                    '`' => {
                        if let Some(owner) = named(&macro_name).or(named(&function))
                            && pattern.marked(owner)
                        {
                            out.push((
                                path.clone(),
                                name.name.clone(),
                                line.number,
                                line.shown(index),
                            ));
                        }
                    }
                    _ => {}
                }
            }
            if line.define_end {
                macro_name = None;
            }
        }
    }
    out
}

/// cscope's listing: after a matching definition, up to its end.
fn listed_callees(files: &Files, pattern: &Pattern, on: &dyn Fn(&str) -> bool) -> Vec<Row> {
    let mut out = Vec::new();
    let mut state = "seek";
    let mut macro_seen = false;
    let mut shown_path: Option<String> = None;
    for (path, lines) in files {
        if !on("carry") {
            state = "seek";
            macro_seen = false;
            shown_path = Some(path.clone());
        } else if state == "seek" {
            shown_path = Some(path.clone());
        } else if state == "list" {
            // A listing that reaches a file's end stops, but the path is not updated.
            state = "seek";
        }
        for line in lines {
            for (index, name) in line.names.iter().enumerate() {
                match (state, name.mark) {
                    ("seek", '#') => {
                        macro_seen = true;
                        state = if pattern.marked(&name.name) {
                            "list"
                        } else {
                            "seek"
                        };
                    }
                    ("seek", '$') => {
                        state = if pattern.marked(&name.name) {
                            "list"
                        } else {
                            "seek"
                        };
                    }
                    ("list", '`') => out.push((
                        shown_path.clone().unwrap_or_default(),
                        name.name.clone(),
                        line.number,
                        line.shown(index),
                    )),
                    ("list", '#') => state = "pass",
                    ("list", ')') if macro_seen => state = "seek",
                    ("list", '}') => state = "seek",
                    ("pass", ')') => state = "list",
                    _ => {}
                }
            }
            if line.define_end {
                if state == "list" && macro_seen {
                    state = "seek";
                } else if state == "pass" {
                    state = "list";
                }
            }
        }
    }
    out
}

/// The rows as the query commands print them.
pub fn shown(rows: &[Row]) -> Vec<String> {
    rows.iter()
        .map(|(path, context, line, text)| format!("{path} {context} {line} {text}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn unit(file: &str, records: serde_json::Value) -> serde_json::Value {
        json!({"file": file, "records": records})
    }

    #[test]
    fn own_and_sequential_semantics_differ_by_their_rules() {
        let units = vec![
            unit(
                "a.c",
                json!([
                    {"line": 1, "fragments": ["#define ", " x"], "symbols": [["#", "M", false]], "define_end": false},
                    {"line": 2, "fragments": ["int ", "(void) { ", "(); }"],
                     "symbols": [["$", "f", false], ["`", "g", false]], "define_end": false},
                ]),
            ),
            unit(
                "b.c",
                json!([
                    {"line": 1, "fragments": ["int ", " = ", ";"],
                     "symbols": [["g", "x", true], [" ", "y", false]], "define_end": false},
                ]),
            ),
        ];
        let exact = Pattern::new("g", false).unwrap();
        assert_eq!(
            shown(&rows(&units, "3", &exact, &[]).unwrap()),
            ["a.c M 2 int f(void) { g(); }"]
        );
        let any = Pattern::new(".*", false).unwrap();
        assert_eq!(
            shown(&rows(&units, "9", &any, &[]).unwrap()),
            ["b.c <global> 1 int x = y;"]
        );
        let own = rows(&units, "0", &any, &[]).unwrap();
        let sequential = rows(&units, "0", &any, &RULES).unwrap();
        assert_ne!(own, sequential);
        assert_eq!(
            shown(
                &rows(
                    &units,
                    "2",
                    &Pattern::new("M", false).unwrap(),
                    &["listing"]
                )
                .unwrap()
            ),
            ["a.c g 2 int f(void) { g(); }"]
        );
        assert!(rows(&units, "0", &any, &["unknown"]).is_err());
        assert!(Pattern::new("G", true).unwrap().marked("g"));
        // Trailing blanks are trimmed as cscope trims them, so `x ` is the identifier `x`.
        assert!(Pattern::new("x \t", false).unwrap().marked("x"));
        assert!(!Pattern::new("[0-9]+", false).unwrap().plain("12"));
    }
}
