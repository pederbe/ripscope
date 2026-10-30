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
fn publication_gate_rejects_a_check_only_rerun_of_an_existing_release() {
    let fixture = Fixture::new();
    let started = "2026-10-30T10:00:00Z";
    let completed = "2026-10-30T10:10:00Z";
    assert!(
        !fixture
            .run(&[
                "publication-window",
                "2026-10-29T10:05:00Z",
                started,
                completed
            ])
            .status
            .success()
    );
    fixture.passed(&[
        "publication-window",
        "2026-10-30T10:05:00Z",
        started,
        completed,
    ]);
    assert!(
        !fixture
            .run(&["publication-window", "", started, completed])
            .status
            .success()
    );
    assert!(
        !fixture
            .run(&[
                "publication-window",
                "2026-10-30T10:11:00Z",
                started,
                completed
            ])
            .status
            .success()
    );
}

#[test]
fn dispatched_ci_scope_guard_rejects_product_changes() {
    let fixture = Fixture::new();
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let workflow =
        fs::read_to_string(repository.join(".github/workflows/release-changelog-check.yml"))
            .unwrap();
    let body = workflow.split_once("        run: |\n").unwrap().1;
    let script = body
        .lines()
        .take_while(|line| line.starts_with("          "))
        .map(|line| &line[10..])
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(fixture.path().join("CHANGELOG.md"), "Previous notes.\n").unwrap();
    fixture.commit("Initial notes");
    fixture.git(&["remote", "add", "origin", fixture.path().to_str().unwrap()]);
    fixture.git(&["checkout", "-b", "contrib/release-changelog-v0.2.0"]);
    fs::write(fixture.path().join("CHANGELOG.md"), "Published notes.\n").unwrap();
    fixture.commit("Record notes");
    let check = |branch: &str| {
        Command::new("bash")
            .args(["-c", &script])
            .env("GITHUB_REF_NAME", branch)
            .current_dir(fixture.path())
            .output()
            .unwrap()
    };
    let output = check("contrib/release-changelog-v0.2.0");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!check("contrib/unrelated").status.success());
    fs::write(fixture.path().join("source.rs"), "changed product code\n").unwrap();
    fixture.commit("Change product code");
    assert!(!check("contrib/release-changelog-v0.2.0").status.success());
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
