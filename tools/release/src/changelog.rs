//! Validate git-cliff's parsed inputs and the coverage of its rendered notes.
use crate::common::{Result, repository};
use regex::Regex;
use serde_json::Value;
use std::{collections::BTreeSet, fs, path::Path, process::Command};

const CATEGORIES: &[&str] = &[
    "Added",
    "Changed",
    "Fixed",
    "Removed",
    "Deprecated",
    "Security",
    "Performance",
    "Dependencies",
    "Documentation",
    "Other changes",
];

#[derive(Debug)]
struct Note {
    category: String,
    text: String,
    breaking: bool,
}

fn note(commit: &Value) -> Result<Note> {
    let category = commit["group"]
        .as_str()
        .ok_or("missing changelog group")?
        .rsplit("-->")
        .next()
        .unwrap_or("")
        .trim();
    if !CATEGORIES.contains(&category) {
        return Err(format!("unknown changelog group: {category}").into());
    }
    let title = commit["message"]
        .as_str()
        .unwrap_or("")
        .lines()
        .next()
        .unwrap_or("")
        .trim();
    let title = Regex::new(r" [(]#[0-9]+[)]$")?.replace(title, "");
    let mut chars = title.chars();
    let first = chars.next().ok_or("empty changelog title")?;
    Ok(Note {
        category: category.into(),
        text: first.to_uppercase().collect::<String>() + chars.as_str(),
        breaking: commit["breaking"] == true,
    })
}

fn context(root: &Path, range: &str, extra: Option<&str>) -> Result<Vec<Value>> {
    let mut command = Command::new("git-cliff");
    command
        .current_dir(root)
        .args(["--config", "cliff.toml", "--context", "--no-exec", range]);
    if let Some(message) = extra {
        command.args(["--with-commit", message]);
    }
    let output = command.output()?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into_owned().into());
    }
    let releases: Vec<Value> = serde_json::from_slice(&output.stdout)?;
    Ok(releases
        .into_iter()
        .flat_map(|r| r["commits"].as_array().cloned().unwrap_or_default())
        .collect())
}

fn words(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn check_section(section: &str, commits: &[Value]) -> Result<()> {
    let mut headings = BTreeSet::new();
    for line in section.lines().filter_map(|line| line.strip_prefix("### ")) {
        if !CATEGORIES.contains(&line) || !headings.insert(line) {
            return Err(format!("unknown or repeated changelog heading: {line}").into());
        }
    }
    let section_words = words(section);
    let reference = Regex::new(r"\(#([0-9]+)\)$")?;
    for commit in commits {
        let message = commit["raw_message"].as_str().unwrap_or("commit");
        let note = note(commit)
            .map_err(|e| format!("{}: {e}", message.lines().next().unwrap_or("commit")))?;
        if !headings.contains(note.category.as_str()) {
            return Err(format!("missing {} section", note.category).into());
        }
        let marker = if note.breaking { "**Breaking:** " } else { "" };
        if !section_words.contains(&format!("- {marker}{}", words(&note.text))) {
            return Err(format!("missing short entry or breaking marker: {}", note.text).into());
        }
        if let Some(captures) = reference.captures(message.lines().next().unwrap_or("")) {
            let link = format!("https://github.com/pederbe/ripscope/pull/{}", &captures[1]);
            if !section.contains(&link) {
                return Err(format!("missing source link: {link}").into());
            }
        }
    }
    if commits.is_empty() || !section.contains("- ") {
        return Err("release notes contain no user-facing changes".into());
    }
    Ok(())
}

pub fn check_release(args: &[String]) -> Result<()> {
    let [previous] = args else {
        return Err("usage: ripscope-release check-release-notes PREVIOUS_TAG".into());
    };
    let root = repository()?;
    let version = crate::common::crate_version(&root.join("Cargo.toml"))?;
    let changelog = fs::read_to_string(root.join("CHANGELOG.md"))?.replace("\r\n", "\n");
    let rest = changelog
        .split_once(&format!("## [{version}] - "))
        .ok_or("missing generated release section")?
        .1;
    let section = rest
        .split_once('\n')
        .ok_or("empty generated release section")?
        .1
        .split("\n## [")
        .next()
        .unwrap_or("");
    check_section(
        section,
        &context(&root, &format!("{previous}..HEAD"), None)?,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_or_repeated_generated_sections_fail() {
        assert!(check_section("### Changed\n\n- A note\n\n### Changed\n", &[]).is_err());
        assert!(check_section("", &[]).is_err());
    }

    #[test]
    #[ignore = "requires the pinned git-cliff CLI, run by the changelog CI job"]
    fn parsed_inputs_and_rendered_notes_cover_current_and_future_releases() {
        let root = tempfile::tempdir().unwrap();
        let config_path = repository().unwrap().join("cliff.toml");
        fs::copy(&config_path, root.path().join("cliff.toml")).unwrap();
        let git = |args: &[&str]| {
            let output = Command::new("git")
                .current_dir(root.path())
                .args(args)
                .output()
                .unwrap();
            assert!(output.status.success(), "{args:?}: {output:?}");
        };
        git(&["init", "--quiet"]);
        git(&["branch", "-M", "main"]);
        git(&["config", "user.name", "Fixture"]);
        git(&["config", "user.email", "fixture@example.invalid"]);
        git(&["commit", "--allow-empty", "-qm", "chore: initial version"]);
        git(&["tag", "v0.2.7"]);
        // Titles alone supply the entry. Bodies and legacy footers stay out.
        for message in [
            "fix: preserve a short entry",
            "fix!: replace old configuration",
            "fix: preserve a short entry\n\n## Solution\n\nPrivate detail stays out.",
            "fix(deps)!: replace the parser\n\nBREAKING CHANGE: Full upgrade steps.",
        ] {
            let commits = context(root.path(), "HEAD..HEAD", Some(message)).unwrap();
            assert!(!commits.is_empty(), "{message}");
            assert!(commits.iter().all(|c| note(c).is_ok()), "{commits:?}");
        }
        let current = [
            "Cross-build Linux releases with Zig while preserving glibc 2.28 (#132)",
            "Leave editor version numbers out of the editor guides (#125)",
            "ci!: manage weekly releases with release-plz (#135)",
        ];
        for message in current {
            git(&["commit", "--allow-empty", "-qm", message]);
        }
        let render = |previous: &str, next: &str| {
            let output = Command::new("git-cliff")
                .current_dir(root.path())
                .args([
                    "--config",
                    "cliff.toml",
                    "--tag",
                    next,
                    "--tag-pattern",
                    &format!("^({}|{})$", regex::escape(previous), regex::escape(next)),
                    &format!("{previous}..HEAD"),
                ])
                .output()
                .unwrap();
            assert!(output.status.success(), "{output:?}");
            String::from_utf8(output.stdout).unwrap()
        };
        git(&["checkout", "-qb", "release-candidate"]);
        git(&[
            "commit",
            "--allow-empty",
            "-qm",
            "chore: candidate metadata",
        ]);
        git(&["checkout", "-q", "main"]);
        git(&["commit", "--allow-empty", "-qm", "ci: base branch metadata"]);
        git(&[
            "merge",
            "--no-ff",
            "-qm",
            "Merge 0123456789 into abcdef0123",
            "release-candidate",
        ]);
        let first = render("v0.2.7", "v0.3.0");
        check_section(&first, &context(root.path(), "v0.2.7..HEAD", None).unwrap()).unwrap();
        assert_eq!(first.matches("### Fixed").count(), 1);
        assert!(!first.contains("manage weekly releases"));
        git(&["commit", "--allow-empty", "-qm", "chore: release v0.3.0"]);
        git(&["tag", "v0.3.0"]);
        for message in [
            "fix: keep multiline instructions (#900)\n\nChangelog: Fixed: Keep the first paragraph.\n\nKeep the second paragraph, including its explanation: nothing is truncated.",
            "docs: explain editor recovery (#901)\n\nChangelog: Documentation: Explain how to recover a disconnected editor session.",
            "ci: change the build cache (#902)",
            "fix: handle lookups (#42) after a file move (#908)",
            "fix(deps): update the parser (#904)\n\nChangelog: Dependencies: Update the parser dependency and keep non-ASCII names intact.",
            "fix(deps)!: replace parser configuration (#905)\n\nChangelog: Dependencies: Upgrade the parser dependency and replace its obsolete configuration.\n\nBREAKING CHANGE: Rename parser_mode to parser-mode in saved configuration.",
            "fix(deps): update dependencies (#906)\n\nChangelog: Dependencies: Update runtime and build dependencies.",
            "fix(deps): update dependencies (#907)\n\nChangelog: Dependencies: Update runtime and build dependencies.",
        ] {
            git(&["commit", "--allow-empty", "-qm", message]);
        }
        // An unpublished failed tag must not split the next published notes.
        git(&["tag", "v0.3.1"]);
        git(&[
            "commit",
            "--allow-empty",
            "-qm",
            "ci: repair a failed build",
        ]);
        let bump = Command::new("git-cliff")
            .current_dir(root.path())
            .args([
                "--config",
                "cliff.toml",
                "--ignore-tags",
                "^v0\\.3\\.1$",
                "--bump",
                "--bumped-version",
                "v0.3.0..HEAD",
            ])
            .output()
            .unwrap();
        assert!(bump.status.success(), "{bump:?}");
        assert_eq!(String::from_utf8(bump.stdout).unwrap().trim(), "v0.4.0");
        let second = render("v0.3.0", "v0.4.0");
        let commits = context(root.path(), "v0.3.0..HEAD", None).unwrap();
        check_section(&second, &commits).unwrap();
        assert!(!second.contains("glibc"));
        assert!(!second.contains("build cache"));
        assert!(!second.contains("second paragraph"));
        assert_eq!(second.matches("### Fixed").count(), 1);
        assert_eq!(second.matches("- Update dependencies ").count(), 2);
        assert!(second.contains("Update the parser"));
        assert!(second.contains("Handle lookups (#42) after a file move ([#908](https://github.com/pederbe/ripscope/pull/908))"));
        assert!(!second.contains("Rename parser_mode"));
        assert!(second.contains("**Breaking:** Replace parser configuration"));
        assert!(check_section(&second.replace("**Breaking:** ", ""), &commits).is_err());
        assert!(
            check_section(&second.replace("Keep multiline instructions", ""), &commits).is_err()
        );
        assert!(check_section(&second.replace("/pull/904", "/pull/999"), &commits).is_err());
    }
}
