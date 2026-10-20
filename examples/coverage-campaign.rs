//! Acquire a frozen project inventory and draw source-assessment witnesses.

#[path = "support/capture.rs"]
mod capture;
#[path = "support/source_inventory.rs"]
mod source_inventory;

use regex::Regex;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
const SEED: &str = "ripscope-coverage-2026-10-v1";

fn read(path: &Path) -> Result<Value> {
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}

fn text(value: &Value) -> Result<&str> {
    value
        .as_str()
        .ok_or_else(|| io::Error::other("expected string").into())
}

fn strings(value: &Value) -> Vec<&str> {
    value.as_array().map_or_else(Vec::new, |items| {
        items.iter().filter_map(Value::as_str).collect()
    })
}

fn checked(result: Value) -> Result<String> {
    if result["complete"] != true {
        return Err(io::Error::other(format!("command incomplete: {result}")).into());
    }
    Ok(fs::read_to_string(text(&result["stdout_path"])?)?)
}

fn git(args: &[String], cwd: &Path, prefix: &Path) -> Result<String> {
    checked(capture::run(
        Path::new("git"),
        args,
        cwd,
        prefix,
        1800,
        false,
    )?)
}

fn version(name: &str) -> Vec<u64> {
    name.split(|ch: char| !ch.is_ascii_digit())
        .filter_map(|part| part.parse().ok())
        .collect()
}

fn acquire(entry: &Value, root: &Path) -> Result<Value> {
    let name = text(&entry["name"])?;
    let source = root
        .join("sources")
        .join(entry["source_directory"].as_str().unwrap_or(name));
    let logs = root.join("acquisition").join(name);
    fs::create_dir_all(&logs)?;
    let url = text(&entry["url"])?;
    let (tag, revision, paths) = if let Some(archive_url) = entry["archive_url"].as_str() {
        let archive = logs.join("release.tar");
        checked(capture::run(
            Path::new("curl"),
            &[
                "--fail".into(),
                "--location".into(),
                "--retry".into(),
                "2".into(),
                "--output".into(),
                archive.to_string_lossy().into_owned(),
                archive_url.into(),
            ],
            root,
            &logs.join("download"),
            1800,
            false,
        )?)?;
        let listing = checked(capture::run(
            Path::new("tar"),
            &["-tf".into(), archive.to_string_lossy().into_owned()],
            root,
            &logs.join("archive-list"),
            120,
            false,
        )?)?;
        if listing.lines().any(|name| {
            Path::new(name).is_absolute()
                || Path::new(name)
                    .components()
                    .any(|part| part == std::path::Component::ParentDir)
        }) {
            return Err(io::Error::other("unsafe archive path").into());
        }
        if !source.exists() {
            fs::create_dir(&source)?;
            checked(capture::run(
                Path::new("tar"),
                &[
                    "-xf".into(),
                    archive.to_string_lossy().into_owned(),
                    "--strip-components=1".into(),
                    "-C".into(),
                    source.to_string_lossy().into_owned(),
                ],
                root,
                &logs.join("extract"),
                120,
                false,
            )?)?;
        }
        let mut paths = Vec::new();
        tree_files(&source, &source, &mut paths)?;
        (
            Some(text(&entry["release"])?.to_owned()),
            format!("archive-sha256:{}", capture::digest(&archive)?),
            paths.join("\0"),
        )
    } else {
        let refs = git(
            &["ls-remote".into(), "--tags".into(), url.into()],
            root,
            &logs.join("tags"),
        )?;
        let pattern = Regex::new(text(&entry["tag_pattern"])?)?;
        let mut tags = BTreeMap::new();
        for line in refs.lines() {
            let Some((revision, reference)) = line.split_once('\t') else {
                continue;
            };
            let Some(tag) = reference.strip_prefix("refs/tags/") else {
                continue;
            };
            let peeled = tag.ends_with("^{}");
            let tag = tag.trim_end_matches("^{}");
            if pattern.is_match(tag) && (peeled || !tags.contains_key(tag)) {
                tags.insert(tag.to_owned(), revision.to_owned());
            }
        }
        let (tag, revision) = if let Some(forced) = entry["release"].as_str() {
            (
                Some(forced.to_owned()),
                tags.remove(forced)
                    .ok_or_else(|| io::Error::other("declared stable tag missing"))?,
            )
        } else if let Some((tag, revision)) = tags.into_iter().max_by_key(|(tag, _)| version(tag)) {
            (Some(tag), revision)
        } else if entry["allow_head"] == true {
            let head = git(
                &["ls-remote".into(), url.into(), "HEAD".into()],
                root,
                &logs.join("head"),
            )?;
            (
                None,
                head.split_whitespace()
                    .next()
                    .ok_or_else(|| io::Error::other("no HEAD"))?
                    .to_owned(),
            )
        } else {
            return Err(io::Error::other("no matching stable release tag").into());
        };
        if !source.exists() {
            let mut args = vec![
                "-c".into(),
                "core.autocrlf=false".into(),
                "clone".into(),
                "--depth".into(),
                "1".into(),
                "--no-tags".into(),
            ];
            if let Some(tag) = &tag {
                args.extend(["--branch".into(), tag.clone()]);
            }
            args.extend([url.into(), source.to_string_lossy().into_owned()]);
            git(&args, root, &logs.join("clone"))?;
        }
        let actual = git(
            &["rev-parse".into(), "HEAD".into()],
            &source,
            &logs.join("revision"),
        )?;
        if actual.trim() != revision {
            return Err(io::Error::other("checkout differs from frozen revision").into());
        }
        let status = git(
            &["status".into(), "--porcelain".into()],
            &source,
            &logs.join("status"),
        )?;
        if !status.is_empty() {
            return Err(io::Error::other("checkout is not clean").into());
        }
        let paths = git(
            &["ls-files".into(), "-z".into()],
            &source,
            &logs.join("files"),
        )?;
        (tag, revision, paths)
    };
    let excludes = strings(&entry["exclude"]);
    let mut files = Vec::new();
    let mut excluded = Vec::new();
    let mut includes = BTreeSet::new();
    let mut hashes = BTreeMap::<String, Vec<String>>::new();
    for name in paths.split('\0').filter(|name| !name.is_empty()) {
        if !matches!(
            Path::new(name).extension().and_then(|ext| ext.to_str()),
            Some("c" | "h")
        ) {
            continue;
        }
        let path = source.join(name);
        let reason = if excludes.iter().any(|prefix| name.starts_with(prefix)) {
            Some("declared non-C or third-party subtree")
        } else if path.is_symlink() {
            Some("symbolic link inventoried separately")
        } else {
            None
        };
        if let Some(reason) = reason {
            excluded.push(json!({"path":name,"reason":reason}));
            continue;
        }
        let bytes = fs::read(&path)?;
        let hash = capture::digest(&path)?;
        let lines = bytes.iter().filter(|byte| **byte == b'\n').count()
            + usize::from(!bytes.ends_with(b"\n"));
        let test = name.split('/').any(|part| {
            part.starts_with("test")
                || part.starts_with("example")
                || part.starts_with("fuzz")
                || part == "IDE"
                || part == "misc"
        });
        if name.ends_with(".h")
            && let Some(parent) = Path::new(name)
                .parent()
                .filter(|path| !path.as_os_str().is_empty())
        {
            includes.insert(parent.to_string_lossy().into_owned());
        }
        hashes
            .entry(hash.clone())
            .or_default()
            .push(name.to_owned());
        let generated = bytes
            .windows(9)
            .any(|word| word.eq_ignore_ascii_case(b"generated"));
        files.push(json!({"path":name,"bytes":bytes.len(),"lines":lines,"sha256":hash,"test_or_example":test,"deep_excluded":strings(&entry["deep_exclude"]).contains(&name),"generated_notice":generated,"utf8":std::str::from_utf8(&bytes).is_ok()}));
    }
    if files.is_empty() {
        return Err(io::Error::other("empty C selection").into());
    }
    files.sort_by_key(|file| file["path"].as_str().unwrap_or_default().to_owned());
    Ok(
        json!({"name":entry["name"],"family":entry["family"],"group":entry["group"],"url":url,"archive_url":entry["archive_url"],"tag":tag,"revision":revision,"source":source,"inventory_version":2,
        "files":files,"excluded":excluded,"include_directories":includes,"duplicate_source_groups":hashes.values().filter(|paths|paths.len()>1).collect::<Vec<_>>(),
        "selection":"All tracked .c/.h files except the explicitly inventoried exclusions; no configure or generation.","complete":true}),
    )
}

fn tree_files(root: &Path, directory: &Path, files: &mut Vec<String>) -> Result<()> {
    for item in fs::read_dir(directory)? {
        let path = item?.path();
        if path.is_symlink() {
            continue;
        }
        if path.is_dir() {
            tree_files(root, &path, files)?;
        } else {
            files.push(path.strip_prefix(root)?.to_string_lossy().into_owned());
        }
    }
    files.sort();
    Ok(())
}

fn cached_inventory(entry: &Value, mut cached: Value) -> Result<Value> {
    let root = Path::new(text(&cached["source"])?).to_owned();
    let mut files = Vec::new();
    let mut excluded = cached["excluded"].as_array().cloned().unwrap_or_default();
    let mut includes = BTreeSet::new();
    for mut file in cached["files"]
        .as_array()
        .ok_or_else(|| io::Error::other("missing cached files"))?
        .clone()
    {
        let name = text(&file["path"])?.to_owned();
        let path = root.join(&name);
        if capture::digest(&path)? != text(&file["sha256"])? {
            return Err(io::Error::other("cached source changed").into());
        }
        if strings(&entry["exclude"])
            .iter()
            .any(|prefix| name.starts_with(prefix))
        {
            excluded.push(json!({"path":name,"reason":"declared third-party subtree, corrected before navigation measurement"}));
            continue;
        }
        file["test_or_example"] = json!(name.split('/').any(|part| part.starts_with("test")
            || part.starts_with("example")
            || part.starts_with("fuzz")
            || part == "IDE"
            || part == "misc"));
        file["deep_excluded"] = json!(strings(&entry["deep_exclude"]).contains(&name.as_str()));
        file["generated_notice"] = json!(
            fs::read(&path)?
                .windows(9)
                .any(|word| word.eq_ignore_ascii_case(b"generated"))
        );
        if name.ends_with(".h")
            && let Some(parent) = Path::new(&name)
                .parent()
                .filter(|path| !path.as_os_str().is_empty())
        {
            includes.insert(parent.to_string_lossy().into_owned());
        }
        files.push(file);
    }
    cached["files"] = json!(files);
    cached["excluded"] = json!(excluded);
    cached["include_directories"] = json!(includes);
    cached["inventory_version"] = json!(2);
    Ok(cached)
}

fn rank(name: &str) -> String {
    format!("{:x}", Sha256::digest(format!("{SEED}\n{name}").as_bytes()))
}

fn draw(root: &Path, entries: &[Value]) -> Result<Value> {
    let mut groups = BTreeMap::<(String, String), Vec<&Value>>::new();
    for entry in entries {
        groups
            .entry((
                text(&entry["family"])?.into(),
                text(&entry["group"])?.into(),
            ))
            .or_default()
            .push(entry);
    }
    let mut witnesses = Vec::new();
    for ((family, group), mut candidates) in groups {
        candidates.sort_by_key(|entry| rank(text(&entry["name"]).unwrap_or_default()));
        let entry = candidates[0];
        let mut chosen = Vec::new();
        let mut eligibility = BTreeMap::new();
        for suffix in ["c", "h"] {
            let mut files = entry["files"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|file| {
                    text(&file["path"]).is_ok_and(|name| name.ends_with(&format!(".{suffix}")))
                        && file["lines"].as_u64().is_some_and(|lines| lines <= 500)
                        && file["bytes"].as_u64().is_some_and(|bytes| bytes <= 65536)
                        && file["test_or_example"] == false
                        && file["deep_excluded"] != true
                })
                .collect::<Vec<_>>();
            files.sort_by_key(|file| {
                rank(&format!(
                    "{}/{}",
                    entry["name"].as_str().unwrap(),
                    file["path"].as_str().unwrap()
                ))
            });
            eligibility.insert(
                suffix,
                files.iter().map(|file| &file["path"]).collect::<Vec<_>>(),
            );
            if let Some(file) = files.first() {
                chosen.push((*file).clone());
            }
        }
        witnesses.push(json!({"name":entry["name"],"family":family,"group":group,"files":chosen,"eligibility":eligibility}));
    }
    let result = json!({"inventory_version":2,"seed":SEED,"selection":"Lowest SHA-256 rank per family/group, including acquisition failures; lowest ranked eligible production .c and .h. No replacement of empty draws.","projects":witnesses});
    capture::save(&root.join("draws.json"), &result)?;
    Ok(result)
}

fn inventory(root: &Path) -> Result<()> {
    if root.join("method-freeze.json").exists() {
        return Err(io::Error::other(
            "frozen inventory requires a new evidence root and method version",
        )
        .into());
    }
    fs::create_dir_all(root.join("exact"))?;
    fs::create_dir_all(root.join("closures"))?;
    let acquisition = read(&root.join("acquisition.json"))?;
    let words = Regex::new(r"[A-Za-z_][A-Za-z_0-9]*")?;
    let mut shared = BTreeMap::<String, Vec<Value>>::new();
    let mut challenges = Vec::new();
    for entry in acquisition["projects"]
        .as_array()
        .ok_or_else(|| io::Error::other("missing projects"))?
    {
        if entry["complete"] != true {
            continue;
        }
        let name = text(&entry["name"])?;
        let source = Path::new(text(&entry["source"])?).canonicalize()?;
        let mut identifiers = BTreeSet::new();
        let mut closure = BTreeSet::new();
        let mut queue = Vec::new();
        let mut missing = BTreeSet::new();
        let mut unsupported = BTreeMap::new();
        for file in entry["files"].as_array().unwrap() {
            source_inventory::verify(
                &source,
                file,
                entry["inventory_version"].as_u64().unwrap_or(2),
            )?;
            let path = text(&file["path"])?;
            queue.push(path.to_owned());
            closure.insert(path.to_owned());
            shared
                .entry(text(&file["sha256"])?.into())
                .or_default()
                .push(json!([name, path]));
        }
        while let Some(path) = queue.pop() {
            let bytes = fs::read(source.join(&path))?;
            let content = String::from_utf8_lossy(&bytes);
            identifiers.extend(
                words
                    .find_iter(&content)
                    .map(|word| word.as_str().to_owned()),
            );
            for (operand, quoted) in source_inventory::includes(&bytes) {
                match source_inventory::resolve(
                    &source,
                    &operand,
                    quoted,
                    &strings(&entry["include_directories"]),
                ) {
                    source_inventory::Input::Local(found) => {
                        if closure.insert(found.clone()) {
                            queue.push(found);
                        }
                    }
                    source_inventory::Input::Missing => {
                        missing.insert(operand);
                    }
                    source_inventory::Input::Unsupported(reason) => {
                        unsupported.insert(
                            (path.clone(), operand.clone()),
                            json!({"source":path,"operand":operand,"reason":reason}),
                        );
                    }
                }
            }
        }
        let mut identifiers = identifiers
            .into_iter()
            .filter(|word| word.len() > 3)
            .collect::<Vec<_>>();
        identifiers.sort_by_key(|word| rank(&format!("{name}/{word}")));
        identifiers.truncate(8);
        identifiers.push("__ripscope_coverage_absent_202610__".into());
        capture::save(
            &root.join("exact").join(format!("{name}.json")),
            &json!(identifiers),
        )?;
        let closure = closure
            .into_iter()
            .map(|path| source_inventory::freeze(&source, &path))
            .collect::<io::Result<Vec<_>>>()?;
        capture::save(
            &root.join("closures").join(format!("{name}.json")),
            &json!({"inventory_version":3,"input_policy":"project-only","files":closure,"missing_literal_operands":missing,
                "unsupported_inputs":unsupported.into_values().collect::<Vec<_>>(),
                "rule":"Conservative literal-include closure over the frozen directories, retaining discovery spellings and alias identities. External inputs are unsupported; no host bytes or generated files are admitted."}),
        )?;
        for file in entry["files"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|file| {
                file["lines"].as_u64().is_some_and(|lines| lines > 500)
                    && file["test_or_example"] == false
            })
            .take(1)
        {
            challenges.push(json!({"project":name,"file":file,"status":"inventory only; source labels required before scoring","stratum":"larger files"}));
        }
    }
    capture::save(
        &root.join("shared-source.json"),
        &json!(
            shared
                .into_values()
                .filter(|items| items
                    .iter()
                    .map(|item| item[0].as_str())
                    .collect::<BTreeSet<_>>()
                    .len()
                    > 1)
                .collect::<Vec<_>>()
        ),
    )?;
    capture::save(&root.join("challenge-inventory.json"), &json!(challenges))?;
    Ok(())
}

fn prepare(root: &Path) -> Result<()> {
    fs::create_dir_all(root.join("drafts"))?;
    let acquisition = read(&root.join("acquisition.json"))?;
    let draws = read(&root.join("draws.json"))?;
    let words = Regex::new(r"[A-Za-z_][A-Za-z_0-9]*")?;
    let includes = Regex::new(r#"(?m)^\s*#\s*include\s*[<"]([^>"\r\n]+)[>"]"#)?;
    let directive = Regex::new(r"^[ \t]*#")?;
    for draw in draws["projects"].as_array().unwrap() {
        let name = text(&draw["name"])?;
        let facts_path = root.join("source-facts").join(format!("{name}.json"));
        if !facts_path.exists() {
            continue;
        }
        let mut facts = read(&facts_path)?;
        let entry = acquisition["projects"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["name"] == draw["name"])
            .unwrap();
        let source = Path::new(text(&entry["source"])?);
        facts["files"] = json!(
            draw["files"]
                .as_array()
                .unwrap()
                .iter()
                .map(|file| &file["path"])
                .collect::<Vec<_>>()
        );
        let mut identifiers = BTreeSet::new();
        let mut literal = BTreeMap::<String, Vec<Value>>::new();
        let mut regex_rows = Vec::new();
        for file in strings(&facts["files"]) {
            let content = fs::read_to_string(source.join(file))?;
            identifiers.extend(
                words
                    .find_iter(&content)
                    .map(|word| word.as_str().to_owned()),
            );
            for (at, line) in content.split_inclusive('\n').enumerate() {
                let line = line.strip_suffix('\n').unwrap_or(line);
                if directive.is_match(line) {
                    regex_rows.push(json!([
                        file,
                        "<unknown>",
                        at + 1,
                        "regular expression",
                        line
                    ]));
                }
            }
            for item in includes.captures_iter(&content) {
                let whole = item.get(0).unwrap();
                let start = whole.start() + whole.as_str().find('#').unwrap();
                let line = content[..start]
                    .bytes()
                    .filter(|byte| *byte == b'\n')
                    .count()
                    + 1;
                literal.entry(item[1].into()).or_default().push(json!([
                    file,
                    "<global>",
                    line,
                    "literal include"
                ]));
            }
        }
        let mut queries = identifiers
            .into_iter()
            .map(|word| json!([1, word, "all written identifiers"]))
            .collect::<Vec<_>>();
        for context in strings(&facts["contexts"]) {
            queries.push(json!([2, context, "complete call contexts"]));
        }
        queries.extend([
            json!([1, "__ripscope_coverage_absent_202610__", "negative"]),
            json!([2, "__ripscope_coverage_absent_202610__", "negative"]),
            json!([3, "__ripscope_coverage_absent_202610__", "negative"]),
            json!([0, "__ripscope_coverage_absent_202610__", "negative"]),
            json!([9, "__ripscope_coverage_absent_202610__", "negative"]),
            json!([
                8,
                "^__ripscope_coverage_absent_include_202610__\\.h$",
                "absent literal include"
            ]),
        ]);
        let mut extras = facts["extra_queries"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        // Use a literal substring whose complete whole-file answer is independently enumerated.
        let mut literal_rows = Vec::new();
        for file in strings(&facts["files"]) {
            for (at, line) in fs::read_to_string(source.join(file))?
                .split_inclusive('\n')
                .enumerate()
            {
                let line = line.strip_suffix('\n').unwrap_or(line);
                if line.contains("include") {
                    literal_rows.push(json!([file, "<unknown>", at + 1, "literal text", line]));
                }
            }
        }
        for field in [4, 5] {
            extras.push(json!({"field":field,"pattern":"include","construct":"literal text and field 5 alias","expected":literal_rows,"match_text":true}));
        }
        extras.push(json!({"field":6,"pattern":"^[ \t]*#","construct":"regular expression","expected":regex_rows,"match_text":true}));
        for (pattern, expected) in literal {
            extras.push(json!({"field":8,"pattern":format!("^{}$",regex::escape(&pattern)),"construct":"literal include","expected":expected}));
        }
        let closure = read(&root.join("closures").join(format!("{name}.json")))?;
        let names = closure["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|file| text(&file["path"]))
            .collect::<Result<Vec<_>>>()?;
        extras.push(json!({"field":7,"pattern":".*","construct":"complete frozen inventory","expected":names.iter().map(|name|json!([name,"<unknown>",1,"complete frozen inventory"])).collect::<Vec<_>>(),"scopes":names.iter().map(|name|json!([name,1,1])).collect::<Vec<_>>()}));
        facts["index_files"] = json!(
            entry["files"]
                .as_array()
                .unwrap()
                .iter()
                .map(|file| &file["path"])
                .collect::<Vec<_>>()
        );
        facts["include_directories"] = entry["include_directories"].clone();
        facts["closure_files"] = json!(names);
        facts["query_inventory"] = json!(queries);
        facts["extra_queries"] = json!(extras);
        facts["whole_files"] = json!(true);
        facts["query_limit"] = json!(5000);
        if facts.get("controls").is_none() {
            facts["controls"] = json!([]);
        }
        facts["sampling"] = draw.clone();
        facts["missing_literal_operands"] = closure["missing_literal_operands"].clone();
        facts["unsupported_inputs"] = closure["unsupported_inputs"].clone();
        capture::save(&root.join("drafts").join(format!("{name}.json")), &facts)?;
    }
    Ok(())
}

fn size(path: &Path) -> io::Result<u64> {
    if path.is_symlink() {
        return Ok(0);
    }
    if path.is_file() {
        return Ok(path.metadata()?.len());
    }
    fs::read_dir(path)?.try_fold(0u64, |total, item| Ok(total + size(&item?.path())?))
}

fn pilot_projects(identities: &Value, roster: &Value) -> Result<BTreeSet<String>> {
    let Some(configured) = identities.get("pilot_projects") else {
        return Ok([
            "memcached",
            "cjson",
            "libgit2",
            "tinyusb",
            "micropython",
            "winfsp",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect());
    };
    let names = configured
        .as_array()
        .filter(|names| !names.is_empty())
        .ok_or_else(|| io::Error::other("pilot_projects must be a nonempty array"))?;
    let projects = roster["projects"]
        .as_array()
        .ok_or_else(|| io::Error::other("missing roster projects"))?;
    let mut pilots = BTreeSet::new();
    for name in names {
        let name = text(name)?;
        if !pilots.insert(name.to_owned()) {
            return Err(io::Error::other(format!("duplicate pilot project: {name}")).into());
        }
        let project = projects
            .iter()
            .find(|project| project["name"] == name)
            .ok_or_else(|| io::Error::other(format!("unknown pilot project: {name}")))?;
        if project["group"] != "development" {
            return Err(
                io::Error::other(format!("pilot project must be in development: {name}")).into(),
            );
        }
    }
    Ok(pilots)
}

fn selected_phase(entry: &Value, phase: &str, pilots: &BTreeSet<String>) -> bool {
    let pilot = entry["name"]
        .as_str()
        .is_some_and(|name| pilots.contains(name));
    match phase {
        "pilot" => pilot,
        "development" => entry["group"] == "development" && !pilot,
        "reserved" => entry["group"] == "reserved",
        _ => false,
    }
}

fn method_identity(root: &Path, repo: &Path) -> Result<Value> {
    let mut hashes = BTreeMap::new();
    let identities = read(&root.join("identities.json"))?;
    for file in ["roster.json", "acquisition.json"] {
        pilot_projects(&identities, &read(&root.join(file))?)?;
    }
    for key in ["binary", "reference"] {
        let actual = capture::digest(Path::new(text(&identities[key])?))?;
        if actual != text(&identities[format!("{key}_sha256")])? {
            return Err(io::Error::other(format!("frozen {key} identity changed")).into());
        }
        hashes.insert(format!("actual/{key}"), actual);
    }
    for file in [
        "examples/coverage-campaign.rs",
        "examples/assess-navigation.rs",
        "examples/support/capture.rs",
        "examples/support/source_inventory.rs",
        "examples/compass-scale.rs",
        "examples/support/cscope_db.rs",
        "examples/support/reference.rs",
    ] {
        hashes.insert(file.to_owned(), capture::digest(&repo.join(file))?);
    }
    for file in [
        "target/release/examples/coverage-campaign",
        "target/release/examples/assess-navigation",
        "target/release/examples/compass-scale",
    ] {
        hashes.insert(file.to_owned(), capture::digest(&repo.join(file))?);
    }
    for file in [
        "roster.json",
        "acquisition.json",
        "draws.json",
        "identities.json",
    ] {
        hashes.insert(file.to_owned(), capture::digest(&root.join(file))?);
    }
    for directory in ["manifests", "closures", "exact", "sealed"] {
        if root.join(directory).is_dir() {
            for file in fs::read_dir(root.join(directory))? {
                let path = file?.path();
                if path
                    .extension()
                    .is_some_and(|extension| extension == "json")
                {
                    hashes.insert(
                        format!(
                            "{directory}/{}",
                            path.file_name().unwrap().to_string_lossy()
                        ),
                        capture::digest(&path)?,
                    );
                }
            }
        }
    }
    Ok(json!({"version":3,"seed":SEED,"hashes":hashes}))
}

fn measurement_identity(root: &Path, repo: &Path, name: &str, deep: bool) -> Result<Value> {
    let mut identity = method_identity(root, repo)?;
    let hashes = identity["hashes"].as_object_mut().unwrap();
    hashes.retain(|file, _| {
        !file.contains('/')
            || file.starts_with("examples/")
            || file.starts_with("scripts/")
            || file.starts_with("target/")
            || file.starts_with("actual/")
            || file == &format!("manifests/{name}.json")
            || file == &format!("closures/{name}.json")
            || file == &format!("exact/{name}.json")
            || (deep && file == &format!("sealed/{name}.json"))
    });
    Ok(identity)
}

fn measure(root: &Path, phase: &str, deep: bool) -> Result<()> {
    if !matches!(phase, "pilot" | "development" | "reserved") {
        return Err(io::Error::other("unknown campaign phase").into());
    }
    let repo = std::env::current_dir()?.canonicalize()?;
    let phase_identity = method_identity(root, &repo)?;
    if phase != "pilot" && read(&root.join("method-freeze.json"))?["identity"] != phase_identity {
        return Err(io::Error::other(
            "frozen method identity differs from current inputs or tooling",
        )
        .into());
    }
    let identities = read(&root.join("identities.json"))?;
    let pilots = pilot_projects(&identities, &read(&root.join("roster.json"))?)?;
    let binary = Path::new(text(&identities["binary"])?);
    if capture::digest(binary)? != text(&identities["binary_sha256"])? {
        return Err(io::Error::other("frozen product identity changed").into());
    }
    if capture::digest(Path::new(text(&identities["reference"])?))?
        != text(&identities["reference_sha256"])?
    {
        return Err(io::Error::other("frozen reference identity changed").into());
    }
    let kind = if deep { "deep" } else { "broad" };
    fs::create_dir_all(root.join(format!("{kind}-summaries")))?;
    fs::create_dir_all(root.join("evidence"))?;
    fs::create_dir_all(root.join("measurement"))?;
    let acquisition = read(&root.join("acquisition.json"))?;
    for entry in acquisition["projects"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|entry| selected_phase(entry, phase, &pilots))
    {
        if method_identity(root, &repo)? != phase_identity {
            return Err(io::Error::other("campaign method changed between projects").into());
        }
        let name = text(&entry["name"])?;
        let summary = root.join(format!("{kind}-summaries/{name}.json"));
        let identity = measurement_identity(root, &repo, name, deep)?;
        if summary.exists() {
            let previous = read(&summary)?;
            if previous["measurement_identity"] != identity || previous["finalized"] != true {
                return Err(io::Error::other(format!(
                    "cannot reuse {name}: identity differs or evidence unfinalized"
                ))
                .into());
            }
            continue;
        }
        let manifest = root.join(format!("sealed/{name}.json"));
        if deep && !manifest.exists() {
            continue;
        }
        if entry["complete"] != true {
            capture::save(
                &summary,
                &json!({"complete":false,"finalized":true,"measurement_identity":identity,"error":"acquisition incomplete","acquisition":entry}),
            )?;
            continue;
        }
        let source = Path::new(text(&entry["source"])?);
        let closure = read(&root.join(format!("closures/{name}.json")))?;
        for (file, version) in entry["files"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|file| (file, entry["inventory_version"].as_u64().unwrap_or(2)))
            .chain(
                closure["files"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|file| (file, closure["inventory_version"].as_u64().unwrap_or(2))),
            )
        {
            source_inventory::verify(source, file, version)?;
        }
        let mut unsupported = source_inventory::preflight(
            source,
            &entry["files"]
                .as_array()
                .unwrap()
                .iter()
                .map(|file| text(&file["path"]))
                .collect::<Result<Vec<_>>>()?,
            &strings(&entry["include_directories"]),
        )?;
        for item in closure["unsupported_inputs"]
            .as_array()
            .into_iter()
            .flatten()
        {
            if !unsupported.contains(item) {
                unsupported.push(item.clone());
            }
        }
        if !unsupported.is_empty() {
            let outcome = json!({"complete":false,"finalized":true,"run_finished":false,
                "comparison_eligible":false,"status":"unsupported_inputs","unsupported_inputs":unsupported,
                "input_policy":"project-only","project":name,"source":source,"revision":entry["revision"],
                "family":entry["family"],"group":entry["group"],"measurement_identity":identity,
                "binary_sha256":identities["binary_sha256"],"commands":{},"fields":{},
                "launch":{"complete":true,"skipped":"unsupported_inputs"},
                "error":"external inputs are outside the frozen project; no index command was launched"});
            capture::save(
                &root.join(format!("measurement/{kind}-{name}-preflight.json")),
                &outcome,
            )?;
            capture::save(&summary, &outcome)?;
            println!("{kind} {phase}: {name}: unsupported inputs, not measured");
            continue;
        }
        if size(root)? > 76 << 30 {
            return Err(io::Error::other(
                "campaign scratch reserve exhausted; refusing a project that could exceed 100 GiB",
            )
            .into());
        }
        let out = root.join(format!("{kind}-{name}"));
        if out.exists() {
            return Err(io::Error::other(format!(
                "unfinalized evidence directory {}",
                out.display()
            ))
            .into());
        }
        let (program, args, seconds) = if deep {
            (
                repo.join("target/release/examples/assess-navigation"),
                vec![
                    "run".into(),
                    binary.to_string_lossy().into_owned(),
                    text(&entry["source"])?.into(),
                    manifest.to_string_lossy().into_owned(),
                    out.to_string_lossy().into_owned(),
                ],
                86400,
            )
        } else {
            (
                repo.join("target/release/examples/compass-scale"),
                vec![
                    "--binary".into(),
                    binary.to_string_lossy().into_owned(),
                    "--reference".into(),
                    text(&identities["reference"])?.into(),
                    "--source".into(),
                    text(&entry["source"])?.into(),
                    "--output".into(),
                    out.to_string_lossy().into_owned(),
                    "--manifest".into(),
                    root.join(format!("manifests/{name}.json"))
                        .to_string_lossy()
                        .into_owned(),
                    "--exact-inventory".into(),
                    root.join(format!("exact/{name}.json"))
                        .to_string_lossy()
                        .into_owned(),
                    "--closure-manifest".into(),
                    root.join(format!("closures/{name}.json"))
                        .to_string_lossy()
                        .into_owned(),
                    "--project-limit".into(),
                    (18u64 << 30).to_string(),
                ],
                86400,
            )
        };
        println!("{kind} {phase}: {name}");
        let command = capture::run_guarded(
            &program,
            &args,
            &repo,
            &root.join(format!("measurement/{kind}-{name}")),
            seconds,
            true,
            &[(out.clone(), 18 << 30)],
        )?;
        if command["cleanup_error"].is_string() {
            return Err(
                io::Error::other("command scope cleanup failed; raw captures retained").into(),
            );
        }
        let report = out.join(if deep { "report.json" } else { "summary.json" });
        let mut result = if report.exists() {
            read(&report)?
        } else {
            json!({"complete":false,"error":"measurement did not produce a report"})
        };
        result["launch"] = command;
        result["project"] = entry["name"].clone();
        result["family"] = entry["family"].clone();
        result["group"] = entry["group"].clone();
        if result["launch"]["complete"] != true {
            result["complete"] = json!(false);
        }
        result["measurement_identity"] = identity;
        if result["measurement_identity"] != measurement_identity(root, &repo, name, deep)? {
            capture::save(&out.join("identity-changed.json"), &result)?;
            return Err(
                io::Error::other("measurement inputs or harness changed during capture").into(),
            );
        }
        if out.exists() {
            let mut indexes = Vec::new();
            for file in fs::read_dir(&out)? {
                let path = file?.path();
                if path.is_file()
                    && matches!(
                        path.file_name().and_then(|name| name.to_str()),
                        Some("product.db" | "baseline.db" | "index.db")
                    )
                {
                    indexes.push(json!({"file":path.file_name(),"bytes":path.metadata()?.len(),"sha256":capture::digest(&path)?}));
                    fs::remove_file(path)?;
                }
            }
            capture::save(&out.join("disposed-indexes.json"), &json!(indexes))?;
            let archive = root.join(format!("evidence/{kind}-{name}.tar.zst"));
            let compressed = capture::run_guarded(
                Path::new("tar"),
                &[
                    "--zstd".into(),
                    "-cf".into(),
                    "-".into(),
                    "-C".into(),
                    root.to_string_lossy().into_owned(),
                    out.file_name().unwrap().to_string_lossy().into_owned(),
                ],
                &repo,
                &root.join(format!("measurement/compress-{kind}-{name}")),
                1800,
                true,
                &[(root.to_owned(), 100 << 30)],
            )?;
            if compressed["complete"] == true {
                fs::rename(text(&compressed["stdout_path"])?, &archive)?;
                result["archive"] = json!({"path":archive,"bytes":archive.metadata()?.len(),"sha256":capture::digest(&archive)?});
                fs::remove_dir_all(&out)?;
            } else {
                return Err(io::Error::other(
                    "evidence compression failed; raw directory retained",
                )
                .into());
            }
        }
        if size(root)? > 100 << 30 {
            return Err(io::Error::other("campaign scratch limit exceeded").into());
        }
        result["finalized"] = json!(true);
        capture::save(&summary, &result)?;
    }
    Ok(())
}

fn main() -> Result<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if let [mode, root, phase] = args.as_slice()
        && (mode == "measure" || mode == "deep")
    {
        return measure(&Path::new(root).canonicalize()?, phase, mode == "deep");
    }
    if let [mode, root] = args.as_slice() {
        return match mode.as_str() {
            "inventory" => inventory(&Path::new(root).canonicalize()?),
            "prepare" => prepare(&Path::new(root).canonicalize()?),
            "freeze" => {
                let root = Path::new(root).canonicalize()?;
                capture::save(
                    &root.join("method-freeze.json"),
                    &json!({"identity":method_identity(&root,&std::env::current_dir()?)?}),
                )?;
                Ok(())
            }
            _ => Err(io::Error::other("unknown campaign mode").into()),
        };
    }
    if args.len() != 3 || !matches!(args[0].as_str(), "acquire" | "acquire-inner") {
        return Err(io::Error::other(
            "usage: coverage-campaign acquire ROSTER NEW_OR_RESUMED_OUTPUT",
        )
        .into());
    }
    let root = PathBuf::from(&args[2]);
    if args[0] == "acquire" {
        fs::create_dir_all(root.join("acquisition"))?;
        let root = root.canonicalize()?;
        if root.join("acquisition.json").exists() {
            let snapshot = root.join("inventory-versions").join(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)?
                    .as_nanos()
                    .to_string(),
            );
            fs::create_dir_all(&snapshot)?;
            for file in [
                "roster.json",
                "acquisition.json",
                "draws.json",
                "shared-source.json",
                "challenge-inventory.json",
            ] {
                if root.join(file).exists() {
                    fs::copy(root.join(file), snapshot.join(file))?;
                }
            }
            for directory in ["manifests", "closures", "exact"] {
                if root.join(directory).is_dir() {
                    fs::create_dir_all(snapshot.join(directory))?;
                    for file in fs::read_dir(root.join(directory))? {
                        let file = file?.path();
                        if file.is_file() {
                            fs::copy(
                                &file,
                                snapshot.join(directory).join(file.file_name().unwrap()),
                            )?;
                        }
                    }
                }
            }
            capture::save(
                &snapshot.join("reason.json"),
                &json!({"reason":"Acquisition resumed; previous metadata retained. Selection changes and their reasons belong in the new roster.","next_roster_sha256":capture::digest(Path::new(&args[1]))?}),
            )?;
        }
        let mut inner = args.clone();
        inner[0] = "acquire-inner".into();
        checked(capture::run_guarded(
            &std::env::current_exe()?,
            &inner,
            &std::env::current_dir()?,
            &root.join("acquisition/driver"),
            86400,
            true,
            &[(root.clone(), 100 << 30)],
        )?)?;
        return Ok(());
    }
    fs::create_dir_all(root.join("sources"))?;
    fs::create_dir_all(root.join("manifests"))?;
    let root = root.canonicalize()?;
    let roster = read(Path::new(&args[1]))?;
    capture::save(&root.join("roster.json"), &roster)?;
    let mut results = Vec::new();
    for entry in roster["projects"]
        .as_array()
        .ok_or_else(|| io::Error::other("missing projects"))?
    {
        let name = text(&entry["name"])?;
        let manifest = root.join("manifests").join(format!("{name}.json"));
        let cached = manifest.exists().then(|| read(&manifest)).transpose()?;
        let result = if let Some(cached) = cached.filter(|value| {
            value["url"] == entry["url"]
                && value["archive_url"] == entry["archive_url"]
                && (entry["release"].is_null() || value["tag"] == entry["release"])
        }) {
            let result = cached_inventory(entry, cached)?;
            capture::save(&manifest, &result)?;
            result
        } else {
            match acquire(entry, &root) {
                Ok(result) => {
                    capture::save(&manifest, &result)?;
                    result
                }
                Err(error) => {
                    json!({"name":name,"family":entry["family"],"group":entry["group"],"complete":false,"error":error.to_string()})
                }
            }
        };
        println!(
            "{name}: {}",
            if result["complete"] == true {
                "pinned"
            } else {
                "acquisition incomplete"
            }
        );
        results.push(result);
        capture::save(
            &root.join("acquisition.json"),
            &json!({"projects":results,"seed":SEED}),
        )?;
    }
    draw(&root, &results)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inventory_fixture(root: &Path, source: &Path, names: &[&str]) -> Value {
        let files = names
            .iter()
            .map(|name| {
                json!({"path":name,
            "sha256":capture::digest(&source.join(name)).unwrap(),"lines":1,
            "bytes":fs::metadata(source.join(name)).unwrap().len(),"test_or_example":false})
            })
            .collect::<Vec<_>>();
        capture::save(
            &root.join("acquisition.json"),
            &json!({"projects":[{
            "name":"project","complete":true,"source":source,"files":files,
            "include_directories":["include"]}]}),
        )
        .unwrap();
        inventory(root).unwrap();
        read(&root.join("closures/project.json")).unwrap()
    }

    #[cfg(unix)]
    #[test]
    fn inventory_preserves_normalized_discovery_aliases_and_link_chains() {
        let root = tempfile::tempdir().unwrap();
        let source = tempfile::tempdir().unwrap();
        fs::create_dir(source.path().join("include")).unwrap();
        fs::write(
            source.path().join("source.c"),
            "#include \"include/./alias.h\"\n",
        )
        .unwrap();
        fs::write(source.path().join("include/real.h"), "int actual;\n").unwrap();
        std::os::unix::fs::symlink("real.h", source.path().join("include/next.h")).unwrap();
        std::os::unix::fs::symlink("next.h", source.path().join("include/alias.h")).unwrap();
        let result = inventory_fixture(root.path(), source.path(), &["source.c", "include/real.h"]);
        let alias = result["files"]
            .as_array()
            .unwrap()
            .iter()
            .find(|file| file["path"] == "include/alias.h")
            .expect("alias must retain its discovery spelling");
        assert_eq!(alias["target"], "include/real.h");
        assert_eq!(
            alias["links"],
            json!([
            {"path":"include/alias.h","target":"next.h"},
            {"path":"include/next.h","target":"real.h"}])
        );
    }

    #[cfg(unix)]
    #[test]
    fn inventory_records_absolute_operands_reached_through_aliases_without_host_bytes() {
        let root = tempfile::tempdir().unwrap();
        let source = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(
            outside.path().join("host.h"),
            "#include \"/must-not-read/nested.h\"\n",
        )
        .unwrap();
        let operand = outside.path().join("host.h").to_string_lossy().into_owned();
        fs::write(
            source.path().join("source.c"),
            "#include \"view/alias.h\"\n",
        )
        .unwrap();
        fs::create_dir(source.path().join("include")).unwrap();
        fs::write(
            source.path().join("include/real.h"),
            format!("int x; #include \"{operand}\"\n"),
        )
        .unwrap();
        std::os::unix::fs::symlink("include", source.path().join("view")).unwrap();
        std::os::unix::fs::symlink("real.h", source.path().join("include/alias.h")).unwrap();
        let result = inventory_fixture(root.path(), source.path(), &["source.c"]);
        assert_eq!(result["inventory_version"], 3);
        assert_eq!(
            result["unsupported_inputs"],
            json!([{"source":"view/alias.h","operand":operand,
            "reason":"absolute input outside declared project"}])
        );
        let alias = result["files"]
            .as_array()
            .unwrap()
            .iter()
            .find(|file| file["path"] == "view/alias.h")
            .unwrap();
        assert_eq!(
            alias["links"],
            json!([{"path":"view","target":"include"},
            {"path":"include/alias.h","target":"real.h"}])
        );
        assert_eq!(result["files"].as_array().unwrap().len(), 2);
    }

    #[cfg(unix)]
    #[test]
    fn missing_alias_targets_remain_explicit_missing_operands() {
        let root = tempfile::tempdir().unwrap();
        let source = tempfile::tempdir().unwrap();
        fs::write(source.path().join("source.c"), "#include \"alias.h\"\n").unwrap();
        std::os::unix::fs::symlink("missing.h", source.path().join("alias.h")).unwrap();
        let result = inventory_fixture(root.path(), source.path(), &["source.c"]);
        assert_eq!(result["missing_literal_operands"], json!(["alias.h"]));
        assert_eq!(result["files"].as_array().unwrap().len(), 1);
        assert_eq!(result["unsupported_inputs"], json!([]));
    }

    #[test]
    fn absolute_inputs_inside_the_declared_project_retain_their_spelling() {
        let root = tempfile::tempdir().unwrap();
        // A long-name base, since a temporary folder spelled with 8.3 short names (as on
        // hosted Windows runners) is a different spelling of the project.
        let base = std::env::temp_dir().canonicalize().unwrap();
        let base = PathBuf::from(base.to_string_lossy().trim_start_matches(r"\\?\"));
        let source = tempfile::tempdir_in(base).unwrap();
        fs::create_dir(source.path().join("include")).unwrap();
        let header = source.path().join("include").join("real.h");
        fs::write(&header, "int known;\n").unwrap();
        for operand in [header.clone(), header.canonicalize().unwrap()] {
            fs::write(
                source.path().join("source.c"),
                format!("#include \"{}\"\n", operand.display()),
            )
            .unwrap();
            let result = inventory_fixture(root.path(), source.path(), &["source.c"]);
            let selected = result["files"]
                .as_array()
                .unwrap()
                .iter()
                .find(|file| file["path"] == operand.to_string_lossy().as_ref())
                .unwrap_or_else(|| {
                    panic!(
                        "absolute operand {operand:?} in project {:?} missing from {result}",
                        source.path().canonicalize().unwrap(),
                    )
                });
            assert_eq!(
                selected["target"],
                json!(Path::new("include").join("real.h"))
            );
            assert_eq!(selected["sha256"], capture::digest(&header).unwrap());
            assert_eq!(result["unsupported_inputs"], json!([]));
        }
    }

    #[test]
    fn absolute_inputs_outside_the_declared_project_do_not_admit_host_bytes() {
        let root = tempfile::tempdir().unwrap();
        let parent = tempfile::tempdir().unwrap();
        let source = parent.path().join("project");
        let outside = parent.path().join("project-other");
        fs::create_dir(&source).unwrap();
        fs::create_dir(&outside).unwrap();
        let header = outside.join("host.h");
        fs::write(&header, "#include \"/must-not-read/nested.h\"\n").unwrap();
        for operand in [
            header.clone(),
            header.canonicalize().unwrap(),
            source.join("..").join("project-other").join("host.h"),
        ] {
            fs::write(
                source.join("source.c"),
                format!("#include \"{}\"\n", operand.display()),
            )
            .unwrap();
            let result = inventory_fixture(root.path(), &source, &["source.c"]);
            assert_eq!(result["files"].as_array().unwrap().len(), 1);
            assert_eq!(result["missing_literal_operands"], json!([]));
            assert_eq!(
                result["unsupported_inputs"],
                json!([{"source":"source.c","operand":operand,
                "reason":"absolute input outside declared project"}]),
            );
        }
    }

    #[test]
    fn missing_relative_candidates_do_not_hide_in_project_include_directories() {
        let root = tempfile::tempdir().unwrap();
        let parent = tempfile::tempdir().unwrap();
        let source = parent.path().join("source");
        fs::create_dir(&source).unwrap();
        fs::create_dir(source.join("include")).unwrap();
        fs::write(source.join("source.c"), "#include \"../local.h\"\n").unwrap();
        fs::write(source.join("local.h"), "int known;\n").unwrap();
        let result = inventory_fixture(root.path(), &source, &["source.c"]);
        assert_eq!(result["unsupported_inputs"], json!([]));
        assert!(
            result["files"]
                .as_array()
                .unwrap()
                .iter()
                .any(|file| file["path"] == "local.h")
        );
        fs::write(
            parent.path().join("local.h"),
            "#include \"/must-not-read/nested.h\"\n",
        )
        .unwrap();
        let result = inventory_fixture(root.path(), &source, &["source.c"]);
        assert_eq!(
            result["unsupported_inputs"],
            json!([{
                "source":"source.c","operand":"../local.h",
                "reason":"relative input outside declared project"
            }])
        );
        assert_eq!(result["files"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn a_frozen_inventory_cannot_be_rewritten_in_place() {
        let root = tempfile::tempdir().unwrap();
        capture::save(
            &root.path().join("method-freeze.json"),
            &json!({"identity":"original"}),
        )
        .unwrap();
        assert!(inventory(root.path()).is_err());
        assert!(!root.path().join("closures").exists());
        assert_eq!(
            read(&root.path().join("method-freeze.json")).unwrap(),
            json!({"identity":"original"})
        );
    }

    #[test]
    fn a_failed_project_draw_is_not_replaced_by_an_acquired_project() {
        let directory = tempfile::tempdir().unwrap();
        let mut names = ["first", "second"];
        names.sort_by_key(|name| rank(name));
        let result=draw(directory.path(),&[
            json!({"name":names[0],"family":"one","group":"reserved","complete":false}),
            json!({"name":names[1],"family":"one","group":"reserved","complete":true,"files":[{"path":"easy.c","lines":1,"bytes":1,"utf8":true,"test_or_example":false}]}),
        ]).unwrap();
        assert_eq!(result["projects"][0]["name"], names[0]);
        assert_eq!(result["projects"][0]["files"], json!([]));
    }

    #[test]
    fn absent_eligible_headers_remain_missing() {
        let directory = tempfile::tempdir().unwrap();
        let result=draw(directory.path(),&[json!({"name":"project","family":"one","group":"development","complete":true,"files":[
            {"path":"source.c","lines":500,"bytes":65536,"utf8":true,"test_or_example":false},
            {"path":"large.h","lines":501,"bytes":65536,"utf8":true,"test_or_example":false},
            {"path":"tests/control.h","lines":1,"bytes":1,"utf8":true,"test_or_example":true},
        ]})]).unwrap();
        assert_eq!(result["projects"][0]["files"].as_array().unwrap().len(), 1);
        assert_eq!(result["projects"][0]["eligibility"]["h"], json!([]));
        assert!(version("v1.10.1") > version("v1.9.9"));
    }

    #[test]
    fn absent_pilot_configuration_preserves_legacy_phases() {
        let pilots = pilot_projects(&json!({}), &json!({})).unwrap();
        assert_eq!(pilots.len(), 6);
        for name in [
            "memcached",
            "cjson",
            "libgit2",
            "tinyusb",
            "micropython",
            "winfsp",
        ] {
            let entry = json!({"name":name,"group":"development"});
            assert!(selected_phase(&entry, "pilot", &pilots));
            assert!(!selected_phase(&entry, "development", &pilots));
        }
        assert!(selected_phase(
            &json!({"name":"other","group":"development"}),
            "development",
            &pilots
        ));
    }

    #[test]
    fn configured_pilots_partition_new_projects_once() {
        let roster = json!({"projects":[
            {"name":"lz4","group":"development"},
            {"name":"m4","group":"development"},
            {"name":"bzip2","group":"reserved"}
        ]});
        let pilots = pilot_projects(&json!({"pilot_projects":["lz4"]}), &roster).unwrap();
        for (entry, expected) in
            roster["projects"]
                .as_array()
                .unwrap()
                .iter()
                .zip(["pilot", "development", "reserved"])
        {
            let phases = ["pilot", "development", "reserved"]
                .into_iter()
                .filter(|phase| selected_phase(entry, phase, &pilots))
                .collect::<Vec<_>>();
            assert_eq!(phases, [expected]);
        }
    }

    #[test]
    fn invalid_explicit_pilots_are_rejected() {
        let roster = json!({"projects":[
            {"name":"lz4","group":"development"},
            {"name":"bzip2","group":"reserved"}
        ]});
        for names in [
            json!([]),
            json!(null),
            json!("lz4"),
            json!([1]),
            json!(["lz4", "lz4"]),
            json!(["unknown"]),
            json!(["bzip2"]),
        ] {
            assert!(pilot_projects(&json!({"pilot_projects":names}), &roster).is_err());
        }
    }

    #[test]
    fn changed_pilots_invalidate_frozen_method_and_resume_identity() {
        let root = tempfile::tempdir().unwrap();
        let repo = tempfile::tempdir().unwrap();
        for file in [
            "examples/coverage-campaign.rs",
            "examples/assess-navigation.rs",
            "examples/support/capture.rs",
            "examples/support/source_inventory.rs",
            "examples/compass-scale.rs",
            "examples/support/cscope_db.rs",
            "examples/support/reference.rs",
            "target/release/examples/coverage-campaign",
            "target/release/examples/assess-navigation",
            "target/release/examples/compass-scale",
        ] {
            let path = repo.path().join(file);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, b"fixed method fixture").unwrap();
        }
        let binary = root.path().join("product");
        let reference = root.path().join("reference");
        fs::write(&binary, b"fixed product").unwrap();
        fs::write(&reference, b"fixed reference").unwrap();
        let mut identities = json!({
            "binary":binary,"binary_sha256":capture::digest(&binary).unwrap(),
            "reference":reference,"reference_sha256":capture::digest(&reference).unwrap(),
            "pilot_projects":["lz4"]
        });
        let roster = json!({"projects":[
            {"name":"lz4","group":"development"},
            {"name":"nanopb","group":"development"}
        ]});
        for file in ["roster.json", "acquisition.json", "draws.json"] {
            capture::save(&root.path().join(file), &roster).unwrap();
        }
        capture::save(&root.path().join("identities.json"), &identities).unwrap();
        let frozen = method_identity(root.path(), repo.path()).unwrap();
        let previous = measurement_identity(root.path(), repo.path(), "lz4", true).unwrap();
        capture::save(
            &root.path().join("method-freeze.json"),
            &json!({"identity":frozen}),
        )
        .unwrap();
        identities["pilot_projects"] = json!(["nanopb"]);
        capture::save(&root.path().join("identities.json"), &identities).unwrap();
        assert_ne!(
            read(&root.path().join("method-freeze.json")).unwrap()["identity"],
            method_identity(root.path(), repo.path()).unwrap()
        );
        assert_ne!(
            previous,
            measurement_identity(root.path(), repo.path(), "lz4", true).unwrap()
        );
        assert_eq!(read(&root.path().join("roster.json")).unwrap(), roster);

        let mut inconsistent = roster.clone();
        inconsistent["projects"][1]["group"] = json!("reserved");
        capture::save(&root.path().join("acquisition.json"), &inconsistent).unwrap();
        assert!(method_identity(root.path(), repo.path()).is_err());
    }

    #[test]
    fn sources_without_includes_still_declare_all_search_fields() {
        let root = tempfile::tempdir().unwrap();
        let source = tempfile::tempdir().unwrap();
        fs::write(source.path().join("source.c"), "int assigned = 1;\n").unwrap();
        inventory_fixture(root.path(), source.path(), &["source.c"]);
        capture::save(
            &root.path().join("draws.json"),
            &json!({"projects":[{"name":"project","files":[{"path":"source.c"}]}]}),
        )
        .unwrap();
        fs::create_dir(root.path().join("source-facts")).unwrap();
        capture::save(
            &root.path().join("source-facts/project.json"),
            &json!({"definitions":[["source.c",1,"assigned","global initializer"]],
                "calls":[],"contexts":[],"controls":[],"unresolved":[],"unscored_queries":[],
                "extra_queries":[
                    {"field":0,"pattern":"assigned","expected":[["source.c","<global>",1,"global initializer"]]},
                    {"field":9,"pattern":"assigned","expected":[["source.c","<global>",1,"global initializer"]]}
                ]}),
        ).unwrap();
        prepare(root.path()).unwrap();
        let draft = read(&root.path().join("drafts/project.json")).unwrap();
        let fields = draft["query_inventory"]
            .as_array()
            .unwrap()
            .iter()
            .map(|query| query[0].as_u64().unwrap())
            .chain(
                draft["extra_queries"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|query| query["field"].as_u64().unwrap()),
            )
            .collect::<BTreeSet<_>>();
        assert_eq!(fields, (0..=9).collect());
        assert!(
            draft["query_inventory"]
                .as_array()
                .unwrap()
                .iter()
                .any(|query| query[0] == 8)
        );
    }
}
