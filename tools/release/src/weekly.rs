//! Prepare versioned release metadata without changing the development branch.
use crate::common::{Result, repository};
use std::{fs, path::Path, process::Command};

// Git may check out tracked metadata with CR LF on Windows. Parse its logical
// lines consistently and write generated metadata with LF on every platform.
fn read_metadata(path: impl AsRef<Path>) -> Result<String> {
    Ok(fs::read_to_string(path)?.replace("\r\n", "\n"))
}

fn git(root: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git").current_dir(root).args(args).output()?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into_owned().into());
    }
    Ok(String::from_utf8(output.stdout)?)
}

fn version(text: &str) -> Result<[u64; 3]> {
    let parts: Vec<_> = text.trim_start_matches('v').split('.').collect();
    let [a, b, c] = parts.as_slice() else {
        return Err("expected a stable x.y.z version".into());
    };
    Ok([a.parse()?, b.parse()?, c.parse()?])
}

fn meaningful(path: &str) -> bool {
    !path.ends_with(".md")
        && !path.starts_with("docs/")
        && !path.starts_with("site/")
        && !path.starts_with("tools/site/")
        && !path.starts_with(".github/ISSUE_TEMPLATE/")
        && !matches!(
            path,
            "book.toml"
                | ".github/FUNDING.yml"
                | ".github/workflows/docs.yml"
                | ".github/workflows/website-deploy.yml"
        )
}

fn bump(mut old: [u64; 3], breaking: bool) -> [u64; 3] {
    if breaking {
        if old[0] == 0 {
            old[1] += 1;
        } else {
            old[0] += 1;
            old[1] = 0;
        }
        old[2] = 0;
    } else {
        old[2] += 1;
    }
    old
}

/// PREVIOUS_TAG DATE (auto|minor|major) OUTPUT. The output uses Actions key=value syntax.
pub fn prepare(args: &[String]) -> Result<()> {
    let [previous, date, mode, output, rest @ ..] = args else {
        return Err("usage: ripscope-release prepare-weekly PREVIOUS_TAG YYYY-MM-DD (auto|minor|major) OUTPUT [MINIMUM_TAG]".into());
    };
    let root = repository()?;
    if rest.len() > 1 {
        return Err("expected at most one minimum version tag".into());
    }
    prepare_at(
        &root,
        previous,
        date,
        mode,
        Path::new(output),
        rest.first().map(String::as_str),
    )
}

fn prepare_at(
    root: &Path,
    previous: &str,
    date: &str,
    mode: &str,
    output: &Path,
    minimum: Option<&str>,
) -> Result<()> {
    let old = version(previous)?;
    if date.len() != 10
        || !date.chars().enumerate().all(|(i, c)| {
            if i == 4 || i == 7 {
                c == '-'
            } else {
                c.is_ascii_digit()
            }
        })
    {
        return Err("expected release date YYYY-MM-DD".into());
    }
    if !["auto", "minor", "major"].contains(&mode) {
        return Err("invalid version increment".into());
    }
    if !git(root, &["status", "--porcelain"])?.is_empty() {
        return Err("release preparation needs a clean checkout".into());
    }
    // Generated tags are siblings of later main revisions. Their parent is the exact
    // previous development snapshot, so ancestry and change detection use that parent.
    let subject = git(root, &["show", "-s", "--format=%s", previous])?;
    let base = if subject.starts_with("Prepare automated release v") {
        format!("{previous}^")
    } else {
        previous.to_owned()
    };
    git(root, &["merge-base", "--is-ancestor", &base, "HEAD"])?;
    let range = format!("{base}..HEAD");
    let paths = git(root, &["diff", "--name-only", &base, "HEAD"])?;
    if !paths.lines().any(meaningful) {
        fs::write(output, "changed=false\n")?;
        return Ok(());
    }
    let log = git(
        root,
        &["log", "--no-merges", "--format=%s%x00%b%x00%h%x00", &range],
    )?;
    let records: Vec<_> = log.split('\0').collect();
    let mut groups = [Vec::new(), Vec::new(), Vec::new(), Vec::new()];
    let mut breaking = false;
    let mut feature = false;
    for record in records.chunks(3) {
        let [subject, body, hash] = record else {
            continue;
        };
        let subject = subject.trim();
        if subject.is_empty() {
            continue;
        }
        let is_breaking = subject
            .split_once(':')
            .is_some_and(|(prefix, _)| prefix.ends_with('!'))
            || body.contains("BREAKING CHANGE:")
            || body.contains("BREAKING-CHANGE:");
        breaking |= is_breaking;
        feature |= subject.starts_with("feat");
        let group = if is_breaking {
            0
        } else if subject.starts_with("feat") {
            1
        } else if subject.starts_with("fix") {
            2
        } else {
            3
        };
        groups[group].push(format!(
            "- {subject} ([{}](https://github.com/pederbe/ripscope/commit/{}))\n",
            hash.trim(),
            hash.trim()
        ));
    }
    let mut next = bump(old, breaking);
    if mode == "minor" || (feature && old[0] > 0) {
        next = next.max([old[0], old[1] + 1, 0]);
    }
    if mode == "major" {
        next = [old[0] + 1, 0, 0];
    }
    let manifest = read_metadata(root.join("Cargo.toml"))?;
    let current = crate::common::crate_version(&root.join("Cargo.toml"))?;
    // A deliberately prepared newer version on main takes precedence.
    next = next.max(version(&current)?);
    if let Some(minimum) = minimum {
        let floor = version(minimum)?;
        if next <= floor {
            next = bump(floor, false);
        }
    }
    let next = format!("{}.{}.{}", next[0], next[1], next[2]);
    let manifest = manifest.replacen(
        &format!("version = \"{current}\""),
        &format!("version = \"{next}\""),
        1,
    );
    let lock = read_metadata(root.join("Cargo.lock"))?;
    let lock = lock.replace(
        &format!("name = \"ripscope\"\nversion = \"{current}\""),
        &format!("name = \"ripscope\"\nversion = \"{next}\""),
    );
    let changelog = read_metadata(root.join("CHANGELOG.md"))?;
    let (intro, rest) = changelog
        .split_once("## [Unreleased]\n")
        .ok_or("missing Unreleased changelog heading")?;
    let (authored, _) = rest
        .split_once("\n## [")
        .ok_or("missing previous changelog section")?;
    let previous_changelog = git(root, &["show", &format!("{previous}:CHANGELOG.md")])?;
    let previous_rest = previous_changelog
        .split_once("## [Unreleased]\n")
        .ok_or("previous release lacks Unreleased heading")?
        .1;
    let history = previous_rest
        .split_once("\n## [")
        .ok_or("previous release lacks version history")?
        .1;
    // Compare authored entries against the previous development snapshot, not
    // the finalized tag. Main retains those entries after publication.
    let prior_source_changelog = git(root, &["show", &format!("{base}:CHANGELOG.md")])?;
    let prior_authored = prior_source_changelog
        .split_once("## [Unreleased]\n")
        .and_then(|(_, rest)| rest.split_once("\n## ["))
        .map_or("", |(authored, _)| authored);
    let mut notes = authored_delta(authored, prior_authored);
    notes.push_str("\n\n");
    for (heading, entries) in ["Breaking changes", "Added", "Fixed", "Other changes"]
        .into_iter()
        .zip(groups)
    {
        if !entries.is_empty() {
            notes.push_str(&format!("### {heading}\n\n{}\n", entries.concat()));
        }
    }
    notes.push_str(&format!(
        "[Full comparison](https://github.com/pederbe/ripscope/compare/{previous}...v{next})\n"
    ));
    let changelog = format!(
        "{intro}## [Unreleased]\n\n## [{next}] - {date}\n\n{}\n\n## [{history}",
        notes.trim()
    );
    let emacs_path = root.join("contrib/emacs/ripscope.el");
    let emacs = read_metadata(&emacs_path)?;
    let emacs = emacs
        .lines()
        .map(|line| {
            if line.starts_with(";; Version:") {
                format!(";; Version: {next}")
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    fs::write(emacs_path, emacs)?;
    fs::write(root.join("Cargo.toml"), manifest)?;
    fs::write(root.join("Cargo.lock"), lock)?;
    let changelog = changelog
        .lines()
        .map(|line| {
            if line.starts_with("[Unreleased]:") {
                format!("[Unreleased]: https://github.com/pederbe/ripscope/compare/v{next}...HEAD")
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    fs::write(root.join("CHANGELOG.md"), changelog)?;
    fs::write(output, format!("changed=true\ntag=v{next}\n"))?;
    Ok(())
}

// Keep each bullet and its continuation lines together. Headings belong to the
// records below them, so adding one entry does not repeat earlier prose.
fn authored_records(text: &str) -> Vec<(String, String)> {
    let mut records = Vec::new();
    let mut heading = String::new();
    let mut entry = String::new();
    for line in text.lines().chain(std::iter::once("### ")) {
        if line.starts_with("### ") || line.starts_with("- ") || line.is_empty() {
            if !entry.trim().is_empty() {
                records.push((heading.clone(), entry.trim().to_string()));
                entry.clear();
            }
            if line.starts_with("### ") {
                heading = line.to_string();
                continue;
            }
        }
        if !line.is_empty() {
            entry.push_str(line);
            entry.push('\n');
        }
    }
    records
}

fn authored_delta(current: &str, previous: &str) -> String {
    let old = authored_records(previous);
    let mut output = String::new();
    let mut last_heading = String::new();
    for (heading, entry) in authored_records(current) {
        if old.contains(&(heading.clone(), entry.clone())) {
            continue;
        }
        if heading != last_heading && !heading.is_empty() {
            output.push_str(&format!("{heading}\n\n"));
            last_heading = heading;
        }
        output.push_str(&entry);
        output.push_str("\n\n");
    }
    output.trim().to_string()
}

pub fn notes(args: &[String]) -> Result<()> {
    let [tag, output] = args else {
        return Err("usage: ripscope-release release-notes TAG OUTPUT".into());
    };
    let root = repository()?;
    let changelog = read_metadata(root.join("CHANGELOG.md"))?;
    let version = tag.strip_prefix('v').ok_or("expected version tag")?;
    let heading = format!("## [{version}] - ");
    let section = changelog
        .split_once(&heading)
        .ok_or("missing dated release changelog section")?
        .1;
    let (_, section) = section.split_once('\n').ok_or("empty changelog section")?;
    let notes = section.split("\n## [").next().unwrap_or("").trim();
    if notes.is_empty() {
        return Err("empty release notes".into());
    }
    fs::write(output, format!("{notes}\n"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn appending_authored_notes_keeps_only_new_complete_entries() {
        let old = "### Fixed\n\n- First entry\n  continuation\n";
        let new = format!("{old}- Second entry\n  continuation\n");
        assert_eq!(
            authored_delta(&new, old),
            "### Fixed\n\n- Second entry\n  continuation"
        );
        assert_eq!(authored_delta(old, old), "");
        assert!(
            authored_delta("### Fixed\n\n- Revised first entry", old)
                .contains("Revised first entry")
        );
    }

    #[test]
    fn stable_version_and_breaking_bumps() {
        assert_eq!(bump(version("v0.2.9").unwrap(), true), [0, 3, 0]);
        assert_eq!(bump([1, 2, 9], true), [2, 0, 0]);
        assert_eq!(bump([0, 2, 9], false), [0, 2, 10]);
        assert!(version("1.2.3-beta").is_err());
    }
    #[test]
    fn docs_do_not_release_but_packaging_and_dependencies_do() {
        for path in [
            "README.md",
            "docs/a.html",
            "site/app.js",
            ".github/workflows/docs.yml",
        ] {
            assert!(!meaningful(path));
        }
        for path in [
            "Cargo.lock",
            "plugin/ripscope.vim",
            "install.sh",
            "tools/release/src/main.rs",
            ".github/workflows/release.yml",
        ] {
            assert!(meaningful(path));
        }
    }
    #[test]
    fn prepares_metadata_from_real_git_history_and_skips_generated_version_delta() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("contrib/emacs")).unwrap();
        fs::write(
            root.join("contrib/emacs/ripscope.el"),
            ";; Version: 0.2.0\n",
        )
        .unwrap();
        git(root, &["init"]).unwrap();
        git(root, &["config", "user.email", "test@example.com"]).unwrap();
        git(root, &["config", "user.name", "Test"]).unwrap();
        git(root, &["config", "core.autocrlf", "true"]).unwrap();
        fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"ripscope\"\nversion = \"0.2.0\"\n",
        )
        .unwrap();
        fs::write(
            root.join("Cargo.lock"),
            "[[package]]\nname = \"ripscope\"\nversion = \"0.2.0\"\n",
        )
        .unwrap();
        fs::write(
            root.join("CHANGELOG.md"),
            "# Changelog\n\n## [Unreleased]\n\n## [0.2.0] - 2026-10-20\n\nOld\n",
        )
        .unwrap();
        git(root, &["add", "."]).unwrap();
        git(root, &["commit", "-m", "Initial"]).unwrap();
        git(root, &["tag", "v0.2.0"]).unwrap();
        fs::write(root.join("README.md"), "Docs").unwrap();
        git(root, &["add", "."]).unwrap();
        git(root, &["commit", "-m", "Docs"]).unwrap();
        let output = dir
            .path()
            .parent()
            .unwrap()
            .join(format!("release-{}.out", std::process::id()));
        prepare_at(root, "v0.2.0", "2026-10-26", "auto", &output, None).unwrap();
        assert_eq!(fs::read_to_string(&output).unwrap(), "changed=false\n");
        fs::write(root.join("install.sh"), "true").unwrap();
        git(root, &["add", "."]).unwrap();
        git(root, &["commit", "-m", "fix!: Change installer contract"]).unwrap();
        let source = git(root, &["rev-parse", "HEAD"]).unwrap();
        prepare_at(root, "v0.2.0", "2026-10-26", "auto", &output, None).unwrap();
        assert!(fs::read_to_string(&output).unwrap().contains("tag=v0.3.0"));
        assert!(
            fs::read_to_string(root.join("CHANGELOG.md"))
                .unwrap()
                .contains("### Breaking changes")
        );
        git(root, &["add", "."]).unwrap();
        git(root, &["commit", "-m", "Prepare automated release v0.3.0"]).unwrap();
        git(root, &["tag", "v0.3.0"]).unwrap();
        git(root, &["checkout", "--detach", source.trim()]).unwrap();
        prepare_at(root, "v0.3.0", "2026-11-02", "auto", &output, None).unwrap();
        assert_eq!(fs::read_to_string(&output).unwrap(), "changed=false\n");
        fs::write(root.join("install.sh"), "echo revised").unwrap();
        git(root, &["add", "."]).unwrap();
        git(
            root,
            &["commit", "-m", "fix: Preserve later release history"],
        )
        .unwrap();
        prepare_at(root, "v0.3.0", "2026-11-02", "auto", &output, None).unwrap();
        let changelog = fs::read_to_string(root.join("CHANGELOG.md")).unwrap();
        assert!(changelog.contains("## [0.3.1]"));
        assert!(changelog.contains("## [0.3.0]"));
        assert!(changelog.contains("## [0.2.0]"));
        assert_eq!(changelog.matches("Change installer contract").count(), 1);
        assert_eq!(
            fs::read_to_string(root.join("contrib/emacs/ripscope.el")).unwrap(),
            ";; Version: 0.3.1\n"
        );
        git(root, &["reset", "--hard", "HEAD"]).unwrap();
        prepare_at(
            root,
            "v0.3.0",
            "2026-11-02",
            "auto",
            &output,
            Some("v0.3.1"),
        )
        .unwrap();
        assert!(fs::read_to_string(&output).unwrap().contains("tag=v0.3.2"));
        fs::remove_file(output).unwrap();
    }
}
