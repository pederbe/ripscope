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
];

#[derive(Debug)]
struct Note {
    category: String,
    text: String,
    breaking: Option<String>,
}

fn note(commit: &Value) -> Result<Note> {
    if commit["conventional"] != true {
        return Err(
            "use a conventional title and separate footers from the description with a blank line"
                .into(),
        );
    }
    let footers = commit["footers"]
        .as_array()
        .ok_or("missing conventional footers")?;
    let notes: Vec<_> = footers
        .iter()
        .filter(|f| f["token"] == "Changelog")
        .collect();
    if notes.len() != 1 {
        return Err("add one Changelog: CATEGORY: complete user-facing description footer".into());
    }
    let value = notes[0]["value"].as_str().ok_or("missing note text")?;
    let (category, text) = value
        .split_once(':')
        .ok_or("Changelog needs CATEGORY: description")?;
    let (category, text) = (category.trim(), text.trim());
    if !CATEGORIES.contains(&category)
        || text.is_empty()
        || text.contains("<!--")
        || text.contains("\n##")
    {
        return Err(
            "Changelog needs a supported category and prose, without template comments or headings"
                .into(),
        );
    }
    let breaking = if commit["breaking"] == true {
        Some(
            footers
                .iter()
                .find(|f| f["breaking"] == true)
                .and_then(|f| f["value"].as_str())
                .filter(|s| !s.trim().is_empty())
                .ok_or("a breaking change needs BREAKING CHANGE: upgrade instructions")?
                .to_owned(),
        )
    } else {
        None
    };
    Ok(Note {
        category: category.into(),
        text: text.into(),
        breaking,
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

pub fn check_pr(args: &[String]) -> Result<()> {
    let [file] = args else {
        return Err("usage: ripscope-release check-pr-note EVENT_JSON".into());
    };
    let event: Value = serde_json::from_slice(&fs::read(file)?)?;
    let pr = &event["pull_request"];
    if pr["head"]["ref"]
        .as_str()
        .is_some_and(|branch| branch.starts_with("release-plz-"))
    {
        // The release PR carries generated output, checked separately against
        // the complete source range by the required changelog CI job.
        return Ok(());
    }
    let title = pr["title"].as_str().ok_or("pull request title missing")?;
    let body = pr["body"].as_str().unwrap_or("");
    let source = pr["number"]
        .as_u64()
        .map_or(String::new(), |n| format!(" (#{n})"));
    let message = format!("{title}{source}\n\n{body}");
    let commits = context(&repository()?, "HEAD..HEAD", Some(&message))?;
    if commits.is_empty()
        && ["\nChangelog:", "\nBREAKING CHANGE:", "\nBREAKING-CHANGE:"]
            .iter()
            .any(|m| message.contains(m))
    {
        return Err("the recorded note did not parse as a supported conventional footer".into());
    }
    for commit in commits {
        note(&commit)?;
    }
    Ok(())
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
        if !section_words.contains(&words(&note.text)) {
            return Err(format!("missing changelog note: {}", note.text).into());
        }
        if let Some(breaking) = note.breaking
            && !section_words.contains(&words(&breaking))
        {
            return Err("missing breaking-change upgrade instructions".into());
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
        // Exercise the actual parser, including normal GitHub/Renovate footer forms.
        for (message, success) in [
            ("fix: missing note", false),
            (
                "fix: preserve note\n\nIntro\nChangelog: Fixed: Required detail.",
                false,
            ),
            (
                "fix: preserve note\n\nChangelog: Fixed: Required detail.\n\nCloses #107",
                true,
            ),
            (
                "fix: compatibility\n\nChangelog: Changed: New representation.\n\nBREAKING CHANGE: ",
                false,
            ),
            (
                "fix!: compatibility\n\nChangelog: Changed: New representation.",
                false,
            ),
            (
                "fix: preserve note\n\nChangelog: Fixed: Required detail.",
                true,
            ),
            (
                "fix(deps): update dependencies\n\n## Package changes\n\nA realistic Renovate body.\n\n---\n\nChangelog: Dependencies: Update runtime and build dependencies.",
                true,
            ),
        ] {
            let commits = context(root.path(), "HEAD..HEAD", Some(message)).unwrap();
            assert_eq!(
                !commits.is_empty() && commits.iter().all(|c| note(c).is_ok()),
                success,
                "{message}: {commits:?}"
            );
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
        assert_eq!(first.matches("### Changed").count(), 1);
        assert!(!first.contains("manage weekly releases"));
        git(&["commit", "--allow-empty", "-qm", "chore: release v0.3.0"]);
        git(&["tag", "v0.3.0"]);
        for message in [
            "fix: keep multiline instructions (#900)\n\nChangelog: Fixed: Keep the first paragraph.\n\nKeep the second paragraph, including its explanation: nothing is truncated.",
            "docs: explain editor recovery (#901)\n\nChangelog: Documentation: Explain how to recover a disconnected editor session.",
            "ci: change the build cache (#902)",
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
        assert!(
            second.contains("second paragraph, including its explanation: nothing is truncated.")
        );
        assert_eq!(second.matches("### Fixed").count(), 1);
        assert_eq!(
            second
                .matches("Update runtime and build dependencies.")
                .count(),
            1
        );
        assert!(second.contains("non-ASCII names intact"));
        assert!(second.contains("Rename parser_mode to parser-mode in saved configuration."));
        assert!(check_section(&second.replace("Keep the first paragraph.", ""), &commits).is_err());
        assert!(check_section(&second.replace("/pull/904", "/pull/999"), &commits).is_err());
    }
}
