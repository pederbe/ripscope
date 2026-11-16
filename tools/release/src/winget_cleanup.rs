//! Retire this account's superseded Ripscope submissions before validating a new one.
use crate::winget_submission::{REPO, Result, api, gh, search_prs};
use serde_json::{Value, json};
use std::process::Command;

const PACKAGE: &str = "PederBergan.Ripscope";
const SOURCE: &str = "pederbe/ripscope";

fn version(text: &str) -> Result<[u64; 3]> {
    if !text
        .split('.')
        .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
    {
        return Err("expected a stable numeric version".into());
    }
    let parts = text
        .split('.')
        .map(str::parse)
        .collect::<std::result::Result<Vec<u64>, _>>()?;
    parts
        .try_into()
        .map_err(|_| "expected a stable numeric version".into())
}

fn latest(github: &impl Github) -> Result<String> {
    let release = github.read(&format!("repos/{SOURCE}/releases/latest"))?;
    let tag = release["tag_name"]
        .as_str()
        .ok_or("missing latest release tag")?;
    let selected = tag
        .strip_prefix('v')
        .ok_or("expected a v-prefixed release tag")?;
    version(selected)?;
    if release["draft"] != false || release["prerelease"] != false {
        return Err("expected the latest published stable release".into());
    }
    Ok(selected.to_owned())
}

pub fn require_latest(selected: &str) -> Result<()> {
    check_latest(&LiveGithub, selected)
}

fn check_latest(github: &impl Github, selected: &str) -> Result<()> {
    let current = latest(github)?;
    if selected != current {
        return Err(format!(
            "Ripscope {selected} is superseded, submit the latest release {current}"
        )
        .into());
    }
    Ok(())
}

trait Github {
    fn read(&self, path: &str) -> Result<Value>;
    fn submissions(&self, login: &str) -> Result<Vec<Value>>;
    fn close(&self, number: u64) -> Result<()>;
    fn delete_branch(&self, fork: &str, branch: &str, sha: &str) -> Result<()>;
}

struct LiveGithub;
impl Github for LiveGithub {
    fn read(&self, path: &str) -> Result<Value> {
        api(path)
    }

    fn submissions(&self, login: &str) -> Result<Vec<Value>> {
        // Include closed, unmerged PRs so a retry can finish a failed branch deletion.
        search_prs(&format!("is:unmerged author:{login} {PACKAGE} in:title"))
    }

    fn close(&self, number: u64) -> Result<()> {
        gh(
            &[
                "api",
                "--method",
                "PATCH",
                &format!("repos/{REPO}/pulls/{number}"),
                "--input",
                "-",
            ],
            Some(&json!({"state":"closed"})),
        )?;
        Ok(())
    }

    fn delete_branch(&self, fork: &str, branch: &str, sha: &str) -> Result<()> {
        let token = std::env::var("WINGET_FORK_TOKEN")
            .map_err(|_| "WINGET_FORK_TOKEN is required to delete submission branches")?;
        delete_branch(
            &format!("https://github.com/{fork}.git"),
            branch,
            sha,
            &token,
        )
    }
}

fn delete_branch(repository: &str, branch: &str, sha: &str, token: &str) -> Result<()> {
    let output = Command::new("git")
        .args([
            "-c",
            "credential.helper=",
            "-c",
            "credential.helper=!gh auth git-credential",
            "push",
            "--no-follow-tags",
            &format!("--force-with-lease=refs/heads/{branch}:{sha}"),
            repository,
            &format!(":refs/heads/{branch}"),
        ])
        .env("GH_TOKEN", token)
        .output()?;
    if !output.status.success() {
        return Err(format!(
            "submission branch deletion failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    Ok(())
}

fn submitted_version(title: &str) -> Option<&str> {
    let title = title
        .strip_prefix("New package: ")
        .or_else(|| title.strip_prefix("New version: "))?;
    let (package, selected) = title.split_once(" version ")?;
    package
        .eq_ignore_ascii_case(PACKAGE)
        .then_some(selected.strip_prefix('v').unwrap_or(selected))
}

fn retire(github: &impl Github, current: &str) -> Result<()> {
    let current = version(current)?;
    let user = github.read("user")?;
    let login = user["login"]
        .as_str()
        .ok_or("missing authenticated login")?;
    for item in github.submissions(login)? {
        let Some(selected) = submitted_version(item["title"].as_str().unwrap_or("")) else {
            continue;
        };
        let Ok(older) = version(selected) else {
            continue;
        };
        if older >= current {
            continue;
        }
        let number = item["number"].as_u64().ok_or("missing PR number")?;
        let endpoint = format!("repos/{REPO}/pulls/{number}");
        let pr = github.read(&endpoint)?;
        if pr["merged"] != false
            || pr["user"]["login"] != login
            || pr["base"]["repo"]["full_name"] != REPO
        {
            continue;
        }
        // Re-read the title rather than trusting the search index before a public write.
        if submitted_version(pr["title"].as_str().unwrap_or("")) != Some(selected) {
            return Err("submission title changed, inspect before retrying".into());
        }
        let head = &pr["head"];
        if head["repo"]["owner"]["login"] != login {
            continue;
        }
        let fork = head["repo"]["full_name"]
            .as_str()
            .ok_or("missing submission fork")?;
        let branch = head["ref"].as_str().ok_or("missing submission branch")?;
        let sha = head["sha"].as_str().ok_or("missing submission commit")?;
        if head["repo"]["fork"] != true || head["repo"]["default_branch"] == branch {
            return Err("refusing to delete a non-submission branch".into());
        }
        let prefix = format!("manifests/p/PederBergan/Ripscope/{selected}/");
        let files = github.read(&format!("{endpoint}/files?per_page=100"))?;
        let files = files.as_array().ok_or("missing submission files")?;
        if pr["changed_files"] != 3
            || files.len() != 3
            || files.iter().any(|file| {
                file["status"] != "added"
                    || !file["filename"]
                        .as_str()
                        .is_some_and(|path| path.starts_with(&prefix) && path.ends_with(".yaml"))
            })
        {
            return Err("older PR includes other changes, inspect before retiring it".into());
        }
        if pr["state"] == "open" {
            github.close(number)?;
        }
        let closed = github.read(&endpoint)?;
        if closed["state"] != "closed" || closed["merged"] != false || closed["head"]["sha"] != sha
        {
            return Err(
                "submission changed or did not close, inspect before deleting its branch".into(),
            );
        }
        // A missing ref is a completed retry. Other lookup failures must remain failures.
        match github.read(&format!("repos/{fork}/git/ref/heads/{branch}")) {
            Ok(reference) if reference["object"]["sha"] == sha => {
                github.delete_branch(fork, branch, sha)?
            }
            Ok(_) => return Err("submission branch changed, refusing to delete it".into()),
            Err(error) if error.to_string().contains("HTTP 404") => {}
            Err(error) => return Err(error),
        }
        println!("Retired https://github.com/{REPO}/pull/{number} and its submission branch.");
    }
    Ok(())
}

pub fn run(args: &[String]) -> Result<()> {
    if !args.is_empty() {
        return Err("usage: ripscope-release winget-retire".into());
    }
    retire(&LiveGithub, &latest(&LiveGithub)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};

    struct Fixture {
        pr: RefCell<Value>,
        items: Vec<Value>,
        files: Value,
        branch: RefCell<Option<String>>,
        fail_delete: Cell<bool>,
        reference_error: Option<&'static str>,
        writes: RefCell<Vec<String>>,
    }

    impl Fixture {
        fn new(selected: &str) -> Self {
            let title = format!("New package: {PACKAGE} version {selected}");
            Self {
                pr: RefCell::new(json!({
                    "title":title, "state":"open", "merged":false, "changed_files":3,
                    "user":{"login":"maintainer"}, "base":{"repo":{"full_name":REPO}},
                    "head":{"sha":"checked-sha", "ref":"submission", "repo":{
                        "full_name":"maintainer/winget-pkgs", "owner":{"login":"maintainer"},
                        "fork":true, "default_branch":"master"
                    }}
                })),
                items: vec![json!({"number":7, "title":title})],
                files: Value::Array(["Ripscope.yaml", "Ripscope.installer.yaml", "Ripscope.locale.en-US.yaml"].map(|name| json!({
                    "filename":format!("manifests/p/PederBergan/Ripscope/{selected}/{name}"), "status":"added"
                })).into()),
                branch: RefCell::new(Some("checked-sha".to_owned())),
                fail_delete: Cell::new(false), reference_error: None,
                writes: RefCell::new(Vec::new()),
            }
        }
    }

    impl Github for Fixture {
        fn read(&self, path: &str) -> Result<Value> {
            match path {
                "repos/pederbe/ripscope/releases/latest" => {
                    Ok(json!({"tag_name":"v0.3.1", "draft":false, "prerelease":false}))
                }
                "user" => Ok(json!({"login":"maintainer"})),
                "repos/microsoft/winget-pkgs/pulls/7" => Ok(self.pr.borrow().clone()),
                "repos/microsoft/winget-pkgs/pulls/7/files?per_page=100" => Ok(self.files.clone()),
                "repos/maintainer/winget-pkgs/git/ref/heads/submission" => {
                    if let Some(error) = self.reference_error {
                        return Err(error.into());
                    }
                    self.branch
                        .borrow()
                        .as_ref()
                        .map(|sha| json!({"object":{"sha":sha}}))
                        .ok_or_else(|| "HTTP 404".into())
                }
                _ => panic!("unexpected read {path}"),
            }
        }
        fn submissions(&self, login: &str) -> Result<Vec<Value>> {
            assert_eq!(login, "maintainer");
            Ok(self.items.clone())
        }
        fn close(&self, number: u64) -> Result<()> {
            assert_eq!(number, 7);
            self.writes.borrow_mut().push("close".into());
            self.pr.borrow_mut()["state"] = json!("closed");
            Ok(())
        }
        fn delete_branch(&self, fork: &str, branch: &str, sha: &str) -> Result<()> {
            assert_eq!(
                (fork, branch, sha),
                ("maintainer/winget-pkgs", "submission", "checked-sha")
            );
            if self.fail_delete.replace(false) {
                return Err("branch write denied".into());
            }
            assert_eq!(self.pr.borrow()["state"], "closed");
            self.writes.borrow_mut().push("delete".into());
            self.branch.replace(None);
            Ok(())
        }
    }

    #[test]
    fn closes_older_submission_and_deletes_only_its_branch() {
        let fixture = Fixture::new("0.2.7");
        retire(&fixture, "0.3.1").unwrap();
        assert_eq!(*fixture.writes.borrow(), ["close", "delete"]);
    }

    #[test]
    fn rejects_a_superseded_submission_before_any_writes() {
        let fixture = Fixture::new("0.2.7");
        assert!(
            check_latest(&fixture, "0.2.7")
                .unwrap_err()
                .to_string()
                .contains("latest release 0.3.1")
        );
        check_latest(&fixture, "0.3.1").unwrap();
        assert!(fixture.writes.borrow().is_empty());
    }

    #[test]
    fn git_deletion_lease_preserves_a_branch_that_advanced() {
        let root = tempfile::tempdir().unwrap();
        let git = |args: &[&str]| {
            let output = Command::new("git")
                .args(args)
                .current_dir(root.path())
                .output()
                .unwrap();
            assert!(output.status.success(), "{args:?}: {output:?}");
            String::from_utf8(output.stdout).unwrap().trim().to_owned()
        };
        git(&["init", "--bare", "."]);
        let tree = git(&["mktree"]);
        let commit = |message: &str| {
            git(&[
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "commit-tree",
                &tree,
                "-m",
                message,
            ])
        };
        let checked = commit("checked");
        let newer = commit("newer");
        git(&["update-ref", "refs/heads/submission", &newer]);
        let repository = root.path().to_str().unwrap();
        assert!(delete_branch(repository, "submission", &checked, "fixture-token").is_err());
        assert_eq!(git(&["rev-parse", "refs/heads/submission"]), newer);
        delete_branch(repository, "submission", &newer, "fixture-token").unwrap();
        assert!(
            git(&[
                "for-each-ref",
                "--format=%(refname)",
                "refs/heads/submission"
            ])
            .is_empty()
        );
    }

    #[test]
    fn keeps_current_and_newer_submissions_and_unrelated_packages() {
        for selected in ["0.3.1", "0.4.0", "0.10.0", "0.3.1-beta"] {
            let fixture = Fixture::new(selected);
            retire(&fixture, "0.3.1").unwrap();
            assert!(fixture.writes.borrow().is_empty());
        }
        let mut fixture = Fixture::new("0.2.7");
        fixture.items[0]["title"] = json!("New package: PederBergan.Ripscope.Extra version 0.2.7");
        retire(&fixture, "0.3.1").unwrap();
        assert!(fixture.writes.borrow().is_empty());
        assert!(version("0.9.9").unwrap() < version("0.10.0").unwrap());
    }

    #[test]
    fn leaves_merged_and_other_authors_submissions_alone() {
        for pointer in ["/merged", "/user/login", "/head/repo/owner/login"] {
            let fixture = Fixture::new("0.2.7");
            *fixture.pr.borrow_mut().pointer_mut(pointer).unwrap() = if pointer == "/merged" {
                json!(true)
            } else {
                json!("someone-else")
            };
            retire(&fixture, "0.3.1").unwrap();
            assert!(fixture.writes.borrow().is_empty());
        }
    }

    #[test]
    fn refuses_mixed_changes_and_default_branch_before_closing() {
        let mut fixture = Fixture::new("0.2.7");
        fixture.files[0]["filename"] = json!("README.md");
        assert!(retire(&fixture, "0.3.1").is_err());
        assert!(fixture.writes.borrow().is_empty());
        let fixture = Fixture::new("0.2.7");
        fixture.pr.borrow_mut()["head"]["repo"]["default_branch"] = json!("submission");
        assert!(retire(&fixture, "0.3.1").is_err());
        assert!(fixture.writes.borrow().is_empty());
    }

    #[test]
    fn retry_finishes_failed_branch_deletion_without_reclosing() {
        let fixture = Fixture::new("0.2.7");
        fixture.fail_delete.set(true);
        assert!(retire(&fixture, "0.3.1").is_err());
        assert_eq!(*fixture.writes.borrow(), ["close"]);
        retire(&fixture, "0.3.1").unwrap();
        assert_eq!(*fixture.writes.borrow(), ["close", "delete"]);
        retire(&fixture, "0.3.1").unwrap();
        assert_eq!(*fixture.writes.borrow(), ["close", "delete"]);
    }

    #[test]
    fn changed_branch_and_failed_lookups_are_never_deleted() {
        let fixture = Fixture::new("0.2.7");
        fixture.branch.replace(Some("new-work".into()));
        assert!(retire(&fixture, "0.3.1").is_err());
        assert_eq!(*fixture.writes.borrow(), ["close"]);
        let mut fixture = Fixture::new("0.2.7");
        fixture.reference_error = Some("HTTP 403");
        assert!(retire(&fixture, "0.3.1").is_err());
        assert_eq!(*fixture.writes.borrow(), ["close"]);
    }
}
