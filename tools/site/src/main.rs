use regex::{Captures, Regex};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

const SITE: &str = "https://ripscope.com";
const INSTALLERS: [&str; 2] = ["install.sh", "install.ps1"];
const USAGE: &str = "usage: scripts/build-site.sh (--dev | --deploy) [--render-diagrams] [--check-external] [--docs-only]\nOutput: target/site. --docs-only is a development check, never a deployment build.";
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Debug)]
struct Chapter {
    title: String,
    source: PathBuf,
    html: PathBuf,
}

#[derive(Default)]
struct Options {
    dev: bool,
    deploy: bool,
    render: bool,
    external: bool,
    docs_only: bool,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("site build: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let mut options = Options::default();
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--list-inputs" => {
                for input in source_inputs(&root)? {
                    println!("{}", input.display());
                }
                return Ok(());
            }
            "--dev" => options.dev = true,
            "--deploy" => options.deploy = true,
            "--render-diagrams" => options.render = true,
            "--check-external" => options.external = true,
            "--docs-only" => options.docs_only = true,
            "--help" | "-h" => {
                println!("{USAGE}");
                return Ok(());
            }
            _ => return Err(USAGE.into()),
        }
    }
    if options.dev == options.deploy || (options.docs_only && !options.dev) {
        return Err(USAGE.into());
    }
    let release_tag = std::env::var("RIPSCOPE_SITE_RELEASE_TAG").ok();
    let release_version = site_version(&root, release_tag.as_deref())?;
    if options.deploy {
        check_deployable_version(&root, &release_version)?;
    }
    let mdbook = std::env::var("MDBOOK").unwrap_or_else(|_| "mdbook".into());
    let version = output(Command::new(&mdbook).arg("--version"))?;
    if version.trim() != "mdbook v0.5.4" {
        return Err(format!("expected mdbook v0.5.4, got {}", version.trim()).into());
    }
    fs::create_dir_all(root.join("target"))?;
    let work = tempfile::Builder::new()
        .prefix("site-build-")
        .tempdir_in(root.join("target"))?;
    let staged = work.path().join("source");
    let site = work.path().join("output");
    let chapters = stage(&root, &staged)?;
    let docs = site.join("docs");
    let status = Command::new(&mdbook)
        .arg("build")
        .arg(&staged)
        .arg("--dest-dir")
        .arg(&docs)
        .status()?;
    if !status.success() {
        return Err("mdBook failed".into());
    }
    finish_docs(&root, &staged, &docs, &chapters, options.render)?;
    stage_evidence(&root, &site)?;
    if !options.docs_only {
        if !root.join("site/index.html").is_file() {
            return Err("site/index.html is required for the combined build".into());
        }
        copy_site(&root.join("site"), &site)?;
        stage_installers(&root, &site)?;
        let landing = site.join("index.html");
        let page = fs::read_to_string(&landing)?;
        fs::write(
            &landing,
            links(&page.replace("@VERSION@", &release_version)),
        )?;
        check_metadata(
            &landing,
            &fs::read_to_string(&landing)?,
            &format!("{SITE}/"),
        )?;
    }
    write_indexes(&site, &chapters, !options.docs_only)?;
    let external = validate(&site)?;
    if options.external {
        check_external(work.path(), &external)?;
    }
    let target = root.join("target/site");
    if target.exists() {
        fs::remove_dir_all(&target)?;
    }
    fs::rename(&site, &target)?;
    println!(
        "{} chapters built and internal links checked in {}{}",
        chapters.len(),
        target.display(),
        if options.docs_only {
            " (documentation-only development check)"
        } else {
            ""
        }
    );
    if !options.external {
        println!("External URLs collected; use --check-external to verify their remote responses.");
    }
    Ok(())
}

// Shared documentation closure for the public-tree exporter. Site and tooling files
// have their own explicit export selection; this lists no internal documentation.
fn source_inputs(root: &Path) -> Result<BTreeSet<PathBuf>> {
    let summary = fs::read_to_string(root.join("docs/SUMMARY.md"))?;
    let mut inputs = BTreeSet::from([PathBuf::from("docs/SUMMARY.md")]);
    let include = Regex::new(r"\{\{#include\s+([^}\s]+)\s*\}\}")?;
    let mut pending: Vec<_> = chapters(&summary)?
        .into_iter()
        .map(|c| root.join("docs").join(c.source))
        .collect();
    for source in &pending {
        let text = expand(root, source, &mut Vec::new())?;
        for asset in asset_inputs(root, source, &text)? {
            inputs.insert(asset.strip_prefix(root)?.to_owned());
        }
    }
    while let Some(source) = pending.pop() {
        let source = source.canonicalize()?;
        let relative = source
            .strip_prefix(root)
            .map_err(|_| "documentation include escapes checkout")?;
        if !inputs.insert(relative.to_owned()) {
            continue;
        }
        let text = fs::read_to_string(&source)?;
        for found in include.captures_iter(&text) {
            if found[1].contains(':') {
                return Err("unsupported documentation include selector".into());
            }
            pending.push(source.parent().unwrap().join(&found[1]));
        }
    }
    for source in installer_files(root)? {
        inputs.insert(source.strip_prefix(root)?.to_owned());
    }
    for source in evidence_files(root)?.values() {
        inputs.insert(source.strip_prefix(root)?.to_owned());
    }
    Ok(inputs)
}

// One explicit source selection serves the website and public source export.
// Captures stay canonical in tests/fixtures; only their delivered paths vary.
fn evidence_files(root: &Path) -> Result<BTreeMap<PathBuf, PathBuf>> {
    let map = PathBuf::from("evidence/sources.json");
    let mut result = BTreeMap::from([(map.clone(), evidence_source(root, &map)?)]);
    let manifest: serde_json::Value = serde_json::from_str(&fs::read_to_string(root.join(&map))?)?;
    let object = manifest
        .as_object()
        .ok_or("evidence map must be an object")?;
    if object.len() != 3 || object.get("version").and_then(serde_json::Value::as_u64) != Some(1) {
        return Err(
            "expected evidence map version 1 with only version, metadata and mappings".into(),
        );
    }
    let metadata = object
        .get("metadata")
        .and_then(serde_json::Value::as_array)
        .ok_or("evidence metadata must be an array")?;
    for file in metadata {
        let path = evidence_path(file.as_str().ok_or("evidence metadata must name a file")?)?;
        if !path.starts_with("evidence") || path.starts_with("evidence/captures") || path == map {
            return Err(
                "metadata must name files under evidence, outside captures and sources.json".into(),
            );
        }
        let source = evidence_source(root, &path)?;
        if result.insert(path, source).is_some() {
            return Err("duplicate evidence metadata".into());
        }
    }
    let mappings = object
        .get("mappings")
        .and_then(serde_json::Value::as_array)
        .ok_or("evidence mappings must be an array")?;
    for mapping in mappings {
        let mapping = mapping
            .as_object()
            .ok_or("an evidence mapping must be an object")?;
        if mapping.len() != 2 {
            return Err("an evidence mapping contains only source and destination".into());
        }
        let source = evidence_path(
            mapping
                .get("source")
                .and_then(serde_json::Value::as_str)
                .ok_or("evidence source missing")?,
        )?;
        let destination = evidence_path(
            mapping
                .get("destination")
                .and_then(serde_json::Value::as_str)
                .ok_or("evidence destination missing")?,
        )?;
        if !source.starts_with("tests/fixtures")
            || !destination.starts_with("evidence/captures")
            || destination == Path::new("evidence/captures")
        {
            return Err("evidence mappings must copy tests/fixtures into evidence/captures".into());
        }
        let file = evidence_source(root, &source)?;
        if result.insert(destination, file).is_some() {
            return Err("conflicting evidence destination".into());
        }
    }
    let mut folded = BTreeSet::new();
    for path in result.keys() {
        if !folded.insert(path.to_string_lossy().to_lowercase()) {
            return Err("case-insensitive evidence destination collision".into());
        }
    }
    for path in &folded {
        for parent in Path::new(path).ancestors().skip(1) {
            if folded.contains(parent.to_str().ok_or("invalid evidence destination")?) {
                return Err("evidence file is also a directory".into());
            }
        }
    }
    Ok(result)
}

fn evidence_source(root: &Path, source: &Path) -> Result<PathBuf> {
    let mut path = root.to_owned();
    for part in source.components() {
        path.push(part);
        if fs::symlink_metadata(&path)?.file_type().is_symlink() {
            return Err("symlink evidence input is not allowed".into());
        }
    }
    if !path.is_file() {
        return Err("evidence input must be a regular file".into());
    }
    Ok(path)
}

fn evidence_path(value: &str) -> Result<PathBuf> {
    let path = Path::new(value);
    if value.is_empty()
        || value.contains(['\\', ':', '\n', '\r', '\0'])
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err("evidence paths must be plain relative paths".into());
    }
    Ok(path.to_owned())
}

fn stage_evidence(root: &Path, output: &Path) -> Result<()> {
    for (destination, source) in evidence_files(root)? {
        let destination = output.join(destination);
        fs::create_dir_all(destination.parent().ok_or("evidence parent missing")?)?;
        fs::copy(source, destination)?;
    }
    Ok(())
}

fn output(command: &mut Command) -> Result<String> {
    let result = command.output()?;
    if !result.status.success() {
        return Err(String::from_utf8_lossy(&result.stderr).into_owned().into());
    }
    Ok(String::from_utf8(result.stdout)?)
}

fn package_version(root: &Path) -> Result<String> {
    let manifest = fs::read_to_string(root.join("Cargo.toml"))?;
    Ok(Regex::new(r#"(?m)^version = "([^"]+)""#)?
        .captures(&manifest)
        .ok_or("package version missing")?[1]
        .to_owned())
}

fn site_version(root: &Path, release_tag: Option<&str>) -> Result<String> {
    match release_tag {
        None => package_version(root),
        Some(tag) => {
            if !Regex::new(r"^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$")?.is_match(tag) {
                return Err("RIPSCOPE_SITE_RELEASE_TAG must be a stable vX.Y.Z tag".into());
            }
            Ok(tag[1..].to_owned())
        }
    }
}

#[cfg(test)]
fn check_deployable(root: &Path) -> Result<()> {
    check_deployable_version(root, &package_version(root)?)
}

fn check_deployable_version(root: &Path, version: &str) -> Result<()> {
    // The landing page links to this version's release downloads.
    let tag = format!("v{version}");
    if output(Command::new("git").current_dir(root).args([
        "rev-parse",
        "--verify",
        "--quiet",
        &format!("refs/tags/{tag}^{{commit}}"),
    ]))
    .is_err()
    {
        return Err(format!("deployment build requires the release tag {tag}").into());
    }
    if !output(Command::new("git").current_dir(root).args([
        "status",
        "--porcelain",
        "--untracked-files=normal",
    ]))?
    .trim()
    .is_empty()
    {
        return Err("deployment build requires a clean source checkout".into());
    }
    Ok(())
}

fn chapters(summary: &str) -> Result<Vec<Chapter>> {
    let pattern = Regex::new(r"\[([^\]]+)\]\(([^)]+\.md)\)")?;
    let mut seen = BTreeSet::new();
    let mut result = Vec::new();
    for found in pattern.captures_iter(summary) {
        let source = PathBuf::from(&found[2]);
        if source
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
            || !seen.insert(source.clone())
        {
            return Err(
                format!("invalid or duplicate SUMMARY chapter: {}", source.display()).into(),
            );
        }
        let html = if source.file_name().and_then(|s| s.to_str()) == Some("README.md") {
            source.with_file_name("index.html")
        } else {
            source.with_extension("html")
        };
        result.push(Chapter {
            title: found[1].into(),
            source,
            html,
        });
    }
    if result.is_empty() {
        return Err("SUMMARY.md contains no chapters".into());
    }
    Ok(result)
}

fn stage(root: &Path, staged: &Path) -> Result<Vec<Chapter>> {
    fs::create_dir_all(staged.join("docs"))?;
    fs::copy(root.join("book.toml"), staged.join("book.toml"))?;
    let summary = fs::read_to_string(root.join("docs/SUMMARY.md"))?;
    let chapters = chapters(&summary)?;
    fs::write(staged.join("docs/SUMMARY.md"), summary)?;
    for chapter in &chapters {
        let source = root.join("docs").join(&chapter.source);
        let markdown = expand(root, &source, &mut Vec::new())?;
        write(&staged.join("docs").join(&chapter.source), &markdown)?;
        copy_assets(root, staged, &source, &markdown)?;
    }
    copy_tree(&root.join("docs/theme"), &staged.join("docs/theme"))?;
    Ok(chapters)
}

fn expand(root: &Path, source: &Path, stack: &mut Vec<PathBuf>) -> Result<String> {
    let source = source.canonicalize()?;
    if !source.starts_with(root) || stack.contains(&source) {
        return Err(format!("unsafe or cyclic include: {}", source.display()).into());
    }
    stack.push(source.clone());
    let text = fs::read_to_string(&source)?;
    let include = Regex::new(r"\{\{#include\s+([^}\s]+)\s*\}\}")?;
    let mut result = String::new();
    let mut end = 0;
    for found in include.captures_iter(&text) {
        let all = found.get(0).unwrap();
        result.push_str(&text[end..all.start()]);
        // Named anchors and line selections need an explicit implementation before use.
        if found[1].contains(':') {
            return Err(format!("unsupported include selector in {}", source.display()).into());
        }
        let included = source.parent().unwrap().join(&found[1]);
        result.push_str(&expand(root, &included, stack)?);
        end = all.end();
    }
    result.push_str(&text[end..]);
    stack.pop();
    Ok(result)
}

fn copy_assets(root: &Path, staged: &Path, source: &Path, markdown: &str) -> Result<()> {
    for asset in asset_inputs(root, source, markdown)? {
        if asset.starts_with(root.join("evidence")) {
            continue; // Delivered at /evidence by the shared evidence map.
        }
        let relative = asset.strip_prefix(root.join("docs"))?;
        let dest = staged.join("docs").join(relative);
        fs::create_dir_all(dest.parent().unwrap())?;
        fs::copy(asset, dest)?;
    }
    Ok(())
}

fn asset_inputs(root: &Path, source: &Path, markdown: &str) -> Result<Vec<PathBuf>> {
    let mut assets = Vec::new();
    let pattern = Regex::new(r"!?\[[^\]]*\]\(([^)\s]+)(?:\s+[^)]*)?\)")?;
    for link in pattern.captures_iter(markdown) {
        let path = link[1].split(['#', '?']).next().unwrap();
        if path.is_empty() || path.contains(':') || path.starts_with('/') || path.ends_with(".md") {
            continue;
        }
        let asset = source.parent().unwrap().join(path);
        if !asset.is_file() {
            continue;
        } // Validation reports unresolved links after rendering.
        let asset = asset.canonicalize()?;
        if asset.starts_with(root.join("evidence")) {
            if !matches!(
                asset.extension().and_then(|s| s.to_str()),
                Some("json" | "tsv" | "txt" | "c" | "h" | "l" | "y" | "inc")
            ) {
                return Err(format!(
                    "linked evidence must be a capture or C source file: {}",
                    asset.display()
                )
                .into());
            }
            if !evidence_files(root)?.contains_key(asset.strip_prefix(root)?) {
                return Err("linked evidence file is absent from the evidence map".into());
            }
            // Metadata and canonical capture inputs come from the map, never physical aliases.
            continue;
        }
        if !matches!(
            asset.extension().and_then(|s| s.to_str()),
            Some("svg" | "png" | "jpg" | "jpeg" | "gif" | "webp" | "avif" | "woff2")
        ) {
            return Err(format!(
                "chapter asset must be an image or font, not a raw artifact: {}",
                asset.display()
            )
            .into());
        }
        let relative = asset
            .strip_prefix(root.join("docs"))
            .map_err(|_| "chapter assets must be under docs/")?;
        if relative
            .components()
            .any(|c| matches!(c, Component::Normal(n) if n == "history" || n == "agents"))
        {
            return Err(format!("internal asset is not publishable: {}", asset.display()).into());
        }
        assets.push(asset);
    }
    Ok(assets)
}

fn write(path: &Path, text: &str) -> Result<()> {
    fs::create_dir_all(path.parent().unwrap())?;
    fs::write(path, text)?;
    Ok(())
}

fn files(path: &Path) -> Result<Vec<PathBuf>> {
    let mut result = Vec::new();
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        if kind.is_symlink() {
            return Err(format!(
                "symlinks are not allowed in site inputs: {}",
                entry.path().display()
            )
            .into());
        }
        if kind.is_dir() {
            result.extend(files(&entry.path())?);
        } else if kind.is_file() {
            result.push(entry.path());
        }
    }
    result.sort();
    Ok(result)
}

fn copy_tree(source: &Path, target: &Path) -> Result<()> {
    for file in files(source)? {
        let dest = target.join(file.strip_prefix(source)?);
        fs::create_dir_all(dest.parent().unwrap())?;
        fs::copy(file, dest)?;
    }
    Ok(())
}

fn installer_files(root: &Path) -> Result<[PathBuf; 2]> {
    let paths = INSTALLERS.map(|name| root.join(name));
    for path in &paths {
        if !fs::symlink_metadata(path)?.file_type().is_file() {
            return Err("canonical installers must be regular files, not symlinks".into());
        }
    }
    Ok(paths)
}

fn stage_installers(root: &Path, target: &Path) -> Result<()> {
    for source in installer_files(root)? {
        fs::copy(&source, target.join(source.file_name().unwrap()))?;
    }
    Ok(())
}

fn copy_site(source: &Path, target: &Path) -> Result<()> {
    if INSTALLERS
        .iter()
        .any(|name| fs::symlink_metadata(source.join(name)).is_ok())
    {
        return Err(
            "site/install.sh and site/install.ps1 would shadow canonical root installers".into(),
        );
    }
    if source.join("docs").exists() || source.join("evidence").exists() {
        return Err("site/docs and site/evidence are reserved for generated output".into());
    }
    for file in files(source)? {
        let relative = file.strip_prefix(source)?;
        if relative.starts_with("diagrams") {
            continue;
        }
        let dest = target.join(relative);
        fs::create_dir_all(dest.parent().unwrap())?;
        fs::copy(file, dest)?;
    }
    Ok(())
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn unescape(text: &str) -> String {
    text.replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

fn route(url: &str) -> String {
    if url.starts_with("//") || (url.contains(':') && !url.starts_with(&format!("{SITE}/"))) {
        return url.into();
    }
    let suffix = url.find(['?', '#']).unwrap_or(url.len());
    let (path, extra) = url.split_at(suffix);
    let path = if (path == "index.html" || path.ends_with("/index.html"))
        && let Some(prefix) = path.strip_suffix("index.html")
    {
        if prefix.is_empty() { "./" } else { prefix }
    } else {
        path.strip_suffix(".html").unwrap_or(path)
    };
    format!("{path}{extra}")
}

fn links(html: &str) -> String {
    let tag = Regex::new(r"(?s)<(?:a|link|iframe)\b[^>]*>").unwrap();
    let attr = Regex::new(r#"((?:href|src)=["'])(.*?)(["'])"#).unwrap();
    tag.replace_all(html, |m: &Captures<'_>| {
        attr.replace_all(&m[0], |a: &Captures<'_>| {
            format!("{}{}{}", &a[1], escape(&route(&unescape(&a[2]))), &a[3])
        })
        .into_owned()
    })
    .into_owned()
}

fn description(markdown: &str, title: &str) -> String {
    let comments = Regex::new(r"(?s)<!--.*?-->").unwrap();
    let markdown = comments.replace_all(markdown, "");
    let first = markdown
        .split("\n\n")
        .find(|p| !p.trim().is_empty() && !p.trim_start().starts_with(['#', '|', '`', '-']))
        .unwrap_or(title);
    let linked = Regex::new(r"\[([^\]]+)\]\([^)]+\)")
        .unwrap()
        .replace_all(first, "$1");
    let plain = linked
        .replace(['`', '*'], "")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    plain.chars().take(240).collect()
}

fn finish_docs(
    root: &Path,
    staged: &Path,
    docs: &Path,
    chapters: &[Chapter],
    render: bool,
) -> Result<()> {
    let desc = Regex::new(r#"<meta name="description" content="[^"]*">"#)?;
    for chapter in chapters {
        let markdown = fs::read_to_string(staged.join("docs").join(&chapter.source))?;
        write(&docs.join(&chapter.source), &markdown)?;
        let path = docs.join(&chapter.html);
        let mut html = fs::read_to_string(&path)?;
        let summary = escape(&description(&markdown, &chapter.title));
        if desc.find_iter(&html).count() != 1 {
            return Err(format!("{}: description markup changed", path.display()).into());
        }
        html = desc
            .replace(
                &html,
                format!("<meta name=\"description\" content=\"{summary}\">"),
            )
            .into_owned();
        let url = route(&format!("{SITE}/docs/{}", chapter.html.to_string_lossy()));
        let alternate = chapter.source.file_name().unwrap().to_string_lossy();
        let metadata = format!(
            "<link rel=\"canonical\" href=\"{url}\">\n<link rel=\"alternate\" type=\"text/markdown\" href=\"{alternate}\">\n<meta property=\"og:url\" content=\"{url}\">\n<meta property=\"og:description\" content=\"{summary}\">\n<meta name=\"twitter:description\" content=\"{summary}\">\n"
        );
        html = html.replacen("</head>", &format!("{metadata}</head>"), 1);
        fs::write(&path, html)?;
    }
    render_diagrams(root, docs, render)?;
    // Auxiliary pages must not compete with canonical chapter URLs in search engines.
    for file in files(docs)? {
        if file.extension().and_then(|s| s.to_str()) != Some("html") {
            continue;
        }
        let mut html = fs::read_to_string(&file)?;
        if !chapters.iter().any(|c| docs.join(&c.html) == file) {
            html = html.replacen(
                "</head>",
                "<meta name=\"robots\" content=\"noindex\">\n</head>",
                1,
            );
        }
        if html.contains("id=\"mdbook-sidebar-toggle\"") {
            html = navigation(&html)?;
        }
        fs::write(file, html)?;
    }
    rewrite_assets(docs)?;
    for file in files(docs)? {
        if file.extension().and_then(|s| s.to_str()) == Some("html") {
            fs::write(&file, links(&fs::read_to_string(&file)?))?;
        }
    }
    for chapter in chapters {
        let path = docs.join(&chapter.html);
        check_metadata(
            &path,
            &fs::read_to_string(&path)?,
            &route(&format!("{SITE}/docs/{}", chapter.html.to_string_lossy())),
        )?;
    }
    Ok(())
}

fn navigation(html: &str) -> Result<String> {
    let button = Regex::new(r#"(?s)<label([^>]*\bid="mdbook-sidebar-toggle"[^>]*)>(.*?)</label>"#)?;
    if button.find_iter(html).count() != 1
        || html.matches("<main>").count() != 1
        || html.matches("<body>").count() != 1
    {
        return Err("mdBook navigation markup changed; update the site builder".into());
    }
    let result = button.replace(html, |m: &Captures<'_>| format!("<button type=\"button\"{}>{}</button><noscript><a class=\"sidebar-contents-link\" href=\"/docs/toc\">Contents</a></noscript>", m[1].replace(" for=\"mdbook-sidebar-toggle-anchor\"", ""), &m[2]));
    let result = result
        .replacen(
            "<body>",
            "<body>\n<a class=\"skip-link\" href=\"#main-content\">Skip to content</a>",
            1,
        )
        .replacen("<main>", "<main id=\"main-content\" tabindex=\"-1\">", 1)
        .replacen(
            "<h1 class=\"menu-title\">Ripscope</h1>",
            "<p class=\"menu-title\"><a class=\"docs-brand\" href=\"/\"><span class=\"brand-reticle\" aria-hidden=\"true\"></span>Ripscope</a><a class=\"docs-home\" href=\"/docs/\">Docs</a></p>",
            1,
        )
        .replace(
            "<div class=\"table-wrapper\">",
            "<div class=\"table-wrapper\" tabindex=\"0\">",
        );
    Ok(Regex::new(r"(<pre\b[^>]*>\s*<code\b)([^>]*)(>)")?
        .replace_all(&result, "$1$2 tabindex=\"0\"$3")
        .into_owned())
}

fn rewrite_assets(docs: &Path) -> Result<()> {
    let mut renames = BTreeMap::new();
    let toc_pattern = Regex::new(
        r"\s*if \(current_page\.endsWith\('/'\)\) \{\s*current_page \+= 'index\.html';\s*\}",
    )?;
    let search_pattern = Regex::new(r#""doc_urls":(\[[^\]]*\])"#)?;
    for prefix in ["toc-", "searchindex-"] {
        let matches: Vec<_> = files(docs)?
            .into_iter()
            .filter(|p| {
                p.parent() == Some(docs)
                    && p.file_name().unwrap().to_string_lossy().starts_with(prefix)
                    && p.extension().and_then(|s| s.to_str()) == Some("js")
            })
            .collect();
        if matches.len() != 1 {
            return Err(format!("expected one mdBook {prefix} asset").into());
        }
        let old = &matches[0];
        let text = fs::read_to_string(old)?;
        let text = if prefix == "toc-" {
            let pattern = &toc_pattern;
            if pattern.find_iter(&text).count() != 1 {
                return Err("mdBook TOC matching changed".into());
            }
            pattern.replace(&links(&text), "").into_owned()
        } else {
            let pattern = &search_pattern;
            if pattern.find_iter(&text).count() != 1 {
                return Err("mdBook search index changed".into());
            }
            pattern
                .replace(&text, |m: &Captures<'_>| {
                    let urls: Vec<String> =
                        serde_json::from_str(&m[1]).expect("mdBook search URL array");
                    format!(
                        "\"doc_urls\":{}",
                        serde_json::to_string(&urls.iter().map(|u| route(u)).collect::<Vec<_>>())
                            .unwrap()
                    )
                })
                .into_owned()
        };
        let name = format!(
            "{prefix}{}.js",
            base16ct::lower::encode_string(&Sha256::digest(text.as_bytes()))
        );
        fs::write(docs.join(&name), text)?;
        fs::remove_file(old)?;
        renames.insert(
            old.file_name().unwrap().to_string_lossy().into_owned(),
            name,
        );
    }
    for file in files(docs)? {
        if file.extension().and_then(|s| s.to_str()) != Some("html") {
            continue;
        }
        let mut text = fs::read_to_string(&file)?;
        for (old, new) in &renames {
            text = text.replace(old, new);
        }
        fs::write(file, text)?;
    }
    Ok(())
}

fn render_diagrams(root: &Path, docs: &Path, refresh: bool) -> Result<()> {
    let diagram = Regex::new(
        r#"(?s)<pre><code class="language-mermaid">(.*?)</code></pre>\s*(?:<!-- description: (.*?) -->)?"#,
    )?;
    let cache = root.join("site/diagrams/rendered");
    let mut used = BTreeSet::new();
    for page in files(docs)? {
        if page.extension().and_then(|s| s.to_str()) != Some("html") {
            continue;
        }
        let html = fs::read_to_string(&page)?;
        let mut result = String::new();
        let mut end = 0;
        for found in diagram.captures_iter(&html) {
            let whole = found.get(0).unwrap();
            result.push_str(&html[end..whole.start()]);
            let description = found
                .get(2)
                .ok_or_else(|| {
                    format!(
                        "{}: each Mermaid diagram needs a <!-- description: ... --> caption",
                        page.display()
                    )
                })?
                .as_str();
            let source = unescape(&found[1]);
            let mut images = String::new();
            for theme in ["dark", "light"] {
                let config = root.join(format!("tools/site/diagrams/{theme}.json"));
                let mut digest = Sha256::new();
                digest.update(source.as_bytes());
                digest.update(b"\0");
                digest.update(fs::read(&config)?);
                let name = format!(
                    "{}-{theme}.svg",
                    base16ct::lower::encode_string(&digest.finalize())
                );
                let render = cache.join(&name);
                if !render.is_file() {
                    if !refresh {
                        return Err(format!("missing diagram {name}; use --render-diagrams, inspect and commit the SVGs").into());
                    }
                    render_svg(root, &source, &config, &render)?;
                }
                used.insert(name.clone());
                let dest = docs.join("diagrams").join(&name);
                fs::create_dir_all(dest.parent().unwrap())?;
                fs::copy(&render, &dest)?;
                images.push_str(&format!("<img class=\"diagram-{theme}\" src=\"/docs/diagrams/{name}\" alt=\"\" loading=\"lazy\">"));
            }
            result.push_str(&format!(
                "<figure class=\"diagram\">{images}<figcaption>{}</figcaption></figure>",
                escape(description.trim())
            ));
            end = whole.end();
        }
        result.push_str(&html[end..]);
        fs::write(page, result)?;
    }
    if cache.is_dir() {
        let unused: Vec<_> = files(&cache)?
            .into_iter()
            .filter(|p| !used.contains(p.file_name().unwrap().to_string_lossy().as_ref()))
            .collect();
        if !unused.is_empty() {
            return Err(format!(
                "unused committed diagram renders: {unused:?}; inspect and remove obsolete SVGs"
            )
            .into());
        }
    }
    Ok(())
}

fn render_svg(root: &Path, source: &str, config: &Path, target: &Path) -> Result<()> {
    let temp = tempfile::tempdir()?;
    let input = temp.path().join("diagram.mmd");
    fs::write(&input, source)?;
    fs::create_dir_all(target.parent().unwrap())?;
    let mmdc = std::env::var_os("RIPSCOPE_MMDC")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("tools/site/node_modules/.bin/mmdc"));
    let status = Command::new(mmdc)
        .args(["--quiet", "--backgroundColor", "transparent", "--input"])
        .arg(&input)
        .arg("--output")
        .arg(target)
        .arg("--configFile")
        .arg(config)
        .arg("--puppeteerConfigFile")
        .arg(root.join("tools/site/diagrams/puppeteer.json"))
        .status()?;
    if !status.success() {
        return Err("Mermaid renderer failed".into());
    }
    let svg = fs::read_to_string(target)?;
    if svg.contains("<script") || svg.contains("<foreignObject") {
        return Err("diagram must be a static SVG without script or foreignObject".into());
    }
    let box_pattern = Regex::new(r#"viewBox="[-\d.]+ [-\d.]+ ([\d.]+) ([\d.]+)"#)?;
    let view = box_pattern
        .captures(&svg)
        .ok_or("diagram viewBox missing")?;
    let width = view[1].parse::<f64>()?.ceil() as u64;
    let height = view[2].parse::<f64>()?.ceil() as u64;
    let svg = svg.replacen(
        "width=\"100%\"",
        &format!("width=\"{width}\" height=\"{height}\""),
        1,
    );
    fs::write(target, svg)?;
    Ok(())
}

fn check_metadata(path: &Path, html: &str, expected: &str) -> Result<()> {
    for pattern in [
        r#"<link rel="canonical" href="([^"]+)""#,
        r#"<meta property="og:url" content="([^"]+)""#,
    ] {
        let pattern = Regex::new(pattern)?;
        let values: Vec<_> = pattern
            .captures_iter(html)
            .map(|m| m[1].to_owned())
            .collect();
        if values != [expected] {
            return Err(format!(
                "{}: canonical/social URL must be exactly {expected}",
                path.display()
            )
            .into());
        }
    }
    Ok(())
}

fn write_indexes(site: &Path, chapters: &[Chapter], landing: bool) -> Result<()> {
    let mut sitemap = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">\n",
    );
    if landing {
        sitemap.push_str(&format!("  <url><loc>{SITE}/</loc></url>\n"));
    }
    let mut llms = String::from(
        "# Ripscope\n\n> C source navigation through cscope searches, with atomic index updates and Vim, Neovim and Emacs integrations.\n\n## Documentation\n\n",
    );
    for chapter in chapters {
        sitemap.push_str(&format!(
            "  <url><loc>{}</loc></url>\n",
            escape(&route(&format!(
                "{SITE}/docs/{}",
                chapter.html.to_string_lossy()
            )))
        ));
        llms.push_str(&format!(
            "- [{}]({SITE}/docs/{})\n",
            chapter.title,
            chapter.source.to_string_lossy()
        ));
    }
    sitemap.push_str("</urlset>\n");
    fs::write(site.join("sitemap.xml"), sitemap)?;
    fs::write(site.join("llms.txt"), llms)?;
    fs::write(
        site.join("robots.txt"),
        format!("User-agent: *\nAllow: /\n\nSitemap: {SITE}/sitemap.xml\n"),
    )?;
    Ok(())
}

fn resolve(site: &Path, page: &Path, url: &str) -> Result<Option<PathBuf>> {
    let url = url.strip_prefix(SITE).unwrap_or(url);
    if url.starts_with("//") || url.contains(':') {
        return Ok(None);
    }
    let target = url.split(['?', '#']).next().unwrap();
    let base = if target.starts_with('/') {
        site.to_owned()
    } else {
        page.parent().unwrap().to_owned()
    };
    let mut path = base;
    for part in Path::new(target.trim_start_matches('/')).components() {
        match part {
            Component::Normal(name) => path.push(name),
            Component::ParentDir => {
                path.pop();
            }
            Component::CurDir => {}
            _ => return Err("invalid URL path".into()),
        }
    }
    if !path.starts_with(site) {
        return Err(format!("link escapes site output: {url}").into());
    }
    if target.is_empty() {
        return Ok(Some(page.to_owned()));
    }
    if path.is_file() {
        return Ok(Some(path));
    }
    if path.join("index.html").is_file() {
        return Ok(Some(path.join("index.html")));
    }
    if path.extension().is_none() && path.with_extension("html").is_file() {
        return Ok(Some(path.with_extension("html")));
    }
    Err(format!(
        "{}: missing internal link {url}",
        page.strip_prefix(site)?.display()
    )
    .into())
}

fn validate(site: &Path) -> Result<BTreeSet<String>> {
    let attrs = Regex::new(r#"\b(?:href|src|poster)=["']([^"']+)["']"#)?;
    let tags = Regex::new(r"(?s)<[a-zA-Z][^>]*>")?;
    let mut external = BTreeSet::new();
    let mut errors = BTreeSet::new();
    for page in files(site)? {
        if page.extension().and_then(|s| s.to_str()) != Some("html") {
            continue;
        }
        let html = fs::read_to_string(&page)?;
        for tag in tags.find_iter(&html) {
            for attr in attrs.captures_iter(tag.as_str()) {
                let url = unescape(&attr[1]);
                if url.starts_with("http://")
                    || (url.starts_with("https://") && !url.starts_with(&format!("{SITE}/")))
                {
                    external.insert(url);
                    continue;
                }
                match resolve(site, &page, &url) {
                    Ok(Some(target)) => {
                        if let Some((_, fragment)) = url.split_once('#')
                            && !fragment.is_empty()
                            && target.extension().and_then(|s| s.to_str()) == Some("html")
                        {
                            let target_html = fs::read_to_string(&target)?;
                            if !target_html.contains(&format!("id=\"{fragment}\""))
                                && !target_html.contains(&format!("id='{fragment}'"))
                            {
                                errors.insert(format!(
                                    "{}: missing anchor {url}",
                                    page.strip_prefix(site)?.display()
                                ));
                            }
                        }
                    }
                    Ok(None) => {}
                    Err(error) => {
                        errors.insert(error.to_string());
                    }
                }
            }
        }
    }
    if !errors.is_empty() {
        return Err(format!(
            "internal link validation failed:\n{}",
            errors.into_iter().collect::<Vec<_>>().join("\n")
        )
        .into());
    }
    Ok(external)
}

fn check_external(work: &Path, urls: &BTreeSet<String>) -> Result<()> {
    let input = work.join("external-links.txt");
    fs::write(&input, urls.iter().cloned().collect::<Vec<_>>().join("\n"))?;
    let status = Command::new("lychee")
        .args(["--no-progress", "--max-retries", "2"])
        .arg(&input)
        .status()
        .map_err(|e| format!("external check requires lychee on PATH: {e}"))?;
    if !status.success() {
        return Err("external link validation failed; inspect lychee's diagnostics".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deployment_guard_requires_released_version_and_clean_source() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        write(&root.join("Cargo.toml"), "[package]\nversion = \"0.1.0\"\n").unwrap();
        let git = |args: &[&str]| {
            output(
                Command::new("git")
                    .current_dir(root)
                    .args([
                        "-c",
                        "user.name=Deployment fixture",
                        "-c",
                        "user.email=fixture@example.invalid",
                    ])
                    .args(args),
            )
            .unwrap()
        };
        git(&["init", "--initial-branch=main"]);
        git(&["add", "Cargo.toml"]);
        git(&["commit", "-m", "Deployment fixture"]);
        assert!(
            check_deployable(root).is_err(),
            "unreleased version accepted"
        );
        git(&["tag", "v0.1.0"]);
        check_deployable(root).unwrap();
        write(&root.join("untracked.txt"), "pending").unwrap();
        assert!(check_deployable(root).is_err(), "untracked source accepted");
        git(&["add", "untracked.txt"]);
        assert!(check_deployable(root).is_err(), "staged source accepted");
        git(&["commit", "-m", "Later fixture source"]);
        check_deployable(root).unwrap();
        write(&root.join("Cargo.toml"), "[package]\nversion = \"0.2.0\"\n").unwrap();
        git(&["commit", "-am", "Unreleased version"]);
        assert!(
            check_deployable(root).is_err(),
            "unreleased version accepted"
        );
        assert_eq!(site_version(root, Some("v0.1.0")).unwrap(), "0.1.0");
        check_deployable_version(root, &site_version(root, Some("v0.1.0")).unwrap()).unwrap();
        assert!(check_deployable_version(root, "0.3.0").is_err());
        assert!(site_version(root, Some("v0.1.0-rc.1")).is_err());
        assert!(site_version(root, Some("main")).is_err());
    }

    #[test]
    fn clean_routes_preserve_fragments_queries_and_external_addresses() {
        for (before, after) in [
            ("index.html", "./"),
            ("../guide/index.html#x", "../guide/#x"),
            ("/docs/guide.html?a=1#x", "/docs/guide?a=1#x"),
            (
                "https://ripscope.com/docs/index.html",
                "https://ripscope.com/docs/",
            ),
            ("https://example.org/a.html", "https://example.org/a.html"),
            ("//example.org/index.html", "//example.org/index.html"),
            ("chapter.md", "chapter.md"),
            (
                "0005-one-sqlite-file-per-index.html",
                "0005-one-sqlite-file-per-index",
            ),
        ] {
            assert_eq!(route(before), after);
        }
    }

    #[test]
    fn link_rewrites_touch_actual_tags_without_altering_code_examples() {
        let input = "<a href=\"../guide/index.html?x=1&amp;y=2#x\">Guide</a><pre>&lt;a href=\"a.html\"&gt;</pre>";
        assert_eq!(
            links(input),
            input.replace("../guide/index.html", "../guide/")
        );
    }

    #[test]
    fn chapter_allowlist_rejects_traversal_and_duplicates() {
        assert!(chapters("[private](../secret.md)").is_err());
        assert!(chapters("[one](one.md)\n[two](one.md)").is_err());
        let list = chapters("[Intro](README.md)\n- [Decisions](adr/README.md)").unwrap();
        assert_eq!(list[0].html, Path::new("index.html"));
        assert_eq!(list[1].html, Path::new("adr/index.html"));
    }

    #[test]
    fn staging_excludes_unlisted_documents_and_expands_selected_includes() {
        let root = tempfile::tempdir().unwrap();
        let root = root.path();
        write(&root.join("book.toml"), "[book]\nsrc='docs'\n").unwrap();
        write(&root.join("GLOSSARY.md"), "# Glossary\n\nA term.\n").unwrap();
        write(
            &root.join("docs/SUMMARY.md"),
            "[Glossary](reference/glossary.md)",
        )
        .unwrap();
        write(
            &root.join("docs/reference/glossary.md"),
            "{{#include ../../GLOSSARY.md}}",
        )
        .unwrap();
        write(&root.join("docs/internal.json"), "secret").unwrap();
        write(&root.join("docs/history/notes.md"), "secret").unwrap();
        write(&root.join("docs/theme/head.hbs"), "head").unwrap();
        write(
            &root.join("evidence/sources.json"),
            r#"{"version":1,"metadata":[],"mappings":[]}"#,
        )
        .unwrap();
        let dest = root.join("staged");
        stage(root, &dest).unwrap();
        assert_eq!(
            fs::read_to_string(dest.join("docs/reference/glossary.md")).unwrap(),
            "# Glossary\n\nA term.\n"
        );
        assert!(!dest.join("docs/internal.json").exists());
        assert!(!dest.join("docs/history").exists());
    }

    #[test]
    fn canonical_installers_cannot_be_shadowed_or_replaced_by_directories() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        write(&root.join("install.sh"), "canonical").unwrap();
        write(&root.join("install.ps1"), "canonical powershell").unwrap();
        write(&root.join("site/install.sh"), "shadow").unwrap();
        assert!(copy_site(&root.join("site"), &root.join("output")).is_err());
        fs::remove_file(root.join("install.ps1")).unwrap();
        fs::create_dir(root.join("install.ps1")).unwrap();
        assert!(installer_files(root).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn canonical_installers_reject_symlinks() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        write(&root.join("install.sh"), "canonical").unwrap();
        std::os::unix::fs::symlink("install.sh", root.join("install.ps1")).unwrap();
        assert!(installer_files(root).is_err());
    }

    #[test]
    fn include_cycles_and_paths_outside_checkout_are_rejected() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("cycle.md");
        write(&file, "{{#include cycle.md}}").unwrap();
        assert!(expand(root.path(), &file, &mut Vec::new()).is_err());
        let other = tempfile::NamedTempFile::new().unwrap();
        assert!(expand(root.path(), other.path(), &mut Vec::new()).is_err());
    }

    #[test]
    fn evidence_map_stages_canonical_bytes_and_extends_shared_source_closure() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        write(&root.join("docs/SUMMARY.md"), "[Intro](README.md)").unwrap();
        write(
            &root.join("docs/README.md"),
            "# Intro\n\n[Evidence](../evidence/claims.json)",
        )
        .unwrap();
        write(&root.join("tests/fixtures/example.c"), "one\r\ntwo\n").unwrap();
        write(&root.join("evidence/claims.json"), "{}").unwrap();
        write(&root.join("evidence/sources.json"), r#"{"version":1,"metadata":["evidence/claims.json"],"mappings":[{"source":"tests/fixtures/example.c","destination":"evidence/captures/example.c"}]}"#).unwrap();
        for path in [
            "evidence/unexpected-tracked.json",
            "evidence/untracked.json",
            "tests/fixtures/untracked.c",
        ] {
            write(&root.join(path), "private extra").unwrap();
        }
        write(&root.join("install.sh"), "shell installer").unwrap();
        write(&root.join("install.ps1"), "PowerShell installer").unwrap();
        let inputs = source_inputs(root).unwrap();
        for path in [
            "evidence/claims.json",
            "evidence/sources.json",
            "tests/fixtures/example.c",
            "install.sh",
            "install.ps1",
        ] {
            assert!(inputs.contains(Path::new(path)), "{path}");
        }
        assert!(!inputs.contains(Path::new("evidence/captures/example.c")));
        for path in [
            "evidence/unexpected-tracked.json",
            "evidence/untracked.json",
            "tests/fixtures/untracked.c",
        ] {
            assert!(!inputs.contains(Path::new(path)), "{path}");
        }
        let output = root.join("output");
        stage_evidence(root, &output).unwrap();
        assert!(!output.join("evidence/unexpected-tracked.json").exists());
        assert!(!output.join("evidence/untracked.json").exists());
        assert_eq!(
            fs::read(output.join("evidence/captures/example.c")).unwrap(),
            b"one\r\ntwo\n"
        );
        // An exported source tree already has aliases; canonical bytes still win.
        fs::create_dir_all(root.join("evidence/captures")).unwrap();
        fs::write(root.join("evidence/captures/example.c"), "stale alias").unwrap();
        assert_eq!(source_inputs(root).unwrap(), inputs);
        stage_evidence(root, &output).unwrap();
        assert_eq!(
            fs::read(output.join("evidence/captures/example.c")).unwrap(),
            b"one\r\ntwo\n"
        );
        write(&root.join("site/evidence/claims.json"), "{}").unwrap();
        assert!(copy_site(&root.join("site"), &output).is_err());
    }

    #[test]
    fn evidence_map_rejects_private_sources_metadata_overwrites_and_collisions() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        assert!(evidence_files(root).is_err()); // Missing evidence directory is an error.
        fs::create_dir(root.join("evidence")).unwrap();
        assert!(evidence_files(root).is_err()); // The map is mandatory too.
        write(&root.join("tests/fixtures/example.c"), "source").unwrap();
        for mapping in [
            serde_json::json!({"source":"private/notes.txt","destination":"evidence/captures/x"}),
            serde_json::json!({"source":"tests/fixtures/example.c","destination":"evidence/claims.json"}),
            serde_json::json!({"source":"tests/fixtures/example.c","destination":"evidence/captures/../claims.json"}),
            serde_json::json!({"source":"tests/fixtures/missing.c","destination":"evidence/captures/x"}),
            serde_json::json!({"source":"tests/fixtures","destination":"evidence/captures/x"}),
            serde_json::json!({"source":"tests/fixtures/example.c","destination":"evidence/captures/x","unknown":true}),
        ] {
            write(
                &root.join("evidence/sources.json"),
                &serde_json::json!({"version":1,"metadata":[],"mappings":[mapping]}).to_string(),
            )
            .unwrap();
            assert!(evidence_files(root).is_err());
        }
        for destination in [
            "evidence/captures/x",
            "evidence/captures/X",
            "evidence/captures/x/child",
        ] {
            let map = serde_json::json!({"version":1,"metadata":[],"mappings":[
                {"source":"tests/fixtures/example.c","destination":"evidence/captures/x"},
                {"source":"tests/fixtures/example.c","destination":destination}
            ]});
            write(&root.join("evidence/sources.json"), &map.to_string()).unwrap();
            assert!(evidence_files(root).is_err());
        }
        for manifest in [
            r#"{"version":2,"metadata":[],"mappings":[]}"#,
            r#"{"version":1,"mappings":[]}"#,
            r#"{"version":1,"metadata":[],"mappings":[],"extra":true}"#,
        ] {
            write(&root.join("evidence/sources.json"), manifest).unwrap();
            assert!(evidence_files(root).is_err());
        }
        write(&root.join("evidence/claims.json"), "{}").unwrap();
        write(&root.join("evidence/CLAIMS.json"), "{}").unwrap();
        for metadata in [
            serde_json::json!(["evidence/claims.json", "evidence/claims.json"]),
            serde_json::json!(["evidence/claims.json", "evidence/CLAIMS.json"]),
            serde_json::json!(["evidence/sources.json"]),
            serde_json::json!(["evidence/captures/x"]),
            serde_json::json!(["tests/fixtures/example.c"]),
            serde_json::json!(["evidence/missing.json"]),
            serde_json::json!(["evidence"]),
            serde_json::json!([true]),
        ] {
            write(
                &root.join("evidence/sources.json"),
                &serde_json::json!({"version":1,"metadata":metadata,"mappings":[]}).to_string(),
            )
            .unwrap();
            assert!(evidence_files(root).is_err());
        }
    }

    #[cfg(unix)]
    #[test]
    fn evidence_map_rejects_symlink_components() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        write(&root.join("tests/fixtures/example.c"), "source").unwrap();
        std::os::unix::fs::symlink("example.c", root.join("tests/fixtures/link.c")).unwrap();
        write(&root.join("evidence/sources.json"), r#"{"version":1,"metadata":[],"mappings":[{"source":"tests/fixtures/link.c","destination":"evidence/captures/x"}]}"#).unwrap();
        assert!(evidence_files(root).is_err());
        write(&root.join("evidence/real/claims.json"), "{}").unwrap();
        std::os::unix::fs::symlink("real", root.join("evidence/linked")).unwrap();
        write(
            &root.join("evidence/sources.json"),
            r#"{"version":1,"metadata":["evidence/linked/claims.json"],"mappings":[]}"#,
        )
        .unwrap();
        assert!(evidence_files(root).is_err());
    }

    #[test]
    fn validation_resolves_clean_urls_and_rejects_broken_anchors() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        write(
            &root.join("index.html"),
            "<a href=\"/docs/guide#example\">Guide</a>",
        )
        .unwrap();
        write(
            &root.join("docs/guide.html"),
            "<h1 id=\"example\">Example</h1>",
        )
        .unwrap();
        assert!(validate(root).is_ok());
        write(
            &root.join("index.html"),
            "<a href=\"/docs/guide#missing\">Guide</a>",
        )
        .unwrap();
        assert!(
            validate(root)
                .unwrap_err()
                .to_string()
                .contains("missing anchor")
        );
        write(
            &root.join("index.html"),
            "<a href=\"/docs/internal.json\">Internal</a>",
        )
        .unwrap();
        assert!(
            validate(root)
                .unwrap_err()
                .to_string()
                .contains("missing internal link")
        );
    }

    #[test]
    fn changed_mdbook_navigation_fails_visibly() {
        assert!(navigation("<body><main>Changed</main></body>").is_err());
        let html = "<body><label id=\"mdbook-sidebar-toggle\" for=\"mdbook-sidebar-toggle-anchor\">Menu</label><h1 class=\"menu-title\">Ripscope</h1><main><pre><code>Code</code></pre></main></body>";
        let updated = navigation(html).unwrap();
        assert!(updated.contains("<button type=\"button\" id=\"mdbook-sidebar-toggle\""));
        assert!(updated.contains("<main id=\"main-content\" tabindex=\"-1\">"));
        assert!(updated.contains("<code tabindex=\"0\">"));
        assert!(updated.contains("class=\"docs-brand\" href=\"/\""));
        assert!(updated.contains("class=\"docs-home\" href=\"/docs/\""));
        assert!(!updated.contains("<h1 class=\"menu-title\">"));
    }

    #[test]
    fn combined_fixture_build_uses_real_mdbook_and_validates_complete_output() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        write(
            &root.join("book.toml"),
            "[book]\ntitle='Fixture'\nsrc='docs'\n[output.html]\nsite-url='/docs/'\n",
        )
        .unwrap();
        write(
            &root.join("docs/SUMMARY.md"),
            "[Introduction](README.md)\n- [Guide](guide.md)\n",
        )
        .unwrap();
        write(
            &root.join("docs/README.md"),
            "# Introduction\n\nA test introduction.\n\n[Guide](guide.md#example)\n",
        )
        .unwrap();
        write(
            &root.join("docs/guide.md"),
            "# Guide\n\nA different description.\n\n## Example\n\nUse this example.\n\n[Evidence](../evidence/claims.json)\n[Capture](../evidence/captures/example.json)\n[Source](../evidence/captures/example.c)\n",
        )
        .unwrap();
        write(
            &root.join("tests/fixtures/example.json"),
            "{\"source\":\"synthetic\"}\n",
        )
        .unwrap();
        write(&root.join("evidence/claims.json"), "{}").unwrap();
        write(&root.join("tests/fixtures/example.c"), "int proof;\n").unwrap();
        // A public source export already contains this mapped alias at build time.
        write(&root.join("evidence/captures/example.c"), "stale alias\n").unwrap();
        write(&root.join("evidence/sources.json"), r#"{"version":1,"metadata":["evidence/claims.json"],"mappings":[{"source":"tests/fixtures/example.json","destination":"evidence/captures/example.json"},{"source":"tests/fixtures/example.c","destination":"evidence/captures/example.c"}]}"#).unwrap();
        write(&root.join("docs/theme/head.hbs"), "").unwrap();
        write(&root.join("site/index.html"), &format!("<html><head><link rel=\"canonical\" href=\"{SITE}/\"><meta property=\"og:url\" content=\"{SITE}/\"></head><body><a href=\"/docs/\">Docs</a></body></html>")).unwrap();
        let staged = root.join("staged");
        let site = root.join("output");
        let chapters = stage(root, &staged).unwrap();
        let docs = site.join("docs");
        assert!(
            Command::new("mdbook")
                .arg("build")
                .arg(&staged)
                .arg("--dest-dir")
                .arg(&docs)
                .status()
                .unwrap()
                .success()
        );
        finish_docs(root, &staged, &docs, &chapters, false).unwrap();
        stage_evidence(root, &site).unwrap();
        write(&root.join("install.sh"), "shell installer").unwrap();
        write(&root.join("install.ps1"), "PowerShell installer").unwrap();
        copy_site(&root.join("site"), &site).unwrap();
        stage_installers(root, &site).unwrap();
        assert_eq!(
            fs::read(site.join("install.sh")).unwrap(),
            b"shell installer"
        );
        assert_eq!(
            fs::read(site.join("install.ps1")).unwrap(),
            b"PowerShell installer"
        );
        write_indexes(&site, &chapters, true).unwrap();
        validate(&site).unwrap();
        assert_eq!(
            fs::read(site.join("evidence/captures/example.c")).unwrap(),
            b"int proof;\n"
        );
        assert!(
            !source_inputs(root)
                .unwrap()
                .contains(Path::new("evidence/captures/example.c"))
        );
        assert!(site.join("docs/README.md").is_file());
        assert!(site.join("llms.txt").is_file());
        assert!(
            fs::read_to_string(site.join("sitemap.xml"))
                .unwrap()
                .contains("https://ripscope.com/docs/guide")
        );
        let page = fs::read_to_string(site.join("docs/index.html")).unwrap();
        assert!(page.contains("href=\"guide#example\""));
        assert!(page.contains("Skip to content"));
    }
}
