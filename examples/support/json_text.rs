//! JSON text laid out as the checked-in fixtures and captures already are: object
//! members in the order they were set, and the indent, separators and escapes those
//! files use, so a recapture changes only what the reference changed.

use std::fmt::Write;

/// A JSON value whose objects keep their members in the order they were set.
#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    Array(Vec<Json>),
    Object(Vec<(String, Json)>),
}

/// How a value is written.
#[derive(Clone, Copy)]
pub struct Layout {
    /// Spaces per level, or one line with `, ` and `: ` between items.
    pub indent: Option<usize>,
    /// Escape every character outside printable ASCII as `\uXXXX`.
    pub ascii: bool,
}

/// The probe fixtures and intended answers: one space per level, raw UTF-8.
pub const FIXTURE: Layout = Layout {
    indent: Some(1),
    ascii: false,
};
/// Reports and receipts: two spaces per level, ASCII only.
pub const REPORT: Layout = Layout {
    indent: Some(2),
    ascii: true,
};
/// One line per value, as in the JSON-lines manifests.
pub const LINE: Layout = Layout {
    indent: None,
    ascii: true,
};

impl Json {
    pub fn object<K: Into<String>>(members: impl IntoIterator<Item = (K, Json)>) -> Json {
        Json::Object(
            members
                .into_iter()
                .map(|(key, value)| (key.into(), value))
                .collect(),
        )
    }

    /// Sets a member, keeping its place when it is already there.
    pub fn set(&mut self, key: &str, value: impl Into<Json>) {
        let Json::Object(members) = self else {
            panic!("set on a JSON value that is not an object");
        };
        let value = value.into();
        match members.iter_mut().find(|(name, _)| name == key) {
            Some((_, slot)) => *slot = value,
            None => members.push((key.to_owned(), value)),
        }
    }

    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Object(members) => members
                .iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value),
            _ => None,
        }
    }

    pub fn get_mut(&mut self, key: &str) -> Option<&mut Json> {
        match self {
            Json::Object(members) => members
                .iter_mut()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value),
            _ => None,
        }
    }

    pub fn text(&self, layout: Layout) -> String {
        let mut out = String::new();
        self.write(&mut out, layout, 0);
        out
    }

    fn write(&self, out: &mut String, layout: Layout, level: usize) {
        match self {
            Json::Null => out.push_str("null"),
            Json::Bool(value) => out.push_str(if *value { "true" } else { "false" }),
            Json::Int(value) => {
                let _ = write!(out, "{value}");
            }
            Json::Float(value) => out.push_str(&float(*value)),
            Json::Str(value) => string(out, value, layout.ascii),
            Json::Array(items) => {
                if items.is_empty() {
                    out.push_str("[]");
                    return;
                }
                out.push('[');
                for (at, item) in items.iter().enumerate() {
                    separate(out, layout, level + 1, at == 0);
                    item.write(out, layout, level + 1);
                }
                close(out, layout, level);
                out.push(']');
            }
            Json::Object(members) => {
                if members.is_empty() {
                    out.push_str("{}");
                    return;
                }
                out.push('{');
                for (at, (key, value)) in members.iter().enumerate() {
                    separate(out, layout, level + 1, at == 0);
                    string(out, key, layout.ascii);
                    out.push_str(": ");
                    value.write(out, layout, level + 1);
                }
                close(out, layout, level);
                out.push('}');
            }
        }
    }
}

fn separate(out: &mut String, layout: Layout, level: usize, first: bool) {
    match layout.indent {
        Some(width) => {
            if !first {
                out.push(',');
            }
            out.push('\n');
            out.extend(std::iter::repeat_n(' ', width * level));
        }
        None if !first => out.push_str(", "),
        None => {}
    }
}

fn close(out: &mut String, layout: Layout, level: usize) {
    if let Some(width) = layout.indent {
        out.push('\n');
        out.extend(std::iter::repeat_n(' ', width * level));
    }
}

fn string(out: &mut String, text: &str, ascii: bool) {
    out.push('"');
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            ch if ch < ' ' || (ascii && ch > '~') => {
                let mut units = [0; 2];
                for unit in ch.encode_utf16(&mut units) {
                    let _ = write!(out, "\\u{unit:04x}");
                }
            }
            ch => out.push(ch),
        }
    }
    out.push('"');
}

/// The shortest text that reads back as the same number, as the captures write it.
pub fn float(value: f64) -> String {
    if !value.is_finite() {
        return if value.is_nan() {
            "NaN".to_owned()
        } else if value > 0.0 {
            "Infinity".to_owned()
        } else {
            "-Infinity".to_owned()
        };
    }
    let text = format!("{value:?}");
    match text.split_once('e') {
        Some((mantissa, exponent)) => {
            let (sign, digits) = match exponent.strip_prefix('-') {
                Some(digits) => ('-', digits),
                None => ('+', exponent),
            };
            format!("{mantissa}e{sign}{digits:0>2}")
        }
        None => text,
    }
}

impl From<serde_json::Value> for Json {
    fn from(value: serde_json::Value) -> Json {
        match value {
            serde_json::Value::Null => Json::Null,
            serde_json::Value::Bool(value) => Json::Bool(value),
            serde_json::Value::Number(number) => match number.as_i64() {
                Some(value) => Json::Int(value),
                None => Json::Float(number.as_f64().unwrap_or(f64::NAN)),
            },
            serde_json::Value::String(value) => Json::Str(value),
            serde_json::Value::Array(items) => {
                Json::Array(items.into_iter().map(Json::from).collect())
            }
            serde_json::Value::Object(members) => {
                Json::Object(members.into_iter().map(|(k, v)| (k, v.into())).collect())
            }
        }
    }
}

impl From<&str> for Json {
    fn from(value: &str) -> Json {
        Json::Str(value.to_owned())
    }
}

impl From<String> for Json {
    fn from(value: String) -> Json {
        Json::Str(value)
    }
}

impl From<bool> for Json {
    fn from(value: bool) -> Json {
        Json::Bool(value)
    }
}

impl From<i64> for Json {
    fn from(value: i64) -> Json {
        Json::Int(value)
    }
}

impl From<u64> for Json {
    fn from(value: u64) -> Json {
        Json::Int(i64::try_from(value).unwrap_or(i64::MAX))
    }
}

impl From<usize> for Json {
    fn from(value: usize) -> Json {
        Json::Int(i64::try_from(value).unwrap_or(i64::MAX))
    }
}

impl From<f64> for Json {
    fn from(value: f64) -> Json {
        Json::Float(value)
    }
}

impl<T: Into<Json>> From<Option<T>> for Json {
    fn from(value: Option<T>) -> Json {
        value.map_or(Json::Null, Into::into)
    }
}

impl<T: Into<Json>> From<Vec<T>> for Json {
    fn from(items: Vec<T>) -> Json {
        Json::Array(items.into_iter().map(Into::into).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layouts_match_the_checked_in_files() {
        let value = Json::object([
            ("name", Json::from("é \"q\"\n\u{7f}")),
            ("empty", Json::Array(Vec::new())),
            ("none", Json::object(Vec::<(String, Json)>::new())),
            ("rows", Json::from(vec![1_i64, 2])),
            ("seconds", Json::from(0.25)),
        ]);
        assert_eq!(
            value.text(FIXTURE),
            "{\n \"name\": \"é \\\"q\\\"\\n\u{7f}\",\n \"empty\": [],\n \"none\": {},\n \"rows\": [\n  1,\n  2\n ],\n \"seconds\": 0.25\n}"
        );
        assert_eq!(
            value.text(LINE),
            "{\"name\": \"\\u00e9 \\\"q\\\"\\n\\u007f\", \"empty\": [], \"none\": {}, \"rows\": [1, 2], \"seconds\": 0.25}"
        );
        assert_eq!(
            Json::from("\u{1f600}\u{1}").text(REPORT),
            "\"\\ud83d\\ude00\\u0001\""
        );
        assert_eq!(float(1e-7), "1e-07");
        assert_eq!(float(1e16), "1e+16");
        assert_eq!(float(12.0), "12.0");
    }

    #[test]
    fn setting_a_member_keeps_its_place() {
        let mut value = Json::object([("a", Json::from(1_i64)), ("b", Json::from(2_i64))]);
        value.set("a", 3_i64);
        value.set("c", 4_i64);
        assert_eq!(value.text(LINE), "{\"a\": 3, \"b\": 2, \"c\": 4}");
    }
}
