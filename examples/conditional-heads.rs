//! Candidate discovery, preservation and fixed sampling, without correctness labels.
//! See scripts/conditional-heads.md for the CLI and inspection limits.

use regex::bytes::Regex;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{self, BufRead};
use std::path::{Component, Path};
use std::process::Command;
use std::sync::LazyLock;

#[path = "support/capture.rs"]
#[allow(dead_code)]
mod capture;
#[path = "support/source_inventory.rs"]
#[allow(dead_code)]
mod source_inventory;

type Heads = Vec<(u64, String)>;

const METHOD: &str = "conditional-heads-v2";
const MAX_PROJECTS: usize = 40;
const MAX_COMPONENTS: usize = MAX_PROJECTS * 3;
const MAX_PRIMITIVES: usize = 4;
const HEAD_BEFORE_LINES: usize = 32;
const HEAD_BEFORE_BYTES: usize = 4 << 10;
const HEAD_AFTER_LINES: usize = 128;
const HEAD_AFTER_BYTES: usize = 12 << 10;
const BODY_LINES: usize = 256;
const BODY_BYTES: usize = 24 << 10;
const NEIGHBOR_LINES: usize = 64;
const NEIGHBOR_BYTES: usize = 8 << 10;
const SOURCE_GROUP_BYTES: usize = 256 << 10;
const SOURCE_TAIL_BYTES: usize = 512;
const DUMP_LINE_BYTES: usize = 64 << 20;
const CAPTURE_SECONDS: u64 = 600;
const ARTIFACT_BYTES: u64 = 8 << 30;
const MARKS: &str = " #)$}`~gmsuetc";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Origin {
    Current,
    Baseline,
    Source,
}

impl Origin {
    fn name(self) -> &'static str {
        match self {
            Self::Current => "current",
            Self::Baseline => "baseline",
            Self::Source => "source",
        }
    }
}

#[derive(Clone)]
struct Primitive {
    id: String,
    project: String,
    file: String,
    origin: Origin,
    start: u64,
    end: u64,
    heads: Heads,
    incomplete: bool,
    shape: Option<&'static str>,
    head_window: Value,
    selection_hash: String,
}

impl Primitive {
    fn json(&self) -> Value {
        let mut value = json!({"id":self.id,"project":self.project,"file":self.file,
            "origin":self.origin.name(),"start":self.start,"end":self.end,
            "anchor":self.heads.first().map(|h|h.0),"heads":heads_json(&self.heads),
            "incomplete":self.incomplete,"head_window":self.head_window});
        if let Some(shape) = self.shape {
            value["shape"] = json!(shape);
        }
        if !self.selection_hash.is_empty() {
            value["selection_hash"] = json!(self.selection_hash);
        }
        value
    }
}

struct OverlapComponent {
    id: String,
    project: String,
    file: String,
    start: u64,
    end: u64,
    stratum: Origin,
    primitives: Vec<Primitive>,
}
static HEAD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"([A-Za-z_][A-Za-z0-9_]*)[ \t\r\n]*\(").expect("head-like suffix")
});
static BARE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"([A-Za-z_][A-Za-z0-9_]*)[ \t\r\n]*$").expect("bare suffix"));

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::other(message.into())
}

fn string(value: &Value) -> io::Result<&str> {
    value.as_str().ok_or_else(|| invalid("expected string"))
}

fn array(value: &Value) -> io::Result<&Vec<Value>> {
    value.as_array().ok_or_else(|| invalid("expected array"))
}

fn read(path: &Path) -> io::Result<Value> {
    serde_json::from_slice(&fs::read(path)?).map_err(io::Error::other)
}

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn limits() -> Value {
    json!({"components":MAX_COMPONENTS,"primitives_per_component":MAX_PRIMITIVES,
        "head_before_lines":HEAD_BEFORE_LINES,"head_before_bytes":HEAD_BEFORE_BYTES,
        "head_after_lines":HEAD_AFTER_LINES,"head_after_bytes":HEAD_AFTER_BYTES,
        "body_lines":BODY_LINES,"body_bytes":BODY_BYTES,"neighbor_lines":NEIGHBOR_LINES,"neighbor_bytes":NEIGHBOR_BYTES,
        "source_group_bytes":SOURCE_GROUP_BYTES,"source_tail_bytes":SOURCE_TAIL_BYTES,
        "capture_seconds_max":CAPTURE_SECONDS,"artifact_bytes":ARTIFACT_BYTES,
        "body_window_rule":"one window per primitive, earliest independently proved shared-body join",
        "no_call_rule":"independent labels require the entire established body to be inspected"})
}

fn project_sources(project: &Value) -> io::Result<Vec<Value>> {
    let root = Path::new(string(&project["root"])?);
    let manifest_path = Path::new(string(&project["manifest"])?);
    if capture::digest(manifest_path)? != string(&project["manifest_sha256"])? {
        return Err(invalid("original corpus manifest changed"));
    }
    let manifest = read(manifest_path)?;
    let revision = Command::new("git")
        .args(["-C", string(&project["root"])?, "rev-parse", "HEAD"])
        .output()?;
    if !revision.status.success()
        || String::from_utf8_lossy(&revision.stdout).trim()
            != string(&manifest["corpus"]["revision"])?
    {
        return Err(invalid("corpus revision differs from original manifest"));
    }
    let mut sources = Vec::new();
    let mut names = BTreeSet::new();
    for file in array(&manifest["corpus"]["files"])? {
        let name = string(&file["path"])?;
        if name.is_empty()
            || name.contains(['\n', '\r'])
            || Path::new(name)
                .components()
                .any(|part| !matches!(part, Component::Normal(_)))
            || !names.insert(name.to_owned())
        {
            return Err(invalid("corpus file names must be distinct relative paths"));
        }
        let mut source = source_inventory::freeze(root, name)?;
        if source["sha256"] != file["sha256"] || source["bytes"] != file["bytes"] {
            return Err(invalid(format!("selected corpus bytes changed: {name}")));
        }
        source["physical_lines"] = json!(fs::read(root.join(name))?.split(|b| *b == b'\n').count());
        sources.push(source);
    }
    Ok(sources)
}

fn heads_json(heads: &Heads) -> Vec<Value> {
    heads
        .iter()
        .map(|(line, name)| json!({"line":line,"name":name}))
        .collect()
}

fn primitive(
    project: &str,
    file: &str,
    origin: Origin,
    start: u64,
    end: u64,
    heads: Heads,
    incomplete: bool,
) -> Primitive {
    let tuple = format!("{METHOD}\0{project}\0{file}\0{start}\0{end}");
    let key = format!("{tuple}\0{}\0{}", origin.name(), json!(heads_json(&heads)));
    Primitive {
        id: hash(key.as_bytes()),
        project: project.to_owned(),
        file: file.to_owned(),
        origin,
        start,
        end,
        heads,
        incomplete,
        shape: None,
        head_window: Value::Null,
        selection_hash: String::new(),
    }
}

/// Read only the Function events, checking complete file membership and stream shape.
fn mark_runs(
    path: &Path,
    project: &str,
    sources: &[Value],
    origin: Origin,
) -> io::Result<(Vec<Primitive>, BTreeMap<String, Heads>)> {
    let expected = sources
        .iter()
        .map(|s| string(&s["path"]).map(str::to_owned))
        .collect::<io::Result<BTreeSet<_>>>()?;
    let mut seen = BTreeSet::new();
    let mut primitives = Vec::new();
    let mut all_heads = BTreeMap::new();
    for line in io::BufReader::new(fs::File::open(path)?).lines() {
        let line = line?;
        if line.len() > DUMP_LINE_BYTES {
            return Err(invalid("dump record exceeds 64 MiB"));
        }
        let value: Value = serde_json::from_str(&line).map_err(io::Error::other)?;
        let file = string(&value["file"])?;
        if !expected.contains(file) || !seen.insert(file.to_owned()) {
            return Err(invalid("dump membership differs from declared files"));
        }
        let (mut opened, mut heads) = (Heads::new(), Heads::new());
        let source_lines = sources
            .iter()
            .find(|s| s["path"] == file)
            .and_then(|s| s["physical_lines"].as_u64())
            .ok_or_else(|| invalid("missing frozen physical line count"))?;
        let mut previous = 0;
        for record in array(&value["records"])? {
            let number = record["line"]
                .as_u64()
                .filter(|n| *n > 0 && *n >= previous && *n <= source_lines)
                .ok_or_else(|| invalid("unordered or source-out-of-range dump line"))?;
            previous = number;
            let symbols = array(&record["symbols"])?;
            let fragments = array(&record["fragments"])?;
            if fragments.len() != symbols.len() + 1
                || !record["define_end"].is_boolean()
                || fragments.iter().any(|f| {
                    !f.as_str()
                        .is_some_and(|s| s.chars().all(|c| u32::from(c) <= 255))
                })
            {
                return Err(invalid("incomplete dump record"));
            }
            for symbol in symbols {
                let symbol = array(symbol)?;
                if symbol.len() != 3 || !symbol[2].is_boolean() {
                    return Err(invalid("invalid dump symbol"));
                }
                let (mark, name) = (string(&symbol[0])?, string(&symbol[1])?);
                if mark.len() != 1
                    || !MARKS.contains(mark)
                    || !name.chars().all(|c| u32::from(c) <= 255)
                {
                    return Err(invalid("invalid dump mark or Latin-1 name"));
                }
                if mark == "$" {
                    heads.push((number, name.to_owned()));
                    opened.push((number, name.to_owned()));
                } else if mark == "}" {
                    if opened.iter().map(|h| &h.1).collect::<BTreeSet<_>>().len() >= 2 {
                        primitives.push(primitive(
                            project,
                            file,
                            origin,
                            opened[0].0,
                            number,
                            opened.clone(),
                            false,
                        ));
                    }
                    opened.clear();
                }
            }
        }
        if opened.iter().map(|h| &h.1).collect::<BTreeSet<_>>().len() >= 2 {
            primitives.push(primitive(
                project,
                file,
                origin,
                opened[0].0,
                source_lines,
                opened,
                true,
            ));
        }
        all_heads.insert(file.to_owned(), heads);
    }
    if seen != expected {
        return Err(invalid("dump omitted a selected file"));
    }
    Ok((primitives, all_heads))
}

fn offsets(bytes: &[u8]) -> Vec<usize> {
    std::iter::once(0)
        .chain(
            bytes
                .iter()
                .enumerate()
                .filter_map(|(at, b)| (*b == b'\n').then_some(at + 1)),
        )
        .collect()
}

fn physical_line(starts: &[usize], at: usize) -> u64 {
    starts.partition_point(|start| *start <= at) as u64
}

/// Mask comments and quoted tokens, retaining physical offsets. This proves no C fact.
fn mask(bytes: &[u8]) -> io::Result<Vec<u8>> {
    let mut masked = bytes.to_vec();
    let mut at = 0;
    while at < bytes.len() {
        let start = at;
        if bytes[at..].starts_with(b"/*") {
            at += 2;
            while at + 1 < bytes.len() && !bytes[at..].starts_with(b"*/") {
                at += 1;
            }
            if at + 1 >= bytes.len() {
                return Err(invalid("unterminated block comment"));
            }
            at += 2;
        } else if bytes[at..].starts_with(b"//") {
            at += 2;
            while at < bytes.len() {
                if bytes[at] == b'\n'
                    && !bytes[..at].ends_with(b"\\")
                    && !bytes[..at].ends_with(b"\\\r")
                {
                    break;
                }
                at += 1;
            }
        } else if matches!(bytes[at], b'\'' | b'"') {
            let quote = bytes[at];
            at += 1;
            loop {
                if at >= bytes.len() || bytes[at] == b'\n' {
                    return Err(invalid("unterminated quoted token"));
                }
                if bytes[at] == quote {
                    at += 1;
                    break;
                }
                if bytes[at] == b'\\' {
                    at += 1;
                    if bytes.get(at..at + 2) == Some(b"\r\n") {
                        at += 2;
                    } else {
                        at += 1;
                    }
                } else {
                    at += 1;
                }
            }
        } else {
            let splice = if bytes[at..].starts_with(b"\\\r\n") {
                3
            } else if bytes[at..].starts_with(b"\\\n") {
                2
            } else {
                0
            };
            if splice > 0
                && at > 0
                && bytes.get(at + splice).is_some_and(|next| {
                    let before = bytes[at - 1];
                    (before.is_ascii_alphanumeric() || before == b'_')
                        && (next.is_ascii_alphanumeric() || *next == b'_')
                        || matches!((before, *next), (b'/', b'*') | (b'*', b'/') | (b'/', b'/'))
                })
            {
                return Err(invalid("unsupported token splice"));
            }
            if splice > 0 {
                for byte in &mut masked[at..at + splice] {
                    if *byte != b'\n' {
                        *byte = b' ';
                    }
                }
                at += splice;
                continue;
            }
            at += 1;
            continue;
        }
        for byte in &mut masked[start..at.min(bytes.len())] {
            if *byte != b'\n' {
                *byte = b' ';
            }
        }
    }
    Ok(masked)
}

struct Group {
    start: usize,
    arms: Vec<(usize, usize)>,
    arm: usize,
    had_else: bool,
}

fn source_candidates(project: &str, file: &str, bytes: &[u8]) -> (Vec<Primitive>, Vec<Value>) {
    let starts = offsets(bytes);
    let mut masked = match mask(bytes) {
        Ok(masked) => masked,
        Err(error) => {
            return (
                vec![],
                vec![json!({"file":file,"reason":error.to_string(),"scope":"file"})],
            );
        }
    };
    let (mut groups, mut stack, mut limits) = (Vec::new(), Vec::<Group>::new(), Vec::new());
    let mut at = 0;
    while at < bytes.len() {
        let begin = at;
        let mut end = at;
        loop {
            while end < bytes.len() && bytes[end] != b'\n' {
                end += 1;
            }
            let continued = bytes[..end].ends_with(b"\\") || bytes[..end].ends_with(b"\\\r");
            if end < bytes.len() {
                end += 1;
            }
            if !continued || end >= bytes.len() {
                break;
            }
        }
        let line = masked[begin..end].trim_ascii_start();
        if line.first() == Some(&b'#') {
            let word = line[1..].trim_ascii_start();
            let word = &word[..word
                .iter()
                .position(|b| !b.is_ascii_alphabetic())
                .unwrap_or(word.len())];
            match word {
                b"if" | b"ifdef" | b"ifndef" => stack.push(Group {
                    start: begin,
                    arms: Vec::new(),
                    arm: end,
                    had_else: false,
                }),
                b"else" | b"elif" => {
                    if let Some(group) = stack.last_mut() {
                        if group.had_else {
                            limits.push(json!({"file":file,"line":physical_line(&starts,begin),"reason":"conditional arm after else"}));
                        }
                        group.arms.push((group.arm, begin));
                        group.arm = end;
                        group.had_else |= word == b"else";
                    } else {
                        limits.push(json!({"file":file,"line":physical_line(&starts,begin),"reason":"orphan conditional directive"}));
                    }
                }
                b"endif" => {
                    if let Some(mut group) = stack.pop() {
                        group.arms.push((group.arm, begin));
                        groups.push((group, end));
                    } else {
                        limits.push(json!({"file":file,"line":physical_line(&starts,begin),"reason":"orphan conditional directive"}));
                    }
                }
                _ => {}
            }
            for byte in &mut masked[begin..end] {
                if *byte != b'\n' {
                    *byte = b' ';
                }
            }
        }
        at = end;
    }
    for group in stack {
        limits.push(json!({"file":file,"line":physical_line(&starts,group.start),"reason":"unmatched conditional group"}));
    }
    let mut candidates = Vec::new();
    for (group, end) in groups {
        if group.arms.len() < 2 {
            continue;
        }
        if end - group.start > SOURCE_GROUP_BYTES {
            limits.push(json!({"file":file,"line":physical_line(&starts,group.start),"reason":"conditional group exceeds 256 KiB"}));
            continue;
        }
        let after_end = masked.len().min(end + SOURCE_TAIL_BYTES);
        let after = masked[end..after_end].trim_ascii_start();
        let mut shared_parameters = false;
        if after.first() == Some(&b'(') {
            let mut depth = 0usize;
            let mut close = None;
            for (index, byte) in after.iter().enumerate() {
                if *byte == b'(' {
                    depth += 1;
                } else if *byte == b')' {
                    depth -= 1;
                    if depth == 0 {
                        close = Some(index + 1);
                        break;
                    }
                }
            }
            shared_parameters = close.is_some_and(|index| after[index..].contains(&b'{'));
            if !shared_parameters
                && after_end < masked.len()
                && !close
                    .is_some_and(|index| after[index..].trim_ascii_start().first() == Some(&b';'))
            {
                limits.push(json!({"file":file,"line":physical_line(&starts,end),"reason":"post-endif shared parameter/body exceeds 512-byte window"}));
            }
        } else if after.is_empty() && after_end < masked.len() {
            limits.push(json!({"file":file,"line":physical_line(&starts,end),"reason":"post-endif trivia exceeds 512-byte window"}));
        }
        let shared_brace = after.first() == Some(&b'{');
        let mut heads = Vec::new();
        let mut usable = true;
        for (begin, finish) in &group.arms {
            let tail = masked[*begin..*finish].trim_ascii_end();
            let skip = tail.len().saturating_sub(SOURCE_TAIL_BYTES);
            let tail = &tail[skip..];
            let open = tail.last() == Some(&b'{');
            let cue = if shared_parameters {
                BARE.captures(tail)
            } else if open || shared_brace {
                HEAD.captures(tail)
            } else {
                None
            };
            if let Some(cue) = cue {
                let name = cue.get(1).expect("named expression");
                heads.push((
                    physical_line(&starts, begin + skip + name.start()),
                    String::from_utf8_lossy(name.as_bytes()).into_owned(),
                ));
            } else {
                usable = false;
                if skip > 0 {
                    limits.push(json!({"file":file,"line":physical_line(&starts,*begin),"reason":"arm suffix outside supported 512-byte head window"}));
                }
                break;
            }
        }
        if usable && heads.len() >= 2 {
            let mut candidate = primitive(
                project,
                file,
                Origin::Source,
                physical_line(&starts, group.start),
                physical_line(&starts, end.saturating_sub(1)),
                heads,
                false,
            );
            candidate.shape = Some(if shared_parameters {
                "split names/shared parameters"
            } else if shared_brace {
                "heads/shared opening brace"
            } else {
                "sibling opening braces/shared continuation"
            });
            candidates.push(candidate);
        }
    }
    (candidates, limits)
}

fn head_window(bytes: &[u8], anchor: u64) -> Value {
    let starts = offsets(bytes);
    let Some(index) = anchor
        .checked_sub(1)
        .and_then(|i| usize::try_from(i).ok())
        .filter(|i| *i < starts.len())
    else {
        return json!({"unassessed_reason":"anchor outside source"});
    };
    let here = starts[index];
    let mut begin = index;
    while begin > 0
        && index - begin < HEAD_BEFORE_LINES
        && here - starts[begin - 1] <= HEAD_BEFORE_BYTES
    {
        begin -= 1;
    }
    let mut finish = index;
    while finish < starts.len() && finish - index < HEAD_AFTER_LINES {
        let end = starts.get(finish + 1).copied().unwrap_or(bytes.len());
        if end - here > HEAD_AFTER_BYTES {
            break;
        }
        finish += 1;
    }
    json!({"start_line":begin+1,"end_line":finish,"start_byte":starts[begin],
        "end_byte":starts.get(finish).copied().unwrap_or(bytes.len()),
        "anchor_line":anchor,"anchor_complete":finish>index})
}

fn components(primitives: &[Primitive]) -> Vec<OverlapComponent> {
    let mut sorted = primitives.to_vec();
    sorted.sort_by_key(|p| {
        (
            p.project.clone(),
            p.file.clone(),
            p.start,
            p.end,
            p.id.clone(),
        )
    });
    let mut result = Vec::<OverlapComponent>::new();
    for p in sorted {
        let overlaps = result
            .last()
            .is_some_and(|c| c.project == p.project && c.file == p.file && p.start <= c.end);
        if overlaps {
            let c = result.last_mut().unwrap();
            c.end = c.end.max(p.end);
            c.primitives.push(p);
        } else {
            result.push(OverlapComponent {
                id: String::new(),
                project: p.project.clone(),
                file: p.file.clone(),
                start: p.start,
                end: p.end,
                stratum: p.origin,
                primitives: vec![p],
            });
        }
    }
    for component in &mut result {
        component.stratum = component
            .primitives
            .iter()
            .map(|p| p.origin)
            .min()
            .expect("nonempty overlap component");
        let tuple = format!(
            "{METHOD}\0{}\0{}\0{}\0{}",
            component.project, component.file, component.start, component.end
        );
        component.id = hash(tuple.as_bytes());
        for p in &mut component.primitives {
            p.selection_hash = hash(
                format!(
                    "{tuple}\0{}\0{}",
                    p.origin.name(),
                    json!(heads_json(&p.heads))
                )
                .as_bytes(),
            );
        }
    }
    result
}

fn sample(mut components: Vec<OverlapComponent>) -> Value {
    let mut chosen = BTreeMap::<(String, Origin), String>::new();
    for c in &components {
        let key = (c.project.clone(), c.stratum);
        let id = c.id.clone();
        chosen
            .entry(key)
            .and_modify(|current| {
                if id < *current {
                    *current = id.clone();
                }
            })
            .or_insert(id);
    }
    let mut sampled = Vec::new();
    for c in &mut components {
        let selected = chosen[&(c.project.clone(), c.stratum)] == c.id;
        c.primitives
            .sort_by_key(|p| (p.selection_hash.clone(), p.id.clone()));
        let mut picked = BTreeSet::new();
        if selected {
            for origin in [Origin::Current, Origin::Baseline, Origin::Source] {
                if let Some(p) = c.primitives.iter().find(|p| p.origin == origin) {
                    picked.insert(p.id.clone());
                }
            }
            for p in &c.primitives {
                if picked.len() >= MAX_PRIMITIVES {
                    break;
                }
                picked.insert(p.id.clone());
            }
        }
        let primitives=c.primitives.iter().map(|p| {
            let inspected=picked.contains(&p.id);
            let mut record=p.json();
            record["selected"] = json!(inspected);
            record["inspection_status"] = json!(if inspected {
                "pending independent inspection"
            } else {
                "unassessed"
            });
            record["body_window"] =
                json!({"join_line":null,"status":"pending independent proof","limits":limits()});
            record
        }).collect::<Vec<_>>();
        sampled.push(json!({"id":c.id,"project":c.project,"file":c.file,"start":c.start,"end":c.end,"stratum":c.stratum.name(),"selected":selected,"primitives":primitives}));
    }
    json!({"method":METHOD,"limits":limits(),"components":sampled})
}

fn inventory(binary: &Path, frame_path: &Path, destination: &Path) -> io::Result<()> {
    let frame = read(frame_path)?;
    let frame_sha256 = capture::digest(frame_path)?;
    if frame["method"] != METHOD || !frame["method_sha256"].is_string() {
        return Err(invalid("frame must freeze conditional-heads-v2 method"));
    }
    if capture::digest(binary)? != string(&frame["binary_sha256"])? {
        return Err(invalid("frozen binary changed"));
    }
    let projects = array(&frame["projects"])?;
    if projects.len() > MAX_PROJECTS {
        return Err(invalid("at most 40 maintained projects"));
    }
    let seconds = match frame.get("capture_seconds") {
        None => CAPTURE_SECONDS,
        Some(value) => value
            .as_u64()
            .filter(|seconds| *seconds <= CAPTURE_SECONDS)
            .ok_or_else(|| invalid("capture_seconds must shorten the 600-second limit"))?,
    };
    let mut names = BTreeSet::new();
    let mut sealed = Vec::new();
    for project in projects {
        let name = string(&project["name"])?;
        if name.is_empty()
            || !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
            || !names.insert(name)
        {
            return Err(invalid("invalid or duplicate project name"));
        }
        sealed.push(project_sources(project)?);
    }
    fs::create_dir(destination)?;
    capture::save(&destination.join("frame.json"), &frame)?;
    let (mut primitives, mut source_limits, mut identities) = (Vec::new(), Vec::new(), Vec::new());
    for (project, sources) in projects.iter().zip(&sealed) {
        let name = string(&project["name"])?;
        let root = Path::new(string(&project["root"])?);
        let output = destination.join(name);
        fs::create_dir(&output)?;
        let listing = output.join("files.txt");
        let files = sources
            .iter()
            .map(|s| string(&s["path"]))
            .collect::<io::Result<Vec<_>>>()?;
        fs::write(&listing, format!("{}\n", files.join("\n")))?;
        capture::save(&output.join("sources.json"), &json!(sources))?;
        for (origin, reading) in [
            ("baseline", ""),
            ("enabled", string(&frame["enabled_corrections"])?),
        ] {
            let command = capture::run_guarded(
                binary,
                &[
                    "--dump-units".into(),
                    "--corrections".into(),
                    reading.into(),
                    "-i".into(),
                    listing.display().to_string(),
                ],
                root,
                &output.join(origin),
                seconds,
                frame["memory_scope"].as_bool().unwrap_or(true),
                &[(destination.to_owned(), ARTIFACT_BYTES)],
            )?;
            if command["complete"] != true || command["stderr_bytes"] != 0 {
                return Err(invalid(format!("incomplete {name}/{origin} dump")));
            }
        }
        let (current, enabled_heads) = mark_runs(
            &output.join("enabled.stdout"),
            name,
            sources,
            Origin::Current,
        )?;
        let (baseline, _) = mark_runs(
            &output.join("baseline.stdout"),
            name,
            sources,
            Origin::Baseline,
        )?;
        primitives.extend(current);
        primitives.extend(baseline.into_iter().filter(|p| {
            enabled_heads[&p.file]
                .iter()
                .filter(|h| h.0 >= p.start && h.0 <= p.end)
                .map(|h| &h.1)
                .collect::<BTreeSet<_>>()
                .len()
                < 2
        }));
        for file in &files {
            let bytes = fs::read(root.join(file))?;
            let (candidates, mut limitations) = source_candidates(name, file, &bytes);
            for limit in &mut limitations {
                limit["project"] = json!(name);
            }
            source_limits.extend(limitations);
            primitives.extend(candidates);
            for p in primitives
                .iter_mut()
                .filter(|p| p.project == name && p.file == *file)
            {
                p.head_window = head_window(&bytes, p.heads.first().map_or(0, |h| h.0));
            }
        }
        if project_sources(project)? != *sources {
            return Err(invalid("corpus identity changed during capture"));
        }
        identities.push(json!({"project":name,"selected_files":sources.len(),"manifest_sha256":project["manifest_sha256"]}));
        println!(
            "{name}: {} primitives",
            primitives.iter().filter(|p| p.project == name).count()
        );
    }
    if capture::digest(binary)? != string(&frame["binary_sha256"])? {
        return Err(invalid("binary changed during inventory"));
    }
    if capture::digest(frame_path)? != frame_sha256 {
        return Err(invalid("frame changed during inventory"));
    }
    for (project, sources) in projects.iter().zip(&sealed) {
        if project_sources(project)? != *sources {
            return Err(invalid(
                "corpus identity changed before inventory completion",
            ));
        }
    }
    let components = components(&primitives);
    let counts=projects.iter().map(|project|{
        let name=project["name"].as_str().expect("validated project name");
        let limitation_count=source_limits.iter().filter(|limit|limit["project"]==name).count();
        json!({"project":name,"primitives":primitives.iter().filter(|p|p.project==name).count(),
            "current":primitives.iter().filter(|p|p.project==name && p.origin==Origin::Current).count(),
            "baseline":primitives.iter().filter(|p|p.project==name && p.origin==Origin::Baseline).count(),
            "source":primitives.iter().filter(|p|p.project==name && p.origin==Origin::Source).count(),
            "components":components.iter().filter(|c|c.project==name).count(),
            "source_discovery_complete":limitation_count==0,"source_limitation_count":limitation_count,
            "source_limitations":{"artifact":"candidates.json","array":"source_limitations","project":name}})
    }).collect::<Vec<_>>();
    capture::save(
        &destination.join("candidates.json"),
        &json!({"method":METHOD,"primitives":primitives.iter().map(Primitive::json).collect::<Vec<_>>(),"source_limitations":source_limits,"counts":counts}),
    )?;
    let mut sampled = sample(components);
    sampled["execution_complete"] = json!(true);
    sampled["source_discovery_complete"] = json!(source_limits.is_empty());
    sampled["source_limitations"] =
        json!({"artifact":"candidates.json","count":source_limits.len()});
    capture::save(
        &destination.join("report.json"),
        &json!({"complete":true,"complete_means":"execution, validated paired mark captures and source identity checks; lexical discovery may be partial",
        "counts_mean":"discovered primitive candidates and overlap components; not confirmed relationships or losses",
        "frame_sha256":frame_sha256,
        "binary_sha256":capture::digest(binary)?,"runner_sha256":capture::digest(&std::env::current_exe()?)?,
        "method_sha256":frame["method_sha256"],"projects":identities,"counts":counts,
        "paired_mark_capture_complete":true,"source_discovery_complete":source_limits.is_empty(),
        "claim":"candidate inventory only; independent relationship and call labels pending"}),
    )?;
    capture::save(&destination.join("sample.json"), &sampled)
}

pub fn cli(args: &[String]) -> io::Result<()> {
    match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["inventory", binary, frame, destination] => {
            let destination = Path::new(destination);
            if destination.exists() {
                return Err(invalid("output directory must be unused"));
            }
            let result = inventory(Path::new(binary), Path::new(frame), destination);
            if let Err(error) = &result
                && destination.is_dir()
            {
                let _ = fs::remove_file(destination.join("sample.json"));
                let _ = capture::save(
                    &destination.join("report.json"),
                    &json!({"complete":false,"error":error.to_string(),"sample_valid":false}),
                );
            }
            result
        }
        _ => Err(invalid(
            "usage: conditional-heads inventory BINARY FRAME NEW_OUTPUT_DIR",
        )),
    }
}

fn main() -> io::Result<()> {
    cli(&std::env::args().skip(1).collect::<Vec<_>>())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    use sha2::{Digest, Sha256};
    use std::{fs, os::unix::fs::PermissionsExt, path::Path, process::Command};

    fn digest(path: &Path) -> String {
        format!("{:x}", Sha256::digest(fs::read(path).unwrap()))
    }

    fn fixture(source: &str, baseline: Value, enabled: Value) -> tempfile::TempDir {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("source");
        fs::create_dir(&root).unwrap();
        fs::write(root.join("case.c"), source).unwrap();
        for args in [
            vec!["init", "--quiet"],
            vec!["add", "case.c"],
            vec![
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "commit",
                "--quiet",
                "-m",
                "fixture",
            ],
        ] {
            assert!(
                Command::new("git")
                    .current_dir(&root)
                    .args(args)
                    .status()
                    .unwrap()
                    .success()
            );
        }
        let revision = Command::new("git")
            .current_dir(&root)
            .args(["rev-parse", "HEAD"])
            .output()
            .unwrap();
        let manifest = json!({"corpus":{"revision":String::from_utf8(revision.stdout).unwrap().trim(),
            "files":[{"path":"case.c","bytes":source.len(),"sha256":digest(&root.join("case.c"))}]}});
        fs::write(
            temp.path().join("manifest.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        for (name, value) in [("baseline", baseline), ("enabled", enabled)] {
            fs::write(temp.path().join(name), format!("{value}\n")).unwrap();
        }
        let program = temp.path().join("product");
        fs::write(&program, format!("#!/bin/sh\ncase \"$*\" in *'--corrections enabled'*) cat '{}' ;; *) cat '{}' ;; esac\n",temp.path().join("enabled").display(),temp.path().join("baseline").display())).unwrap();
        fs::set_permissions(&program, fs::Permissions::from_mode(0o700)).unwrap();
        let frame = json!({"method":"conditional-heads-v2","method_sha256":"fixture-method",
            "binary_sha256":digest(&program),"enabled_corrections":"enabled","memory_scope":false,
            "projects":[{"name":"fixture","root":root,"manifest":temp.path().join("manifest.json"),
                "manifest_sha256":digest(&temp.path().join("manifest.json"))}]});
        fs::write(
            temp.path().join("frame.json"),
            serde_json::to_vec(&frame).unwrap(),
        )
        .unwrap();
        temp
    }

    fn dump(heads: &[(&str, u64)], end: u64) -> Value {
        let mut records = heads.iter().map(|(name,line)|json!({"line":line,"symbols":[["$",name,false]],"fragments":["", ""],"define_end":false})).collect::<Vec<_>>();
        records.push(
            json!({"line":end,"symbols":[["}","",false]],"fragments":["", ""],"define_end":false}),
        );
        json!({"file":"case.c","records":records})
    }

    fn invoke(temp: &tempfile::TempDir) -> io::Result<()> {
        cli(&[
            "inventory".into(),
            temp.path().join("product").display().to_string(),
            temp.path().join("frame.json").display().to_string(),
            temp.path().join("out").display().to_string(),
        ])
    }

    fn read(temp: &tempfile::TempDir, file: &str) -> Value {
        serde_json::from_slice(&fs::read(temp.path().join("out").join(file)).unwrap()).unwrap()
    }

    #[test]
    fn cli_preserves_overlapping_origins_and_fixed_component_selection() {
        let source = "#if A\nvoid left(void) {\n#elif B\nvoid middle(void) {\n#else\nvoid right(void) {\n#endif\ntarget();\n}\n";
        let temp = fixture(
            source,
            dump(
                &[("wrapper", 1), ("left", 2), ("middle", 4), ("right", 6)],
                9,
            ),
            dump(&[("left", 2), ("middle", 4), ("right", 6)], 9),
        );
        invoke(&temp).unwrap();
        let candidates = read(&temp, "candidates.json");
        let primitives = candidates["primitives"].as_array().unwrap();
        assert!(primitives.iter().any(|p| p["origin"] == "current"));
        assert!(primitives.iter().any(|p| p["origin"] == "source"));
        let sample = read(&temp, "sample.json");
        let components = sample["components"].as_array().unwrap();
        assert_eq!(components.len(), 1);
        assert!(components[0]["primitives"].as_array().unwrap().len() >= 2);
        assert_eq!(sample["limits"]["components"], 120);
        assert_eq!(sample["limits"]["primitives_per_component"], 4);
        assert_eq!(sample["limits"]["body_lines"], 256);
        assert_eq!(sample["limits"]["body_bytes"], 24576);
        assert!(primitives.iter().all(|p| p.get("classification").is_none()));
        assert!(
            invoke(&temp).is_err(),
            "existing captures must not be overwritten"
        );
    }

    #[test]
    fn cli_exposes_source_only_split_names_and_masked_directives() {
        let source = "char *s = \"#if FAKE\";\n/* #if HIDDEN */\nint\n#if A\nleft\n#else\nright\n#endif\n(void)\n{ target(); }\n";
        let temp = fixture(source, dump(&[("right", 7)], 10), dump(&[("right", 7)], 10));
        invoke(&temp).unwrap();
        let candidates = read(&temp, "candidates.json");
        let primitives = candidates["primitives"].as_array().unwrap();
        assert_eq!(primitives.len(), 1, "{candidates}");
        assert_eq!(primitives[0]["origin"], "source");
        assert_eq!(primitives[0]["heads"][0]["name"], "left");
        assert_eq!(primitives[0]["heads"][1]["name"], "right");
        assert_eq!(
            read(&temp, "sample.json")["components"][0]["stratum"],
            "source"
        );
    }

    #[test]
    fn cli_rejects_drift_and_missing_capture_members_before_a_valid_sample() {
        let temp = fixture("int x;\n", dump(&[], 1), dump(&[], 1));
        fs::write(temp.path().join("source/case.c"), "int changed;\n").unwrap();
        assert!(invoke(&temp).is_err());
        assert!(!temp.path().join("out/sample.json").exists());
        let temp = fixture(
            "int x;\n",
            dump(&[], 1),
            json!({"file":"other.c","records":[]}),
        );
        assert!(invoke(&temp).is_err());
        assert!(!temp.path().join("out/sample.json").exists());
        assert!(
            temp.path().join("out/fixture/enabled.stdout").exists(),
            "retain failed raw output"
        );
    }

    #[test]
    fn cli_caps_primitives_in_an_overlap_component_without_replacement() {
        let mut source = String::new();
        let mut heads = Vec::new();
        for i in 0..5 {
            source.push_str(&format!(
                "#if A\nvoid left{i}(void) {{\n#else\nvoid right{i}(void) {{\n#endif\n"
            ));
            heads.push((format!("left{i}"), i * 5 + 2));
            heads.push((format!("right{i}"), i * 5 + 4));
        }
        source.push_str("target();\n}\n");
        let head_refs = heads
            .iter()
            .map(|(name, line)| (name.as_str(), *line))
            .collect::<Vec<_>>();
        let temp = fixture(&source, dump(&head_refs, 27), dump(&head_refs, 27));
        invoke(&temp).unwrap();
        let sample = read(&temp, "sample.json");
        let components = sample["components"].as_array().unwrap();
        assert_eq!(components.len(), 1);
        let primitives = components[0]["primitives"].as_array().unwrap();
        assert_eq!(primitives.len(), 6);
        assert_eq!(
            primitives.iter().filter(|p| p["selected"] == true).count(),
            4
        );
        assert_eq!(
            primitives
                .iter()
                .filter(|p| p["inspection_status"] == "unassessed")
                .count(),
            2
        );
        assert!(
            primitives
                .iter()
                .filter(|p| p["selected"] == true)
                .any(|p| p["origin"] == "current")
        );
        assert!(
            primitives
                .iter()
                .filter(|p| p["selected"] == true)
                .any(|p| p["origin"] == "source")
        );
        cli(&[
            "inventory".into(),
            temp.path().join("product").display().to_string(),
            temp.path().join("frame.json").display().to_string(),
            temp.path().join("repeat").display().to_string(),
        ])
        .unwrap();
        let repeated: Value =
            serde_json::from_slice(&fs::read(temp.path().join("repeat/sample.json")).unwrap())
                .unwrap();
        assert_eq!(
            sample, repeated,
            "same declared frame must select the same primitive records"
        );
    }

    #[test]
    fn cli_retains_same_names_and_source_bounds_without_correctness_judgments() {
        let source = "#if A \\\n+ || B\nvoid same(void) {\n#else\nvoid same(void) {\n#endif\n}\n";
        let temp = fixture(
            source,
            dump(&[("same", 3), ("same", 5)], 7),
            dump(&[("same", 3), ("same", 5)], 7),
        );
        invoke(&temp).unwrap();
        let candidates = read(&temp, "candidates.json");
        assert_eq!(candidates["primitives"].as_array().unwrap().len(), 1);
        assert_eq!(candidates["primitives"][0]["origin"], "source");
        assert_eq!(candidates["primitives"][0]["heads"][0]["name"], "same");
        let source = format!(
            "#if A\nvoid left(void) {{\n/*{}*/\n#else\nvoid right(void) {{\n#endif\n}}\n",
            " ".repeat(270000)
        );
        let temp = fixture(
            &source,
            dump(&[("left", 2), ("right", 5)], 7),
            dump(&[("left", 2), ("right", 5)], 7),
        );
        invoke(&temp).unwrap();
        assert_eq!(
            read(&temp, "report.json")["source_discovery_complete"],
            false
        );
        assert!(
            read(&temp, "candidates.json")["source_limitations"]
                .as_array()
                .unwrap()
                .iter()
                .any(|l| l["reason"] == "conditional group exceeds 256 KiB")
        );
        let sample = read(&temp, "sample.json");
        assert_eq!(
            sample["limits"]["body_window_rule"],
            "one window per primitive, earliest independently proved shared-body join"
        );
        assert!(
            sample["components"][0]["primitives"][0]
                .get("classification")
                .is_none()
        );
    }

    #[test]
    fn cli_keeps_corrected_and_incomplete_marks_without_inventing_relationships() {
        let source = "#define WRAP(X) X\nint WRAP(left(void));\nint right(void) {\n";
        let baseline = json!({"file":"case.c","records":[
            {"line":2,"symbols":[["$","WRAP",false],["$","left",false]],"fragments":["","",""],"define_end":false},
            {"line":3,"symbols":[["$","right",false]],"fragments":["",""],"define_end":false}]});
        let enabled = json!({"file":"case.c","records":[
            {"line":3,"symbols":[["$","right",false]],"fragments":["",""],"define_end":false}]});
        let temp = fixture(source, baseline, enabled);
        invoke(&temp).unwrap();
        let candidates = read(&temp, "candidates.json");
        assert_eq!(candidates["primitives"][0]["origin"], "baseline");
        assert_eq!(candidates["primitives"][0]["incomplete"], true);
        assert_eq!(candidates["primitives"][0]["end"], 4);
        assert!(candidates["primitives"][0].get("classification").is_none());
    }

    #[test]
    fn cli_retains_a_failed_command_as_incomplete_without_a_sample() {
        let temp = fixture("int x;\n", dump(&[], 1), dump(&[], 1));
        let program = temp.path().join("product");
        fs::write(&program, "#!/bin/sh\nprintf partial\nexit 1\n").unwrap();
        let frame_path = temp.path().join("frame.json");
        let mut frame: Value = serde_json::from_slice(&fs::read(&frame_path).unwrap()).unwrap();
        frame["binary_sha256"] = json!(digest(&program));
        fs::write(&frame_path, serde_json::to_vec(&frame).unwrap()).unwrap();
        assert!(invoke(&temp).is_err());
        assert!(!temp.path().join("out/sample.json").exists());
        assert_eq!(read(&temp, "report.json")["complete"], false);
        assert_eq!(
            fs::read_to_string(temp.path().join("out/fixture/baseline.stdout")).unwrap(),
            "partial"
        );
    }

    #[test]
    fn cli_rejects_malformed_source_locations_and_symbols() {
        for fault in ["line", "fragment", "mark", "json"] {
            let mut enabled = dump(&[("head", 1)], 1);
            match fault {
                "line" => {
                    enabled["records"][0]["line"] = json!(900);
                    enabled["records"][1]["line"] = json!(900);
                }
                "fragment" => enabled["records"][0]["fragments"][0] = json!(99),
                "mark" => enabled["records"][0]["symbols"][0][0] = json!("unknown"),
                _ => {}
            }
            let temp = fixture("int x;\n", dump(&[], 1), enabled);
            if fault == "json" {
                fs::write(temp.path().join("enabled"), "{broken").unwrap();
            }
            assert!(invoke(&temp).is_err(), "malformed {fault} must fail");
            assert_eq!(read(&temp, "report.json")["complete"], false);
            assert!(!temp.path().join("out/sample.json").exists());
            assert!(temp.path().join("out/fixture/enabled.stdout").exists());
        }
    }

    #[test]
    fn cli_reports_orphan_spliced_and_truncated_conditional_discovery() {
        for (source, expected) in [
            (
                "#else\n#elif A\n#endif\n".to_owned(),
                "orphan conditional directive",
            ),
            (
                "#i\\\nf A\nleft\n#else\nright\n#endif\n(void) { target(); }\n".to_owned(),
                "unsupported token splice",
            ),
            (
                format!(
                    "#if A\nleft\n#else\nright\n#endif\n({}void) {{ target(); }}\n",
                    "int parameter, ".repeat(80)
                ),
                "post-endif shared parameter/body exceeds 512-byte window",
            ),
        ] {
            let temp = fixture(&source, dump(&[], 1), dump(&[], 1));
            invoke(&temp).unwrap();
            assert_eq!(
                read(&temp, "report.json")["source_discovery_complete"],
                false,
                "{expected}"
            );
            let limitations = read(&temp, "candidates.json");
            assert!(
                limitations["source_limitations"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|l| l["reason"] == expected),
                "{limitations}"
            );
        }
    }

    #[test]
    fn cli_preserves_nested_and_parameter_observations_but_excludes_complete_bodies() {
        let independent =
            "#if A\nvoid left(void) { target(); }\n#else\nvoid right(void) { target(); }\n#endif\n";
        let mut separate = dump(&[("left", 2)], 2);
        let second = dump(&[("right", 4)], 4);
        separate["records"]
            .as_array_mut()
            .unwrap()
            .extend(second["records"].as_array().unwrap().clone());
        let temp = fixture(independent, separate.clone(), separate);
        invoke(&temp).unwrap();
        assert!(
            read(&temp, "candidates.json")["primitives"]
                .as_array()
                .unwrap()
                .is_empty()
        );

        let parameter = "void real(void callback(int)) { target(); }\n";
        let marks = dump(&[("real", 1), ("callback", 1)], 1);
        let temp = fixture(parameter, marks.clone(), marks);
        invoke(&temp).unwrap();
        let candidates = read(&temp, "candidates.json");
        assert_eq!(
            candidates["primitives"][0]["heads"],
            json!([{"line":1,"name":"real"},{"line":1,"name":"callback"}])
        );
        assert!(candidates["primitives"][0].get("classification").is_none());

        let nested = "#if OUTER\n#if INNER\nvoid first(void) {\n#else\nvoid second(void) {\n#endif\n#else\nvoid third(void) {\n#endif\ntarget();\n}\n";
        let marks = dump(&[("first", 3), ("second", 5), ("third", 8)], 11);
        let temp = fixture(nested, marks.clone(), marks);
        invoke(&temp).unwrap();
        let sample = read(&temp, "sample.json");
        let primitives = sample["components"][0]["primitives"].as_array().unwrap();
        assert_eq!(
            primitives
                .iter()
                .filter(|p| p["origin"] == "source")
                .count(),
            2
        );
        assert_eq!(
            primitives
                .iter()
                .filter(|p| p["origin"] == "source")
                .map(|p| p["start"].as_u64().unwrap())
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([1, 2])
        );
    }

    #[test]
    fn cli_limits_actual_head_offsets_and_does_not_judge_a_partial_body() {
        let source = format!("{}target();\n", (" ".repeat(200) + "\n").repeat(200));
        let temp = fixture(
            &source,
            dump(&[("left", 50), ("right", 51)], 201),
            dump(&[("left", 50), ("right", 51)], 201),
        );
        invoke(&temp).unwrap();
        let candidates = read(&temp, "candidates.json");
        let window = &candidates["primitives"][0]["head_window"];
        assert_eq!(window["start_line"], 30);
        assert_eq!(window["end_line"], 110);
        assert!(
            window["end_byte"].as_u64().unwrap() - window["start_byte"].as_u64().unwrap() <= 16384
        );

        let source = format!(
            "#if A\nvoid left(void) {{\n#else\nvoid right(void) {{\n#endif\n{}late_call();\n}}\n",
            "int local;\n".repeat(300)
        );
        let temp = fixture(
            &source,
            dump(&[("left", 2), ("right", 4)], 307),
            dump(&[("left", 2), ("right", 4)], 307),
        );
        invoke(&temp).unwrap();
        let sample = read(&temp, "sample.json");
        assert_eq!(sample["limits"]["body_lines"], 256);
        let selected = &sample["components"][0]["primitives"][0];
        assert_eq!(selected["body_window"]["join_line"], Value::Null);
        assert_eq!(
            selected["inspection_status"],
            "pending independent inspection"
        );
        assert!(selected.get("classification").is_none());
        assert!(
            source
                .lines()
                .position(|line| line.contains("late_call"))
                .unwrap()
                + 1
                > 6 + 256
        );
    }

    #[test]
    fn cli_deadline_keeps_partial_evidence_and_invalidates_the_sample() {
        let temp = fixture("int x;\n", dump(&[], 1), dump(&[], 1));
        let program = temp.path().join("product");
        fs::write(&program, "#!/bin/sh\nprintf partial\nsleep 5\n").unwrap();
        let frame_path = temp.path().join("frame.json");
        let mut frame: Value = serde_json::from_slice(&fs::read(&frame_path).unwrap()).unwrap();
        frame["binary_sha256"] = json!(digest(&program));
        frame["capture_seconds"] = json!(0);
        fs::write(&frame_path, serde_json::to_vec(&frame).unwrap()).unwrap();
        assert!(invoke(&temp).is_err());
        assert_eq!(read(&temp, "report.json")["complete"], false);
        assert!(!temp.path().join("out/sample.json").exists());
        let capture: Value = serde_json::from_slice(
            &fs::read(temp.path().join("out/fixture/baseline.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(capture["stopped"], "timeout");
    }

    #[test]
    fn cli_retains_valid_source_suspects_beside_explicit_partial_discovery() {
        let source = "#if A\nleft\n#else\nright\n#endif\n(void) { target(); }\n#else\n";
        let temp = fixture(source, dump(&[("right", 4)], 6), dump(&[("right", 4)], 6));
        invoke(&temp).unwrap();
        let report = read(&temp, "report.json");
        assert_eq!(report["complete"], true, "execution completed");
        assert_eq!(report["paired_mark_capture_complete"], true);
        assert_eq!(report["source_discovery_complete"], false);
        assert_eq!(report["counts"][0]["source"], 1);
        assert_eq!(report["counts"][0]["source_discovery_complete"], false);
        assert_eq!(report["counts"][0]["source_limitation_count"], 1);
        assert_eq!(
            report["counts"][0]["source_limitations"]["artifact"],
            "candidates.json"
        );
        let sample = read(&temp, "sample.json");
        assert_eq!(sample["execution_complete"], true);
        assert_eq!(sample["source_discovery_complete"], false);
        assert_eq!(sample["components"].as_array().unwrap().len(), 1);
        assert_eq!(sample["components"][0]["stratum"], "source");
        assert_eq!(sample["components"][0]["selected"], true);
        assert_eq!(
            sample["components"][0]["primitives"][0]["heads"],
            json!([{"line":2,"name":"left"},{"line":4,"name":"right"}])
        );
        let candidates = read(&temp, "candidates.json");
        assert_eq!(candidates["source_limitations"][0]["line"], 7);
        assert_eq!(
            candidates["source_limitations"][0]["reason"],
            "orphan conditional directive"
        );
    }

    #[test]
    fn cli_preserves_declaration_and_directive_continuations_at_physical_lines() {
        for ending in ["\n", "\r\n"] {
            for (text, lines) in [
                (
                    "#if A\nvoid left \\\n(void) {\n#else\nvoid right \\\n(void) {\n#endif\ntarget();\n}\n",
                    [2, 5],
                ),
                (
                    "# \\\nif A\nleft\n# \\\nelse\nright\n# \\\nendif\n(void) { target(); }\n",
                    [3, 6],
                ),
            ] {
                let source = text.replace('\n', ending);
                let temp = fixture(
                    &source,
                    dump(&[("right", lines[1])], source.lines().count() as u64),
                    dump(&[("right", lines[1])], source.lines().count() as u64),
                );
                invoke(&temp).unwrap();
                let candidates = read(&temp, "candidates.json");
                assert_eq!(
                    candidates["primitives"].as_array().unwrap().len(),
                    1,
                    "{ending:?}: {candidates}"
                );
                let candidate = &candidates["primitives"][0];
                assert_eq!(candidate["origin"], "source");
                assert_eq!(
                    candidate["heads"],
                    json!([{"line":lines[0],"name":"left"},{"line":lines[1],"name":"right"}])
                );
                assert_eq!(
                    read(&temp, "report.json")["source_discovery_complete"],
                    true
                );
            }
        }
    }
}
