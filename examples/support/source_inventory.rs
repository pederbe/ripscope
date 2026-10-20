//! Freeze discovery paths and their targets without admitting external source bytes.

use regex::bytes::Regex;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::LazyLock;

static INCLUDES: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"#\s*include\s*([<"])([^>"\r\n]+)[>"]"#).expect("literal include expression")
});

pub fn includes(bytes: &[u8]) -> Vec<(String, bool)> {
    INCLUDES
        .captures_iter(bytes)
        .map(|item| {
            (
                String::from_utf8_lossy(&item[2]).into_owned(),
                &item[1] == b"\"",
            )
        })
        .collect()
}

fn normalize(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir if result.file_name().is_some_and(|part| part != "..") => {
                result.pop();
            }
            _ => result.push(part.as_os_str()),
        }
    }
    if result.as_os_str().is_empty() {
        result.push(".");
    }
    result
}

/// Compare project components without changing the discovery spelling.
fn project_relative<'a>(path: &'a Path, root: &Path) -> Option<&'a Path> {
    let mut parts = path.components();
    for expected in root.components() {
        let actual = parts.next()?;
        if actual == expected {
            continue;
        }
        // Windows canonicalization adds a verbatim prefix to an ordinary drive.
        #[cfg(windows)]
        if let (Component::Prefix(actual), Component::Prefix(expected)) = (actual, expected) {
            use std::path::Prefix::{Disk, VerbatimDisk};
            if let (Disk(actual) | VerbatimDisk(actual), Disk(expected) | VerbatimDisk(expected)) =
                (actual.kind(), expected.kind())
                && actual == expected
            {
                continue;
            }
        }
        return None;
    }
    Some(parts.as_path())
}

pub fn freeze(root: &Path, name: &str) -> io::Result<Value> {
    let root = root.canonicalize()?;
    let path = root.join(name);
    let target = path.canonicalize()?;
    if !target.starts_with(&root) || !target.is_file() {
        return Err(io::Error::other(format!(
            "source escapes pinned tree: {name}"
        )));
    }
    let mut pending = project_relative(&path, &root)
        .ok_or_else(|| io::Error::other("source escapes pinned tree"))?
        .components()
        .map(|part| part.as_os_str().to_owned())
        .collect::<VecDeque<_>>();
    let mut current = root.clone();
    let mut links = Vec::new();
    while let Some(part) = pending.pop_front() {
        if part == ".." {
            if current == root {
                return Err(io::Error::other("source escapes pinned tree"));
            }
            current.pop();
            continue;
        }
        if part == "." {
            continue;
        }
        current.push(part);
        if fs::symlink_metadata(&current)?.is_symlink() {
            if links.len() >= 64 {
                return Err(io::Error::other("too many source links"));
            }
            let link = fs::read_link(&current)?;
            links.push(json!({"path":current.strip_prefix(&root).map_err(io::Error::other)?,"target":link}));
            current.pop();
            let relative = if link.is_absolute() {
                current = root.clone();
                project_relative(&link, &root)
                    .ok_or_else(|| io::Error::other("source escapes pinned tree"))?
            } else {
                &link
            };
            for part in relative.components().rev() {
                pending.push_front(part.as_os_str().to_owned());
            }
        }
    }
    Ok(
        json!({"path":name,"target":target.strip_prefix(&root).map_err(io::Error::other)?,
        "links":links,"sha256":crate::capture::digest(&target)?,"bytes":target.metadata()?.len()}),
    )
}

pub fn verify(root: &Path, source: &Value, version: u64) -> io::Result<()> {
    let name = source["path"]
        .as_str()
        .ok_or_else(|| io::Error::other("missing source path"))?;
    if !source["sha256"].is_string()
        || (version >= 3 && (!source["target"].is_string() || !source["links"].is_array()))
    {
        return Err(io::Error::other(format!(
            "incomplete frozen source identity: {name}"
        )));
    }
    let actual = freeze(root, name)
        .map_err(|error| io::Error::other(format!("frozen source changed: {name}: {error}")))?;
    if ["sha256", "target", "links"]
        .into_iter()
        .any(|key| source.get(key).is_some_and(|value| *value != actual[key]))
    {
        return Err(io::Error::other(format!("frozen source changed: {name}")));
    }
    Ok(())
}

pub enum Input {
    Local(String),
    Missing,
    Unsupported(&'static str),
}

pub fn resolve(root: &Path, operand: &str, quoted: bool, directories: &[&str]) -> Input {
    let operand = Path::new(operand);
    if operand.is_absolute() && project_relative(&normalize(operand), root).is_none() {
        return Input::Unsupported("absolute input outside declared project");
    }
    let mut candidates = Vec::new();
    if quoted {
        candidates.push(operand.to_owned());
    }
    candidates.extend(
        directories
            .iter()
            .map(|directory| Path::new(directory).join(operand)),
    );
    for candidate in candidates {
        let candidate = normalize(&candidate);
        let path = normalize(&root.join(&candidate));
        if project_relative(&path, root).is_none() {
            if path.is_file() {
                return Input::Unsupported("relative input outside declared project");
            }
            continue;
        }
        let Ok(target) = path.canonicalize() else {
            continue;
        };
        if !target.is_file() {
            continue;
        }
        if !target.starts_with(root) {
            return Input::Unsupported("alias target outside declared project");
        }
        return Input::Local(candidate.to_string_lossy().into_owned());
    }
    Input::Missing
}

/// A conservative safety pass, including local inputs an old manifest did not seal.
/// Include-looking comments and strings may stop a run; they never admit host bytes.
pub fn preflight(root: &Path, names: &[&str], directories: &[&str]) -> io::Result<Vec<Value>> {
    let root = root.canonicalize()?;
    let mut queue = names
        .iter()
        .map(|name| (*name).to_owned())
        .collect::<Vec<_>>();
    let mut seen = BTreeSet::new();
    let mut unsupported = BTreeMap::new();
    while let Some(name) = queue.pop() {
        let path = root.join(&name).canonicalize()?;
        if !path.starts_with(&root) {
            return Err(io::Error::other("source escapes pinned tree"));
        }
        if !seen.insert(name.clone()) {
            continue;
        }
        for (operand, quoted) in includes(&fs::read(path)?) {
            match resolve(&root, &operand, quoted, directories) {
                Input::Local(name) => queue.push(name),
                Input::Unsupported(reason) => {
                    unsupported.insert(
                        (name.clone(), operand.clone()),
                        json!({"source":name,"operand":operand,"reason":reason}),
                    );
                }
                Input::Missing => {}
            }
        }
    }
    Ok(unsupported.into_values().collect())
}
