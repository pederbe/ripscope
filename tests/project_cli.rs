//! `ripscope --project PATH`, which the editor integrations ask for a file's Project: its
//! root, its index in the cache directory, the selection arguments and whether the
//! index is published (ADR 0014).
use serde_json::Value;
use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn ripscope(cwd: &Path, home: &Path, args: &[&str], input: Option<&str>) -> Output {
    ripscope_with(cwd, home, args, input, &[])
}

/// ripscope in a profile of its own at HOME, with the EXTRA variables set too.
fn ripscope_with(
    cwd: &Path,
    home: &Path,
    args: &[&str],
    input: Option<&str>,
    extra: &[(&str, &Path)],
) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_ripscope"));
    command
        .current_dir(cwd)
        .args(args)
        .env_remove("RIPSCOPE_CACHE_DIR")
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("XDG_CACHE_HOME", home.join("cache"))
        .env("LOCALAPPDATA", home.join("local"));
    for (name, value) in extra {
        command.env(name, value);
    }
    match input {
        None => command.output().unwrap(),
        Some(input) => {
            use std::io::Write;
            let mut child = command
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .unwrap();
            child
                .stdin
                .take()
                .unwrap()
                .write_all(input.as_bytes())
                .unwrap();
            child.wait_with_output().unwrap()
        }
    }
}

fn project(cwd: &Path, home: &Path, path: &Path) -> Value {
    let output = ripscope(cwd, home, &["--project", path.to_str().unwrap()], None);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn slashes(path: &Path) -> String {
    path.to_str().unwrap().replace('\\', "/")
}

fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item.as_str().unwrap().to_owned())
        .collect()
}

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

#[test]
fn a_repository_project_is_indexed_recursively_in_the_cache() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let root = home.join("src/tool");
    fs::create_dir_all(root.join(".git")).unwrap();
    write(&root.join("main.c"), "int main(void) { return greet(); }\n");
    write(
        &root.join("lib/deep/greet.c"),
        "int greet(void) { return 1; }\n",
    );
    let reported = project(&home, &home, &root.join("main.c"));
    assert_eq!(reported["root"], slashes(&root));
    assert_eq!(strings(&reported["selection"]), ["-k", "-R"]);
    assert_eq!(reported["published"], false);
    let index = reported["index"].as_str().unwrap().to_owned();
    let cache = if cfg!(windows) {
        home.join("local/ripscope")
    } else {
        home.join("cache/ripscope")
    };
    assert!(cache.is_dir(), "the cache directory is made");
    assert!(index.starts_with(&slashes(&cache)), "{index}");
    assert!(index.contains("/tool-"), "{index}");
    assert!(!Path::new(&index).exists());

    // A session in the root with the selection arguments builds the right index.
    let selection = strings(&reported["selection"]);
    let mut session = vec!["-l", "--json"];
    session.extend(selection.iter().map(String::as_str));
    session.extend(["-f", &index]);
    let output = ripscope(&root, &home, &session, Some("1greet\nq\n"));
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("lib/deep/greet.c"),
        "{output:?}"
    );
    assert_eq!(
        project(&home, &home, &root.join("main.c"))["published"],
        true
    );

    // An index whose first build stopped before publishing is not published.
    rusqlite::Connection::open(&index)
        .unwrap()
        .execute_batch(
            "UPDATE meta SET value = 0 WHERE key = 'revision';
             DELETE FROM files; DELETE FROM names; DELETE FROM units;",
        )
        .unwrap();
    assert_eq!(
        project(&home, &home, &root.join("main.c"))["published"],
        false
    );
    let output = ripscope(&root, &home, &session, Some("1greet\nq\n"));
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("lib/deep/greet.c"),
        "{output:?}"
    );
}

#[test]
fn a_file_list_project_selects_its_list() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let root = home.join("kernel");
    write(&root.join("cscope.files"), "src/main.c\n");
    write(&root.join("src/main.c"), "int listed;\n");
    write(&root.join("extra/other.c"), "int unlisted;\n");
    let reported = project(&root.join("src"), &home, Path::new("main.c"));
    assert_eq!(reported["root"], slashes(&root));
    assert_eq!(
        strings(&reported["selection"]),
        ["-k", "-i", "cscope.files"]
    );
    let index = reported["index"].as_str().unwrap().to_owned();
    let selection = strings(&reported["selection"]);
    let mut build = vec!["-b"];
    build.extend(selection.iter().map(String::as_str));
    build.extend(["-f", &index]);
    assert!(ripscope(&root, &home, &build, None).status.success());
    let unlisted = ripscope(&root, &home, &["-dL", "-f", &index, "-1", "unlisted"], None);
    assert!(unlisted.stdout.is_empty(), "{unlisted:?}");
    let listed = ripscope(&root, &home, &["-dL", "-f", &index, "-1", "listed"], None);
    assert!(String::from_utf8_lossy(&listed.stdout).contains("src/main.c"));
}

#[test]
fn an_unmarked_file_outside_the_current_directory_is_its_own_project() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let stray = temp.path().join("stray");
    write(&stray.join("one.c"), "int one;\n");
    fs::create_dir_all(&home).unwrap();
    // From the home directory, which is never a project by default.
    let reported = project(&home, &home, &stray.join("one.c"));
    assert_eq!(reported["root"], slashes(&stray));
    let reported = project(&home, &home, &home.join("top.c"));
    assert_eq!(reported["root"], slashes(&home));
    assert_eq!(strings(&reported["selection"]), ["-k"]);
}

#[test]
fn ripscope_cache_dir_names_the_cache_itself() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let tree = temp.path().join("tree");
    let chosen = temp.path().join("chosen");
    write(&tree.join("cscope.files"), "a.c\n");
    write(&tree.join("a.c"), "int a;\n");
    fs::create_dir_all(&home).unwrap();
    let output = ripscope_with(
        &tree,
        &home,
        &["--project", tree.join("a.c").to_str().unwrap()],
        None,
        &[("RIPSCOPE_CACHE_DIR", &chosen)],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let reported: Value = serde_json::from_slice(&output.stdout).unwrap();
    let index = reported["index"].as_str().unwrap();
    assert_eq!(
        Path::new(index).parent().map(slashes),
        Some(slashes(&chosen))
    );
}

/// Builds PROJECT's index as an editor does, with the selection `--project` reports.
fn build(root: &Path, home: &Path, reported: &Value) {
    let mut args = vec!["-b"];
    let selection = strings(&reported["selection"]);
    args.extend(selection.iter().map(String::as_str));
    args.extend(["-f", reported["index"].as_str().unwrap()]);
    let output = ripscope(root, home, &args, None);
    assert!(output.status.success(), "{output:?}");
}

#[test]
fn an_index_built_before_its_list_changed_is_unpublished() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let root = home.join("tool");
    fs::create_dir_all(root.join(".git")).unwrap();
    write(&root.join("main.c"), "int main(void) { return 0; }\n");
    write(&root.join("lib/extra.c"), "int extra;\n");
    let file = root.join("main.c");
    let reported = project(&home, &home, &file);
    build(&root, &home, &reported);
    assert_eq!(project(&home, &home, &file)["published"], true);

    // A list added at the root: the index still discovers, so it is built again.
    write(&root.join("cscope.files"), "main.c\n");
    let reported = project(&home, &home, &file);
    assert_eq!(
        strings(&reported["selection"]),
        ["-k", "-i", "cscope.files"]
    );
    assert_eq!(reported["published"], false);
    build(&root, &home, &reported);
    assert_eq!(project(&home, &home, &file)["published"], true);

    // The list removed again: the index still reads it, so it is built again.
    fs::remove_file(root.join("cscope.files")).unwrap();
    let reported = project(&home, &home, &file);
    assert_eq!(strings(&reported["selection"]), ["-k", "-R"]);
    assert_eq!(reported["published"], false);
    build(&root, &home, &reported);
    let reported = project(&home, &home, &file);
    assert_eq!(reported["published"], true);
    let index = reported["index"].as_str().unwrap();
    let extra = ripscope(&root, &home, &["-dL", "-f", index, "-0", "extra"], None);
    assert!(
        String::from_utf8_lossy(&extra.stdout).contains("lib/extra.c"),
        "{extra:?}"
    );
}

#[test]
fn a_file_in_a_new_directory_belongs_to_the_nearest_existing_one() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let stray = temp.path().join("stray");
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&stray).unwrap();
    // Nothing marks a project and the file is outside the current directory.
    let reported = project(&home, &home, &stray.join("new/deeper/file.c"));
    assert_eq!(reported["root"], slashes(&stray));
}

#[test]
fn the_emacs_package_has_the_crate_version() {
    // The build hooks install the executable of the version the checkout names, which
    // Emacs's package manager reads from this header.
    let package = Path::new(env!("CARGO_MANIFEST_DIR")).join("contrib/emacs/ripscope.el");
    let text = fs::read_to_string(package).unwrap();
    let version = text
        .lines()
        .find_map(|line| line.strip_prefix(";; Version: "))
        .map(str::trim);
    assert_eq!(version, Some(env!("CARGO_PKG_VERSION")));
}

#[cfg(unix)]
#[test]
fn a_project_reached_through_a_link_shares_its_index() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let physical = temp.path().join("disk/tool");
    fs::create_dir_all(physical.join(".git")).unwrap();
    write(&physical.join("main.c"), "int main(void) { return 0; }\n");
    fs::create_dir_all(&home).unwrap();
    let logical = home.join("tool");
    std::os::unix::fs::symlink(&physical, &logical).unwrap();
    let through_link = project(&home, &home, &logical.join("main.c"));
    let direct = project(&home, &home, &physical.join("main.c"));
    // Each keeps the path the editor named, and both name one index.
    assert_eq!(through_link["root"], slashes(&logical));
    assert_eq!(direct["root"], slashes(&physical));
    assert_eq!(through_link["index"], direct["index"]);
    build(&physical, &home, &direct);
    assert_eq!(
        project(&home, &home, &logical.join("main.c"))["published"],
        true
    );
}

#[test]
fn build_options_name_the_index_and_sources_published_is_judged_by() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let root = home.join("tool");
    fs::create_dir_all(root.join(".git")).unwrap();
    write(&root.join("main.c"), "int main(void) { return 0; }\n");
    write(&root.join("files"), "main.c\n");
    let file = root.join("main.c");
    let file = file.to_str().unwrap();
    let ask = |extra: &[&str]| {
        let mut args = vec!["--project", file];
        args.extend(extra);
        let output = ripscope(&home, &home, &args, None);
        assert!(output.status.success(), "{output:?}");
        serde_json::from_slice::<Value>(&output.stdout).unwrap()
    };
    // The index is named as an index build in the root names it.
    let own = ask(&["-f", "own.db", "-k", "-i", "files"]);
    assert_eq!(own["index"], slashes(&root.join("own.db")));
    assert_eq!(own["root"], slashes(&root));
    // An index file that holds nothing published is not published.
    fs::write(root.join("own.db"), "").unwrap();
    assert_eq!(
        ask(&["-f", "own.db", "-k", "-i", "files"])["published"],
        false
    );
    fs::remove_file(root.join("own.db")).unwrap();
    let built = ripscope(&root, &home, &["-bk", "-f", "own.db", "-i", "files"], None);
    assert!(built.status.success(), "{built:?}");
    assert_eq!(
        ask(&["-f", "own.db", "-k", "-i", "files"])["published"],
        true
    );
    // Other sources than the index was built with: not published.
    assert_eq!(ask(&["-f", "own.db", "-k", "-R"])["published"], false);
    assert_eq!(
        ask(&["-f", "own.db", "-k", "--", "main.c"])["published"],
        false
    );
    // An index alone is judged by the project's own selection, discovery here.
    assert_eq!(ask(&["-f", "own.db"])["published"], false);
    let built = ripscope(&root, &home, &["-bk", "-R", "-f", "own.db"], None);
    assert!(built.status.success(), "{built:?}");
    assert_eq!(ask(&["-f", "own.db"])["published"], true);
}
