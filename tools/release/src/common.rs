//! Shared source inventory, notice delivery and receipt writing for the release archives.

use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// The checkout the tool was built from.
pub fn repository() -> Result<PathBuf> {
    resolved(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."))
}

/// An absolute, normalized path. Linux resolves links; Windows keeps the ordinary drive
/// form that compilers and recorded commands use.
pub fn resolved(path: &Path) -> Result<PathBuf> {
    if cfg!(windows) {
        Ok(std::path::absolute(path)?)
    } else {
        match path.canonicalize() {
            Ok(path) => Ok(path),
            Err(_) => Ok(std::path::absolute(path)?),
        }
    }
}

/// Sets a file's or directory's modification time without opening it for writing, so
/// read-only files keep their permissions. On Unix it goes by path, which needs no read
/// permission either.
pub fn set_modified(path: &Path, time: std::time::SystemTime) -> Result<()> {
    #[cfg(unix)]
    {
        use rustix::fs::{AtFlags, CWD, Timespec, Timestamps, UTIME_OMIT, utimensat};
        let since = time.duration_since(std::time::UNIX_EPOCH)?;
        let times = Timestamps {
            last_access: Timespec {
                tv_sec: 0,
                tv_nsec: UTIME_OMIT,
            },
            last_modification: Timespec {
                tv_sec: i64::try_from(since.as_secs())?,
                tv_nsec: since.subsec_nanos().into(),
            },
        };
        utimensat(CWD, path, &times, AtFlags::empty())?;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // FILE_WRITE_ATTRIBUTES, which a read-only file still grants.
        File::options()
            .access_mode(0x100)
            .open(path)?
            .set_modified(time)?;
    }
    Ok(())
}

/// An absolute path with every existing part's links resolved, junctions included on
/// Windows, for a path that may not exist yet.
pub fn resolved_new(path: &Path) -> Result<PathBuf> {
    let absolute = std::path::absolute(path)?;
    let mut existing = absolute.as_path();
    let mut rest = Vec::new();
    while !existing.exists() {
        rest.push(
            existing
                .file_name()
                .ok_or("path has no existing ancestor")?,
        );
        existing = existing.parent().ok_or("path has no existing ancestor")?;
    }
    let mut out = existing.canonicalize()?;
    out.extend(rest.iter().rev());
    Ok(out)
}

/// Whether `path`, which may not exist yet, lies inside `root` once links and junctions
/// are resolved, so a link cannot make a path in the checkout look like an outside one.
pub fn inside(path: &Path, root: &Path) -> Result<bool> {
    Ok(resolved_new(path)?.starts_with(root.canonicalize()?))
}

/// `base` joined with a slash-separated relative path, one component at a time, so the
/// result uses the platform's separator throughout.
pub fn under(base: &Path, relative: &str) -> PathBuf {
    relative
        .split('/')
        .fold(base.to_owned(), |path, part| path.join(part))
}

pub fn sha(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = vec![0; 1 << 16];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(base16ct::lower::encode_string(&digest.finalize()))
}

/// Every path below `root`, directories included, without entering linked directories.
pub fn walk(root: &Path) -> Result<Vec<PathBuf>> {
    let mut found = Vec::new();
    let mut pending = vec![root.to_owned()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory)? {
            let path = entry?.path();
            if fs::symlink_metadata(&path)?.is_dir() {
                pending.push(path.clone());
            }
            found.push(path);
        }
    }
    Ok(found)
}

/// Paths in the archives' order: component by component, ignoring case on Windows.
pub fn archive_order(mut paths: Vec<PathBuf>, root: &Path) -> Vec<PathBuf> {
    let key = |path: &PathBuf| -> Vec<String> {
        path.strip_prefix(root)
            .unwrap_or(path)
            .components()
            .map(|part| {
                let text = part.as_os_str().to_string_lossy();
                if cfg!(windows) {
                    text.to_lowercase()
                } else {
                    text.into_owned()
                }
            })
            .collect()
    };
    paths.sort_by_cached_key(key);
    paths
}

/// A path below `base` with forward slashes.
pub fn relative_name(path: &Path, base: &Path) -> Result<String> {
    let relative = path.strip_prefix(base)?;
    let parts: Vec<String> = relative
        .components()
        .map(|part| part.as_os_str().to_string_lossy().into_owned())
        .collect();
    Ok(parts.join("/"))
}

/// Each file below `root` with its SHA-256, in archive order.
pub fn inventory(root: &Path) -> Result<Map<String, Value>> {
    let mut files = Map::new();
    for path in archive_order(walk(root)?, root) {
        if path.is_file() {
            files.insert(relative_name(&path, root)?, Value::from(sha(&path)?));
        }
    }
    Ok(files)
}

/// Text with the platform's line ends, as the archives have always carried them.
pub fn write_text(path: &Path, text: &str) -> Result<()> {
    let text = if cfg!(windows) {
        text.replace('\n', "\r\n")
    } else {
        text.to_owned()
    };
    fs::write(path, text)?;
    Ok(())
}

/// JSON with two-space indents and every character outside printable ASCII escaped, as
/// the receipts and archive metadata have always been written.
pub fn json_text(value: &Value) -> String {
    let mut out = String::new();
    write_json(&mut out, value, 0);
    out
}

fn write_json(out: &mut String, value: &Value, level: usize) {
    let indent = |out: &mut String, level: usize| out.extend(std::iter::repeat_n(' ', 2 * level));
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(value) => out.push_str(if *value { "true" } else { "false" }),
        Value::Number(number) => out.push_str(&number.to_string()),
        Value::String(text) => write_string(out, text),
        Value::Array(items) if items.is_empty() => out.push_str("[]"),
        Value::Object(members) if members.is_empty() => out.push_str("{}"),
        Value::Array(items) => {
            out.push('[');
            for (at, item) in items.iter().enumerate() {
                out.push_str(if at == 0 { "\n" } else { ",\n" });
                indent(out, level + 1);
                write_json(out, item, level + 1);
            }
            out.push('\n');
            indent(out, level);
            out.push(']');
        }
        Value::Object(members) => {
            out.push('{');
            for (at, (key, item)) in members.iter().enumerate() {
                out.push_str(if at == 0 { "\n" } else { ",\n" });
                indent(out, level + 1);
                write_string(out, key);
                out.push_str(": ");
                write_json(out, item, level + 1);
            }
            out.push('\n');
            indent(out, level);
            out.push('}');
        }
    }
}

fn write_string(out: &mut String, text: &str) {
    out.push('"');
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            ' '..='~' => out.push(ch),
            ch => {
                let mut units = [0; 2];
                for unit in ch.encode_utf16(&mut units) {
                    out.push_str(&format!("\\u{unit:04x}"));
                }
            }
        }
    }
    out.push('"');
}

/// One line of JSON with `, ` and `: ` between items, as the result line prints.
pub fn json_line(value: &Value) -> String {
    match value {
        Value::Array(items) => {
            let items: Vec<String> = items.iter().map(json_line).collect();
            format!("[{}]", items.join(", "))
        }
        Value::Object(members) => {
            let members: Vec<String> = members
                .iter()
                .map(|(key, item)| {
                    let mut text = String::new();
                    write_string(&mut text, key);
                    format!("{text}: {}", json_line(item))
                })
                .collect();
            format!("{{{}}}", members.join(", "))
        }
        value => json_text(value),
    }
}

/// A process's exit code, with a Unix signal reported as its negated number.
pub fn exit_code(status: std::process::ExitStatus) -> i32 {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            return -signal;
        }
    }
    status.code().unwrap_or(1)
}

/// How a logged command ended.
#[derive(Debug, PartialEq)]
pub enum Ended {
    Exit(i32),
    /// Its deadline passed and it was killed.
    Deadline,
    /// An interrupt arrived and it was killed.
    Interrupt,
}

/// Keep failures useful in hosted logs while retaining the complete capture on disk.
/// Read only the last 32 KiB, even if a compiler produced a very large log.
pub fn command_failure(name: &str, log: &Path) -> Box<dyn std::error::Error> {
    let tail = (|| -> io::Result<String> {
        let mut file = File::open(log)?;
        let length = file.metadata()?.len();
        file.seek(SeekFrom::Start(length.saturating_sub(32 * 1024)))?;
        let mut bytes = Vec::new();
        file.take(32 * 1024).read_to_end(&mut bytes)?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    })()
    .unwrap_or_else(|error| format!("Could not read captured output: {error}"));
    format!(
        "{name} failed; full output: {}\nLast captured output (up to 32 KiB):\n{tail}",
        log.display()
    )
    .into()
}

/// Runs a command with both outputs in `log`. A deadline, or `stop` being set by an
/// interrupt, kills the process.
pub fn run_logged(
    command: &[String],
    cwd: Option<&Path>,
    environment: Option<&[(OsString, OsString)]>,
    log: &Path,
    deadline: Option<std::time::Duration>,
    stop: Option<&AtomicBool>,
) -> Result<Ended> {
    let stream = File::create(log)?;
    let mut process = std::process::Command::new(&command[0]);
    process
        .args(&command[1..])
        .stdout(stream.try_clone()?)
        .stderr(stream);
    if let Some(cwd) = cwd {
        process.current_dir(cwd);
    }
    if let Some(environment) = environment {
        process
            .env_clear()
            .envs(environment.iter().map(|(k, v)| (k, v)));
    }
    let mut child = process.spawn()?;
    let started = std::time::Instant::now();
    let stopped = || stop.is_some_and(|stop| stop.load(Ordering::SeqCst));
    loop {
        if let Some(status) = child.try_wait()? {
            // A terminal's Ctrl-C reaches the whole process group, so the command often
            // ends before the flag is seen. It was interrupted, not failed. The handler
            // runs on its own thread, so a command a signal ended gives it a moment.
            #[cfg(unix)]
            if stop.is_some() && std::os::unix::process::ExitStatusExt::signal(&status).is_some() {
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
            if stopped() {
                return Ok(Ended::Interrupt);
            }
            return Ok(Ended::Exit(exit_code(status)));
        }
        let ended = if stopped() {
            Some(Ended::Interrupt)
        } else if deadline.is_some_and(|deadline| started.elapsed() >= deadline) {
            Some(Ended::Deadline)
        } else {
            None
        };
        if let Some(ended) = ended {
            let _ = child.kill();
            child.wait()?;
            return Ok(ended);
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

/// A finished command's exit code and outputs.
pub type Captured = (i32, Vec<u8>, Vec<u8>);

/// Runs a prepared command, feeding it `input` and capturing both outputs. A deadline
/// kills the process and gives `None`.
pub fn capture_command(
    command: &mut std::process::Command,
    input: Option<&[u8]>,
    deadline: std::time::Duration,
) -> Result<Option<Captured>> {
    use std::process::Stdio;
    let mut child = command
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let reader = |mut pipe: Box<dyn Read + Send>| {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = pipe.read_to_end(&mut bytes);
            bytes
        })
    };
    let out = reader(Box::new(child.stdout.take().ok_or("no stdout")?));
    let err = reader(Box::new(child.stderr.take().ok_or("no stderr")?));
    if let (Some(input), Some(mut pipe)) = (input, child.stdin.take()) {
        std::io::Write::write_all(&mut pipe, input)?;
    }
    let started = std::time::Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if started.elapsed() >= deadline {
            let _ = child.kill();
            child.wait()?;
            return Ok(None);
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    let out = out.join().map_err(|_| "stdout reader failed")?;
    let err = err.join().map_err(|_| "stderr reader failed")?;
    Ok(Some((exit_code(status), out, err)))
}

/// Runs a short control command and captures its text. A deadline gives `None`.
pub fn capture(
    command: &[String],
    deadline: std::time::Duration,
) -> Result<Option<(i32, String, String)>> {
    let mut process = std::process::Command::new(&command[0]);
    process.args(&command[1..]);
    Ok(
        capture_command(&mut process, None, deadline)?.map(|(exit, out, err)| {
            (
                exit,
                String::from_utf8_lossy(&out).into_owned(),
                String::from_utf8_lossy(&err).into_owned(),
            )
        }),
    )
}

/// A unique name for an owned container or scope.
pub fn unique_id() -> String {
    use std::hash::{BuildHasher, RandomState};
    let first = RandomState::new().hash_one((std::process::id(), std::time::SystemTime::now()));
    let second = RandomState::new().hash_one(first);
    format!("{first:016x}{second:016x}")
}

/// The package version in a `Cargo.toml`.
pub fn crate_version(manifest: &Path) -> Result<String> {
    let table: toml::Table = toml::from_str(&fs::read_to_string(manifest)?)?;
    Ok(table
        .get("package")
        .and_then(|package| package.get("version"))
        .and_then(toml::Value::as_str)
        .ok_or("Cargo.toml has no package version")?
        .to_owned())
}

/// Copies a file's contents to a new file with default permissions.
pub fn copy_file(origin: &Path, target: &Path) -> Result<()> {
    let mut source = File::open(origin)?;
    let mut destination = File::create(target)?;
    io::copy(&mut source, &mut destination)?;
    Ok(())
}

/// Copies a directory tree into a new directory, following links and keeping each
/// file's permissions and every modification time.
pub fn copy_tree(origin: &Path, target: &Path) -> Result<()> {
    fs::create_dir_all(target.parent().ok_or("tree target has no parent")?)?;
    fs::create_dir(target)?;
    for entry in fs::read_dir(origin)? {
        let entry = entry?;
        let (from, to) = (entry.path(), target.join(entry.file_name()));
        if from.is_dir() {
            copy_tree(&from, &to)?;
        } else {
            fs::copy(&from, &to)?;
            set_modified(&to, fs::metadata(&from)?.modified()?)?;
        }
    }
    // Directories keep their modification times too, which the Linux packages carry.
    #[cfg(unix)]
    set_modified(target, fs::metadata(origin)?.modified()?)?;
    Ok(())
}

/// The checkout paths every release build copies and records.
pub const SOURCE_INPUTS: [&str; 12] = [
    "src",
    "tests",
    "examples",
    "packaging",
    "contrib",
    "plugin",
    "lua",
    "README.md",
    "Cargo.toml",
    "Cargo.lock",
    "LICENSE-MIT",
    "LICENSE-APACHE",
];

/// Copies the source inputs and a build's own extra inputs into `source`.
pub fn copy_source(repo: &Path, source: &Path, extras: &[&str]) -> Result<()> {
    for name in SOURCE_INPUTS.iter().chain(extras) {
        let (origin, target) = (repo.join(name), source.join(name));
        fs::create_dir_all(target.parent().ok_or("source target has no parent")?)?;
        if origin.is_dir() {
            copy_tree(&origin, &target)?;
        } else {
            copy_file(&origin, &target)?;
        }
    }
    Ok(())
}

/// Keeps editor sources and their separate licence beside the executable.
///
/// The Vim and Neovim plugin, at the repository root for plugin managers, becomes the
/// archive's `contrib/vim`, the directory the installers copy into the editors.
pub fn delivery_files(source: &Path, package: &Path) -> Result<()> {
    for name in ["LICENSE-MIT", "LICENSE-APACHE", "README.md"] {
        copy_file(&source.join(name), &package.join(name))?;
    }
    copy_tree(&source.join("contrib"), &package.join("contrib"))?;
    for name in ["plugin", "lua"] {
        copy_tree(&source.join(name), &package.join("contrib/vim").join(name))?;
    }
    Ok(())
}

/// Lists every vendored crate's licence and copies its notices, the Rust standard
/// library's licences and SQLite's public-domain dedication into the package.
pub fn collect_notices(vendor: &Path, toolchain: &Path, package: &Path) -> Result<()> {
    let mut crates: Vec<PathBuf> = fs::read_dir(vendor)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<io::Result<_>>()?;
    crates = archive_order(crates, vendor);
    let mut notices = Vec::new();
    for krate in &crates {
        let table: toml::Table = toml::from_str(&fs::read_to_string(krate.join("Cargo.toml"))?)?;
        let meta = table
            .get("package")
            .ok_or("vendored crate without a package table")?;
        let text = |key: &str| -> Value {
            meta.get(key)
                .and_then(toml::Value::as_str)
                .map_or(Value::Null, Value::from)
        };
        let mut notice = Map::new();
        notice.insert("name".into(), text("name"));
        notice.insert("version".into(), text("version"));
        notice.insert("license".into(), text("license"));
        notice.insert("license_file".into(), text("license-file"));
        notices.push(Value::Object(notice));
        let name = krate.file_name().ok_or("vendored crate without a name")?;
        for path in walk(krate)? {
            let lower = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_lowercase();
            let notice_file = ["license", "copying", "notice", "copyright"]
                .iter()
                .any(|prefix| lower.starts_with(prefix));
            if path.is_file() && notice_file {
                let target = package
                    .join("licenses/dependencies")
                    .join(name)
                    .join(path.strip_prefix(krate)?);
                fs::create_dir_all(target.parent().ok_or("notice without a parent")?)?;
                copy_file(&path, &target)?;
            }
        }
    }
    write_text(
        &package.join("DEPENDENCIES.json"),
        &(json_text(&Value::Array(notices)) + "\n"),
    )?;
    let rust = toolchain.join("share/doc/rust");
    copy_tree(&rust.join("licenses"), &package.join("licenses/rust"))?;
    copy_file(
        &rust.join("COPYRIGHT-library.html"),
        &package.join("licenses/rust/COPYRIGHT-library.html"),
    )?;
    // Bundled SQLite explicitly dedicates its implementation to the public domain.
    let sqlite = crates
        .iter()
        .filter(|krate| {
            krate
                .file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with("libsqlite3-sys-"))
        })
        .map(|krate| krate.join("sqlite3/sqlite3.c"))
        .find(|path| path.is_file())
        .ok_or("vendored SQLite source not found")?;
    let text = String::from_utf8(fs::read(sqlite)?)?
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    let header = text
        .split_once("*/")
        .map_or(text.as_str(), |(head, _)| head);
    write_text(
        &package.join("licenses/SQLITE-NOTICE.txt"),
        &format!("{header}*/\n"),
    )?;
    Ok(())
}

/// Fails unless every recorded source input still has its recorded bytes in both the
/// copy and the checkout.
pub fn unchanged_sources(
    recorded: &Map<String, Value>,
    source: &Path,
    repo: &Path,
    failure: &str,
) -> Result<()> {
    for (name, digest) in recorded {
        let digest = digest.as_str();
        if digest != Some(sha(&source.join(name))?.as_str())
            || digest != Some(sha(&repo.join(name))?.as_str())
        {
            return Err(failure.into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_copy_keeps_examples_and_shared_test_inputs() {
        let root = tempfile::tempdir().unwrap();
        let (repo, source) = (root.path().join("repo"), root.path().join("source"));
        fs::create_dir(&repo).unwrap();
        fs::create_dir(&source).unwrap();
        for name in SOURCE_INPUTS {
            let path = repo.join(name);
            if [
                "src",
                "tests",
                "examples",
                "packaging",
                "contrib",
                "plugin",
                "lua",
            ]
            .contains(&name)
            {
                fs::create_dir(path).unwrap();
            } else {
                fs::write(path, "input").unwrap();
            }
        }
        let inputs = [
            "examples/assessment.rs",
            "examples/support/shared.rs",
            "tests/support/fixture.rs",
            "packaging/check-installed.sh",
            "packaging/templates/PKGBUILD",
        ];
        for name in inputs {
            let path = repo.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, name).unwrap();
        }
        copy_source(&repo, &source, &[]).unwrap();
        for name in inputs {
            assert_eq!(fs::read_to_string(source.join(name)).unwrap(), name);
        }
    }

    #[test]
    fn editor_files_are_delivered_unchanged() {
        let root = tempfile::tempdir().unwrap();
        let (source, package) = (root.path().join("source"), root.path().join("package"));
        fs::create_dir(&source).unwrap();
        fs::create_dir(&package).unwrap();
        for name in [
            "LICENSE-MIT",
            "LICENSE-APACHE",
            "README.md",
            "contrib/emacs/ripscope.el",
            "plugin/ripscope.vim",
            "lua/ripscope.lua",
        ] {
            let path = source.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, name).unwrap();
        }
        delivery_files(&source, &package).unwrap();
        let expected: Map<String, Value> = inventory(&source)
            .unwrap()
            .into_iter()
            .map(|(name, digest)| {
                let delivered = if name.starts_with("plugin/") || name.starts_with("lua/") {
                    format!("contrib/vim/{name}")
                } else {
                    name
                };
                (delivered, digest)
            })
            .collect();
        assert_eq!(expected, inventory(&package).unwrap());
    }

    #[test]
    fn archive_order_compares_whole_components() {
        let root = Path::new("r");
        let sorted = archive_order(
            ["a-b/x", "a/b", "a", "B"]
                .iter()
                .map(|name| root.join(name))
                .collect(),
            root,
        );
        let names: Vec<String> = sorted
            .iter()
            .map(|path| relative_name(path, root).unwrap())
            .collect();
        if cfg!(windows) {
            assert_eq!(names, ["a", "a/b", "a-b/x", "B"]);
        } else {
            assert_eq!(names, ["B", "a", "a/b", "a-b/x"]);
        }
    }

    #[test]
    fn receipts_escape_text_outside_printable_ascii() {
        let value = serde_json::json!({"b": "é\u{7f}\n", "a": [], "c": {}});
        assert_eq!(
            json_text(&value),
            "{\n  \"b\": \"\\u00e9\\u007f\\n\",\n  \"a\": [],\n  \"c\": {}\n}"
        );
    }

    /// A new work path under a link into the checkout resolves into the checkout, so the
    /// builds' guard refuses it.
    #[cfg(unix)]
    #[test]
    fn a_new_path_resolves_through_existing_links() {
        let root = tempfile::tempdir().unwrap();
        let checkout = root.path().join("checkout");
        fs::create_dir(&checkout).unwrap();
        std::os::unix::fs::symlink(&checkout, root.path().join("link")).unwrap();
        let resolved = resolved_new(&root.path().join("link/new/work")).unwrap();
        assert_eq!(resolved, checkout.canonicalize().unwrap().join("new/work"));
        assert!(inside(&root.path().join("link/new/work"), &checkout).unwrap());
        assert!(!inside(&root.path().join("elsewhere"), &checkout).unwrap());
    }

    /// The Windows guard sees through a directory junction into the checkout.
    #[cfg(windows)]
    #[test]
    fn a_new_path_resolves_through_existing_junctions() {
        let root = tempfile::tempdir().unwrap();
        let checkout = root.path().join("checkout");
        fs::create_dir(&checkout).unwrap();
        let junction = root.path().join("junction");
        let status = std::process::Command::new("cmd")
            .arg("/C")
            .arg("mklink")
            .arg("/J")
            .arg(&junction)
            .arg(&checkout)
            .stdout(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(status.success());
        assert!(inside(&junction.join("new").join("work"), &checkout).unwrap());
        assert!(!inside(&root.path().join("elsewhere"), &checkout).unwrap());
    }

    /// Copying keeps a read-only file's modification time without making it writable.
    #[test]
    fn modification_times_are_set_on_read_only_files() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("read-only.txt");
        fs::write(&file, b"text").unwrap();
        let mut permissions = fs::metadata(&file).unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&file, permissions).unwrap();
        let time = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
        set_modified(&file, time).unwrap();
        let metadata = fs::metadata(&file).unwrap();
        assert_eq!(metadata.modified().unwrap(), time);
        let mut permissions = metadata.permissions();
        assert!(permissions.readonly());
        // Windows cannot remove a read-only file with the temporary directory.
        #[allow(clippy::permissions_set_readonly_false)]
        permissions.set_readonly(false);
        fs::set_permissions(&file, permissions).unwrap();
    }

    /// On Unix the time is set by path, so a file without read permission takes it too.
    #[cfg(unix)]
    #[test]
    fn modification_times_are_set_on_unreadable_files() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("unreadable");
        fs::write(&file, b"text").unwrap();
        fs::set_permissions(&file, fs::Permissions::from_mode(0o200)).unwrap();
        let time = std::time::UNIX_EPOCH + std::time::Duration::from_nanos(1_700_000_000_123);
        set_modified(&file, time).unwrap();
        assert_eq!(fs::metadata(&file).unwrap().modified().unwrap(), time);
    }

    #[test]
    fn failure_diagnostic_reads_a_bounded_tail_and_preserves_the_full_log() {
        let root = tempfile::tempdir().unwrap();
        let log = root.path().join("large.log");
        let mut bytes = b"OLD_PREFIX\n".to_vec();
        bytes.extend(vec![b'x'; 64 * 1024]);
        bytes.extend(b"\nFINAL_FAILURE\xff");
        fs::write(&log, &bytes).unwrap();
        let error = command_failure("build", &log).to_string();
        assert!(error.contains("FINAL_FAILURE"));
        assert!(!error.contains("OLD_PREFIX"));
        assert!(error.len() < 32 * 1024 + 512);
        assert_eq!(fs::read(log).unwrap(), bytes);
    }

    /// A stop flag set while the command ends, as a terminal's Ctrl-C does to the whole
    /// process group, records an interrupt rather than the command's own exit.
    #[cfg(unix)]
    #[test]
    fn a_command_ending_with_the_stop_flag_set_was_interrupted() {
        let root = tempfile::tempdir().unwrap();
        let log = root.path().join("command.log");
        let command = ["true".to_owned()];
        let stop = AtomicBool::new(true);
        assert_eq!(
            run_logged(&command, None, None, &log, None, Some(&stop)).unwrap(),
            Ended::Interrupt
        );
        stop.store(false, Ordering::SeqCst);
        assert_eq!(
            run_logged(&command, None, None, &log, None, Some(&stop)).unwrap(),
            Ended::Exit(0)
        );
    }
}
