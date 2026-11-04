use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "release-changelog-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let fixture = Self(root);
        fixture.git(&["init", "-b", "main"]);
        fixture.git(&["config", "user.name", "Release check"]);
        fixture.git(&["config", "user.email", "release-check@example.com"]);
        fixture.git(&["config", "commit.gpgsign", "false"]);
        fixture
    }
    fn path(&self) -> &Path {
        &self.0
    }
    fn git(&self, args: &[&str]) -> String {
        let output = Command::new("git")
            .args(args)
            .current_dir(self.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    }
    fn commit(&self, subject: &str) {
        self.git(&["add", "."]);
        self.git(&["commit", "-m", subject]);
    }
    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_release-changelog"))
            .args(args)
            .current_dir(self.path())
            .output()
            .unwrap()
    }
    fn passed(&self, args: &[&str]) {
        let output = self.run(args);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(self.path()).unwrap();
    }
}

#[test]
fn actual_tag_reconciliation_preserves_later_work_and_is_idempotent() {
    let fixture = Fixture::new();
    let changelog = fixture.path().join("CHANGELOG.md");
    let pending = "# Changelog\n\n## [Unreleased]\n\n### Added\n\n- Shipping feature.\n\n## [0.1.0] - 2026-10-01\n\nInitial release.\n";
    fs::write(&changelog, pending).unwrap();
    fs::write(fixture.path().join("source.rs"), "original code\n").unwrap();
    fixture.commit("Add the shipping feature");
    let source = fixture.git(&["rev-parse", "HEAD"]);
    fixture.passed(&["finalize", "v0.2.0", "2026-10-30"]);
    fixture.commit("Prepare automated release v0.2.0");
    fixture.git(&["tag", "v0.2.0"]);
    let released = fs::read_to_string(&changelog).unwrap();
    fixture.git(&["checkout", "--detach", source.trim()]);
    fs::write(
        &changelog,
        pending.replace(
            "- Shipping feature.",
            "- Later feature.\n\n- Shipping feature.",
        ),
    )
    .unwrap();
    fs::write(fixture.path().join("source.rs"), "later code\n").unwrap();
    fixture.commit("Add a later feature");
    let plan = fixture.path().join(".git/plan");
    fixture.passed(&["record-release", "v0.2.0", plan.to_str().unwrap()]);
    assert_eq!(fs::read_to_string(&plan).unwrap(), "changed=true\n");
    assert_eq!(fixture.git(&["diff", "--name-only"]).trim(), "CHANGELOG.md");
    let recorded = fs::read_to_string(&changelog).unwrap();
    assert!(
        recorded.contains(
            "## [Unreleased]\n\n### Added\n\n- Later feature.\n\n## [0.2.0] - 2026-10-30"
        )
    );
    assert_eq!(recorded.matches("Shipping feature.").count(), 1);
    assert_eq!(fixture.git(&["show", "v0.2.0:CHANGELOG.md"]), released);
    fixture.passed(&["record-release", "v0.2.0", plan.to_str().unwrap()]);
    assert_eq!(fs::read_to_string(plan).unwrap(), "changed=false\n");
}

#[test]
fn recording_scope_guard_rejects_product_changes_and_stale_main() {
    let fixture = Fixture::new();
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let script = merge_script(repository)
        .split("          state=failure")
        .next()
        .unwrap()
        .to_owned();
    fs::write(fixture.path().join("CHANGELOG.md"), "Previous notes.\n").unwrap();
    fixture.commit("Initial notes");
    fixture.git(&["remote", "add", "origin", fixture.path().to_str().unwrap()]);
    fixture.git(&["fetch", "origin", "main"]);
    fixture.git(&["checkout", "-b", "contrib/release-changelog-v0.2.0"]);
    let work = fixture.path().join(".git/prepared");
    fs::create_dir(&work).unwrap();
    fs::write(work.join("CHANGELOG.md"), "Published notes.\n").unwrap();
    let preparation =
        fs::read_to_string(repository.join("scripts/record-release-changelog.sh")).unwrap();
    let commit = preparation
        .split_once("cp \"$work/CHANGELOG.md\" CHANGELOG.md\n")
        .unwrap()
        .1
        .split_once("git push origin")
        .unwrap()
        .0;
    fs::copy(
        work.join("CHANGELOG.md"),
        fixture.path().join("CHANGELOG.md"),
    )
    .unwrap();
    let output = Command::new("bash")
        .args(["-euc", commit])
        .env("tag", "v0.2.0")
        .current_dir(fixture.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        fixture
            .git(&["show", "-s", "--format=%s"])
            .contains("[skip ci]")
    );
    let checked = fixture.git(&["rev-parse", "HEAD"]);
    let check = || {
        Command::new("bash")
            .args(["-c", &script])
            .current_dir(fixture.path())
            .output()
            .unwrap()
    };
    let output = check();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    fs::write(fixture.path().join("source.rs"), "changed product code\n").unwrap();
    fixture.commit("Change product code");
    assert!(!check().status.success());
    fixture.git(&["checkout", "main"]);
    fs::write(fixture.path().join("later.md"), "Later work.\n").unwrap();
    fixture.commit("Advance main");
    fixture.git(&["checkout", "--detach", checked.trim()]);
    assert!(!check().status.success());
}

#[test]
fn failed_finalization_does_not_modify_the_file_and_notes_are_version_scoped() {
    let fixture = Fixture::new();
    let changelog = fixture.path().join("CHANGELOG.md");
    let pending = "# Changelog\n\n## [Unreleased]\n\nInitial release.\n";
    fs::write(&changelog, pending).unwrap();
    assert!(
        !fixture
            .run(&["finalize", "v0.1.0", "2026-02-30"])
            .status
            .success()
    );
    assert_eq!(fs::read_to_string(&changelog).unwrap(), pending);
    fixture.passed(&["finalize", "v0.1.0", "2026-10-30"]);
    fixture.passed(&["check", "v0.1.0"]);
    fixture.passed(&["notes", "v0.1.0", "notes"]);
    assert_eq!(
        fs::read_to_string(fixture.path().join("notes")).unwrap(),
        "Initial release.\n"
    );
}

#[cfg(unix)]
#[test]
fn production_ci_status_bridge_reports_only_verified_success() {
    let fixture = Fixture::new();
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let script = merge_script(repository);
    let reporting = &script[script.find("          state=failure").unwrap()..];
    let fake_source = fixture.path().join("fake-gh.rs");
    fs::write(
        &fake_source,
        r#"#![forbid(unsafe_code)]
use std::{env, fs::OpenOptions, io::Write};

fn field(args: &[String], name: &str) -> String {
    let prefix = format!("{name}=");
    args.windows(2)
        .filter(|pair| matches!(pair[0].as_str(), "-f" | "--field" | "--raw-field"))
        .find_map(|pair| pair[1].strip_prefix(&prefix))
        .expect("missing API field")
        .to_owned()
}

fn main() {
    let args: Vec<_> = env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("api") => {
            assert!(args.windows(2).any(|pair| pair == ["--method", "POST"]));
            let endpoint = args.iter().find(|arg| arg.starts_with("repos/")).unwrap();
            let mut log = OpenOptions::new()
                .create(true)
                .append(true)
                .open(env::var_os("STATUS_LOG").unwrap())
                .unwrap();
            writeln!(log, "{}\t{}\t{}\t{endpoint}",
                field(&args, "state"), field(&args, "context"), field(&args, "target_url"))
                .unwrap();
        }
        Some("pr") => {
            assert_eq!(args[1], "merge");
            assert!(args.windows(2).any(|pair| pair == ["--match-head-commit", &env::var("SHA").unwrap()]));
            let mut log = OpenOptions::new().append(true).open(env::var_os("STATUS_LOG").unwrap()).unwrap();
            writeln!(log, "merged").unwrap();
        }
        _ => panic!("unexpected gh arguments: {args:?}"),
    }
}
"#,
    )
    .unwrap();
    let fake_bin = fixture.path().join("bin");
    fs::create_dir(&fake_bin).unwrap();
    let compiled = Command::new("rustc")
        .args(["--edition=2024", "--crate-name", "fake_gh"])
        .arg(&fake_source)
        .arg("-o")
        .arg(fake_bin.join("gh"))
        .output()
        .unwrap();
    assert!(
        compiled.status.success(),
        "{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    let path = std::env::join_paths(
        std::iter::once(fake_bin).chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
    )
    .unwrap();
    let sha = "1111111111111111111111111111111111111111";
    for result in ["success", "failure", "cancelled", "skipped", ""] {
        let log = fixture.path().join("status-log");
        fs::write(&log, "").unwrap();
        let output = Command::new("bash")
            .args(["-euc", reporting])
            .env("PATH", &path)
            .env_remove("GH_TOKEN")
            .env_remove("GITHUB_TOKEN")
            .env("GITHUB_REPOSITORY", "example/project")
            .env("GITHUB_SERVER_URL", "https://github.com")
            .env("GITHUB_RUN_ID", "42")
            .env("SHA", sha)
            .env("PR", "12")
            .env("STATUS_LOG", &log)
            .env("DOCUMENTATION_RESULT", result)
            .current_dir(fixture.path())
            .output()
            .unwrap();
        let success = result == "success";
        assert_eq!(
            output.status.success(),
            success,
            "{result}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let state = if success { "success" } else { "failure" };
        let merged = if success { "merged\n" } else { "" };
        assert_eq!(
            fs::read_to_string(&log).unwrap(),
            format!(
                "{state}\tCI\thttps://github.com/example/project/actions/runs/42\trepos/example/project/statuses/{sha}\n{merged}"
            )
        );
    }
}

fn merge_script(repository: &Path) -> String {
    let workflow =
        fs::read_to_string(repository.join(".github/workflows/release-changelog.yml")).unwrap();
    workflow
        .split_once("      - name: Report checks and merge the checked changelog")
        .unwrap()
        .1
        .split_once("        run: |\n")
        .unwrap()
        .1
        .to_owned()
}
