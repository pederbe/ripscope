//! Keep every automatic PR check behind the required CI result.

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn workflow() -> String {
    fs::read_to_string(root().join(".github/workflows/ci.yml"))
        .unwrap()
        .replace("\r\n", "\n")
}

#[test]
fn required_ci_waits_for_every_job_and_every_pr_workflow() {
    let workflow = workflow();
    let jobs = workflow.split_once("\njobs:\n").unwrap().1;
    let job_names: BTreeSet<_> = jobs
        .lines()
        .filter_map(|line| line.strip_prefix("  "))
        .filter(|line| !line.starts_with(' ') && line.ends_with(':'))
        .map(|line| line.trim_end_matches(':'))
        .filter(|name| *name != "ci")
        .collect();
    let gate = jobs.split_once("  ci:\n").unwrap().1;
    let dependencies: BTreeSet<_> = gate
        .lines()
        .find_map(|line| line.strip_prefix("    needs: ["))
        .unwrap()
        .trim_end_matches(']')
        .split(", ")
        .collect();
    assert_eq!(dependencies, job_names, "CI must wait for every job");
    assert!(gate.contains("    if: always()\n"));
    assert!(gate.contains("RESULTS: ${{ join(needs.*.result, ' ') }}"));

    for entry in fs::read_dir(root().join(".github/workflows")).unwrap() {
        let path = entry.unwrap().path();
        let content = fs::read_to_string(&path).unwrap().replace("\r\n", "\n");
        if content.contains("\n  pull_request:") {
            assert_eq!(path.file_name().unwrap(), "ci.yml");
        }
    }
}

#[cfg(unix)]
#[test]
fn actual_ci_step_rejects_failed_cancelled_or_missing_results() {
    let workflow = workflow();
    let gate = workflow.split_once("\n  ci:\n").unwrap().1;
    let script = gate.split_once("        run: |\n").unwrap().1;
    let script = script
        .lines()
        .map(|line| line.strip_prefix("          ").unwrap_or(line))
        .collect::<Vec<_>>()
        .join("\n");
    for (results, expected) in [
        ("success success", true),
        ("success skipped success", true),
        ("success failure skipped", false),
        ("success cancelled skipped", false),
        ("success timed_out", false),
        ("", false),
    ] {
        let output = std::process::Command::new("bash")
            .args(["-ec", &script])
            .env("RESULTS", results)
            .output()
            .unwrap();
        assert_eq!(output.status.success(), expected, "results: {results:?}");
    }
}
