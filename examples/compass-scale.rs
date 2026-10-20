//! Native Linux compass runs for a full source tree or a pinned system include tree.
//!
//! Reuses the cscope database decoder, but retains at most one file's records. Large
//! query streams stay on disk; GNU sort supplies duplicate-preserving comparisons. This
//! supplements the corpus runner, without changing its pinned projects or cached results.
//! See `scripts/compass-scale.md`.

use regex::bytes::Regex;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::sync::LazyLock;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[path = "support/capture.rs"]
#[allow(dead_code)]
mod capture;
#[path = "support/captured_text.rs"]
#[allow(dead_code)]
mod captured_text;
#[path = "support/cscope_db.rs"]
#[allow(dead_code)]
mod cscope_db;
#[path = "support/json_text.rs"]
#[allow(dead_code)]
mod json_text;
#[path = "support/reference.rs"]
#[allow(dead_code)]
mod reference;
#[path = "support/source_inventory.rs"]
#[allow(dead_code)]
mod source_inventory;

use capture::digest;
use json_text::{Json, LINE, REPORT};
use reference::CSCOPE_SHA256 as REFERENCE_SHA256;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const FIELDS: [u8; 7] = [1, 0, 2, 3, 7, 8, 9];
const MARK: &[u8] = b"##units-query## ";
static ROW_TAIL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?-u)^([^ ]+) ([0-9]+) (.*)\n$").expect("row expression"));
static VIEW_BYTES: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?-u)[\x01-\x08\x0b-\x1f\x80-\xff]").expect("view expression"));

fn main() -> ExitCode {
    // Usage errors exit 2.
    let options = match options() {
        Ok(_) if !cfg!(unix) => {
            Err("run on native Linux storage; the runner needs GNU sort and /usr/bin/time".into())
        }
        other => other,
    };
    let options = match options {
        Ok(options) => options,
        Err(error) => {
            eprintln!("compass-scale: {error}");
            return ExitCode::from(2);
        }
    };
    match Run::new(options).and_then(|mut run| run.run()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("compass-scale: {error}");
            ExitCode::FAILURE
        }
    }
}

const USAGE: &str = "compass-scale --binary EXE --reference EXE --source DIR --output NEW_DIR \
[--sysroot DIR] [--timeout SECONDS] [--build-timeout SECONDS] [--manifest FILE] \
[--closure-manifest FILE] [--exact-inventory FILE] [--memory-scope] [--output-limit BYTES] \
[--project-limit BYTES]";

#[derive(Clone)]
struct Options {
    binary: PathBuf,
    reference: PathBuf,
    source: PathBuf,
    output: PathBuf,
    /// A pinned tree holding `include/` and its multiarch directory.
    sysroot: Option<PathBuf>,
    /// Deadline per child process, in seconds.
    timeout: u64,
    build_timeout: u64,
    /// Frozen explicit C selection and include directories.
    manifest: Option<PathBuf>,
    /// Frozen include-closure paths and hashes.
    closure_manifest: Option<PathBuf>,
    /// Frozen array of exact-name patterns.
    exact_inventory: Option<PathBuf>,
    /// Enforce 8 GiB through a user systemd scope.
    memory_scope: bool,
    output_limit: u64,
    project_limit: u64,
}

fn options() -> Result<Options> {
    let mut values: BTreeMap<String, String> = BTreeMap::new();
    let mut memory_scope = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                println!("{USAGE}");
                std::process::exit(0);
            }
            "--memory-scope" => memory_scope = true,
            name if name.starts_with("--") => {
                let value = args.next().ok_or_else(|| format!("{name} needs a value"))?;
                values.insert(name[2..].to_owned(), value);
            }
            _ => return Err(format!("unexpected argument {arg:?}\n{USAGE}").into()),
        }
    }
    let path = |name: &str| values.get(name).map(PathBuf::from);
    let required = |name: &str| path(name).ok_or_else(|| format!("--{name} is required\n{USAGE}"));
    let number = |name: &str, default: u64| -> Result<u64> {
        Ok(match values.get(name) {
            Some(value) => value
                .parse()
                .map_err(|_| format!("--{name} needs a whole number"))?,
            None => default,
        })
    };
    let known = [
        "binary",
        "reference",
        "source",
        "output",
        "sysroot",
        "timeout",
        "build-timeout",
        "manifest",
        "closure-manifest",
        "exact-inventory",
        "output-limit",
        "project-limit",
    ];
    if let Some(unknown) = values.keys().find(|key| !known.contains(&key.as_str())) {
        return Err(format!("unknown option --{unknown}\n{USAGE}").into());
    }
    Ok(Options {
        binary: required("binary")?,
        reference: required("reference")?,
        source: required("source")?,
        output: required("output")?,
        sysroot: path("sysroot"),
        timeout: number("timeout", 900)?,
        build_timeout: number("build-timeout", 1800)?,
        manifest: path("manifest"),
        closure_manifest: path("closure-manifest"),
        exact_inventory: path("exact-inventory"),
        memory_scope,
        output_limit: number("output-limit", 2 << 30)?,
        project_limit: number("project-limit", 20 << 30)?,
    })
}

/// An absolute path with symbolic links resolved as far as the path exists.
fn resolve(path: &Path) -> io::Result<PathBuf> {
    let absolute = std::path::absolute(path)?;
    if let Ok(real) = absolute.canonicalize() {
        return Ok(real);
    }
    match (absolute.parent(), absolute.file_name()) {
        (Some(parent), Some(name)) => Ok(resolve(parent)?.join(name)),
        _ => Ok(absolute),
    }
}

fn read_json(path: &Path) -> Result<Value> {
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}

fn git(source: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let output = Command::new("git")
        .args(args)
        .current_dir(source)
        .output()?;
    if !output.status.success() {
        return Err(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    Ok(output.stdout)
}

/// The first executable named `name` in `path`'s directories.
fn which(name: &str, path: &str) -> Option<PathBuf> {
    path.split(':')
        .map(|directory| Path::new(directory).join(name))
        .find(|candidate| {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                candidate
                    .metadata()
                    .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
            }
            #[cfg(not(unix))]
            candidate.is_file()
        })
}

/// How commands are launched; tests replace it to stage captured streams.
type Launch = fn(&mut Run, &str, &[String], Option<&[u8]>, bool) -> Result<Json>;

struct Run {
    options: Options,
    out: PathBuf,
    source: PathBuf,
    env: Vec<(String, String)>,
    sort: String,
    selection: Vec<String>,
    manifest: Option<Value>,
    closure: Option<Value>,
    binary: String,
    reference: String,
    report: Json,
    unsupported_inputs: Vec<Value>,
    launch: Launch,
}

impl Run {
    fn new(options: Options) -> Result<Run> {
        let out = resolve(&options.output)?;
        let source = resolve(&options.source)?;
        if out.exists() {
            return Err(format!("output directory already exists: {}", out.display()).into());
        }
        fs::create_dir_all(out.parent().ok_or("output has no parent directory")?)?;
        fs::create_dir(&out)?;
        let mut env = vec![
            ("PATH".to_owned(), "/usr/bin:/bin".to_owned()),
            ("HOME".to_owned(), out.to_string_lossy().into_owned()),
            ("LC_ALL".to_owned(), "C".to_owned()),
        ];
        for key in ["XDG_RUNTIME_DIR", "DBUS_SESSION_BUS_ADDRESS"] {
            if let Ok(value) = std::env::var(key) {
                env.push((key.to_owned(), value));
            }
        }
        let sort = which("gnusort", "/usr/bin:/bin")
            .or_else(|| which("sort", "/usr/bin:/bin"))
            .ok_or("GNU sort is required")?;
        let version = Command::new(&sort)
            .arg("--version")
            .env_clear()
            .envs(env.iter().map(|(k, v)| (k, v)))
            .output()?;
        let sort_version = String::from_utf8_lossy(&version.stdout).into_owned();
        if !version.status.success() || !sort_version.contains("GNU coreutils") {
            return Err(
                "GNU sort is required (install gnusort if sort is another implementation)".into(),
            );
        }
        let mut selection = vec!["-R".to_owned()];
        let mut manifest = None;
        if let Some(path) = &options.manifest {
            let value = read_json(path)?;
            let names = manifest_paths(&value)?;
            let listing = out.join("selection.files");
            fs::write(
                &listing,
                names
                    .iter()
                    .map(|name| format!("\"{name}\"\n"))
                    .collect::<String>(),
            )?;
            selection = vec!["-i".to_owned(), listing.to_string_lossy().into_owned()];
            for directory in value["include_directories"]
                .as_array()
                .into_iter()
                .flatten()
            {
                selection.push("-I".to_owned());
                selection.push(
                    directory
                        .as_str()
                        .ok_or("include directory is not text")?
                        .to_owned(),
                );
            }
            manifest = Some(value);
        }
        let closure = options
            .closure_manifest
            .as_deref()
            .map(read_json)
            .transpose()?;
        if let Some(sysroot) = &options.sysroot {
            let root = resolve(sysroot)?;
            env.push((
                "INCDIR".to_owned(),
                root.join("include").to_string_lossy().into_owned(),
            ));
            env.push((
                "INCLUDEDIRS".to_owned(),
                root.join("include/x86_64-linux-gnu")
                    .to_string_lossy()
                    .into_owned(),
            ));
        } else {
            selection.push("-k".to_owned());
        }
        let binary = resolve(&options.binary)?.to_string_lossy().into_owned();
        let reference = resolve(&options.reference)?.to_string_lossy().into_owned();
        if digest(Path::new(&reference))? != REFERENCE_SHA256 {
            return Err("reference executable does not match ADR 0001".into());
        }
        if out.starts_with(&source) {
            return Err("outputs must be outside the source tree".into());
        }
        let revision = match &manifest {
            Some(value) => value["revision"]
                .as_str()
                .ok_or("manifest without a revision")?
                .to_owned(),
            None => String::from_utf8_lossy(&git(&source, &["rev-parse", "HEAD"])?)
                .trim()
                .to_owned(),
        };
        let mut run = Run {
            options,
            out,
            source,
            env,
            sort: sort.to_string_lossy().into_owned(),
            selection,
            manifest,
            closure,
            binary,
            reference,
            report: Json::object(Vec::<(String, Json)>::new()),
            unsupported_inputs: Vec::new(),
            launch: Run::execute,
        };
        run.check_source()?;
        let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
        run.report = Json::object([
            ("complete", Json::from(false)),
            (
                "source",
                Json::from(run.source.to_string_lossy().into_owned()),
            ),
            ("revision", Json::from(revision)),
            ("binary", Json::from(run.binary.as_str())),
            ("binary_sha256", Json::from(digest(Path::new(&run.binary))?)),
            ("reference", Json::from(run.reference.as_str())),
            ("reference_sha256", Json::from(REFERENCE_SHA256)),
            (
                "harness_sha256",
                Json::from(digest(&repo.join("examples/compass-scale.rs"))?),
            ),
            (
                "harness_executable_sha256",
                Json::from(digest(&std::env::current_exe()?)?),
            ),
            (
                "decoder_sha256",
                Json::from(digest(&repo.join("examples/support/cscope_db.rs"))?),
            ),
            ("sort", Json::from(run.sort.as_str())),
            (
                "sort_version",
                Json::from(sort_version.lines().next().unwrap_or_default()),
            ),
            (
                "environment",
                Json::object(
                    run.env
                        .iter()
                        .map(|(k, v)| (k.clone(), Json::from(v.as_str()))),
                ),
            ),
            ("selection", Json::from(run.selection.clone())),
            ("commands", Json::object(Vec::<(String, Json)>::new())),
            ("fields", Json::object(Vec::<(String, Json)>::new())),
        ]);
        if let Some(path) = &run.options.manifest {
            let manifest_digest = digest(path)?;
            run.report.set("manifest_sha256", manifest_digest);
        }
        run.write()?;
        Ok(run)
    }

    fn check_source(&mut self) -> Result<()> {
        self.unsupported_inputs = Vec::new();
        if let Some(manifest) = &self.manifest {
            let closure_files = self
                .closure
                .as_ref()
                .map(|closure| closure["files"].as_array().cloned().unwrap_or_default())
                .unwrap_or_default();
            let version = self.closure.as_ref().map(|closure| {
                closure
                    .get("inventory_version")
                    .and_then(Value::as_u64)
                    .unwrap_or(2)
            });
            if version.is_some_and(|version| version >= 3) {
                for file in &closure_files {
                    if !file["target"].is_string() || !file["links"].is_array() {
                        return Err(
                            format!("incomplete frozen source identity: {}", file["path"]).into(),
                        );
                    }
                }
            }
            let files = manifest["files"]
                .as_array()
                .ok_or("manifest without files")?;
            for file in files.iter().chain(&closure_files) {
                source_inventory::verify(&self.source, file, 0)?;
            }
            let names = manifest_paths(manifest)?;
            let names: Vec<&str> = names.iter().map(String::as_str).collect();
            let directories: Vec<&str> = manifest["include_directories"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect();
            self.unsupported_inputs =
                source_inventory::preflight(&self.source, &names, &directories)?;
            let declared = self
                .closure
                .as_ref()
                .and_then(|closure| closure["unsupported_inputs"].as_array().cloned())
                .unwrap_or_default();
            for item in declared {
                if !self.unsupported_inputs.contains(&item) {
                    self.unsupported_inputs.push(item);
                }
            }
            let revision = manifest["revision"]
                .as_str()
                .ok_or("manifest without a revision")?;
            if revision.starts_with("archive-sha256:") {
                return Ok(());
            }
            let actual = String::from_utf8_lossy(&git(&self.source, &["rev-parse", "HEAD"])?)
                .trim()
                .to_owned();
            if actual != revision {
                return Err("source revision differs from the frozen manifest".into());
            }
        }
        let changed = git(
            &self.source,
            &["status", "--porcelain", "--untracked-files=all"],
        )?;
        if !changed.is_empty() {
            return Err(format!(
                "source checkout is not clean: {}",
                bytes_repr(&changed[..changed.len().min(500)])
            )
            .into());
        }
        Ok(())
    }

    fn write(&self) -> Result<()> {
        fs::write(
            self.out.join("summary.json"),
            self.report.text(REPORT) + "\n",
        )?;
        Ok(())
    }

    fn command(
        &mut self,
        name: &str,
        args: &[String],
        input: Option<&[u8]>,
        required: bool,
    ) -> Result<Json> {
        (self.launch)(self, name, args, input, required)
    }

    fn execute(
        &mut self,
        name: &str,
        args: &[String],
        input: Option<&[u8]>,
        required: bool,
    ) -> Result<Json> {
        let mut reference_hash = None;
        if args[0] == self.reference {
            let actual = digest(Path::new(&self.reference))?;
            if actual != REFERENCE_SHA256 {
                return Err("reference executable changed before capture".into());
            }
            reference_hash = Some(actual);
        }
        let mut environment = self.env.clone();
        let libraries = if reference_hash.is_some() {
            reference::Reference::pinned().libraries
        } else {
            None
        };
        if let Some(libraries) = libraries.as_ref().filter(|value| !value.is_empty()) {
            environment.push(("LD_LIBRARY_PATH".to_owned(), libraries.clone()));
        }
        let stdout = self.out.join(format!("{name}.stdout"));
        let stderr = self.out.join(format!("{name}.stderr"));
        let metrics = self.out.join(format!("{name}.time"));
        let started = Instant::now();
        let unit = self.options.memory_scope.then(|| {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            format!("ripscope-compass-{}-{nanos}.scope", std::process::id())
        });
        let mut launch: Vec<String> = ["/usr/bin/time", "-f", "%e\t%M", "-o"]
            .map(str::to_owned)
            .to_vec();
        launch.push(metrics.to_string_lossy().into_owned());
        launch.extend(args.iter().cloned());
        if let Some(unit) = &unit {
            let mut scoped: Vec<String> = [
                "systemd-run",
                "--user",
                "--quiet",
                "--scope",
                "-p",
                "MemoryMax=8G",
                "-p",
                "MemorySwapMax=0",
                "--unit",
            ]
            .map(str::to_owned)
            .to_vec();
            scoped.push(unit.clone());
            scoped.push("--".to_owned());
            scoped.extend(launch);
            launch = scoped;
        }
        let mut command = Command::new(&launch[0]);
        command
            .args(&launch[1..])
            .current_dir(&self.source)
            .env_clear()
            .envs(environment.iter().map(|(k, v)| (k, v)))
            .stdout(File::create(&stdout)?)
            .stderr(File::create(&stderr)?)
            .stdin(if input.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            });
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        let mut child = command.spawn()?;
        let deadline = Duration::from_secs(if name.contains("build") {
            self.options.build_timeout
        } else {
            self.options.timeout
        });
        let limits = |run: &Run| -> io::Result<Option<&'static str>> {
            if fs::metadata(&stdout)?.len() + fs::metadata(&stderr)?.len()
                > run.options.output_limit
            {
                return Ok(Some("output_limit"));
            }
            if directory_size(&run.out)? > run.options.project_limit {
                return Ok(Some("project_artifact_limit"));
            }
            Ok(None)
        };
        let mut timed_out = false;
        let mut stopped: Option<&'static str> = None;
        let watched = (|| -> Result<()> {
            if let (Some(bytes), Some(mut pipe)) = (input, child.stdin.take()) {
                pipe.write_all(bytes)?;
            }
            while child.try_wait()?.is_none() {
                if started.elapsed() >= deadline {
                    timed_out = true;
                    stopped = Some("timeout");
                } else {
                    stopped = limits(self)?;
                }
                if stopped.is_some() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Ok(())
        })();
        if child.try_wait()?.is_none() {
            kill_group(child.id())?;
        }
        if let Some(unit) = &unit {
            for action in [
                &["kill", "--kill-whom=all", "--signal=SIGKILL"][..],
                &["stop"][..],
            ] {
                let mut words = action.to_vec();
                words.push(unit);
                systemctl(&words, &environment)?;
            }
            let (success, state) = systemctl(
                &["show", "--property=ActiveState", "--value", unit],
                &environment,
            )?;
            let value = String::from_utf8_lossy(&state).trim().to_owned();
            if !success || !["inactive", "failed"].contains(&value.as_str()) {
                child.wait()?;
                return Err(
                    format!("cannot confirm scope termination: {unit}; captures retained").into(),
                );
            }
        }
        let status = child.wait()?;
        watched?;
        // A short-lived child may finish between polls after exceeding a limit.
        if started.elapsed() >= deadline {
            timed_out = true;
            stopped = Some("timeout");
        } else if let Some(limit) = limits(self)? {
            stopped = Some(limit);
        }
        let exit = exit_code(status);
        let seconds = (started.elapsed().as_secs_f64() * 1000.0).round() / 1000.0;
        let mut entry = Json::object([
            ("args", Json::from(args.to_vec())),
            ("exit", Json::from(i64::from(exit))),
            ("timed_out", Json::from(timed_out)),
            ("stopped", Json::from(stopped)),
            ("memory_unit", Json::from(unit.clone())),
            ("seconds", Json::from(seconds)),
            ("stdout_sha256", Json::from(digest(&stdout)?)),
            ("stderr_sha256", Json::from(digest(&stderr)?)),
            ("stderr_bytes", Json::from(fs::metadata(&stderr)?.len())),
        ]);
        if let Some(hash) = reference_hash {
            entry.set("reference_sha256", hash);
            entry.set("reference_library_path", libraries);
        }
        if fs::metadata(&metrics).is_ok_and(|meta| meta.len() > 0) {
            let text = fs::read_to_string(&metrics)?;
            if let Some(last) = captured_text::lines(&text).last() {
                let values: Vec<&str> = last.split('\t').collect();
                if let [elapsed, peak] = values[..]
                    && let (Ok(elapsed), Ok(peak)) =
                        (elapsed.trim().parse::<f64>(), peak.trim().parse::<i64>())
                {
                    entry.set("process_seconds", elapsed);
                    entry.set("peak_rss_kib", peak);
                }
            }
        }
        match self.report.get_mut("commands") {
            Some(commands) => commands.set(name, entry.clone()),
            None => self
                .report
                .set("commands", Json::object([(name, entry.clone())])),
        }
        self.write()?;
        println!("{name}: exit {exit}, {} s", json_text::float(seconds));
        if required && (exit != 0 || stopped.is_some()) {
            return Err(format!("{name} failed; see {}", stderr.display()).into());
        }
        Ok(entry)
    }

    /// Pin every file actually selected, including headers reached via the sysroot.
    fn input_manifest(&self, names: &[String], path: &Path, follow: bool) -> Result<()> {
        let mut roots = vec![self.source.clone()];
        if let Some(sysroot) = &self.options.sysroot {
            roots.push(resolve(sysroot)?);
        }
        let mut output = BufWriter::new(File::create(path)?);
        for name in names {
            let file = self.source.join(name);
            if !follow && fs::symlink_metadata(&file).is_ok_and(|meta| meta.is_symlink()) {
                let link = fs::read_link(&file)?.to_string_lossy().into_owned();
                let line = Json::object([
                    ("file", Json::from(name.as_str())),
                    ("symlink", Json::from(link)),
                ]);
                writeln!(output, "{}", line.text(LINE))?;
                continue;
            }
            let file = file.canonicalize()?;
            if !roots.iter().any(|root| file.starts_with(root)) {
                return Err(format!(
                    "selected file escapes pinned trees: {name} -> {}",
                    file.display()
                )
                .into());
            }
            let line = Json::object([
                ("file", Json::from(name.as_str())),
                ("bytes", Json::from(file.metadata()?.len())),
                ("sha256", Json::from(digest(&file)?)),
            ]);
            writeln!(output, "{}", line.text(LINE))?;
        }
        output.flush()?;
        Ok(())
    }

    fn scanner(&mut self, database: &Path, names: &[String]) -> Result<()> {
        let listing = self.out.join("reference-files.txt");
        // The hidden scanner accepts one literal path per line, including spaces.
        fs::write(&listing, names.join("\n") + "\n")?;
        let args = [
            self.binary.clone(),
            "--dump-units".to_owned(),
            "-i".to_owned(),
            listing.to_string_lossy().into_owned(),
        ];
        self.command("scanner", &args, None, true)?;
        let (mut files, mut agreeing, mut records_total, mut matching) =
            (0_u64, 0_u64, 0_u64, 0_u64);
        let mut differing = Vec::new();
        let mut reference = cscope_db::Units::from_reader(BufReader::new(File::open(database)?))?;
        let mut mine = BufReader::new(File::open(self.out.join("scanner.stdout"))?).split(b'\n');
        loop {
            let (expected, raw) = (reference.next(), mine.next());
            let (expected, raw) = match (expected, raw) {
                (None, None) => break,
                (Some(expected), Some(raw)) => (expected?, raw?),
                _ => return Err("scanner and reference file counts differ".into()),
            };
            let (name, records) = expected;
            let item: Value = serde_json::from_slice(&raw)?;
            if item["file"].as_str() != Some(name.as_str()) {
                return Err("scanner changed the requested file order".into());
            }
            let actual = cscope_db::view_records(
                &item["records"]
                    .as_array()
                    .ok_or("scanner unit without records")?
                    .iter()
                    .map(cscope_db::Record::from_json)
                    .collect::<io::Result<Vec<_>>>()?,
            );
            files += 1;
            records_total += records.len() as u64;
            matching += actual.iter().zip(&records).filter(|(x, y)| x == y).count() as u64;
            if actual == records {
                agreeing += 1;
            } else {
                let first = (0..actual.len().max(records.len()))
                    .find(|at| actual.get(*at) != records.get(*at))
                    .unwrap_or(0);
                differing.push(Json::object([
                    ("file", Json::from(name.as_str())),
                    ("first_difference", Json::from(first)),
                    ("ripscope_records", Json::from(actual.len())),
                    ("reference_records", Json::from(records.len())),
                    (
                        "ripscope",
                        actual
                            .get(first)
                            .map_or(Json::Null, cscope_db::Record::to_json),
                    ),
                    (
                        "reference",
                        records
                            .get(first)
                            .map_or(Json::Null, cscope_db::Record::to_json),
                    ),
                ]));
            }
        }
        let diagnostics = self
            .report
            .get("commands")
            .and_then(|commands| commands.get("scanner"))
            .and_then(|entry| entry.get("stderr_bytes"))
            .cloned()
            .unwrap_or(Json::Null);
        let stats = Json::object([
            ("files", Json::from(files)),
            ("agreeing_files", Json::from(agreeing)),
            ("records", Json::from(records_total)),
            ("matching_records", Json::from(matching)),
            ("differing_files", Json::Array(differing)),
            ("diagnostics_bytes", diagnostics),
        ]);
        self.report.set("scanner", stats);
        self.write()?;
        println!("scanner: {agreeing}/{files} files identical");
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn fields(
        &mut self,
        fields: &[u8],
        indexes: [&Path; 3],
        selected: &[String],
        pattern: &str,
        label: &str,
        deadline: Option<u64>,
    ) -> Result<()> {
        let [reference, baseline, product] = indexes;
        let paths = file_prefixes(selected);
        for field in fields {
            let mut streams = Json::object(Vec::<(String, Json)>::new());
            for side in ["reference", "baseline", "product"] {
                let name = format!("{label}-{field}-{side}");
                let path_text = |path: &Path| path.to_string_lossy().into_owned();
                let (args, request) = match side {
                    "reference" => (
                        vec![
                            self.reference.clone(),
                            "-dL".into(),
                            "-f".into(),
                            path_text(reference),
                            format!("-{field}"),
                            pattern.into(),
                        ],
                        None,
                    ),
                    "baseline" => (
                        vec![
                            self.binary.clone(),
                            "--store-query".into(),
                            "-f".into(),
                            path_text(baseline),
                        ],
                        Some(format!("{field}{pattern}\n").into_bytes()),
                    ),
                    _ => (
                        vec![
                            self.binary.clone(),
                            "-dL".into(),
                            "-f".into(),
                            path_text(product),
                            format!("-{field}"),
                            pattern.into(),
                        ],
                        None,
                    ),
                };
                let original = self.options.timeout;
                if let Some(deadline) = deadline {
                    self.options.timeout = deadline;
                }
                let result = self.command(&name, &args, request.as_deref(), false);
                self.options.timeout = original;
                let result = result?;
                let rows = self.out.join(format!("{name}.rows"));
                let mut info = normalize(
                    &self.out.join(format!("{name}.stdout")),
                    &rows,
                    &paths,
                    side == "baseline",
                    side != "reference",
                )?;
                let usable = result.get("exit") == Some(&Json::Int(0))
                    && result.get("timed_out") == Some(&Json::Bool(false))
                    && result
                        .get("stopped")
                        .is_none_or(|value| *value == Json::Null)
                    && result
                        .get("stderr_bytes")
                        .is_none_or(|value| *value == Json::Int(0))
                    && info.get("malformed") == Some(&Json::Array(Vec::new()))
                    && info.get("framing_ok") == Some(&Json::Bool(true));
                info.set("usable", usable);
                streams.set(side, info);
                let sort_args = vec![
                    self.sort.clone(),
                    "-S".into(),
                    "256M".into(),
                    "-T".into(),
                    path_text(&self.out),
                    "-o".into(),
                    path_text(&self.out.join(format!("{name}.sorted"))),
                    path_text(&rows),
                ];
                self.command(
                    &format!("sort-{label}-{field}-{side}"),
                    &sort_args,
                    None,
                    true,
                )?;
            }
            let mut comparisons = Json::object(Vec::<(String, Json)>::new());
            let mut diagnostics = Json::object(Vec::<(String, Json)>::new());
            let stream_value =
                |side: &str, key: &str| streams.get(side).and_then(|info| info.get(key)).cloned();
            for side in ["baseline", "product"] {
                let mut stats = compare_sorted(
                    &self.out.join(format!("{label}-{field}-{side}.sorted")),
                    &self.out.join(format!("{label}-{field}-reference.sorted")),
                    &paths,
                )?;
                stats.set(
                    "ordered_equal",
                    stream_value(side, "sha256") == stream_value("reference", "sha256"),
                );
                let count = |value: Option<Json>| match value {
                    Some(Json::Int(count)) => count,
                    _ => -1,
                };
                let matched = count(stats.get("matched_rows").cloned());
                if matched + count(stats.get("ripscope_only").cloned())
                    != count(stream_value(side, "rows"))
                    || matched + count(stats.get("reference_only").cloned())
                        != count(stream_value("reference", "rows"))
                {
                    return Err("sorting changed the captured row counts".into());
                }
                diagnostics.set(side, stats.clone());
                if stream_value(side, "usable") == Some(Json::Bool(true))
                    && stream_value("reference", "usable") == Some(Json::Bool(true))
                {
                    comparisons.set(side, stats);
                }
            }
            let result = Json::object([
                ("streams", streams),
                ("comparisons", comparisons),
                ("diagnostic_comparisons", diagnostics),
            ]);
            let target = if label == "field" {
                self.report
                    .get_mut("fields")
                    .ok_or("report without fields")?
            } else {
                if self.report.get("exact_queries").is_none() {
                    self.report
                        .set("exact_queries", Json::object(Vec::<(String, Json)>::new()));
                }
                let exact = self.report.get_mut("exact_queries").expect("exact queries");
                if exact.get(pattern).is_none() {
                    exact.set(pattern, Json::object(Vec::<(String, Json)>::new()));
                }
                exact.get_mut(pattern).expect("pattern group")
            };
            target.set(&field.to_string(), result);
            self.write()?;
        }
        Ok(())
    }

    fn run(&mut self) -> Result<()> {
        if !self.unsupported_inputs.is_empty() {
            self.report.set("complete", false);
            self.report.set("run_finished", false);
            self.report.set("comparison_eligible", false);
            self.report.set("status", "unsupported_inputs");
            self.report.set(
                "unsupported_inputs",
                Json::Array(
                    self.unsupported_inputs
                        .iter()
                        .cloned()
                        .map(Json::from)
                        .collect(),
                ),
            );
            self.report.set("input_policy", "project-only");
            self.report.set(
                "error",
                "external inputs are outside the frozen project; no index command was launched",
            );
            return self.write();
        }
        let reference = self.out.join("cscope.out");
        let quick = self.out.join("cscope-q.out");
        let baseline = self.out.join("baseline.db");
        let product = self.out.join("product.db");
        // Reading tracked files establishes a warm source cache and a pre-run identity.
        let tracked_names: Vec<String> = match &self.manifest {
            Some(manifest) => manifest_paths(manifest)?,
            None => git(&self.source, &["ls-files", "-z"])?
                .split(|byte| *byte == 0)
                .filter(|name| !name.is_empty())
                .map(|name| String::from_utf8_lossy(name).into_owned())
                .collect(),
        };
        self.input_manifest(&tracked_names, &self.out.join("source-before.jsonl"), false)?;
        let (cscope, ripscope) = (self.reference.clone(), self.binary.clone());
        let builds = [
            ("reference-build", &cscope, &["-b"][..], &reference, true),
            ("product-build", &ripscope, &["-b"][..], &product, true),
            (
                "reference-quick-build",
                &cscope,
                &["-bq"][..],
                &quick,
                false,
            ),
            (
                "baseline-build",
                &ripscope,
                &["--store-build"][..],
                &baseline,
                true,
            ),
            (
                "product-refresh",
                &ripscope,
                &["--store-build", "--corrections", "enabled"][..],
                &product,
                true,
            ),
            (
                "product-refresh-cli",
                &ripscope,
                &["-b"][..],
                &product,
                true,
            ),
        ];
        for (name, program, flags, index, required) in builds {
            let mut args = vec![program.clone()];
            args.extend(flags.iter().map(|flag| (*flag).to_owned()));
            args.extend(self.selection.iter().cloned());
            args.extend(["-f".to_owned(), index.to_string_lossy().into_owned()]);
            self.command(name, &args, None, required)?;
        }
        let mut names = Vec::new();
        for unit in cscope_db::Units::from_reader(BufReader::new(File::open(&reference)?))? {
            names.push(unit?.0);
        }
        let read_names = |file: &str| -> Result<Vec<String>> {
            let bytes = fs::read(self.out.join(file))?;
            Ok(captured_text::lines(&String::from_utf8_lossy(&bytes))
                .into_iter()
                .map(str::to_owned)
                .collect())
        };
        let baseline_names = read_names("baseline-build.stdout")?;
        let product_names = read_names("product-refresh.stdout")?;
        let known: BTreeSet<&String> = names.iter().collect();
        let built: BTreeSet<&String> = baseline_names.iter().collect();
        self.report.set(
            "files",
            Json::object([
                ("reference", Json::from(names.len())),
                ("baseline", Json::from(baseline_names.len())),
                ("product", Json::from(product_names.len())),
                ("baseline_order_equal", Json::from(baseline_names == names)),
                ("product_order_equal", Json::from(product_names == names)),
                (
                    "baseline_only",
                    Json::from(
                        built
                            .difference(&known)
                            .map(|n| n.as_str())
                            .collect::<Vec<_>>(),
                    ),
                ),
                (
                    "reference_only",
                    Json::from(
                        known
                            .difference(&built)
                            .map(|n| n.as_str())
                            .collect::<Vec<_>>(),
                    ),
                ),
            ]),
        );
        let selected: Vec<String> = names
            .iter()
            .chain(&baseline_names)
            .chain(&product_names)
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        if let Some(closure) = &self.closure {
            let pinned: BTreeSet<String> = manifest_paths(closure)?.into_iter().collect();
            let unexpected: Vec<&String> = selected
                .iter()
                .filter(|name| !pinned.contains(*name))
                .collect();
            if !unexpected.is_empty() {
                return Err(format!(
                    "discovered unpinned inputs: {:?}",
                    &unexpected[..unexpected.len().min(20)]
                )
                .into());
            }
        }
        self.input_manifest(&selected, &self.out.join("selected-before.jsonl"), true)?;
        self.write()?;
        self.scanner(&reference, &names)?;
        let indexes = [reference.as_path(), baseline.as_path(), product.as_path()];
        self.fields(&FIELDS, indexes, &selected, ".*", "field", None)?;
        if let Some(path) = self.options.exact_inventory.clone() {
            let patterns = read_json(&path)?;
            for (at, pattern) in patterns
                .as_array()
                .ok_or("exact inventory is not a list")?
                .iter()
                .enumerate()
            {
                let pattern = pattern.as_str().ok_or("exact pattern is not text")?;
                self.fields(
                    &FIELDS,
                    indexes,
                    &selected,
                    pattern,
                    &format!("exact-{at}"),
                    Some(120),
                )?;
            }
        }
        self.input_manifest(&selected, &self.out.join("selected-after.jsonl"), true)?;
        self.input_manifest(&tracked_names, &self.out.join("source-after.jsonl"), false)?;
        self.check_source()?;
        let mut unchanged = true;
        for kind in ["source", "selected"] {
            unchanged &= digest(&self.out.join(format!("{kind}-before.jsonl")))?
                == digest(&self.out.join(format!("{kind}-after.jsonl")))?;
        }
        self.report.set("inputs_unchanged", unchanged);
        if !unchanged
            || Some(&Json::from(digest(Path::new(&self.binary))?))
                != self.report.get("binary_sha256")
            || digest(Path::new(&self.reference))? != REFERENCE_SHA256
        {
            return Err("inputs changed during the run".into());
        }
        self.report.set(
            "index_bytes",
            Json::object([
                ("reference", Json::from(fs::metadata(&reference)?.len())),
                ("baseline", Json::from(fs::metadata(&baseline)?.len())),
                ("product", Json::from(fs::metadata(&product)?.len())),
            ]),
        );
        let mut quick_bytes = 0;
        for entry in fs::read_dir(&self.out)? {
            let entry = entry?;
            if entry.file_name().to_string_lossy().starts_with("cscope-q.") {
                quick_bytes += entry.metadata()?.len();
            }
        }
        self.report.set("reference_quick_bytes", quick_bytes);
        let mut groups = vec![self.report.get("fields").cloned()];
        if let Some(Json::Object(exact)) = self.report.get("exact_queries") {
            groups.extend(exact.iter().map(|(_, group)| Some(group.clone())));
        }
        let complete = groups.iter().flatten().all(|group| match group {
            Json::Object(fields) => fields.iter().all(|(_, field)| match field.get("streams") {
                Some(Json::Object(streams)) => streams
                    .iter()
                    .all(|(_, info)| info.get("usable") == Some(&Json::Bool(true))),
                _ => true,
            }),
            _ => true,
        });
        self.report.set("complete", complete);
        self.report.set("run_finished", true);
        self.write()
    }
}

fn manifest_paths(manifest: &Value) -> Result<Vec<String>> {
    manifest["files"]
        .as_array()
        .ok_or("manifest without files")?
        .iter()
        .map(|file| {
            file["path"]
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| "manifest file without a path".into())
        })
        .collect()
}

fn directory_size(directory: &Path) -> io::Result<u64> {
    let mut total = 0;
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let meta = entry.metadata()?;
        if meta.is_file() {
            total += meta.len();
        }
    }
    Ok(total)
}

fn kill_group(pid: u32) -> Result<()> {
    #[cfg(unix)]
    {
        use rustix::process::{Pid, Signal, kill_process_group};
        let pid = Pid::from_raw(i32::try_from(pid)?).ok_or("invalid process id")?;
        match kill_process_group(pid, Signal::KILL) {
            Ok(()) | Err(rustix::io::Errno::SRCH) => {}
            Err(error) => return Err(error.into()),
        }
    }
    #[cfg(not(unix))]
    let _ = pid;
    Ok(())
}

/// Runs `systemctl --user` with `words`, giving whether it succeeded and its output. A
/// stuck user manager fails the run after 10 seconds instead of hanging it.
fn systemctl(words: &[&str], environment: &[(String, String)]) -> Result<(bool, Vec<u8>)> {
    let mut child = Command::new("systemctl")
        .arg("--user")
        .args(words)
        .env_clear()
        .envs(environment.iter().map(|(k, v)| (k, v)))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let mut pipe = child.stdout.take().ok_or("systemctl output")?;
    let reader = std::thread::spawn(move || {
        let mut output = Vec::new();
        let _ = io::Read::read_to_end(&mut pipe, &mut output);
        output
    });
    let started = Instant::now();
    while child.try_wait()?.is_none() {
        if started.elapsed() >= Duration::from_secs(10) {
            let _ = child.kill();
            child.wait()?;
            return Err(format!("systemctl --user {} timed out", words.join(" ")).into());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let success = child.wait()?.success();
    Ok((success, reader.join().unwrap_or_default()))
}

fn exit_code(status: std::process::ExitStatus) -> i32 {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            return -signal;
        }
    }
    status.code().unwrap_or(1)
}

/// Complete paths by a row's first word, longest candidate first.
fn file_prefixes(names: &[String]) -> BTreeMap<Vec<u8>, Vec<Vec<u8>>> {
    let mut prefixes: BTreeMap<Vec<u8>, Vec<Vec<u8>>> = BTreeMap::new();
    for name in names {
        let path = name.as_bytes().to_vec();
        let word = path
            .split(|byte| *byte == b' ')
            .next()
            .unwrap_or_default()
            .to_vec();
        prefixes.entry(word).or_default().push(path);
    }
    for paths in prefixes.values_mut() {
        paths.sort_by_key(|path| std::cmp::Reverse(path.len()));
    }
    prefixes
}

/// A row as (path, context, line number, text), when it starts with a known path.
fn row_parts<'a>(line: &'a [u8], paths: &BTreeMap<Vec<u8>, Vec<Vec<u8>>>) -> Option<[&'a [u8]; 4]> {
    let word = line.split(|byte| *byte == b' ').next().unwrap_or_default();
    for path in paths.get(word)? {
        if line.len() > path.len() && line.starts_with(path) && line[path.len()] == b' ' {
            let tail = &line[path.len() + 1..];
            if let Some(found) = ROW_TAIL.captures(tail) {
                let part = |at: usize| found.get(at).map_or(&b""[..], |m| m.as_bytes());
                return Some([&line[..path.len()], part(1), part(2), part(3)]);
            }
        }
    }
    None
}

/// The bytes as a byte-string literal, as malformed rows have always been recorded.
fn bytes_repr(bytes: &[u8]) -> String {
    let quote = if bytes.contains(&b'\'') && !bytes.contains(&b'"') {
        '"'
    } else {
        '\''
    };
    let mut out = format!("b{quote}");
    for byte in bytes {
        match byte {
            b'\\' => out.push_str("\\\\"),
            b'\t' => out.push_str("\\t"),
            b'\n' => out.push_str("\\n"),
            b'\r' => out.push_str("\\r"),
            byte if char::from(*byte) == quote => {
                out.push('\\');
                out.push(quote);
            }
            0x20..=0x7e => out.push(char::from(*byte)),
            byte => out.push_str(&format!("\\x{byte:02x}")),
        }
    }
    out.push(quote);
    out
}

/// UTF-8 text with each undecodable byte written as `\xNN`.
fn backslash_replace(bytes: &[u8]) -> String {
    let mut out = String::new();
    for chunk in bytes.utf8_chunks() {
        out.push_str(chunk.valid());
        for byte in chunk.invalid() {
            out.push_str(&format!("\\x{byte:02x}"));
        }
    }
    out
}

/// Keeps byte-exact rows, checking hidden-command counts and reference errors.
fn normalize(
    source: &Path,
    target: &Path,
    paths: &BTreeMap<Vec<u8>, Vec<Vec<u8>>>,
    hidden: bool,
    view: bool,
) -> Result<Json> {
    let (mut rows, mut markers, mut bad) = (0_u64, Vec::new(), Vec::new());
    let mut after_marker = false;
    let mut incoming = BufReader::new(File::open(source)?);
    let mut outgoing = BufWriter::new(File::create(target)?);
    let mut line = Vec::new();
    loop {
        line.clear();
        if incoming.read_until(b'\n', &mut line)? == 0 {
            break;
        }
        if hidden && line.starts_with(MARK) {
            let count = std::str::from_utf8(&line[MARK.len()..])
                .ok()
                .and_then(|text| text.trim().parse::<i64>().ok())
                .unwrap_or(-1);
            markers.push(count);
            after_marker = true;
            continue;
        }
        let parts = row_parts(&line, paths);
        let Some([path, context, number, text]) = parts.filter(|_| !after_marker) else {
            if bad.len() < 5 {
                bad.push(Json::from(bytes_repr(&line[..line.len().min(300)])));
            }
            continue;
        };
        if view && VIEW_BYTES.is_match(&line) {
            let shown = |bytes: &[u8]| -> Vec<u8> {
                cscope_db::reference_view(&cscope_db::latin1(bytes))
                    .chars()
                    .map(|ch| u8::try_from(u32::from(ch)).unwrap_or(b'?'))
                    .collect()
            };
            let rewritten =
                [path.to_vec(), shown(context), number.to_vec(), shown(text)].join(&b' ');
            outgoing.write_all(&rewritten)?;
            outgoing.write_all(b"\n")?;
        } else {
            outgoing.write_all(&line)?;
        }
        rows += 1;
    }
    outgoing.flush()?;
    drop(outgoing);
    let framing_ok = !hidden || markers == [rows as i64];
    Ok(Json::object([
        ("rows", Json::from(rows)),
        ("malformed", Json::Array(bad)),
        ("markers", Json::from(markers)),
        ("framing_ok", Json::from(framing_ok)),
        ("sha256", Json::from(digest(target)?)),
    ]))
}

/// Merges sorted rows, preserving duplicates and recording per-file differences.
fn compare_sorted(
    mine: &Path,
    theirs: &Path,
    paths: &BTreeMap<Vec<u8>, Vec<Vec<u8>>>,
) -> Result<Json> {
    let (mut matched, mut missing, mut extra) = (0_u64, 0_u64, 0_u64);
    let mut files: Vec<(String, u64)> = Vec::new();
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    let mut samples = Vec::new();
    let mut a = BufReader::new(File::open(mine)?);
    let mut b = BufReader::new(File::open(theirs)?);
    let next = |reader: &mut BufReader<File>| -> io::Result<Option<Vec<u8>>> {
        let mut line = Vec::new();
        Ok((reader.read_until(b'\n', &mut line)? > 0).then_some(line))
    };
    let (mut x, mut y) = (next(&mut a)?, next(&mut b)?);
    while x.is_some() || y.is_some() {
        if x.is_some() && x == y {
            matched += 1;
            x = next(&mut a)?;
            y = next(&mut b)?;
            continue;
        }
        let own = match (&x, &y) {
            (_, None) => true,
            (Some(x), Some(y)) => x < y,
            (None, Some(_)) => false,
        };
        let row = if own { x.as_ref() } else { y.as_ref() }.expect("a row on the chosen side");
        let [path, ..] = row_parts(row, paths).ok_or("sorted stream contains a malformed row")?;
        let name = String::from_utf8_lossy(path).into_owned();
        match seen.get(&name) {
            Some(at) => files[*at].1 += 1,
            None => {
                seen.insert(name.clone(), files.len());
                files.push((name, 1));
            }
        }
        if samples.len() < 10 {
            samples.push(Json::object([
                (
                    "side",
                    Json::from(if own { "ripscope" } else { "reference" }),
                ),
                (
                    "row",
                    Json::from(backslash_replace(row).trim_end_matches('\n').to_owned()),
                ),
            ]));
        }
        if own {
            extra += 1;
            x = next(&mut a)?;
        } else {
            missing += 1;
            y = next(&mut b)?;
        }
    }
    let differing = files.len();
    // Most differences first; equal counts keep the order the files first differed in.
    files.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
    Ok(Json::object([
        ("matched_rows", Json::from(matched)),
        ("ripscope_only", Json::from(extra)),
        ("reference_only", Json::from(missing)),
        ("differing_files", Json::from(differing)),
        (
            "differing_files_top",
            Json::Array(
                files
                    .into_iter()
                    .take(20)
                    .map(|(name, count)| Json::Array(vec![Json::from(name), Json::from(count)]))
                    .collect(),
            ),
        ),
        ("samples", Json::Array(samples)),
    ]))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use serde_json::json;
    use std::os::unix::fs::symlink;

    fn bare(source: &Path, out: &Path) -> Run {
        Run {
            options: Options {
                binary: PathBuf::from("product"),
                reference: PathBuf::from("reference"),
                source: source.to_owned(),
                output: out.to_owned(),
                sysroot: None,
                timeout: 10,
                build_timeout: 10,
                manifest: None,
                closure_manifest: None,
                exact_inventory: None,
                memory_scope: false,
                output_limit: 2 << 30,
                project_limit: 10000,
            },
            out: out.to_owned(),
            source: source.to_owned(),
            env: vec![
                ("PATH".to_owned(), "/usr/bin:/bin".to_owned()),
                ("LC_ALL".to_owned(), "C".to_owned()),
            ],
            sort: "sort".to_owned(),
            selection: Vec::new(),
            manifest: None,
            closure: None,
            binary: "product".to_owned(),
            reference: "reference".to_owned(),
            report: Json::object([
                ("commands", Json::object(Vec::<(String, Json)>::new())),
                ("fields", Json::object(Vec::<(String, Json)>::new())),
            ]),
            unsupported_inputs: Vec::new(),
            launch: Run::execute,
        }
    }

    fn sha(path: &Path) -> String {
        digest(path).unwrap()
    }

    #[test]
    fn fast_exit_does_not_bypass_output_limit() {
        if !Path::new("/usr/bin/time").is_file() {
            eprintln!("GNU time is not installed; skipping");
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let mut run = bare(root.path(), root.path());
        run.options.output_limit = 1;
        let args = ["/bin/sh", "-c", "printf abcde"].map(str::to_owned);
        let result = run.command("fast", &args, None, false).unwrap();
        assert_eq!(result.get("exit"), Some(&Json::Int(0)));
        assert_eq!(result.get("stopped"), Some(&Json::from("output_limit")));
    }

    #[test]
    fn changed_frozen_closure_stops_before_capture() {
        let root = tempfile::tempdir().unwrap();
        let mut run = bare(root.path(), root.path());
        let header = root.path().join("extra.inc");
        fs::write(&header, "before").unwrap();
        run.manifest = Some(json!({"revision": "archive-sha256:example", "files": []}));
        run.closure = Some(json!({"files": [{"path": "extra.inc", "sha256": sha(&header)}]}));
        run.check_source().unwrap();
        fs::write(&header, "after").unwrap();
        let error = run.check_source().unwrap_err().to_string();
        assert!(error.contains("frozen source changed"), "{error}");
    }

    #[test]
    fn alias_retargeting_to_equal_bytes_invalidates_frozen_inputs() {
        let root = tempfile::tempdir().unwrap();
        let mut run = bare(root.path(), root.path());
        let (one, two, alias) = (
            root.path().join("one.h"),
            root.path().join("two.h"),
            root.path().join("alias.h"),
        );
        fs::write(&one, "int same;\n").unwrap();
        fs::write(&two, "int same;\n").unwrap();
        symlink("one.h", &alias).unwrap();
        run.manifest = Some(json!({"revision": "archive-sha256:example", "files": []}));
        run.closure = Some(json!({"inventory_version": 3, "files": [{"path": "alias.h",
            "target": "one.h", "links": [{"path": "alias.h", "target": "one.h"}],
            "sha256": sha(&one)}], "unsupported_inputs": []}));
        run.check_source().unwrap();
        fs::remove_file(&alias).unwrap();
        symlink("two.h", &alias).unwrap();
        let error = run.check_source().unwrap_err().to_string();
        assert!(error.contains("frozen source changed"), "{error}");
    }

    fn must_not_launch(
        _: &mut Run,
        _: &str,
        _: &[String],
        _: Option<&[u8]>,
        _: bool,
    ) -> Result<Json> {
        panic!("an index command must not run");
    }

    #[test]
    fn legacy_alias_frame_stops_absolute_operands_before_index_commands() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        let out = root.path().join("output");
        fs::create_dir(&source).unwrap();
        fs::create_dir(&out).unwrap();
        fs::write(source.join("source.c"), "#include \"alias.h\"\n").unwrap();
        fs::write(
            source.join("real.h"),
            "#include \"/unrecorded-host/missing.h\"\n",
        )
        .unwrap();
        symlink("real.h", source.join("alias.h")).unwrap();
        let files: Vec<Value> = ["source.c", "real.h"]
            .iter()
            .map(|name| json!({"path": name, "sha256": sha(&source.join(name))}))
            .collect();
        let mut run = bare(&source, &out);
        run.manifest = Some(json!({"revision": "archive-sha256:example", "files": files}));
        // No version 3 safety metadata.
        run.closure = Some(json!({"files": files}));
        run.launch = must_not_launch;
        run.check_source().unwrap();
        run.run().unwrap();
        assert_eq!(run.report.get("complete"), Some(&Json::Bool(false)));
        assert_eq!(
            run.report.get("comparison_eligible"),
            Some(&Json::Bool(false))
        );
        assert_eq!(
            run.report.get("status"),
            Some(&Json::from("unsupported_inputs"))
        );
        assert_eq!(
            run.report.get("fields"),
            Some(&Json::object(Vec::<(String, Json)>::new()))
        );
        let Some(Json::Array(items)) = run.report.get("unsupported_inputs") else {
            panic!("no unsupported inputs");
        };
        assert!(
            items
                .iter()
                .any(|item| item.get("operand") == Some(&Json::from("/unrecorded-host/missing.h")))
        );
    }

    #[test]
    fn parent_alias_link_spelling_and_missing_targets_are_checked() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().canonicalize().unwrap();
        let mut run = bare(&source, &source);
        fs::create_dir(source.join("include")).unwrap();
        let header = source.join("include/real.h");
        fs::write(&header, "int same;\n").unwrap();
        symlink("include", source.join("view")).unwrap();
        let alias = source.join("include/alias.h");
        symlink("real.h", &alias).unwrap();
        run.manifest = Some(json!({"revision": "archive-sha256:example", "files": []}));
        run.closure = Some(
            json!({"files": [{"path": "view/alias.h", "target": "include/real.h",
            "links": [{"path": "view", "target": "include"},
                      {"path": "include/alias.h", "target": "real.h"}],
            "sha256": sha(&header)}]}),
        );
        run.check_source().unwrap();
        fs::remove_file(&alias).unwrap();
        symlink("./real.h", &alias).unwrap();
        assert!(
            run.check_source()
                .unwrap_err()
                .to_string()
                .contains("frozen source changed")
        );
        fs::remove_file(&alias).unwrap();
        symlink("real.h", &alias).unwrap();
        fs::remove_file(&header).unwrap();
        assert!(
            run.check_source()
                .unwrap_err()
                .to_string()
                .contains("frozen source changed")
        );
    }

    #[test]
    fn v3_closure_cannot_omit_target_and_link_identities() {
        let root = tempfile::tempdir().unwrap();
        let mut run = bare(root.path(), root.path());
        fs::write(root.path().join("a.h"), "int known;\n").unwrap();
        run.manifest = Some(json!({"revision": "archive-sha256:example", "files": []}));
        run.closure = Some(json!({"inventory_version": 3, "files": [{"path": "a.h",
            "sha256": sha(&root.path().join("a.h"))}], "unsupported_inputs": []}));
        let error = run.check_source().unwrap_err().to_string();
        assert!(
            error.contains("incomplete frozen source identity"),
            "{error}"
        );
    }

    #[test]
    fn preflight_distinguishes_local_includes_and_unrecorded_external_inputs() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("a.c"), "#include \"local.h\"\n").unwrap();
        fs::write(source.join("local.h"), "int known;\n").unwrap();
        assert!(
            source_inventory::preflight(&source, &["a.c"], &[])
                .unwrap()
                .is_empty()
        );
        let outside = root.path().join("outside.h");
        fs::write(&outside, "#include \"/must-not-inspect/next.h\"\n").unwrap();
        symlink(&outside, source.join("external.h")).unwrap();
        fs::write(source.join("local.h"), "#include \"external.h\"\n").unwrap();
        assert_eq!(
            source_inventory::preflight(&source, &["a.c"], &[]).unwrap(),
            [json!({"source": "local.h", "operand": "external.h",
                    "reason": "alias target outside declared project"})]
        );
        fs::write(
            source.join("local.h"),
            "int x; #include \"/missing-host/input.h\"\n",
        )
        .unwrap();
        assert_eq!(
            source_inventory::preflight(&source, &["a.c"], &[]).unwrap()[0]["operand"],
            "/missing-host/input.h"
        );
    }

    #[test]
    fn missing_relative_candidate_does_not_hide_an_in_project_include() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        fs::create_dir_all(source.join("include")).unwrap();
        fs::write(source.join("a.c"), "#include \"../local.h\"\n").unwrap();
        fs::write(source.join("local.h"), "int known;\n").unwrap();
        assert!(
            source_inventory::preflight(&source, &["a.c"], &["include"])
                .unwrap()
                .is_empty()
        );
        fs::write(
            root.path().join("local.h"),
            "#include \"/must-not-read/nested.h\"\n",
        )
        .unwrap();
        assert_eq!(
            source_inventory::preflight(&source, &["a.c"], &["include"]).unwrap(),
            [json!({"source": "a.c", "operand": "../local.h",
                    "reason": "relative input outside declared project"})]
        );
    }

    fn staged(
        run: &mut Run,
        name: &str,
        args: &[String],
        _: Option<&[u8]>,
        _: bool,
    ) -> Result<Json> {
        if name.starts_with("sort-") {
            fs::write(
                &args[args.len() - 2],
                fs::read(&args[args.len() - 1]).unwrap(),
            )
            .unwrap();
        } else {
            let mut data = b"file.c <global> 1 int value;\n".to_vec();
            if name.ends_with("reference") {
                data.extend_from_slice(b"malformed tail\n");
            }
            if name.ends_with("baseline") {
                data.extend_from_slice(b"##units-query## 1\n");
            }
            fs::write(run.out.join(format!("{name}.stdout")), data).unwrap();
        }
        Ok(Json::object([
            ("exit", Json::Int(0)),
            ("timed_out", Json::Bool(false)),
            ("stopped", Json::Null),
            ("stderr_bytes", Json::Int(0)),
        ]))
    }

    #[test]
    fn malformed_reference_has_no_agreement_denominator() {
        let root = tempfile::tempdir().unwrap();
        let mut run = bare(root.path(), root.path());
        run.launch = staged;
        let indexes = [
            Path::new("ref"),
            Path::new("baseline"),
            Path::new("product"),
        ];
        run.fields(&[1], indexes, &["file.c".to_owned()], ".*", "field", None)
            .unwrap();
        let field = run
            .report
            .get("fields")
            .and_then(|fields| fields.get("1"))
            .unwrap();
        let stream = |side: &str| field.get("streams").and_then(|s| s.get(side)).unwrap();
        assert_eq!(stream("reference").get("usable"), Some(&Json::Bool(false)));
        assert_eq!(
            field.get("comparisons"),
            Some(&Json::object(Vec::<(String, Json)>::new()))
        );
        assert_eq!(
            field
                .get("diagnostic_comparisons")
                .and_then(|d| d.get("product"))
                .and_then(|p| p.get("matched_rows")),
            Some(&Json::Int(1))
        );
    }

    #[test]
    fn hidden_stream_requires_its_complete_counted_frame() {
        let row = b"file.c <global> 1 int value;\n".to_vec();
        let root = tempfile::tempdir().unwrap();
        let (source, target) = (root.path().join("in"), root.path().join("out"));
        let marker = |count: &str| format!("##units-query## {count}\n").into_bytes();
        for (data, framed, malformed) in [
            ([row.clone(), marker("1")].concat(), true, false),
            (row.clone(), false, false),
            ([row.clone(), marker("2")].concat(), false, false),
            ([row.clone(), marker("1"), row.clone()].concat(), true, true),
            (
                b"cannot get source line from database\n".to_vec(),
                false,
                true,
            ),
        ] {
            fs::write(&source, &data).unwrap();
            let result = normalize(
                &source,
                &target,
                &file_prefixes(&["file.c".to_owned()]),
                true,
                false,
            )
            .unwrap();
            assert_eq!(
                result.get("framing_ok"),
                Some(&Json::Bool(framed)),
                "{data:?}"
            );
            assert_eq!(
                result.get("malformed") != Some(&Json::Array(Vec::new())),
                malformed,
                "{data:?}"
            );
        }
    }

    #[test]
    fn multiset_comparison_keeps_duplicate_rows() {
        let one = b"one.c <global> 1 int one;\n";
        let two = b"two.c <global> 2 int two;\n";
        let root = tempfile::tempdir().unwrap();
        let (mine, theirs) = (root.path().join("mine"), root.path().join("theirs"));
        fs::write(&mine, [&one[..], &one[..]].concat()).unwrap();
        fs::write(&theirs, [&one[..], &two[..]].concat()).unwrap();
        let result = compare_sorted(
            &mine,
            &theirs,
            &file_prefixes(&["one.c".to_owned(), "two.c".to_owned()]),
        )
        .unwrap();
        assert_eq!(result.get("matched_rows"), Some(&Json::Int(1)));
        assert_eq!(result.get("ripscope_only"), Some(&Json::Int(1)));
        assert_eq!(result.get("reference_only"), Some(&Json::Int(1)));
        assert_eq!(result.get("differing_files"), Some(&Json::Int(2)));
    }

    #[test]
    fn spaced_paths_are_complete_and_not_confused_with_prefixes() {
        let paths =
            file_prefixes(&["two", "two words.c", "two words.c again.c"].map(str::to_owned));
        let row = b"two words.c again.c <global> 1 int value;\n";
        let root = tempfile::tempdir().unwrap();
        let (source, target, empty) = (
            root.path().join("in"),
            root.path().join("out"),
            root.path().join("empty"),
        );
        fs::write(&source, row).unwrap();
        fs::write(&empty, b"").unwrap();
        let result = normalize(&source, &target, &paths, false, false).unwrap();
        assert_eq!(result.get("malformed"), Some(&Json::Array(Vec::new())));
        assert_eq!(fs::read(&target).unwrap(), row);
        let diff = compare_sorted(&target, &empty, &paths).unwrap();
        assert_eq!(
            diff.get("differing_files_top"),
            Some(&Json::Array(vec![Json::Array(vec![
                Json::from("two words.c again.c"),
                Json::Int(1)
            ])]))
        );
    }

    #[test]
    fn sorter_added_blank_rows_are_rejected() {
        let root = tempfile::tempdir().unwrap();
        let (mine, theirs) = (root.path().join("mine"), root.path().join("theirs"));
        fs::write(&mine, b"\nfile.c <global> 1 int value;\n").unwrap();
        fs::write(&theirs, b"").unwrap();
        let error =
            compare_sorted(&mine, &theirs, &file_prefixes(&["file.c".to_owned()])).unwrap_err();
        assert!(error.to_string().contains("malformed row"), "{error}");
    }

    #[test]
    fn malformed_rows_are_recorded_as_byte_literals() {
        assert_eq!(bytes_repr(b"a'b\\\n\xff"), "b\"a'b\\\\\\n\\xff\"");
        assert_eq!(bytes_repr(b"tail\n"), "b'tail\\n'");
        assert_eq!(backslash_replace(b"ok \xff\n"), "ok \\xff\n");
    }
}
