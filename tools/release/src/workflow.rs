//! Checks the guarded release workflow runs before it builds or publishes anything.

use crate::common::{Result, crate_version, repository};
use serde_json::Value;
use std::fs;

/// Fails unless `tag` names the version in `Cargo.toml`, as `v0.1.0` names 0.1.0.
pub fn check_tag(args: &[String]) -> Result<()> {
    let [tag] = args else {
        return Err("usage: ripscope-release check-tag TAG".into());
    };
    let version = crate_version(&repository()?.join("Cargo.toml"))?;
    if *tag != format!("v{version}") {
        return Err(format!("tag {tag} does not name version {version}").into());
    }
    Ok(())
}

/// Fails unless the GitHub environment description in `FILE` is the `release`
/// environment with at least one valid required reviewer.
pub fn check_environment(args: &[String]) -> Result<()> {
    let [file] = args else {
        return Err("usage: ripscope-release check-environment FILE".into());
    };
    environment(&serde_json::from_slice(&fs::read(file)?)?)
}

fn environment(value: &Value) -> Result<()> {
    let rules: Vec<&Value> = value["protection_rules"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|rule| rule["type"] == "required_reviewers")
        .collect();
    if value["name"] != "release" || rules.is_empty() {
        return Err("release environment with reviewers required".into());
    }
    let reviewers = rules[0]["reviewers"]
        .as_array()
        .filter(|reviewers| !reviewers.is_empty())
        .ok_or("at least one required reviewer required")?;
    let valid = |reviewer: &Value| {
        (reviewer["type"] == "User" || reviewer["type"] == "Team")
            && reviewer["reviewer"]["id"].as_u64().is_some_and(|id| id > 0)
    };
    if !reviewers.iter().all(valid) {
        return Err("invalid required reviewer".into());
    }
    Ok(())
}

/// Prints one field of a release profile, such as `linux mold_url`.
pub fn profile(args: &[String]) -> Result<()> {
    let [platform, field] = args else {
        return Err("usage: ripscope-release profile (linux | windows) FIELD".into());
    };
    if !["linux", "windows"].contains(&platform.as_str()) {
        return Err(format!("no {platform} release profile").into());
    }
    let path = repository()?.join(format!("tools/release/{platform}-release-profile.json"));
    let profile: Value = serde_json::from_slice(&fs::read(path)?)?;
    match &profile[field] {
        Value::String(text) => println!("{text}"),
        Value::Null => return Err(format!("the {platform} profile has no {field}").into()),
        value => println!("{value}"),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn publication_needs_the_release_environment_with_real_reviewers() {
        let valid = json!({"name": "release", "protection_rules": [
            {"type": "wait_timer"},
            {"type": "required_reviewers", "reviewers": [{"type": "User", "reviewer": {"id": 7}}]},
        ]});
        environment(&valid).unwrap();
        for invalid in [
            json!({"name": "staging", "protection_rules": valid["protection_rules"].clone()}),
            json!({"name": "release", "protection_rules": [{"type": "wait_timer"}]}),
            json!({"name": "release", "protection_rules": [{"type": "required_reviewers", "reviewers": []}]}),
            json!({"name": "release", "protection_rules": [{"type": "required_reviewers",
                "reviewers": [{"type": "Bot", "reviewer": {"id": 7}}]}]}),
            json!({"name": "release", "protection_rules": [{"type": "required_reviewers",
                "reviewers": [{"type": "Team", "reviewer": {"id": 0}}]}]}),
            json!({"name": "release", "protection_rules": [{"type": "required_reviewers",
                "reviewers": [{"type": "User", "reviewer": {"id": "7"}}]}]}),
        ] {
            assert!(environment(&invalid).is_err(), "{invalid}");
        }
    }
}
