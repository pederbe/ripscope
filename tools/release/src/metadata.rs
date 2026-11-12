//! Read the dated changelog section committed by the release pull request.

use crate::common::{Result, repository};
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
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
    Ok(format!("{}\n", flowing_paragraphs(notes)))
}

// GitHub release pages turn Markdown soft breaks into visible line breaks.
// Fold prose wrapping while leaving Markdown blocks and explicit breaks intact.
fn flowing_paragraphs(notes: &str) -> String {
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_GFM;
    let mut output = String::with_capacity(notes.len());
    let mut copied = 0;
    let mut quotes = 0;
    for (event, range) in Parser::new_ext(notes, options).into_offset_iter() {
        match event {
            Event::Start(Tag::BlockQuote(_)) => quotes += 1,
            Event::End(TagEnd::BlockQuote(_)) => quotes -= 1,
            Event::SoftBreak if quotes == 0 => {
                output.push_str(&notes[copied..range.start]);
                output.push(' ');
                copied = range.end;
                while matches!(notes.as_bytes().get(copied), Some(b' ' | b'\t')) {
                    copied += 1;
                }
            }
            _ => {}
        }
    }
    output.push_str(&notes[copied..]);
    output
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

    #[test]
    fn github_notes_flow_wrapped_prose_and_keep_upgrade_paragraphs() {
        let changelog = "## [0.3.1] - 2026-11-12\n\n### Fixed\n\n- Include the complete changelog in Linux and Windows\n  release\n  archives, so upgrade instructions are available\n  offline.\n\n  Rename `old-option` to `new-option`\n  in your configuration.\n\n- Preserve naïve and 日本語 names\n  in search results.\n";
        assert_eq!(
            release_notes(changelog, "v0.3.1").unwrap(),
            "### Fixed\n\n- Include the complete changelog in Linux and Windows release archives, so upgrade instructions are available offline.\n\n  Rename `old-option` to `new-option` in your configuration.\n\n- Preserve naïve and 日本語 names in search results.\n"
        );
    }

    #[test]
    fn github_notes_preserve_lists_code_quotes_tables_and_explicit_breaks() {
        let structured = "### Upgrade\n\n1. Change the setting.\n2. Restart the editor.\n\n   ```sh\n   first command\n   second command\n   ```\n\n> Quoted line one\n> Quoted line two\n\n| Setting | Value |\n| --- | --- |\n| mode | auto |\n\nAn intentional break.  \nAnother intentional break.\\\nFinal line.\n";
        assert_eq!(flowing_paragraphs(structured), structured);
    }
}
