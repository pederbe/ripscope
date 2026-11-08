//! Submit checked manifests with Komac, then complete the upstream template.
use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Stdio},
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
const REPO: &str = "microsoft/winget-pkgs";

fn gh(args: &[&str], input: Option<&Value>) -> Result<String> {
    let mut command = Command::new("gh");
    command
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if input.is_some() {
        command.stdin(Stdio::piped());
    }
    let mut child = command.spawn()?;
    if let Some(input) = input {
        child
            .stdin
            .take()
            .ok_or("missing gh stdin")?
            .write_all(&serde_json::to_vec(input)?)?;
    }
    let out = child.wait_with_output()?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).into_owned().into());
    }
    Ok(String::from_utf8(out.stdout)?)
}

fn api(path: &str) -> Result<Value> {
    Ok(serde_json::from_str(&gh(&["api", path], None)?)?)
}

fn field<'a>(text: &'a str, name: &str) -> Option<&'a str> {
    text.lines()
        .find_map(|line| line.strip_prefix(&format!("{name}: ")).map(str::trim))
}

fn same_manifest(expected: &str, actual: &str) -> Result<bool> {
    // WinGetCreate reserializes YAML, changing comments, ordering and whitespace.
    // Compare every field, including installer URLs and hashes, as YAML values.
    let expected: Value = serde_saphyr::from_str(expected)?;
    let actual: Value = serde_saphyr::from_str(actual)?;
    Ok(expected == actual)
}

fn seed_body(body: &str, template: &str) -> Result<String> {
    const PREFIX: &str =
        "### Pull request has been created with [komac](https://github.com/russellbanks/Komac)";
    if body.contains("## 📖 Description") {
        return Ok(body.to_owned());
    }
    let (first, rest) = body.split_once('\n').unwrap_or((body, ""));
    let Some(generated) = first.strip_prefix(PREFIX) else {
        return Ok(body.to_owned());
    };
    let mut parts = generated.trim_start().splitn(3, ' ');
    let version = parts.next().ok_or("missing Komac introduction version")?;
    let emoji = parts.next().ok_or("missing Komac introduction marker")?;
    if !version.strip_prefix('v').is_some_and(|v| {
        v.split('.').count() == 3
            && v.split('.')
                .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
    }) || !emoji
        .strip_prefix(':')
        .and_then(|s| s.strip_suffix(':'))
        .is_some_and(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_lowercase() || b == b'_'))
    {
        return Err("unrecognized Komac introduction, preserve it for review".into());
    }
    Ok(format!(
        "{template}\n{}\n{rest}",
        parts.next().unwrap_or("")
    ))
}

fn manifests(
    directory: &Path,
    package: &str,
    version: &str,
) -> Result<(String, Vec<(String, String)>)> {
    let mut schema = None;
    let mut files = Vec::new();
    for entry in fs::read_dir(directory)? {
        let path = entry?.path();
        if path.extension().and_then(|s| s.to_str()) != Some("yaml") {
            continue;
        }
        let text = fs::read_to_string(&path)?.replace("\r\n", "\n");
        if field(&text, "PackageIdentifier") != Some(package)
            || field(&text, "PackageVersion") != Some(version)
        {
            return Err("manifest identity does not match the selected release".into());
        }
        let current = field(&text, "ManifestVersion")
            .ok_or("missing manifest schema")?
            .to_owned();
        if schema.as_ref().is_some_and(|s| s != &current) {
            return Err("mixed manifest schemas".into());
        }
        schema = Some(current);
        files.push((
            path.file_name()
                .ok_or("missing filename")?
                .to_string_lossy()
                .into_owned(),
            text,
        ));
    }
    if files.len() != 3 {
        return Err("expected one three-file manifest".into());
    }
    Ok((schema.ok_or("missing manifests")?, files))
}

fn matches_version(title: &str, package: &str, version: &str) -> bool {
    let words: Vec<_> = title
        .split(|c: char| !(c.is_ascii_alphanumeric() || ".+-".contains(c)))
        .collect();
    words.iter().any(|word| word.eq_ignore_ascii_case(package))
        && words
            .iter()
            .any(|word| *word == version || word.strip_prefix('v') == Some(version))
}

fn find_pr(package: &str, version: &str) -> Result<Option<u64>> {
    let search = gh(
        &[
            "api",
            "--method",
            "GET",
            "search/issues",
            "-f",
            &format!("q=repo:{REPO} is:pr is:open {package} in:title"),
            "-f",
            "per_page=100",
        ],
        None,
    )?;
    let search: Value = serde_json::from_str(&search)?;
    if search["incomplete_results"] != false
        || search["total_count"]
            .as_u64()
            .ok_or("missing search count")?
            > 100
    {
        return Err("incomplete WinGet duplicate search".into());
    }
    let matches: Vec<_> = search["items"]
        .as_array()
        .ok_or("missing search results")?
        .iter()
        .filter(|pr| matches_version(pr["title"].as_str().unwrap_or(""), package, version))
        .collect();
    if matches.len() > 1 {
        return Err("multiple open PRs for this package version, inspect before retrying".into());
    }
    matches
        .first()
        .map(|pr| {
            pr["number"]
                .as_u64()
                .ok_or_else(|| "missing PR number".into())
        })
        .transpose()
}

fn complete_body(
    body: &str,
    package: &str,
    version: &str,
    schema: &str,
    evidence: &str,
    cla: bool,
) -> Result<String> {
    if !body.contains("## 📖 Description") || !body.contains("## 📦 Manifest Checklist") {
        return Err(
            "unrecognized upstream template, review the prepared body before editing".into(),
        );
    }
    let mut lines = Vec::new();
    let mut in_description = false;
    let mut description_empty = true;
    for line in body.lines() {
        if [
            "<!-- PR Title Format",
            "<!-- Describe",
            "<!-- Place an",
            "<!-- Example:",
        ]
        .iter()
        .any(|prefix| line.trim_start().starts_with(prefix))
            || line.contains("Resolves #[Issue Number]")
        {
            continue;
        }
        if line == "## 📖 Description" {
            in_description = true;
        } else if line.starts_with("## ") {
            in_description = false;
        } else if in_description
            && !line.trim().is_empty()
            && !line.trim_start().starts_with("<!--")
        {
            description_empty = false;
        }
        let demonstrated = line.contains("Checked that there aren't other open")
            || line.contains("This PR only modifies one (1) manifest")
            || line.contains("Validated manifest locally with `winget validate")
            || line.contains("Tested manifest locally with `winget install")
            || (cla && line.contains("Signed the [Contributor License Agreement]"))
            || (line.contains("Manifest conforms to the")
                && line.contains(&format!(
                    "[{schema_trim} schema]",
                    schema_trim = schema.strip_suffix(".0").unwrap_or(schema)
                )));
        let line = if demonstrated {
            line.replacen("- [ ]", "- [x]", 1)
        } else {
            line.to_owned()
        };
        lines.push(line);
    }
    let mut result = lines.join("\n");
    if description_empty {
        let description = format!(
            "## 📖 Description\n\nAdd {package} {version} from its published release.\n\nWindows manifest validation and installation passed in [the distribution check]({evidence}).\n\nManifestVersion: {schema}. A checklist item naming a different schema remains unchecked.\n"
        );
        result = result.replacen("## 📖 Description", &description, 1);
    }
    let has_issue = result
        .split('#')
        .skip(1)
        .any(|part| part.starts_with(|c: char| c.is_ascii_digit()))
        || result.contains("/issues/");
    if has_issue {
        result = result.replace(". No linked issue for this package submission.", "");
    }
    if result.contains("- [ ] Linked to an issue (if applicable)")
        && !result.contains("No linked issue")
        && !has_issue
    {
        result = result.replacen("- [ ] Linked to an issue (if applicable)", "- [ ] Linked to an issue (if applicable). No linked issue for this package submission.", 1);
    }
    Ok(format!("{}\n", result.trim_end()))
}

pub fn run(args: &[String]) -> Result<()> {
    let [directory, package, version, mode] = args else {
        return Err("usage: winget-submit DIRECTORY PACKAGE VERSION (--prepare|--submit)".into());
    };
    if !matches!(mode.as_str(), "--prepare" | "--submit")
        || !package
            .split('.')
            .all(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
        || version.split('.').count() != 3
        || !version
            .split('.')
            .all(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit()))
    {
        return Err("expected a package identifier and stable numeric version".into());
    }
    let (schema, files) = manifests(Path::new(directory), package, version)?;
    if std::env::var("WINGET_CHECKED").as_deref() != Ok("true") {
        return Err("Windows manifest checks have not passed".into());
    }
    let prefix = format!(
        "manifests/{}/{}/{}",
        package[..1].to_ascii_lowercase(),
        package.replace('.', "/"),
        version
    );
    // A failed API read must never be interpreted as permission to create a duplicate.
    match api(&format!("repos/{REPO}/contents/{prefix}")) {
        Ok(_) => {
            println!("WinGet already contains {package} {version}.");
            return Ok(());
        }
        Err(e) if e.to_string().contains("HTTP 404") => {}
        Err(e) => return Err(e),
    }
    let mut number = find_pr(package, version)?;
    if number.is_none() && mode == "--submit" {
        if schema != "1.28.0" {
            return Err("new submissions require the repository's current 1.28 schema; regenerate the manifests before submitting".into());
        }
        if std::env::var("WINGET_CHECKED").as_deref() != Ok("true") {
            return Err("Windows manifest checks have not passed".into());
        }
        let client = std::env::var("WINGET_SUBMIT")
            .map_err(|_| "WINGET_SUBMIT must name the verified Komac executable")?;
        let status = Command::new(client)
            .args(["submit", directory, "--yes"])
            .env(
                "GITHUB_TOKEN",
                std::env::var("GH_TOKEN").map_err(|_| "GH_TOKEN is required for submission")?,
            )
            .status()?;
        // Even if the client fails after creating the PR, a retry will find it.
        if !status.success() {
            return Err(
                "Komac submission failed, retry this version after inspecting its result".into(),
            );
        }
        for _ in 0..12 {
            number = find_pr(package, version)?;
            if number.is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_secs(5));
        }
    }
    let evidence = match (
        std::env::var("GITHUB_REPOSITORY"),
        std::env::var("GITHUB_RUN_ID"),
    ) {
        (Ok(repo), Ok(run)) => format!("https://github.com/{repo}/actions/runs/{run}"),
        _ => {
            return Err(
                "GITHUB_REPOSITORY and GITHUB_RUN_ID are required for check evidence".into(),
            );
        }
    };
    let Some(number) = number else {
        if mode == "--submit" {
            return Err("submitted PR is not searchable yet, retry the same version".into());
        }
        let template = gh(
            &[
                "api",
                &format!("repos/{REPO}/contents/.github/PULL_REQUEST_TEMPLATE.md"),
                "-H",
                "Accept: application/vnd.github.raw+json",
            ],
            None,
        )?;
        fs::write(
            "winget-pr-body.md",
            complete_body(&template, package, version, &schema, &evidence, false)?,
        )?;
        println!("Prepared winget-pr-body.md. No upstream writes requested.");
        return Ok(());
    };
    let endpoint = format!("repos/{REPO}/pulls/{number}");
    let pr = api(&endpoint)?;
    if mode == "--submit" {
        let login = api("user")?["login"]
            .as_str()
            .ok_or("missing authenticated login")?
            .to_owned();
        if pr["user"]["login"] != login {
            return Err("existing PR belongs to another contributor, leave it for review".into());
        }
    }
    let sha = pr["head"]["sha"].as_str().ok_or("missing PR head")?;
    let changed = api(&format!("{endpoint}/files?per_page=100"))?;
    let changed = changed.as_array().ok_or("missing changed files")?;
    if changed.len() != files.len()
        || changed.iter().any(|f| {
            !f["filename"]
                .as_str()
                .unwrap_or("")
                .starts_with(&format!("{prefix}/"))
        })
    {
        return Err("existing PR modifies a different manifest, inspect before editing".into());
    }
    for (name, expected) in &files {
        let actual = gh(
            &[
                "api",
                &format!("repos/{REPO}/contents/{prefix}/{name}?ref={sha}"),
                "-H",
                "Accept: application/vnd.github.raw+json",
            ],
            None,
        )?;
        if !same_manifest(expected, &actual)? {
            return Err("existing PR manifest content differs from the checked release".into());
        }
        let kind = field(expected, "ManifestType").ok_or("missing manifest type")?;
        let header = format!(
            "# yaml-language-server: $schema=https://aka.ms/winget-manifest.{kind}.{schema}.schema.json"
        );
        if !actual.lines().any(|line| line == header) {
            return Err("submitted manifest is missing its matching schema header".into());
        }
    }
    let mut cla = false;
    for attempt in 0..12 {
        let checks = gh(
            &[
                "api",
                "--paginate",
                "--slurp",
                &format!("repos/{REPO}/commits/{sha}/check-runs?per_page=100"),
            ],
            None,
        )?;
        let checks: Vec<Value> = serde_json::from_str(&checks)?;
        let latest = checks
            .iter()
            .filter_map(|page| page["check_runs"].as_array())
            .flatten()
            .filter(|c| {
                c["name"] == "license/cla" && c["app"]["slug"] == "microsoft-github-policy-service"
            })
            .max_by_key(|c| c["id"].as_u64());
        cla = latest.is_some_and(|c| c["conclusion"] == "success");
        if cla || mode == "--prepare" || attempt == 11 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_secs(5));
    }
    let body = pr["body"].as_str().ok_or("missing PR body")?;
    // Komac creates its own short introduction. Replace only that recognized
    // generated line with the upstream template, retaining any appended notes.
    let seeded = if body.starts_with(
        "### Pull request has been created with [komac](https://github.com/russellbanks/Komac)",
    ) {
        let template = gh(
            &[
                "api",
                &format!("repos/{REPO}/contents/.github/PULL_REQUEST_TEMPLATE.md"),
                "-H",
                "Accept: application/vnd.github.raw+json",
            ],
            None,
        )?;
        seed_body(body, &template)?
    } else {
        body.to_owned()
    };
    let completed = complete_body(&seeded, package, version, &schema, &evidence, cla)?;
    fs::write("winget-pr-body.md", &completed)?;
    if mode == "--submit" && completed.trim_end() != body.trim_end() {
        if std::env::var("WINGET_CHECKED").as_deref() != Ok("true") {
            return Err("Windows manifest checks have not passed".into());
        }
        let current = api(&endpoint)?;
        if current["body"] != pr["body"]
            || current["head"]["sha"] != sha
            || current["state"] != "open"
        {
            return Err("PR changed during preparation, retry to preserve the latest edits".into());
        }
        gh(
            &["api", "--method", "PATCH", &endpoint, "--input", "-"],
            Some(&json!({"body": completed})),
        )?;
        if api(&endpoint)?["body"] != completed {
            return Err("public description readback differs, inspect before retrying".into());
        }
    }
    println!("https://github.com/{REPO}/pull/{number} (CLA check accepted: {cla}; mode: {mode})");
    if !cla {
        println!(
            "CLA checkbox left unchanged. Inspect the official license/cla check; no agreement was signed."
        );
        if mode == "--submit" {
            return Err("CLA verification is pending or unsuccessful; retry the same version after its official check passes".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn komac_template_seeding_preserves_same_line_and_later_edits() {
        let body = "### Pull request has been created with [komac](https://github.com/russellbanks/Komac) v2.17.2 :rocket: Same-line note.\n\nLater note.\n";
        let seeded = seed_body(body, "## 📖 Description\n## 📦 Manifest Checklist").unwrap();
        assert!(seeded.contains("Same-line note."));
        assert!(seeded.contains("Later note."));
        assert!(!seeded.contains("Pull request has been created"));
        assert_eq!(seed_body(&seeded, "replacement").unwrap(), seeded);
        assert!(seed_body(&body.replace("v2.17.2", "custom-version"), "template").is_err());
    }
    #[test]
    fn comparison_accepts_wingetcreate_formatting_but_rejects_changed_content() {
        let original = "PackageIdentifier: Publisher.Tool\nPackageVersion: 1.2.3\nInstallerType: portable\nInstallers:\n- Architecture: x64\n  InstallerSha256: ABCDEF123\n";
        let serialized = "# Created using wingetcreate 1.12.13.0\r\n\r\nPackageVersion: 1.2.3\r\nPackageIdentifier: Publisher.Tool\r\nInstallers:\r\n- InstallerSha256: ABCDEF123\r\n  Architecture: x64\r\nInstallerType: portable\r\n";
        assert!(same_manifest(original, serialized).unwrap());
        assert!(!same_manifest(original, &serialized.replace("ABCDEF123", "DEADBEEF")).unwrap());
    }
    const BODY: &str = "<!-- PR Title Format -->\n## 📖 Description\n<!-- Describe -->\n\n## ✅ Checklist\n- [ ] Signed the [Contributor License Agreement](https://cla.opensource.microsoft.com)\n- [ ] Linked to an issue (if applicable)\n  - Resolves #[Issue Number]\n## 📦 Manifest Checklist\n- [ ] Checked that there aren't other open pull requests\n- [ ] This PR only modifies one (1) manifest\n- [ ] Validated manifest locally with `winget validate --manifest <path>`\n- [ ] Tested manifest locally with `winget install --manifest <path>`\n- [ ] Manifest conforms to the [1.28 schema](url)\n";
    #[test]
    fn completion_is_truthful_and_retry_preserves_manual_edits() {
        let completed = complete_body(
            BODY,
            "Publisher.Tool",
            "1.2.3",
            "1.12.0",
            "https://example.com/run",
            false,
        )
        .unwrap();
        assert!(!completed.contains("[Issue Number]"));
        assert!(completed.contains("- [ ] Signed"));
        assert!(completed.contains("- [ ] Manifest conforms"));
        assert!(completed.contains("- [x] Tested manifest"));
        let edited = completed.replace(
            "Add Publisher.Tool",
            "Maintainer description: Publisher.Tool",
        ) + "\nMaintainer note.\n";
        let retry = complete_body(
            &edited,
            "Publisher.Tool",
            "1.2.3",
            "1.12.0",
            "https://example.com/retry",
            true,
        )
        .unwrap();
        assert!(retry.contains("Maintainer description"));
        assert!(retry.contains("Maintainer note."));
        assert!(retry.contains("- [x] Signed"));
        assert_eq!(
            retry,
            complete_body(
                &retry,
                "Publisher.Tool",
                "1.2.3",
                "1.12.0",
                "different",
                true
            )
            .unwrap()
        );
    }
    #[test]
    fn schema_check_requires_actual_schema_and_unknown_templates_fail() {
        assert!(
            complete_body(BODY, "Publisher.Tool", "1.2.3", "1.28.0", "url", true)
                .unwrap()
                .contains("- [x] Manifest conforms")
        );
        assert!(
            complete_body(
                "custom body",
                "Publisher.Tool",
                "1.2.3",
                "1.28.0",
                "url",
                true
            )
            .is_err()
        );
    }
    #[test]
    fn preserves_existing_issue_references_and_manual_comments() {
        let first = complete_body(BODY, "Publisher.Tool", "1.2.3", "1.28.0", "url", true).unwrap();
        assert!(first.contains("No linked issue"));
        let body = format!("{first}\nResolves #123\n<!-- Maintainer note -->\n");
        let completed =
            complete_body(&body, "Publisher.Tool", "1.2.3", "1.28.0", "url", true).unwrap();
        assert!(completed.contains("Resolves #123"));
        assert!(completed.contains("<!-- Maintainer note -->"));
        assert!(!completed.contains("No linked issue"));
    }
    #[test]
    fn duplicate_detection_matches_whole_package_and_version() {
        assert!(matches_version(
            "New package: Publisher.Tool version v1.2.3",
            "Publisher.Tool",
            "1.2.3"
        ));
        assert!(!matches_version(
            "Publisher.Tool.Extra version 1.2.3",
            "Publisher.Tool",
            "1.2.3"
        ));
        assert!(!matches_version(
            "Publisher.Tool version 1.2.30",
            "Publisher.Tool",
            "1.2.3"
        ));
        assert!(!matches_version(
            "Publisher.Tool version 1.2.3-beta",
            "Publisher.Tool",
            "1.2.3"
        ));
    }
}
