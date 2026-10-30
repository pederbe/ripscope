use std::{
    fs,
    path::Path,
    process::{Command, ExitCode},
};

fn git(args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .args(args)
        .output()
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into());
    }
    String::from_utf8(output.stdout).map_err(|e| e.to_string())
}

fn run(args: &[String]) -> Result<(), String> {
    match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["publication-window", published, started, completed] => {
            if !release_changelog::published_during_run(published, started, completed) {
                return Err("this run did not publish the selected release".into());
            }
        }
        ["finalize", tag, date] => {
            let current = fs::read_to_string("CHANGELOG.md").map_err(|e| e.to_string())?;
            let result = release_changelog::finalize(&current, tag, date)?;
            fs::write("CHANGELOG.md", result).map_err(|e| e.to_string())?;
        }
        ["check", tag] => {
            let current = fs::read_to_string("CHANGELOG.md").map_err(|e| e.to_string())?;
            release_changelog::check(&current, tag)?;
        }
        ["notes", tag, output] => {
            let current = fs::read_to_string("CHANGELOG.md").map_err(|e| e.to_string())?;
            fs::write(output, release_changelog::notes(&current, tag)?).map_err(|e| e.to_string())?;
        }
        ["record-release", tag, plan] => {
            release_changelog::version(tag)?;
            let commit = git(&["rev-parse", "--verify", &format!("refs/tags/{tag}^{{commit}}")])?;
            let subject = git(&["show", "-s", "--format=%s", commit.trim()])?;
            let source = if subject.starts_with("Prepare automated release v") { format!("{}^", commit.trim()) } else { commit.trim().to_owned() };
            let released = git(&["show", &format!("{}:CHANGELOG.md", commit.trim())])?;
            let source = git(&["show", &format!("{source}:CHANGELOG.md")])?;
            let current = fs::read_to_string("CHANGELOG.md").map_err(|e| e.to_string())?;
            let result = release_changelog::record(&current, &source, &released, tag)?;
            let changed = current != result;
            if changed { fs::write("CHANGELOG.md", result).map_err(|e| e.to_string())?; }
            fs::write(Path::new(plan), format!("changed={changed}\n")).map_err(|e| e.to_string())?;
        }
        _ => return Err("usage: release-changelog (finalize VERSION YYYY-MM-DD | check TAG | notes TAG OUTPUT | record-release TAG PLAN)".into()),
    }
    Ok(())
}

fn main() -> ExitCode {
    match run(&std::env::args().skip(1).collect::<Vec<_>>()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("release-changelog: {message}");
            ExitCode::FAILURE
        }
    }
}
