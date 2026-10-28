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

/// The automatic publication environment has tag restrictions and no human gate.
pub fn check_automated_environment(args: &[String]) -> Result<()> {
    let [file] = args else {
        return Err("usage: ripscope-release check-automated-environment FILE".into());
    };
    automated_environment(&serde_json::from_slice(&fs::read(file)?)?)
}

fn automated_environment(value: &Value) -> Result<()> {
    if value["name"] != "automated-release"
        || value["deployment_branch_policy"]["custom_branch_policies"] != true
        || !value["protection_rules"]
            .as_array()
            .is_some_and(|rules| rules.iter().all(|rule| rule["type"] == "branch_policy"))
    {
        return Err(
            "automated-release needs custom tag restrictions and no approval or wait rules".into(),
        );
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
    fn automatic_publication_requires_its_own_restricted_environment() {
        let valid = json!({"name":"automated-release", "protection_rules":[{"type":"branch_policy"}],
            "deployment_branch_policy":{"custom_branch_policies":true}});
        automated_environment(&valid).unwrap();
        for invalid in [
            json!({"name":"release"}),
            json!({"name":"automated-release", "protection_rules":[]}),
            json!({"name":"automated-release", "protection_rules":[{"type":"required_reviewers"}], "deployment_branch_policy":{"custom_branch_policies":true}}),
        ] {
            assert!(automated_environment(&invalid).is_err());
        }
    }

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

#[cfg(all(test, unix))]
mod delivery_tests {
    use std::{fs, os::unix::fs::PermissionsExt, path::Path, process::Command};

    fn executable(root: &Path, name: &str, script: &str) {
        let path = root.join("bin").join(name);
        fs::write(&path, script).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn workflow_script(name: &str) -> String {
        fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../.github/workflows")
                .join(name),
        )
        .unwrap()
        .split_once("        run: |\n")
        .unwrap()
        .1
        .lines()
        .map(|line| line.strip_prefix("          ").unwrap_or(line))
        .collect::<Vec<_>>()
        .join("\n")
    }

    fn run(root: &Path, script: &str, sha: &str, mode: &str) -> std::process::Output {
        Command::new("bash")
            .args(["-c", script])
            .env(
                "PATH",
                format!(
                    "{}:{}",
                    root.join("bin").display(),
                    std::env::var("PATH").unwrap()
                ),
            )
            .env("RUNNER_TEMP", root)
            .env("GITHUB_STEP_SUMMARY", root.join("summary"))
            .env("GITHUB_REPOSITORY", "fixture/ripscope")
            .env("RELEASE_SHA", sha)
            .env("RELEASE_RUN_ID", if sha.is_empty() { "" } else { "42" })
            .env("CLOUDFLARE_DEPLOY_HOOK", "https://example.invalid/hook")
            .env("SUPERSEDE", "false")
            .env("INCREMENT", "auto")
            .env("FIXTURE_TRACE", root.join("trace"))
            .env("FIXTURE_MODE", mode)
            .output()
            .unwrap()
    }

    #[test]
    fn website_refresh_is_independent_and_skips_stale_completions() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("bin")).unwrap();
        executable(
            root.path(),
            "gh",
            r#"#!/bin/bash
set -euo pipefail
if [[ "$*" = *'/jobs?'* ]]; then
  if [ "$FIXTURE_MODE" = build-only ]; then echo false; else echo true; fi
else
  printf delivered
fi
"#,
        );
        executable(
            root.path(),
            "curl",
            r#"#!/bin/bash
set -euo pipefail
echo requested >> "$FIXTURE_TRACE"
if [ "$FIXTURE_MODE" = failure ]; then exit 22; fi
printf '{"success":true,"result":{"build_uuid":"fixture"}}' > "${!#}"
"#,
        );
        let script = workflow_script("website-deploy.yml");
        for (sha, mode, success, requested) in [
            ("stale", "success", true, false),
            ("delivered", "success", true, true),
            ("", "success", true, true),
            ("delivered", "build-only", true, false),
            ("delivered", "failure", false, true),
        ] {
            fs::write(root.path().join("trace"), "").unwrap();
            let output = run(root.path(), &script, sha, mode);
            assert_eq!(
                output.status.success(),
                success,
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(
                !fs::read_to_string(root.path().join("trace"))
                    .unwrap()
                    .is_empty(),
                requested
            );
        }
    }

    #[test]
    fn selected_website_ci_must_succeed() {
        let workflow = fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.github/workflows/ci.yml"),
        )
        .unwrap();
        let gate = workflow.split_once("\n  ci:\n").unwrap().1;
        let script = gate
            .split_once("        run: |\n")
            .unwrap()
            .1
            .lines()
            .map(|line| line.strip_prefix("          ").unwrap_or(line))
            .collect::<Vec<_>>()
            .join("\n");
        for (event, changes, selected, website, expected) in [
            ("pull_request", "success", "true", "success", true),
            ("pull_request", "success", "false", "skipped", true),
            ("pull_request", "success", "true", "skipped", false),
            ("pull_request", "success", "true", "failure", false),
            ("pull_request", "success", "true", "cancelled", false),
            ("pull_request", "success", "", "skipped", false),
            ("pull_request", "failure", "false", "skipped", false),
            ("schedule", "skipped", "", "skipped", true),
        ] {
            let result = Command::new("bash")
                .args(["-ec", &script])
                .env("EVENT", event)
                .env("CHANGES_RESULT", changes)
                .env("WEBSITE_SELECTED", selected)
                .env("WEBSITE_RESULT", website)
                .env("RESULTS", "success skipped")
                .status()
                .unwrap();
            assert_eq!(result.success(), expected, "{event}/{selected}/{website}");
        }
    }

    #[test]
    fn weekly_scheduler_ignores_website_failure_but_repairs_editor_delivery() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("bin")).unwrap();
        executable(
            root.path(),
            "git",
            r#"#!/bin/bash
set -euo pipefail
case "$1" in
  tag) echo v0.2.4;;
  show) echo 'Prepare automated release v0.2.4';;
  rev-parse) echo delivered;;
  ls-remote) printf '%s\trefs/heads/release\n' "$FIXTURE_MODE";;
  *) exit 1;;
esac
"#,
        );
        executable(
            root.path(),
            "gh",
            r#"#!/bin/bash
set -euo pipefail
case "$*" in
  api*) echo v0.2.4;;
  'run list'*)
    # A delivered version must not query a release failure caused by its website.
    test "$FIXTURE_MODE" != delivered
    echo '[{"databaseId":42,"status":"completed","conclusion":"failure"}]';;
  'run rerun 42 --failed') echo repaired >> "$FIXTURE_TRACE";;
  *) exit 1;;
esac
"#,
        );
        executable(
            root.path(),
            "cargo",
            "#!/bin/bash\nset -euo pipefail\nprintf 'changed=false\\n' > \"$RUNNER_TEMP/plan\"\necho prepared >> \"$FIXTURE_TRACE\"\n",
        );
        let script = workflow_script("weekly-release.yml");
        for (branch, expected) in [("delivered", "prepared\n"), ("older", "repaired\n")] {
            fs::write(root.path().join("trace"), "").unwrap();
            let output = run(root.path(), &script, "", branch);
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(
                fs::read_to_string(root.path().join("trace")).unwrap(),
                expected
            );
        }
    }
}
