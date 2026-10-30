//! Finalize pending release notes and reconcile published notes with development.

type Result<T> = std::result::Result<T, String>;

#[derive(Clone, Debug)]
struct Section {
    version: String,
    heading: String,
    body: String,
}

struct Document {
    intro: String,
    pending: String,
    releases: Vec<Section>,
}

pub fn version(tag: &str) -> Result<&str> {
    let value = tag.strip_prefix('v').unwrap_or(tag);
    let core = value.split(['-', '+']).next().unwrap_or("");
    if core.split('.').count() != 3
        || core.split('.').any(|part| part.parse::<u64>().is_err())
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".-+".contains(&b))
    {
        return Err("expected a semantic release version, such as v0.2.0".into());
    }
    Ok(value)
}

fn valid_date(date: &str) -> bool {
    if date.len() != 10
        || !date.bytes().enumerate().all(|(i, b)| {
            if matches!(i, 4 | 7) {
                b == b'-'
            } else {
                b.is_ascii_digit()
            }
        })
    {
        return false;
    }
    let Some((year, month, day)) = date[..4]
        .parse::<u32>()
        .ok()
        .zip(date[5..7].parse::<u32>().ok())
        .zip(date[8..].parse::<u32>().ok())
        .map(|((y, m), d)| (y, m, d))
    else {
        return false;
    };
    let leap = year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
    let days = match month {
        2 => 28 + u32::from(leap),
        4 | 6 | 9 | 11 => 30,
        1..=12 => 31,
        _ => 0,
    };
    year > 0 && day > 0 && day <= days
}

/// GitHub timestamps use UTC seconds. A validation rerun is not publication.
pub fn published_during_run(published: &str, started: &str, completed: &str) -> bool {
    fn timestamp(value: &str) -> bool {
        value.len() == 20
            && value.bytes().enumerate().all(|(index, byte)| match index {
                4 | 7 => byte == b'-',
                10 => byte == b'T',
                13 | 16 => byte == b':',
                19 => byte == b'Z',
                _ => byte.is_ascii_digit(),
            })
            && valid_date(&value[..10])
            && value[11..13].parse::<u8>().is_ok_and(|hour| hour < 24)
            && value[14..16].parse::<u8>().is_ok_and(|minute| minute < 60)
            && value[17..19].parse::<u8>().is_ok_and(|second| second < 60)
    }
    [published, started, completed].into_iter().all(timestamp)
        && started <= published
        && published <= completed
}

fn parse(text: &str) -> Result<Document> {
    let text = text.replace("\r\n", "\n");
    let mut starts = Vec::new();
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        if line.starts_with("## ") {
            starts.push(offset);
        }
        offset += line.len();
    }
    let Some(&first) = starts.first() else {
        return Err("changelog has no release section".into());
    };
    starts.push(text.len());
    let mut document = Document {
        intro: text[..first].into(),
        pending: String::new(),
        releases: Vec::new(),
    };
    let mut saw_pending = false;
    for pair in starts.windows(2) {
        let section = &text[pair[0]..pair[1]];
        let (heading, body) = section.split_once('\n').unwrap_or((section, ""));
        if matches!(heading, "## [Unreleased]" | "## Unreleased")
            || heading
                .strip_prefix("## ")
                .is_some_and(|h| h.ends_with(", unreleased"))
        {
            if saw_pending || !document.releases.is_empty() {
                return Err("Unreleased must occur once, before released entries".into());
            }
            saw_pending = true;
            document.pending = body.trim().into();
            continue;
        }
        let Some((name, date)) = heading
            .strip_prefix("## [")
            .and_then(|h| h.split_once("] - "))
        else {
            return Err(format!("unrecognized changelog heading: {heading}"));
        };
        version(name)?;
        if !valid_date(date) {
            return Err(format!("invalid changelog date: {date}"));
        }
        if document.releases.iter().any(|s| s.version == name) {
            return Err(format!("duplicate release section: {name}"));
        }
        document.releases.push(Section {
            version: name.into(),
            heading: heading.into(),
            body: body.trim().into(),
        });
    }
    Ok(document)
}

fn render(document: &Document) -> String {
    let mut result = document.intro.trim_end().to_owned();
    result.push_str("\n\n");
    if !document.pending.trim().is_empty() {
        result.push_str("## [Unreleased]\n\n");
        result.push_str(document.pending.trim());
        result.push_str("\n\n");
    }
    for section in &document.releases {
        result.push_str(&section.heading);
        result.push_str("\n\n");
        result.push_str(section.body.trim());
        result.push_str("\n\n");
    }
    result.trim_end().to_owned() + "\n"
}

/// Close the pending section. Repeating the operation retains its frozen date.
pub fn finalize(text: &str, tag: &str, date: &str) -> Result<String> {
    let name = version(tag)?;
    if !valid_date(date) {
        return Err("expected release date YYYY-MM-DD".into());
    }
    let mut document = parse(text)?;
    if document.releases.iter().any(|s| s.version == name) {
        if !document.pending.is_empty() {
            return Err("this version is already finalized and has new pending entries".into());
        }
        return Ok(render(&document));
    }
    if document.pending.is_empty() {
        return Err("no pending release notes to finalize".into());
    }
    document.releases.insert(
        0,
        Section {
            version: name.into(),
            heading: format!("## [{name}] - {date}"),
            body: std::mem::take(&mut document.pending),
        },
    );
    Ok(render(&document))
}

/// Validate a frozen release, including old releases with an empty placeholder.
pub fn check(text: &str, tag: &str) -> Result<()> {
    let name = version(tag)?;
    let document = parse(text)?;
    if !document.pending.is_empty() {
        return Err("release changelog still contains pending entries".into());
    }
    let section = document
        .releases
        .iter()
        .find(|s| s.version == name)
        .ok_or_else(|| format!("missing dated changelog section for {name}"))?;
    if section.body.is_empty() {
        return Err("release notes are empty".into());
    }
    Ok(())
}

pub fn notes(text: &str, tag: &str) -> Result<String> {
    check(text, tag)?;
    let name = version(tag)?;
    let document = parse(text)?;
    Ok(document
        .releases
        .iter()
        .find(|s| s.version == name)
        .unwrap()
        .body
        .clone()
        + "\n")
}

fn records(text: &str) -> Vec<(String, String)> {
    let mut heading = String::new();
    let mut body = String::new();
    let mut result = Vec::new();
    for line in text.lines() {
        if line.starts_with("### ") || line.starts_with("- ") {
            if !body.trim().is_empty() {
                result.push((heading.clone(), body.trim_end().into()));
                body.clear();
            }
            if line.starts_with("### ") {
                heading = line.into();
                continue;
            }
        }
        if !line.trim().is_empty() || !body.is_empty() {
            body.push_str(line);
            body.push('\n');
        }
    }
    if !body.trim().is_empty() {
        result.push((heading, body.trim_end().into()));
    }
    result
}

fn pending_delta(current: &str, shipped: &str) -> String {
    let shipped = records(shipped);
    let mut result = String::new();
    let mut previous_heading = String::new();
    for (heading, body) in records(current) {
        if shipped.contains(&(heading.clone(), body.clone())) {
            continue;
        }
        if heading != previous_heading && !heading.is_empty() {
            result.push_str(&heading);
            result.push_str("\n\n");
        }
        result.push_str(&body);
        result.push_str("\n\n");
        previous_heading = heading;
    }
    result.trim().into()
}

/// Record a published release without losing notes or edits made after freezing.
pub fn record(current: &str, source: &str, released: &str, tag: &str) -> Result<String> {
    let name = version(tag)?;
    check(released, tag)?;
    let mut current = parse(current)?;
    let source = parse(source)?;
    let published = parse(released)?;
    let selected = published
        .releases
        .iter()
        .find(|s| s.version == name)
        .unwrap();
    current.pending = pending_delta(
        &current.pending,
        &format!("{}\n\n{}", source.pending, selected.body),
    );
    let mut releases = Vec::new();
    for section in published.releases {
        let mut merged = current
            .releases
            .iter()
            .find(|s| s.version == section.version)
            .cloned()
            .unwrap_or_else(|| section.clone());
        if section.version == name {
            merged.heading = section.heading;
        }
        releases.push(merged);
    }
    for section in current.releases {
        if !releases.iter().any(|s| s.version == section.version) {
            releases.push(section);
        }
    }
    current.releases = releases;
    Ok(render(&current)
        .lines()
        .map(|line| {
            if let Some(url) = line.strip_prefix("[Unreleased]: ")
                && let Some((repository, _)) = url.split_once("/compare/")
            {
                return format!("[Unreleased]: {repository}/compare/{tag}...HEAD");
            }
            line.to_owned()
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    const OLD: &str = "# Changelog\n\n## [0.1.0] - 2026-10-01\n\nInitial release.\n";
    const SOURCE: &str = "# Changelog\n\n## [Unreleased]\n\n### Added\n\n- A capability.\n  A continuation.\n\n## [0.1.0] - 2026-10-01\n\nInitial release.\n";

    #[test]
    fn closes_the_heading_and_retains_the_date_on_retry() {
        let released = finalize(SOURCE, "v0.2.0", "2026-10-30").unwrap();
        assert!(released.starts_with("# Changelog\n\n## [0.2.0] - 2026-10-30\n"));
        assert!(!released.contains("Unreleased"));
        assert!(released.ends_with("Initial release.\n"));
        assert_eq!(
            finalize(&released, "v0.2.0", "2026-10-31").unwrap(),
            released
        );
    }

    #[test]
    fn first_release_and_legacy_pending_heading_work() {
        for heading in ["## Unreleased", "## [Unreleased]", "## 0.1.0, unreleased"] {
            assert_eq!(
                finalize(
                    &format!("# Changelog\n\n{heading}\n\nInitial release.\n"),
                    "0.1.0",
                    "2026-10-01"
                )
                .unwrap(),
                OLD
            );
        }
    }

    #[test]
    fn recording_publication_keeps_concurrent_entries_and_historical_edits() {
        let released = finalize(SOURCE, "v0.2.0", "2026-10-30").unwrap();
        let current = SOURCE
            .replace(
                "- A capability.",
                "- Another capability.\n\n- A capability.",
            )
            .replace("Initial release.", "Corrected historical description.");
        let recorded = record(&current, SOURCE, &released, "v0.2.0").unwrap();
        assert!(recorded.starts_with(
            "# Changelog\n\n## [Unreleased]\n\n### Added\n\n- Another capability.\n\n## [0.2.0]"
        ));
        assert_eq!(recorded.matches("- A capability.").count(), 1);
        assert!(recorded.contains("Corrected historical description."));
        assert_eq!(
            record(&recorded, SOURCE, &released, "v0.2.0").unwrap(),
            recorded
        );
    }

    #[test]
    fn recording_closes_only_the_shipped_notes() {
        let released = finalize(SOURCE, "v0.2.0", "2026-10-30").unwrap();
        assert_eq!(
            record(SOURCE, SOURCE, &released, "v0.2.0").unwrap(),
            released
        );
        // The prepared source may already have its dated heading.
        assert_eq!(
            record(SOURCE, &released, &released, "v0.2.0").unwrap(),
            released
        );
        assert_eq!(
            record(&released, SOURCE, &released, "v0.2.0").unwrap(),
            released
        );
    }

    #[test]
    fn publication_imports_missing_history_but_keeps_curated_notes() {
        let released = finalize(SOURCE, "v0.2.0", "2026-10-30").unwrap();
        assert_eq!(record(OLD, OLD, &released, "v0.2.0").unwrap(), released);
        let curated = released.replace(
            "- A capability.\n  A continuation.",
            "- A clearer capability description.",
        );
        assert_eq!(
            record(&curated, SOURCE, &released, "v0.2.0").unwrap(),
            curated
        );
    }

    #[test]
    fn edited_pending_notes_are_preserved() {
        let released = finalize(SOURCE, "v0.2.0", "2026-10-30").unwrap();
        let changed = SOURCE.replace("A continuation.", "A newly changed continuation.");
        assert!(record(&changed, SOURCE, &released, "v0.2.0").unwrap().contains("## [Unreleased]\n\n### Added\n\n- A capability.\n  A newly changed continuation."));
    }

    #[test]
    fn pending_comparison_links_follow_the_recorded_release() {
        let released = finalize(SOURCE, "v0.2.0", "2026-10-30").unwrap();
        let current = SOURCE.replace("- A capability.", "- Later work.\n\n- A capability.")
            + "\n[Unreleased]: https://example.test/project/compare/v0.1.0...HEAD\n";
        let recorded = record(&current, SOURCE, &released, "v0.2.0").unwrap();
        assert!(
            recorded.contains("[Unreleased]: https://example.test/project/compare/v0.2.0...HEAD")
        );
    }

    #[test]
    fn malformed_or_empty_notes_and_invalid_dates_are_rejected() {
        for date in [
            "2026-02-29",
            "2026-13-01",
            "2026-00-01",
            "2026-10-00",
            "2026-1-1",
            "ééééé",
        ] {
            assert!(finalize(SOURCE, "v0.2.0", date).is_err());
        }
        assert!(finalize(SOURCE, "v0.2.0", "2028-02-29").is_ok());
        assert!(finalize(OLD, "v0.2.0", "2026-10-30").is_err());
        assert!(check(SOURCE, "v0.2.0").is_err());
        assert!(check(OLD, "v0.2.0").is_err());
        assert!(finalize("# Empty\n", "v0.1.0", "2026-10-30").is_err());
        assert!(version("v0.2.0/branch").is_err());
    }
}
