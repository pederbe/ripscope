//! Read the dated changelog section committed by the release pull request.

use crate::common::{Result, repository};
use std::fs;

pub fn notes(args: &[String]) -> Result<()> {
    let [tag, output] = args else {
        return Err("usage: ripscope-release release-notes TAG OUTPUT".into());
    };
    let changelog = fs::read_to_string(repository()?.join("CHANGELOG.md"))?;
    fs::write(output, release_notes(&changelog, tag)?)?;
    Ok(())
}

fn release_notes(changelog: &str, tag: &str) -> Result<String> {
    let version = tag.strip_prefix('v').ok_or("expected version tag")?;
    let changelog = changelog.replace("\r\n", "\n");
    let section = changelog
        .split_once(&format!("## [{version}] - "))
        .ok_or("missing dated release changelog section")?
        .1;
    let (_, section) = section.split_once('\n').ok_or("empty changelog section")?;
    let notes = section.split("\n## [").next().unwrap_or("").trim();
    if notes.is_empty() {
        return Err("empty release notes".into());
    }
    Ok(format!("{notes}\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_only_the_requested_release_and_preserve_upgrade_instructions() {
        let notes = "### Changed\n\n- **Breaking:** Remove a setting ([#129](https://github.com/pederbe/ripscope/pull/129))\n\nRemove it from your configuration.\n\nUse cscope.files to select sources.\n";
        let changelog = format!(
            "# Changelog\n\n## [Unreleased]\n\nPending work\n\n## [0.3.0] - 2026-11-12\n\n{notes}\n## [0.2.7] - 2026-11-09\n\nOlder notes\n"
        );
        for text in [changelog.clone(), changelog.replace('\n', "\r\n")] {
            assert_eq!(release_notes(&text, "v0.3.0").unwrap(), notes);
            assert!(release_notes(&text, "v0.3.1").is_err());
        }
        assert!(release_notes("## [0.3.0] - 2026-11-12\n\n", "v0.3.0").is_err());
    }
}
