//! Build and smoke a local Linux candidate using pinned, offline containers.

use crate::archive;
use crate::common::{
    Ended, Result, capture, collect_notices, command_failure, copy_file, copy_source,
    crate_version, delivery_files, inside, inventory, json_line, json_text, repository,
    resolved_new, run_logged, sha, unchanged_sources, unique_id, write_text,
};
use regex::Regex;
use serde_json::{Map, Value, json};
use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

const USAGE: &str =
    "ripscope-release linux --work NEW_DIR --toolchain RUST_TOOLCHAIN --mold-archive MOLD_TAR_GZ";

/// Set by an interrupt: the running command stops, and the build cleans up what it owns
/// before it exits.
static INTERRUPTED: AtomicBool = AtomicBool::new(false);

/// The build's own inputs, recorded beside the product source.
const EXTRAS: [&str; 6] = [
    "docs/getting-started/installation.md",
    "tools/release/Cargo.toml",
    "tools/release/Cargo.lock",
    "tools/release/src",
    "tools/release/linux-release-profile.json",
    "scripts/linux-smoke.sh",
];

/// Accepts the ELF runtime only when it asks for nothing beyond the glibc 2.28 base
/// system, baseline x86-64, the standard interpreter and the pinned Mold linker.
pub fn inspect_elf(
    dynamic: &str,
    versions: &str,
    notes: &str,
    comments: &str,
    header: &str,
    interpreter: &str,
) -> Result<Value> {
    let needed: Vec<String> = Regex::new(r"\(NEEDED\).*?\[(.*?)\]")?
        .captures_iter(dynamic)
        .map(|found| found[1].to_owned())
        .collect();
    let allowed = [
        "libgcc_s.so.1",
        "libpthread.so.0",
        "libm.so.6",
        "libdl.so.2",
        "libc.so.6",
        "librt.so.1",
        "ld-linux-x86-64.so.2",
    ];
    if needed.is_empty()
        || needed.iter().any(|name| !allowed.contains(&name.as_str()))
        || Regex::new(r"\((?:RPATH|RUNPATH)\)")?.is_match(dynamic)
    {
        return Err("Unexpected ELF runtime dependency or search path".into());
    }
    let glibc = Regex::new(r"GLIBC_([0-9]+(?:\.[0-9]+)+)")?
        .captures_iter(versions)
        .map(|found| {
            found[1]
                .split('.')
                .map(str::parse::<u64>)
                .collect::<std::result::Result<Vec<_>, _>>()
        })
        .collect::<std::result::Result<std::collections::BTreeSet<_>, _>>()?;
    let highest = glibc.last().cloned().unwrap_or_default();
    if glibc.is_empty() || highest > vec![2, 28] || versions.contains("GLIBC_PRIVATE") {
        return Err("ELF exceeds the accepted glibc 2.28 floor".into());
    }
    if ["x86-64-v2", "x86-64-v3", "x86-64-v4"]
        .iter()
        .any(|level| notes.contains(level))
    {
        return Err("ELF advertises a CPU requirement above baseline x86-64".into());
    }
    if !header.contains("Advanced Micro Devices X86-64") || !header.contains("ELF64") {
        return Err("Not an x86-64 ELF64 artifact".into());
    }
    if !interpreter.contains("/lib64/ld-linux-x86-64.so.2") || !comments.contains("mold 3.0.0") {
        return Err("Unexpected interpreter or missing pinned Mold identification".into());
    }
    let maximum: Vec<String> = highest.iter().map(u64::to_string).collect();
    Ok(json!({
        "needed": needed,
        "maximum_glibc_symbol": maximum.join("."),
        "cpu": "explicit baseline Rust/C flags; no elevated ELF ISA requirement",
    }))
}

/// Native reference commands require GNU's process-group deadline semantics.
/// Ubuntu also provides its GNU utilities under names such as `gnutimeout`.
fn reference_timeout(path: &OsStr) -> Result<PathBuf> {
    for name in ["timeout", "gnutimeout"] {
        let candidate = std::env::split_paths(path)
            .map(|directory| directory.join(name))
            .find(|candidate| candidate.is_file());
        let Some(candidate) = candidate else {
            continue;
        };
        let command = [text(&candidate), "--version".to_owned()];
        let Ok(Some((0, stdout, _))) = capture(&command, Duration::from_secs(5)) else {
            continue;
        };
        if stdout
            .lines()
            .next()
            .is_some_and(|line| line.contains("(GNU coreutils)"))
        {
            return Ok(fs::canonicalize(candidate)?);
        }
    }
    Err(
        "Native reference tests require GNU timeout; install GNU coreutils (gnutimeout on Ubuntu)"
            .into(),
    )
}

/// Stage the selected executable only for child processes, leaving the host unchanged.
fn reference_tools(path: &OsStr, directory: &Path) -> Result<PathBuf> {
    let provider = reference_timeout(path)?;
    fs::create_dir(directory)?;
    fs::copy(&provider, directory.join("timeout"))?;
    Ok(provider)
}

struct Options {
    work: PathBuf,
    toolchain: PathBuf,
    mold_archive: PathBuf,
}

fn options(args: &[String]) -> Result<Options> {
    let (mut work, mut toolchain, mut mold) = (None, None, None);
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        if arg == "--help" || arg == "-h" {
            println!("{USAGE}");
            std::process::exit(0);
        }
        let value = PathBuf::from(args.next().ok_or_else(|| format!("{arg} needs a value"))?);
        match arg.as_str() {
            "--work" => work = Some(value),
            "--toolchain" => toolchain = Some(value),
            "--mold-archive" => mold = Some(value),
            _ => return Err(format!("unknown option {arg}\n{USAGE}").into()),
        }
    }
    let required = |value: Option<PathBuf>, name: &str| {
        value.ok_or_else(|| format!("{name} is required\n{USAGE}"))
    };
    Ok(Options {
        work: required(work, "--work")?,
        toolchain: required(toolchain, "--toolchain")?,
        mold_archive: required(mold, "--mold-archive")?,
    })
}

/// The build's receipt, saved after every change, and its command logs.
struct Build {
    work: PathBuf,
    logs: PathBuf,
    receipt: Map<String, Value>,
}

impl Build {
    fn save(&self) -> Result<()> {
        write_text(
            &self.work.join("receipt.json"),
            &(json_text(&Value::Object(self.receipt.clone())) + "\n"),
        )
    }

    fn commands(&mut self) -> &mut Vec<Value> {
        match self.receipt.get_mut("commands") {
            Some(Value::Array(commands)) => commands,
            _ => unreachable!("the receipt always lists its commands"),
        }
    }

    fn last_command(&mut self) -> &mut Map<String, Value> {
        match self.commands().last_mut() {
            Some(Value::Object(command)) => command,
            _ => unreachable!("a command was just recorded"),
        }
    }

    /// Runs a recorded command with its output in `logs/<name>.log`, returning that log.
    /// An interrupt or the deadline stops it, and the receipt records which.
    fn run(
        &mut self,
        name: &str,
        command: &[String],
        cwd: Option<&Path>,
        seconds: u64,
        environment: Option<&[(OsString, OsString)]>,
    ) -> Result<String> {
        self.commands()
            .push(json!({"name": name, "args": command, "deadline_seconds": seconds}));
        self.save()?;
        let log = self.logs.join(format!("{name}.log"));
        let ended = if INTERRUPTED.load(Ordering::SeqCst) {
            Ended::Interrupt
        } else {
            run_logged(
                command,
                cwd,
                environment,
                &log,
                Some(Duration::from_secs(seconds)),
                Some(&INTERRUPTED),
            )?
        };
        let (incomplete, error) = match ended {
            Ended::Exit(exit) => {
                self.last_command().insert("exit".into(), json!(exit));
                self.save()?;
                if exit != 0 {
                    return Err(command_failure(name, &log));
                }
                return Ok(String::from_utf8_lossy(&fs::read(&log)?).into_owned());
            }
            Ended::Deadline => (
                "TimeoutExpired",
                format!("{name} exceeded its {seconds}-second deadline"),
            ),
            Ended::Interrupt => ("KeyboardInterrupt", format!("{name} was interrupted")),
        };
        let entry = self.last_command();
        entry.insert("exit".into(), Value::Null);
        entry.insert("incomplete".into(), json!(incomplete));
        entry.insert("capture_scope".into(), json!("possibly-open observation"));
        self.save()?;
        Err(error.into())
    }

    fn native(
        &mut self,
        name: &str,
        command: &[String],
        source: &Path,
        environment: Option<&[(OsString, OsString)]>,
        seconds: u64,
    ) -> Result<String> {
        let unit = format!("ripscope-release-{}.scope", unique_id());
        let mut scoped: Vec<String> = ["systemd-run", "--user", "--quiet", "--scope", "--unit"]
            .map(str::to_owned)
            .to_vec();
        scoped.extend([
            unit.clone(),
            "-p".to_owned(),
            format!("RuntimeMaxSec={seconds}"),
            "--".to_owned(),
        ]);
        scoped.extend(command.iter().cloned());
        let outcome = self.run(name, &scoped, Some(source), seconds + 10, environment);
        let (confirmed, before) = scope_state(&unit)?;
        let mut controls = Vec::new();
        if !confirmed {
            controls.push(control(&[
                "systemctl",
                "--user",
                "kill",
                "--kill-whom=all",
                "--signal=SIGKILL",
                &unit,
            ])?);
            controls.push(control(&["systemctl", "--user", "stop", &unit])?);
        }
        let (confirmed, after) = scope_state(&unit)?;
        let missing = [Some(0), Some(4)].contains(&after["exit"].as_i64())
            && after["stdout"]
                .as_str()
                .unwrap_or_default()
                .lines()
                .any(|line| line == "LoadState=not-found");
        let controlled = controls.iter().all(|control| control["exit"] == json!(0));
        let mut cleanup = json!({
            "kind": "native-user-scope", "unit": unit, "before": before, "controls": controls,
            "after": after, "unit_missing_verified": missing,
            "confirmed": confirmed && (missing || controlled),
        });
        let cleaned = cleanup["confirmed"] == json!(true);
        if outcome.is_err() || !cleaned {
            cleanup["separate_proof_scopes"] =
                json!("unverified after failed or interrupted native command");
            cleanup["captures"] =
                json!("possibly-open observations; preserve work before investigating");
        }
        self.cleanup(cleanup)?;
        let output = outcome?;
        if !cleaned {
            return Err(format!(
                "Native scope cleanup unresolved: {unit}; preserve {}",
                self.work.display()
            )
            .into());
        }
        Ok(output)
    }

    fn cleanup(&mut self, entry: Value) -> Result<()> {
        if let Some(Value::Array(owned)) = self.receipt.get_mut("owned_cleanup") {
            owned.push(entry);
        }
        self.save()
    }

    /// Runs a container named for this build and confirms it is gone afterwards.
    fn container(&mut self, name: &str, arguments: Vec<String>, seconds: u64) -> Result<String> {
        let owned = arguments[arguments
            .iter()
            .position(|arg| arg == "--name")
            .ok_or("unnamed container")?
            + 1]
        .clone();
        let outcome = self.run(name, &arguments, None, seconds, None);
        let before = control(&["podman", "container", "exists", &owned])?;
        let mut controls = Vec::new();
        if before["exit"] == json!(0) {
            controls.push(control(&[
                "podman", "stop", "--ignore", "--time", "10", &owned,
            ])?);
            controls.push(control(&["podman", "rm", "--force", "--ignore", &owned])?);
        }
        let after = control(&["podman", "container", "exists", &owned])?;
        let confirmed = [json!(0), json!(1)].contains(&before["exit"])
            && after["exit"] == json!(1)
            && controls.iter().all(|control| control["exit"] == json!(0));
        self.cleanup(json!({
            "kind": "container", "name": owned, "before": before, "controls": controls,
            "after": after, "confirmed": confirmed,
        }))?;
        let output = outcome?;
        if !confirmed {
            return Err(format!(
                "Container cleanup unresolved: {owned}; preserve {}",
                self.work.display()
            )
            .into());
        }
        Ok(output)
    }
}

/// A short control command's outcome as the receipt records it.
fn control(command: &[&str]) -> Result<Value> {
    let command: Vec<String> = command.iter().map(|word| (*word).to_owned()).collect();
    Ok(match capture(&command, Duration::from_secs(20))? {
        Some((exit, stdout, stderr)) => {
            json!({"args": command, "exit": exit, "stdout": stdout, "stderr": stderr})
        }
        None => json!({"args": command, "exit": null, "error": "control deadline"}),
    })
}

/// Whether the scope is confirmed gone or stopped, with the observation.
fn scope_state(unit: &str) -> Result<(bool, Value)> {
    let result = control(&[
        "systemctl",
        "--user",
        "show",
        unit,
        "--property=LoadState",
        "--property=ActiveState",
    ])?;
    let stdout = result["stdout"].as_str().unwrap_or_default();
    let property = |name: &str| {
        stdout
            .lines()
            .filter_map(|line| line.split_once('='))
            .filter(|(key, _)| *key == name)
            .map(|(_, value)| value.to_owned())
            .next_back()
    };
    let exit = result["exit"].as_i64();
    let confirmed = ([Some(0), Some(4)].contains(&exit)
        && property("LoadState").as_deref() == Some("not-found"))
        || (exit == Some(0)
            && ["inactive", "failed"]
                .contains(&property("ActiveState").as_deref().unwrap_or_default()));
    Ok((confirmed, result))
}

/// The cgroup check a native user scope runs in a child of this tool: the process must
/// sit in a scope under the user's manager and be listed there.
pub fn native_cgroup() -> Result<()> {
    let cgroups = fs::read_to_string("/proc/self/cgroup")?;
    let group = cgroups
        .lines()
        .find_map(|line| line.strip_prefix("0::"))
        .ok_or("no unified cgroup")?;
    if !Path::new(group).components().any(|part| {
        let part = part.as_os_str().to_string_lossy();
        part.starts_with("user@") && part.ends_with(".service")
    }) {
        return Err(format!("not under a user manager: {group}").into());
    }
    let members = fs::read_to_string(format!("/sys/fs/cgroup{group}/cgroup.procs"))?;
    if !members
        .lines()
        .any(|pid| pid == std::process::id().to_string())
    {
        return Err(format!("not listed in {group}").into());
    }
    println!("{group}");
    Ok(())
}

fn text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn words(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| (*item).to_owned()).collect()
}

/// Free bytes for an ordinary user on the filesystem holding `path`.
fn free_bytes(path: &Path) -> Result<u64> {
    #[cfg(unix)]
    {
        let stats = rustix::fs::statvfs(path)?;
        Ok(stats.f_bavail * stats.f_frsize)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Err("free space is only checked on Linux".into())
    }
}

fn ids() -> (u32, u32) {
    #[cfg(unix)]
    return (
        rustix::process::getuid().as_raw(),
        rustix::process::getgid().as_raw(),
    );
    #[cfg(not(unix))]
    (0, 0)
}

pub fn run(args: &[String]) -> Result<()> {
    let options = options(args)?;
    if !cfg!(unix) {
        return Err("Run on Linux, including WSL".into());
    }
    // An interrupt or termination stops the running command, then the usual cleanup
    // stops and removes the containers and scopes this build owns and records it.
    #[cfg(unix)]
    ctrlc::set_handler(|| INTERRUPTED.store(true, Ordering::SeqCst))?;
    let repo = repository()?;
    let profile: Value = serde_json::from_slice(&fs::read(
        repo.join("tools/release/linux-release-profile.json"),
    )?)?;
    let field = |name: &str| -> Result<String> {
        Ok(profile[name]
            .as_str()
            .ok_or_else(|| format!("profile has no {name}"))?
            .to_owned())
    };
    // Resolve links first, so a link into the checkout cannot pass for an outside path.
    let work = resolved_new(&options.work)?;
    let toolchain = options.toolchain.canonicalize()?;
    if inside(&work, &repo)? || work.exists() {
        return Err("Use a new external work directory".into());
    }
    if sha(&options.mold_archive)? != field("mold_archive_sha256")? {
        return Err("Mold archive identity mismatch".into());
    }
    fs::create_dir_all(&work)?;
    let logs = work.join("logs");
    fs::create_dir(&logs)?;
    let mut receipt = Map::new();
    receipt.insert("complete".into(), json!(false));
    receipt.insert("profile".into(), profile.clone());
    receipt.insert("work".into(), json!(text(&work)));
    receipt.insert("commands".into(), json!([]));
    receipt.insert(
        "validation".into(),
        json!({
            "product": {"complete": false, "profile": "release", "targets": ["bin:ripscope", "test:*"],
                        "environment": "pinned glibc 2.28 container"},
            "auxiliary": {"complete": false, "profile": "debug", "targets": ["examples"],
                          "environment": "native Linux user scopes", "ignored_tests": "included"},
        }),
    );
    receipt.insert("owned_cleanup".into(), json!([]));
    let mut build = Build {
        work: work.clone(),
        logs: logs.clone(),
        receipt,
    };
    build.save()?;
    let commit = build.run(
        "source-commit",
        &words(&["git", "-C", &text(&repo), "rev-parse", "HEAD"]),
        None,
        300,
        None,
    )?;
    build
        .receipt
        .insert("source_commit".into(), json!(commit.trim()));
    let epoch: u64 = build
        .run(
            "source-epoch",
            &words(&[
                "git",
                "-C",
                &text(&repo),
                "show",
                "-s",
                "--format=%ct",
                "HEAD",
            ]),
            None,
            300,
            None,
        )?
        .trim()
        .parse()?;
    let source = work.join("source");
    fs::create_dir(&source)?;
    copy_source(&repo, &source, &EXTRAS)?;
    let source_inputs = inventory(&source)?;
    build
        .receipt
        .insert("source_inputs".into(), Value::Object(source_inputs.clone()));
    // These developer proofs need the host's user manager and disk-backed fixtures.
    let host_path = std::env::var_os("PATH").unwrap_or_default();
    let native_tools = work.join("native-tools");
    let timeout = reference_tools(&host_path, &native_tools)?;
    build.receipt.insert(
        "native_reference_timeout".into(),
        json!({"path": timeout, "sha256": sha(&timeout)?}),
    );
    build.run(
        "native-reference-timeout",
        &[text(&native_tools.join("timeout")), "--version".into()],
        None,
        30,
        None,
    )?;
    let storage = build.run(
        "native-storage",
        &words(&["stat", "-f", "-c", "%t", "/var/tmp"]),
        None,
        300,
        None,
    )?;
    let kind = u64::from_str_radix(storage.trim(), 16)?;
    if [0x0102_1994, 0x6969, 0xff53_4d42, 0xfe53_4d42].contains(&kind)
        || free_bytes(Path::new("/var/tmp"))? < 4 << 30
    {
        return Err("Native proofs require disk-backed /var/tmp with at least 4 GiB free".into());
    }
    // Do not retain the manager's environment, which may contain credentials.
    let manager = std::process::Command::new("systemctl")
        .args(["--user", "show-environment"])
        .stdout(std::process::Stdio::null())
        .stderr(fs::File::create(logs.join("native-manager.log"))?)
        .status()?;
    if !manager.success() {
        return Err("Native proofs require a reachable user systemd manager".into());
    }
    let tool = text(&std::env::current_exe()?);
    let preflight = build.native(
        "native-cgroup",
        &[tool, "native-cgroup".to_owned()],
        &source,
        None,
        30,
    )?;
    build
        .receipt
        .insert("native_preflight".into(), json!(preflight));
    let toolchain_inputs = inventory(&toolchain)?;
    build.receipt.insert(
        "rust_toolchain_inputs".into(),
        Value::Object(toolchain_inputs.clone()),
    );
    let rust = build.run(
        "rust-version",
        &[text(&toolchain.join("bin/rustc")), "-Vv".into()],
        None,
        300,
        None,
    )?;
    if !rust.contains(&format!("release: {}", field("rust_version")?))
        || !rust.contains(&field("rust_commit")?)
    {
        return Err("Unexpected Rust toolchain".into());
    }
    fs::create_dir(work.join("mold"))?;
    build.run(
        "mold-extract",
        &words(&[
            "tar",
            "--no-same-owner",
            "-xzf",
            &text(&options.mold_archive),
            "-C",
            &text(&work.join("mold")),
        ]),
        None,
        300,
        None,
    )?;
    let mold = work.join("mold/mold-3.0.0-x86_64-linux");
    let vendor = work.join("vendor");
    // Cargo's progress and configuration share a retained log. The generated
    // configuration below uses fixed container paths instead of log extraction.
    build.run(
        "vendor",
        &[
            text(&toolchain.join("bin/cargo")),
            "vendor".into(),
            "--locked".into(),
            "--versioned-dirs".into(),
            text(&vendor),
        ],
        Some(&source),
        300,
        None,
    )?;
    fs::create_dir(source.join(".cargo"))?;
    let configs = work.join("cargo-configs");
    fs::create_dir(&configs)?;
    let container_config = "[source.crates-io]\nreplace-with = \"vendored-sources\"\n[source.vendored-sources]\ndirectory = \"/work/vendor\"\n";
    write_text(&configs.join("container.toml"), container_config)?;
    write_text(&source.join(".cargo/config.toml"), container_config)?;
    build.receipt.insert(
        "cargo_config_sha256".into(),
        json!({"container": sha(&configs.join("container.toml"))?}),
    );
    let vendor_inputs = inventory(&vendor)?;
    build
        .receipt
        .insert("vendor_inputs".into(), Value::Object(vendor_inputs.clone()));
    for kind in ["build", "runtime"] {
        let image = field(&format!("{kind}_image"))?;
        build.run(
            &format!("{kind}-image"),
            &words(&["podman", "pull", &image]),
            None,
            300,
            None,
        )?;
        let inspected: Value = serde_json::from_str(&build.run(
            &format!("{kind}-image-inspect"),
            &words(&["podman", "image", "inspect", &image]),
            None,
            300,
            None,
        )?)?;
        // Keep local storage paths in the diagnostic log, outside the package.
        let mut identity = Map::new();
        for key in [
            "Id",
            "Digest",
            "RepoDigests",
            "Architecture",
            "Os",
            "Created",
        ] {
            identity.insert(
                key.into(),
                inspected[0]
                    .get(key)
                    .cloned()
                    .ok_or_else(|| format!("image inspection has no {key}"))?,
            );
        }
        build
            .receipt
            .insert(format!("{kind}_image_identity"), Value::Object(identity));
    }
    // Index tests require a supported filesystem and real permission failures.
    let (uid, gid) = ids();
    let base = |owned: &str| -> Result<Vec<String>> {
        let mut base = words(&[
            "podman",
            "run",
            "--rm",
            "--network=none",
            "--userns=keep-id",
            "--user",
            &format!("{uid}:{gid}"),
            "--tmpfs",
            "/tmp:rw,exec,mode=1777",
            "--tmpfs",
            "/var/tmp:rw,exec,mode=1777",
            "-v",
            &format!("{}:/work:rw", text(&work)),
            "-v",
            &format!("{}:/toolchain:ro", text(&toolchain)),
            "-v",
            &format!("{}:/mold:ro", text(&mold)),
            "-w",
            "/work/source",
            "-e",
            "CARGO_HOME=/work/cargo-home",
            "-e",
            "CARGO_TARGET_DIR=/work/target",
            "-e",
            "CARGO_BUILD_JOBS=4",
            "-e",
            &format!("RUSTFLAGS={}", field("rustflags")?),
            "-e",
            &format!("CFLAGS={}", field("cflags")?),
            "-e",
            &format!("CXXFLAGS={}", field("cflags")?),
            "-e",
            "CC_ENABLE_DEBUG_OUTPUT=1",
            "-e",
            &format!("SOURCE_DATE_EPOCH={epoch}"),
        ]);
        base.extend(["--name".to_owned(), owned.to_owned()]);
        Ok(base)
    };
    let script = |owned: &str, script: &str| -> Result<Vec<String>> {
        let mut arguments = base(owned)?;
        arguments.extend([
            field("build_image")?,
            "bash".into(),
            "-euc".into(),
            format!("export PATH=/toolchain/bin:/mold/bin:$PATH; {script}"),
        ]);
        Ok(arguments)
    };
    let owned = || format!("ripscope-release-{}", unique_id());
    build.container(
        "build-tools",
        script(
            &owned(),
            "rustc -Vv; cargo -V; mold --version; gcc --version; ldd --version",
        )?,
        300,
    )?;
    build.container(
        "release-build",
        script(
            &owned(),
            "cargo build --release --locked --offline --bins --examples -vv",
        )?,
        1800,
    )?;
    let executable = work.join("target/release/ripscope");
    let built_hash = sha(&executable)?;
    build.container(
        "release-product-tests",
        script(&owned(), "cargo test --release --locked --offline --bin ripscope --test \"*\" -- --test-threads=1")?,
        1800,
    )?;
    if sha(&executable)? != built_hash {
        return Err("Artifact changed during release tests".into());
    }
    build.receipt["validation"]["product"]["complete"] = json!(true);
    let native_config =
        container_config.replace("\"/work/vendor\"", &serde_json::to_string(&text(&vendor))?);
    write_text(&configs.join("native.toml"), &native_config)?;
    write_text(&source.join(".cargo/config.toml"), &native_config)?;
    build.receipt["cargo_config_sha256"]["native"] = json!(sha(&configs.join("native.toml"))?);
    // The host's environment, whatever bytes it holds, without flags that would change
    // the build.
    let mut environment: Vec<(OsString, OsString)> = std::env::vars_os()
        .filter(|(key, _)| {
            ![
                "RUSTFLAGS",
                "CARGO_ENCODED_RUSTFLAGS",
                "RUSTC_WRAPPER",
                "RUSTC_WORKSPACE_WRAPPER",
                "CC",
                "CXX",
                "CFLAGS",
                "CXXFLAGS",
            ]
            .iter()
            .any(|name| key == name)
        })
        .collect();
    let set = |environment: &mut Vec<(OsString, OsString)>, key: &str, value: OsString| {
        environment.retain(|(name, _)| name != key);
        environment.push((key.into(), value));
    };
    let mut path = toolchain.join("bin").into_os_string();
    path.push(":");
    path.push(native_tools.as_os_str());
    path.push(":");
    path.push(host_path);
    set(&mut environment, "PATH", path);
    set(
        &mut environment,
        "RUSTC",
        toolchain.join("bin/rustc").into(),
    );
    set(
        &mut environment,
        "CARGO_HOME",
        work.join("native-cargo-home").into(),
    );
    set(
        &mut environment,
        "CARGO_TARGET_DIR",
        work.join("native-target").into(),
    );
    set(&mut environment, "CARGO_NET_OFFLINE", "true".into());
    set(&mut environment, "CARGO_BUILD_JOBS", "4".into());
    let cargo = text(&toolchain.join("bin/cargo"));
    build.native(
        "native-example-build",
        &[
            cargo.clone(),
            "build".into(),
            "--locked".into(),
            "--offline".into(),
            "--examples".into(),
        ],
        &source,
        Some(&environment),
        1800,
    )?;
    build.native(
        "native-example-tests",
        &words(&[
            &cargo,
            "test",
            "--locked",
            "--offline",
            "--examples",
            "--",
            "--include-ignored",
            "--test-threads=1",
        ]),
        &source,
        Some(&environment),
        1800,
    )?;
    if sha(&executable)? != built_hash {
        return Err("Artifact changed during native auxiliary tests".into());
    }
    unchanged_sources(
        &source_inputs,
        &source,
        &repo,
        "Source input changed during native auxiliary tests",
    )?;
    if inventory(&toolchain)? != toolchain_inputs || inventory(&vendor)? != vendor_inputs {
        return Err("Toolchain or vendored dependencies changed during auxiliary tests".into());
    }
    build.receipt["validation"]["auxiliary"]["complete"] = json!(true);
    let mut reports = Vec::new();
    for (name, flags) in [
        ("dynamic", &["-d"][..]),
        ("versions", &["-V"][..]),
        ("notes", &["-n"][..]),
        ("comments", &["-p", ".comment"][..]),
        ("header", &["-h"][..]),
        ("interpreter", &["-l"][..]),
    ] {
        let mut command = vec!["readelf".to_owned()];
        command.extend(words(flags));
        command.push(text(&executable));
        reports.push(build.run(&format!("elf-{name}"), &command, None, 300, None)?);
    }
    let elf = inspect_elf(
        &reports[0],
        &reports[1],
        &reports[2],
        &reports[3],
        &reports[4],
        &reports[5],
    )?;
    build.receipt.insert("elf".into(), elf);
    let version = crate_version(&source.join("Cargo.toml"))?;
    let package = work.join(format!("ripscope-{version}-x86_64-linux-gnu"));
    fs::create_dir_all(package.join("bin"))?;
    copy_file(&executable, &package.join("bin/ripscope"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(
            package.join("bin/ripscope"),
            fs::Permissions::from_mode(0o755),
        )?;
    }
    delivery_files(&source, &package)?;
    copy_file(
        &source.join("docs/getting-started/installation.md"),
        &package.join("INSTALL.md"),
    )?;
    collect_notices(&vendor, &toolchain, &package)?;
    build
        .receipt
        .insert("binary_sha256".into(), json!(built_hash));
    build
        .receipt
        .insert("source_date_epoch".into(), json!(epoch));
    let recorded: Map<String, Value> = build
        .receipt
        .iter()
        .filter(|(key, _)| {
            ![
                "commands",
                "complete",
                "work",
                "owned_cleanup",
                "native_preflight",
            ]
            .contains(&key.as_str())
        })
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    write_text(
        &package.join("BUILD.json"),
        &(json_text(&Value::Object(recorded)) + "\n"),
    )?;
    let manifest: String = inventory(&package)?
        .iter()
        .map(|(name, digest)| format!("{}  {name}\n", digest.as_str().unwrap_or_default()))
        .collect();
    write_text(&package.join("MANIFEST.sha256"), &manifest)?;
    let dist = work.join("dist");
    fs::create_dir(&dist)?;
    let archive_name = format!(
        "{}.tar.gz",
        package
            .file_name()
            .ok_or("package without a name")?
            .to_string_lossy()
    );
    let output = dist.join(&archive_name);
    archive::tar_gz(&package, &output, epoch)?;
    let line = format!("{}  {archive_name}\n", sha(&output)?);
    write_text(&dist.join("SHA256SUMS"), &line)?;
    write_text(&dist.join(format!("{archive_name}.sha256")), &line)?;
    let mut smoke = words(&["podman", "run"]);
    smoke.extend(["--name".to_owned(), owned()]);
    smoke.extend(words(&[
        "--rm",
        "--network=none",
        "--read-only",
        "--user",
        "1000:1000",
        "--tmpfs",
        "/work:rw,exec,mode=1777",
        "-v",
        &format!("{}:/input:ro", text(&dist)),
        "-v",
        &format!(
            "{}:/smoke.sh:ro",
            text(&source.join("scripts/linux-smoke.sh"))
        ),
        &field("runtime_image")?,
        "/bin/sh",
        "/smoke.sh",
        &archive_name,
    ]));
    build.container("runtime-smoke", smoke, 300)?;
    unchanged_sources(
        &source_inputs,
        &source,
        &repo,
        "Source input changed during build",
    )?;
    if inventory(&toolchain)? != toolchain_inputs || inventory(&vendor)? != vendor_inputs {
        return Err("Toolchain or vendored dependencies changed".into());
    }
    if INTERRUPTED.load(Ordering::SeqCst) {
        return Err("Interrupted; the archive is incomplete".into());
    }
    let archive = json!({"path": text(&output), "sha256": sha(&output)?, "bytes": fs::metadata(&output)?.len()});
    build.receipt.insert("archive".into(), archive.clone());
    build.receipt.insert("complete".into(), json!(true));
    build.save()?;
    println!("{}", json_line(&archive));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "linux")]
    #[test]
    fn native_reference_tools_select_gnu_without_changing_the_parent_path() {
        let host_path = std::env::var_os("PATH").unwrap();
        let gnu = reference_timeout(&host_path).unwrap();
        let root = tempfile::tempdir().unwrap();
        let bin = root.path().join("bin");
        fs::create_dir(&bin).unwrap();
        // A non-GNU provider must not satisfy the deadline prerequisite.
        let rustc = std::env::split_paths(&host_path)
            .map(|directory| directory.join("rustc"))
            .find(|candidate| candidate.is_file())
            .unwrap();
        std::os::unix::fs::symlink(rustc, bin.join("timeout")).unwrap();
        assert!(
            reference_timeout(bin.as_os_str())
                .unwrap_err()
                .to_string()
                .contains("require GNU timeout")
        );
        std::os::unix::fs::symlink(&gnu, bin.join("gnutimeout")).unwrap();
        let staged = root.path().join("native-tools");
        assert_eq!(reference_tools(bin.as_os_str(), &staged).unwrap(), gnu);
        let (status, version, _) = capture(
            &[text(&staged.join("timeout")), "--version".into()],
            Duration::from_secs(5),
        )
        .unwrap()
        .unwrap();
        assert_eq!(status, 0);
        assert!(version.contains("(GNU coreutils)"));
        assert_eq!(std::env::var_os("PATH").unwrap(), host_path);
    }

    #[test]
    fn failing_commands_report_their_captured_diagnostic() {
        let root = tempfile::tempdir().unwrap();
        let mut build = Build {
            work: root.path().to_path_buf(),
            logs: root.path().to_path_buf(),
            receipt: json!({"commands": []}).as_object().unwrap().clone(),
        };
        let error = build
            .run(
                "diagnostic-probe",
                &words(&["rustc", "--release-log-fixture-invalid-option"]),
                None,
                10,
                None,
            )
            .unwrap_err();
        assert!(error.to_string().contains("Unrecognized option"), "{error}");
        assert!(
            error
                .to_string()
                .contains("release-log-fixture-invalid-option"),
            "{error}"
        );
        assert_eq!(build.receipt["commands"][0]["exit"], 1);
    }

    #[test]
    fn elf_gate_rejects_new_runtime_and_cpu_requirements() {
        let valid = [
            "(NEEDED) [libc.so.6]",
            "GLIBC_2.2.5 GLIBC_2.28",
            "x86 ISA needed: x86-64-baseline",
            "mold 3.0.0",
            "ELF64 Advanced Micro Devices X86-64",
            "/lib64/ld-linux-x86-64.so.2",
        ];
        let check = |reports: [&str; 6]| {
            inspect_elf(
                reports[0], reports[1], reports[2], reports[3], reports[4], reports[5],
            )
        };
        assert_eq!(check(valid).unwrap()["maximum_glibc_symbol"], "2.28");
        for (index, value) in [
            (1, "GLIBC_2.34"),
            (1, "GLIBC_PRIVATE"),
            (2, "x86 ISA needed: x86-64-v2"),
            (0, "(NEEDED) [libsqlite3.so.0]"),
            (0, "(NEEDED) [libc.so.6]\n(RUNPATH) [/work]"),
            (3, "GNU ld"),
            (4, "ELF32"),
            (5, "/foreign/loader"),
        ] {
            let mut reports = valid;
            reports[index] = value;
            assert!(check(reports).is_err(), "{index} {value}");
        }
    }

    #[test]
    fn archive_metadata_does_not_depend_on_input_mtime_or_permissions() {
        let root = tempfile::tempdir().unwrap();
        let package = root.path().join("package");
        fs::create_dir_all(package.join("bin")).unwrap();
        fs::write(package.join("bin/ripscope"), b"fake executable").unwrap();
        fs::write(package.join("INSTALL.md"), b"instructions").unwrap();
        archive::tar_gz(&package, &root.path().join("one.tar.gz"), 123_456).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(
                package.join("INSTALL.md"),
                fs::Permissions::from_mode(0o600),
            )
            .unwrap();
        }
        fs::File::options()
            .write(true)
            .open(package.join("INSTALL.md"))
            .unwrap()
            .set_modified(std::time::UNIX_EPOCH + Duration::from_secs(999_999))
            .unwrap();
        archive::tar_gz(&package, &root.path().join("two.tar.gz"), 123_456).unwrap();
        let one = fs::read(root.path().join("one.tar.gz")).unwrap();
        assert_eq!(one, fs::read(root.path().join("two.tar.gz")).unwrap());
        let mut tar = Vec::new();
        std::io::Read::read_to_end(&mut flate2::read::GzDecoder::new(&one[..]), &mut tar).unwrap();
        let mut at = 0;
        let mut seen = Vec::new();
        while tar[at] != 0 {
            let name = String::from_utf8_lossy(&tar[at..at + 100])
                .trim_end_matches('\0')
                .to_owned();
            let field = |from: usize, to: usize| {
                u64::from_str_radix(
                    String::from_utf8_lossy(&tar[at + from..at + to]).trim_end_matches('\0'),
                    8,
                )
                .unwrap()
            };
            assert_eq!(
                (field(108, 115), field(116, 123), field(136, 147)),
                (0, 0, 123_456),
                "{name}"
            );
            if name == "package/bin/ripscope" {
                assert_eq!(field(100, 107), 0o755);
            }
            let size = field(124, 135) as usize;
            seen.push(name);
            at += 512 + size.div_ceil(512) * 512;
        }
        // Uppercase names sort first except on Windows, which orders them ignoring case.
        let expected = if cfg!(windows) {
            [
                "package/",
                "package/bin/",
                "package/bin/ripscope",
                "package/INSTALL.md",
            ]
        } else {
            [
                "package/",
                "package/INSTALL.md",
                "package/bin/",
                "package/bin/ripscope",
            ]
        };
        assert_eq!(seen, expected);
    }
}
