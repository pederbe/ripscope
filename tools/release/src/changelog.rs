//! Validate the inputs and coverage of release-plz's human-facing notes.

use crate::common::{Result, repository};
use regex::Regex;
use serde_json::Value;
use std::{collections::BTreeSet, fs, process::Command};

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

fn footer(message: &str, name: &str) -> Option<String> {
    let marker = format!("\n{name}: ");
    let value = message.split_once(&marker)?.1;
    let next = Regex::new(r"(?m)\n(?:[A-Za-z][A-Za-z-]*|BREAKING CHANGE): ").unwrap();
    Some(
        value[..next.find(value).map_or(value.len(), |m| m.start())]
            .trim()
            .to_owned(),
    )
}

fn note(message: &str) -> Result<Option<Note>> {
    let internal = Regex::new(r"^(?:chore|ci|build|test|style|refactor)(?:\([^)]*\))?:").unwrap();
    let subject = message.lines().next().unwrap_or("");
    if !Regex::new(r"^[a-z]+(?:\([^)]*\))?!?: .+")?.is_match(subject) {
        return Err(
            "use a conventional pull request title, such as fix: describe the repair".into(),
        );
    }
    if message.matches("\nChangelog: ").count() > 1 {
        return Err(
            "keep the complete change in one Changelog: paragraph, with continuations if needed"
                .into(),
        );
    }
    let breaking =
        footer(message, "BREAKING CHANGE").or_else(|| footer(message, "BREAKING-CHANGE"));
    let marked = subject
        .split_once(':')
        .is_some_and(|(prefix, _)| prefix.ends_with('!'));
    if marked && breaking.as_deref().is_none_or(str::is_empty) {
        return Err(
            "a breaking change needs a BREAKING CHANGE: paragraph with upgrade instructions".into(),
        );
    }
    let Some(value) = footer(message, "Changelog") else {
        if internal.is_match(subject) && breaking.is_none() {
            return Ok(None);
        }
        return Err(
            "add Changelog: CATEGORY: user-facing description to the pull request body".into(),
        );
    };
    let (category, text) = value
        .split_once(':')
        .ok_or("Changelog needs CATEGORY: description")?;
    let (category, text) = (category.trim(), text.trim());
    if !CATEGORIES.contains(&category)
        || text.is_empty()
        || text.contains("<!--")
        || text.contains("\n##")
    {
        return Err("Changelog needs a supported category and complete prose, without template comments or headings".into());
    }
    Ok(Some(Note {
        category: category.into(),
        text: text.into(),
        breaking,
    }))
}

fn normalize(message: &str, config: &toml::Value) -> Result<String> {
    let mut message = message.to_owned();
    if let Some(rules) = config["git"]["commit_preprocessors"].as_array() {
        for rule in rules {
            let pattern = rule["pattern"]
                .as_str()
                .ok_or("preprocessor pattern missing")?;
            let replace = rule["replace"]
                .as_str()
                .ok_or("preprocessor replacement missing")?;
            message = Regex::new(pattern)?
                .replace_all(&message, replace)
                .into_owned();
        }
    }
    Ok(message)
}

pub fn check_pr(args: &[String]) -> Result<()> {
    let [file] = args else {
        return Err("usage: ripscope-release check-pr-note EVENT_JSON".into());
    };
    let event: Value = serde_json::from_slice(&fs::read(file)?)?;
    let pr = &event["pull_request"];
    let title = pr["title"].as_str().ok_or("pull request title missing")?;
    let body = pr["body"].as_str().unwrap_or("");
    note(&format!("{title}\n\n{body}"))?;
    Ok(())
}

fn words(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn check_section(section: &str, messages: &[String], config: &toml::Value) -> Result<()> {
    let mut headings = BTreeSet::new();
    for line in section.lines().filter_map(|line| line.strip_prefix("### ")) {
        if !CATEGORIES.contains(&line) || !headings.insert(line) {
            return Err(format!("unknown or repeated changelog heading: {line}").into());
        }
    }
    let section_words = words(section);
    let mut count = 0;
    let reference = Regex::new(r"\(#([0-9]+)\)$")?;
    for message in messages {
        let normalized = normalize(message, config)?;
        let Some(note) = note(&normalized)
            .map_err(|e| format!("{}: {e}", message.lines().next().unwrap_or("commit")))?
        else {
            continue;
        };
        if !headings.contains(note.category.as_str()) {
            return Err(format!("missing {} section", note.category).into());
        }
        let expected = if note.category == "Dependencies" {
            "Update runtime and build dependencies."
        } else {
            &note.text
        };
        if !section_words.contains(&words(expected)) {
            return Err(format!("missing changelog note: {}", note.text).into());
        }
        if let Some(breaking) = note.breaking
            && !section_words.contains(&words(&breaking))
        {
            return Err("missing breaking-change upgrade instructions".into());
        }
        // Every merged change retains its PR source, including combined dependency notes.
        if let Some(captures) = reference.captures(message.lines().next().unwrap_or("")) {
            let link = format!("https://github.com/pederbe/ripscope/pull/{}", &captures[1]);
            if !section.contains(&link) {
                return Err(format!("missing source link: {link}").into());
            }
        }
        count += 1;
    }
    if count == 0 || !section.contains("- ") {
        return Err("release notes contain no user-facing changes".into());
    }
    Ok(())
}

pub fn check_release(args: &[String]) -> Result<()> {
    let [previous] = args else {
        return Err("usage: ripscope-release check-release-notes PREVIOUS_TAG".into());
    };
    let root = repository()?;
    let config: toml::Value = toml::from_str(&fs::read_to_string(root.join("cliff.toml"))?)?;
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
    let output = Command::new("git")
        .current_dir(&root)
        .args(["log", "--format=%B%x00", &format!("{previous}..HEAD")])
        .output()?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into_owned().into());
    }
    let messages = String::from_utf8(output.stdout)?
        .split('\0')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    check_section(section, &messages, &config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_changes_need_complete_notes_and_breaking_upgrade_instructions() {
        assert!(note("fix: repair a query").is_err());
        assert!(
            note("refactor!: change result names\n\nChangelog: Changed: Use readable names.")
                .is_err()
        );
        assert!(note("fix: repair a query\n\nChangelog: Fixed: ").is_err());
        assert!(note("ci: check documentation").unwrap().is_none());
        let parsed = note("feat!: new result names\n\nChangelog: Changed: Use readable names.\n\nFurther explanation: retain this paragraph.\n\nBREAKING CHANGE: Update saved queries.\n\nThen rebuild the index.").unwrap().unwrap();
        assert!(parsed.text.contains("Further explanation: retain"));
        assert_eq!(
            parsed.breaking.unwrap(),
            "Update saved queries.\n\nThen rebuild the index."
        );
    }

    #[test]
    fn generated_notes_must_cover_changes_sources_and_upgrades_without_repeated_headings() {
        let config: toml::Value = toml::from_str("[git]\ncommit_preprocessors=[]").unwrap();
        let messages = vec!["feat!: rename results (#12)\n\nChangelog: Changed: Use readable names.\n\nBREAKING CHANGE: Update saved queries.".into()];
        let valid = "### Changed\n\n- **Breaking:** Use readable names.\n\n  Update saved queries. ([#12](https://github.com/pederbe/ripscope/pull/12))\n";
        check_section(valid, &messages, &config).unwrap();
        for invalid in [
            valid.replace("Use readable names.", "Other work."),
            valid.replace("Update saved queries.", ""),
            valid.replace("/pull/12", "/pull/13"),
            format!("{valid}\n### Changed\n\n- Duplicate.\n"),
        ] {
            assert!(
                check_section(&invalid, &messages, &config).is_err(),
                "{invalid}"
            );
        }
    }

    #[test]
    #[ignore = "requires the pinned git-cliff CLI, run by the changelog CI job"]
    fn rendered_notes_cover_current_and_future_releases() {
        let root = tempfile::tempdir().unwrap();
        let config_path = repository().unwrap().join("cliff.toml");
        let config: toml::Value =
            toml::from_str(&fs::read_to_string(&config_path).unwrap()).unwrap();
        let git = |args: &[&str]| {
            let result = Command::new("git")
                .current_dir(root.path())
                .args(args)
                .output()
                .unwrap();
            assert!(result.status.success(), "{args:?}: {result:?}");
        };
        git(&["init", "--quiet"]);
        git(&["config", "user.name", "Fixture"]);
        git(&["config", "user.email", "fixture@example.invalid"]);
        git(&["commit", "--allow-empty", "-qm", "chore: initial version"]);
        git(&["tag", "v0.2.7"]);
        let current = vec![
            "Cross-build Linux releases with Zig while preserving glibc 2.28 (#132)".to_owned(),
            "Leave editor version numbers out of the editor guides (#125)".to_owned(),
            "ci!: manage weekly releases with release-plz (#135)".to_owned(),
        ];
        for message in &current {
            git(&["commit", "--allow-empty", "-qm", message]);
        }
        let render = |previous: &str, next: &str| {
            let output = Command::new("git-cliff")
                .current_dir(root.path())
                .args([
                    "--config",
                    config_path.to_str().unwrap(),
                    "--tag",
                    next,
                    &format!("{previous}..HEAD"),
                ])
                .output()
                .unwrap();
            assert!(output.status.success(), "{output:?}");
            String::from_utf8(output.stdout).unwrap()
        };
        let first = render("v0.2.7", "v0.3.0");
        check_section(&first, &current, &config).unwrap();
        assert_eq!(first.matches("### Changed").count(), 1);
        assert!(!first.contains("manage weekly releases"));
        git(&["commit", "--allow-empty", "-qm", "chore: release v0.3.0"]);
        git(&["tag", "v0.3.0"]);
        let later = vec![
            "fix: keep multiline instructions (#900)\n\nChangelog: Fixed: Keep the first paragraph.\n\nKeep the second paragraph, including its explanation: nothing is truncated.".to_owned(),
            "docs: explain editor recovery (#901)\n\nChangelog: Documentation: Explain how to recover a disconnected editor session.".to_owned(),
            "ci: change the build cache (#902)".to_owned(),
        ];
        for message in &later {
            git(&["commit", "--allow-empty", "-qm", message]);
        }
        let second = render("v0.3.0", "v0.3.1");
        check_section(&second, &later, &config).unwrap();
        assert!(!second.contains("glibc"));
        assert!(!second.contains("build cache"));
        assert!(
            second.contains("second paragraph, including its explanation: nothing is truncated.")
        );
        assert_eq!(second.matches("### Fixed").count(), 1);
        assert!(
            check_section(
                &second.replace("Keep the first paragraph.", ""),
                &later,
                &config
            )
            .is_err()
        );
        assert!(
            check_section(
                &second,
                &["fix: a missing future note (#903)".into()],
                &config
            )
            .is_err()
        );
    }
}
