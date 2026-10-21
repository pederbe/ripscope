//! Bounded real-Vim integration evidence: Vim's built-in cscope reader with a persistent
//! backend, and with `--workflow` the shipped plugin's delivered configuration.
use regex::Regex;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::sync::LazyLock;
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[path = "../../examples/support/editor_process.rs"]
#[allow(dead_code)]
mod editor_process;
use editor_process::{EditorProcess, os_release, process_alive, real_path};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const PINNED_VIM_SHA256: &str = "2370a103e4886c7c35ea7860cdfa0625483d96fb65a6cb52353604be3ffe9c69";
const USAGE: &str = "check-vim --backend EXE --output REPORT.json [--vim EXE] [--vim-sha256 SHA256] \
[--vim-dll-sha256 SHA256] [--workflow] [--repo DIR]";

/// A backend's process id in a `:cs show` connection listing.
static CONNECTION_PID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^\s*\d+\s+(\d+)\s+").expect("connection expression"));

fn main() -> ExitCode {
    match run() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(error) => {
            eprintln!("Vim check failed: {error}");
            ExitCode::FAILURE
        }
    }
}

struct Options {
    workflow: bool,
    backend: PathBuf,
    vim: PathBuf,
    vim_sha256: String,
    output: PathBuf,
    vim_dll_sha256: Option<String>,
    repo: PathBuf,
}

fn options() -> Result<Options> {
    let mut workflow = false;
    let mut backend = None;
    let mut vim = PathBuf::from("/usr/bin/vim");
    let mut vim_sha256 = PINNED_VIM_SHA256.to_owned();
    let mut output = None;
    let mut vim_dll_sha256 = None;
    let mut repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut args = env::args_os().skip(1);
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--help" | "-h") => {
                println!("{USAGE}");
                std::process::exit(0);
            }
            Some("--workflow") => {
                workflow = true;
                continue;
            }
            _ => {}
        }
        let value = args.next().ok_or("missing option value")?;
        match arg.to_str() {
            Some("--backend") => backend = Some(PathBuf::from(value)),
            Some("--vim") => vim = PathBuf::from(value),
            Some("--vim-sha256") => vim_sha256 = value.to_string_lossy().into_owned(),
            Some("--output") => output = Some(PathBuf::from(value)),
            Some("--vim-dll-sha256") => vim_dll_sha256 = Some(value.to_string_lossy().into_owned()),
            Some("--repo") => repo = PathBuf::from(value),
            _ => return Err(format!("unknown option: {}\n{USAGE}", arg.to_string_lossy()).into()),
        }
    }
    Ok(Options {
        workflow,
        backend: backend.ok_or("--backend is required")?,
        vim,
        vim_sha256,
        output: std::path::absolute(output.ok_or("--output is required")?)?,
        vim_dll_sha256,
        repo: std::path::absolute(repo)?,
    })
}

fn digest(path: &Path) -> Result<String> {
    Ok(base16ct::lower::encode_string(&Sha256::digest(fs::read(
        path,
    )?)))
}

fn ensure(condition: bool, message: impl Into<String>) -> Result<()> {
    if condition {
        Ok(())
    } else {
        Err(message.into().into())
    }
}

/// Each fixture file a Windows retry waited on, with how many times it waited.
static FILE_RETRIES: Mutex<Vec<Value>> = Mutex::new(Vec::new());

/// Runs a file operation, retrying for a few seconds while Windows still holds the file
/// for a process that just ended or for a scan of it, and names the file on failure.
fn settled<T>(path: &Path, mut action: impl FnMut() -> std::io::Result<T>) -> Result<T> {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut waits = 0;
    let outcome = loop {
        match action() {
            Ok(value) => break Ok(value),
            // Sharing and lock violations, and a section still mapped from the file.
            Err(error)
                if cfg!(windows)
                    && matches!(error.raw_os_error(), Some(32 | 33 | 1224))
                    && Instant::now() < deadline =>
            {
                waits += 1;
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(error) => break Err(format!("{}: {error}", path.display()).into()),
        }
    };
    if waits > 0 {
        FILE_RETRIES
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(json!({"path": path, "waits": waits}));
    }
    outcome
}

/// Fixture text with the platform's line ends: CRLF on Windows, as files there have them.
fn write_text(path: &Path, text: &str) -> Result<()> {
    let text = if cfg!(windows) {
        text.replace('\n', "\r\n")
    } else {
        text.to_owned()
    };
    settled(path, || fs::write(path, &text))
}

/// A path with forward slashes, as Vim scripts take it on every platform.
fn slashes(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn connection_pids(observations: &Value) -> Vec<u32> {
    observations
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| item.get("connection").and_then(Value::as_str))
        .flat_map(|connection| {
            CONNECTION_PID
                .captures_iter(connection)
                .filter_map(|found| found[1].parse().ok())
                .collect::<Vec<u32>>()
        })
        .collect()
}

fn run() -> Result<bool> {
    let options = options()?;
    #[cfg(unix)]
    if options.workflow && cfg!(target_os = "linux") && rustix::process::geteuid().is_root() {
        return Err("--workflow requires a non-root Linux user on Linux".into());
    }
    if !cfg!(any(target_os = "linux", windows)) {
        return Err("only Linux and native Windows are supported".into());
    }
    let backend = real_path(&options.backend)?;
    let vim = real_path(&options.vim)?;
    if digest(&vim)? != options.vim_sha256 {
        return Err("Vim identity differs; select and record a new profile explicitly".into());
    }
    let vim_dll = cfg!(windows).then(|| vim.with_file_name("vim64.dll"));
    if let Some(dll) = &vim_dll {
        let matching = match &options.vim_dll_sha256 {
            Some(expected) => digest(dll)? == *expected,
            None => false,
        };
        if !matching {
            return Err("provide the matching --vim-dll-sha256 for Windows Vim".into());
        }
    }
    let script = real_path(&options.repo.join("tests/editor/persistent.vim"))?;
    let mut report = json!({
        "backend": backend, "backend_sha256": digest(&backend)?,
        "vim": vim, "vim_sha256": digest(&vim)?,
        "driver_sha256": digest(&options.repo.join("tests/editor/check_vim.rs"))?,
        "driver_executable_sha256": digest(&env::current_exe()?)?,
        "script_sha256": digest(&script)?,
        "platform": format!("{}-{}", env::consts::OS, env::consts::ARCH),
        "os_release": os_release()?,
        "vim_dll_sha256": vim_dll.as_deref().map(digest).transpose()?,
        "locale": "C", "steps": [], "passed": false,
    });
    fs::create_dir_all(
        options
            .output
            .parent()
            .ok_or("report has no parent directory")?,
    )?;
    let result = check(
        &options,
        &backend,
        &vim,
        vim_dll.as_deref(),
        &script,
        &mut report,
    );
    report["file_retries"] = json!(
        *FILE_RETRIES
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    );
    fs::write(
        &options.output,
        format!("{}\n", serde_json::to_string_pretty(&report)?),
    )?;
    result?;
    let gaps = report["qualification_gaps"].as_array().map_or(0, Vec::len);
    println!(
        "PASS bounded checks; {gaps} qualification gap(s): {}",
        options.output.display()
    );
    Ok(true)
}

struct Steps<'a> {
    root: &'a Path,
    env: Vec<(String, String)>,
    report: &'a mut Value,
}

impl Steps<'_> {
    /// Runs one bounded step in the project directory and records its outcome.
    fn run(&mut self, label: &str, program: &Path, args: &[&str]) -> Result<i32> {
        let mut command = Command::new(program);
        command
            .args(args)
            .current_dir(self.root)
            .envs(self.env.iter().map(|(k, v)| (k, v)));
        let outcome =
            EditorProcess::start_captured(command)?.wait_captured(Duration::from_secs(30))?;
        let text = |bytes: &[u8]| String::from_utf8_lossy(bytes).into_owned();
        let Some(code) = outcome.code else {
            self.report["steps"]
                .as_array_mut()
                .expect("steps")
                .push(json!({
                    "label": label, "timeout": true,
                    "stdout": text(&outcome.stdout), "stderr": text(&outcome.stderr),
                }));
            return Err(format!("{label} exceeded its 30-second deadline").into());
        };
        let argv: Vec<String> = std::iter::once(program.to_string_lossy().into_owned())
            .chain(args.iter().map(|arg| (*arg).to_owned()))
            .collect();
        self.report["steps"]
            .as_array_mut()
            .expect("steps")
            .push(json!({
                "label": label, "argv": argv, "exit": code,
                "stdout": text(&outcome.stdout), "stderr": text(&outcome.stderr),
            }));
        Ok(code)
    }

    fn set(&mut self, key: &str, value: impl Into<String>) {
        let value = value.into();
        match self.env.iter_mut().find(|(name, _)| name == key) {
            Some((_, slot)) => *slot = value,
            None => self.env.push((key.to_owned(), value)),
        }
    }
}

fn check(
    options: &Options,
    backend: &Path,
    vim: &Path,
    vim_dll: Option<&Path>,
    script: &Path,
    report: &mut Value,
) -> Result<()> {
    let directory = tempfile::Builder::new()
        .prefix("ripscope-vim-")
        .tempdir_in(if cfg!(windows) {
            env::temp_dir()
        } else {
            PathBuf::from("/var/tmp")
        })?;
    let root = directory.path().to_owned();
    let home = root.to_string_lossy().into_owned();
    let mut steps = Steps {
        root: &root,
        env: vec![
            ("HOME".into(), home.clone()),
            ("XDG_CONFIG_HOME".into(), home.clone()),
            ("USERPROFILE".into(), home),
            ("VIMINIT".into(), String::new()),
            ("EXINIT".into(), String::new()),
            ("LC_ALL".into(), "C".into()),
            ("RIPSCOPE_BACKEND".into(), slashes(backend)),
        ],
        report,
    };
    ensure(
        steps.run("Vim identity", vim, &["--version"])? == 0,
        "Vim --version failed",
    )?;
    let version = steps.report["steps"][0]["stdout"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    ensure(
        version.contains("+cscope") && version.contains("+quickfix"),
        "Vim needs +cscope and +quickfix",
    )?;
    let sources = [
        (
            "alpha.c",
            "int Alpha;\n#include <shared.h>\nint call_alpha(void) { Callee(); }\nint Assigned = 1;\n",
        ),
        (
            "beta.c",
            "int Beta;\nint read_alpha(void) { return Alpha; }\n#include \"shared.h\"\n#include <unique.h>\nint call_beta(int callback(int)) { Callee(); }\nint assign(void) { Assigned += 2; }\n",
        ),
    ];
    steps.report["sources"] = json!(
        sources
            .iter()
            .map(|(n, c)| ((*n).to_owned(), json!(c)))
            .collect::<serde_json::Map<_, _>>()
    );
    for (name, contents) in sources {
        fs::write(root.join(name), contents)?;
    }
    let mut build = vec!["-bk", "-f", "index"];
    build.extend(sources.iter().map(|(name, _)| *name));
    ensure(
        steps.run("build fixture", backend, &build)? == 0,
        "the fixture index build failed",
    )?;
    fs::create_dir(root.join("elsewhere"))?;
    write_text(&root.join("elsewhere/alpha.c"), "int Conflict;\n")?;
    // An index is one SQLite file; its log is folded in when the build ends.
    fs::create_dir(root.join("copy"))?;
    fs::copy(root.join("index"), root.join("copy/index"))?;
    for (name, contents) in sources {
        fs::write(root.join("copy").join(name), contents)?;
    }
    write_text(&root.join("foreign"), "not a ripscope index\n")?;
    // A damaged index: the header survives, the pages after it do not.
    let mut damaged = fs::read(root.join("index"))?;
    ensure(
        damaged.len() > 2 * 4096,
        format!("index is only {} bytes", damaged.len()),
    )?;
    damaged[4096..].fill(0xff);
    fs::write(root.join("damaged"), damaged)?;
    let script_arg = script.to_string_lossy().into_owned();
    let editor = |file: &str| -> Vec<String> {
        [
            "-Nu", "NONE", "-U", "NONE", "-i", "NONE", "-n", "-N", "-es", "-S", file,
        ]
        .map(str::to_owned)
        .to_vec()
    };
    let arguments = editor(&script_arg);
    let arguments: Vec<&str> = arguments.iter().map(String::as_str).collect();
    let result = steps.run("Vim integration", vim, &arguments)?;
    let results: Value = serde_json::from_slice(&fs::read(root.join("vim.json"))?)?;
    steps.report["vim_results"] = results.clone();
    ensure(
        result == 0,
        format!("Vim integration failed: {}", results["errors"]),
    )?;
    ensure(
        results["errors"].as_array().is_some_and(Vec::is_empty),
        format!("Vim integration errors: {}", results["errors"]),
    )?;
    let pids = connection_pids(&results["observations"]);
    let distinct: std::collections::BTreeSet<_> = pids.iter().collect();
    ensure(
        pids.len() == 8 && distinct.len() == 8,
        format!("expected eight backends: {pids:?}"),
    )?;
    for pid in &pids {
        ensure(
            !process_alive(*pid)?,
            format!("backend {pid} is still running"),
        )?;
    }
    steps.report["terminated_backend_pids"] = json!(pids);
    if options.workflow {
        let config = options.repo.join("plugin/ripscope.vim");
        let workflow = real_path(&options.repo.join("tests/editor/workflow.vim"))?;
        let (config_sha, workflow_sha) = (digest(&config)?, digest(&workflow)?);
        steps.report["workflow_config_sha256"] = json!(config_sha);
        steps.report["workflow_script_sha256"] = json!(workflow_sha);
        steps.set("RIPSCOPE_CONFIG", config.to_string_lossy());
        if cfg!(windows) {
            let path = editor_process::system_directory()?
                .to_string_lossy()
                .into_owned();
            steps.set("PATH", path.clone());
            steps.report["workflow_path"] = json!(path);
        }
        steps.report["workflows"] = json!({});
        let workflow_arguments = editor(&workflow.to_string_lossy());
        let workflow_arguments: Vec<&str> = workflow_arguments.iter().map(String::as_str).collect();
        for selection in ["sources", "list"] {
            for refresh in ["automatic", "manual"] {
                steps.set("RIPSCOPE_SELECTION", selection);
                steps.set("RIPSCOPE_AUTO_REFRESH", refresh);
                write_text(&root.join("workflow.c"), "int Workflow;\n")?;
                write_text(&root.join("selection.txt"), "workflow.c\n")?;
                if cfg!(windows) {
                    let target = root.join("backend.exe");
                    settled(&target, || fs::copy(backend, &target))?;
                } else {
                    let wrapper = root.join("backend");
                    fs::write(
                        &wrapper,
                        format!(
                            "#!/bin/sh\nexec {} \"$@\"\n",
                            shell_quote(&backend.to_string_lossy())
                        ),
                    )?;
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o700))?;
                    }
                }
                let label = format!("Vim workflow {selection} {refresh}");
                let result = steps.run(&label, vim, &workflow_arguments)?;
                let observed: Value =
                    serde_json::from_slice(&fs::read(root.join("workflow.json"))?)?;
                steps.report["workflows"][format!("{selection}-{refresh}")] = observed.clone();
                ensure(
                    result == 0 && observed["errors"].as_array().is_some_and(Vec::is_empty),
                    format!("{label} failed: {}", observed["errors"]),
                )?;
                let workflow_pids: std::collections::BTreeSet<u32> =
                    connection_pids(&observed["observations"])
                        .into_iter()
                        .collect();
                ensure(
                    workflow_pids.len() == 3,
                    format!("{label}: expected three backends: {workflow_pids:?}"),
                )?;
                for pid in &workflow_pids {
                    ensure(
                        !process_alive(*pid)?,
                        format!("{label}: backend {pid} is still running"),
                    )?;
                }
                for item in observed["observations"].as_array().into_iter().flatten() {
                    if let Some(pid) = item.get("interrupted_pid").and_then(Value::as_u64) {
                        ensure(
                            !process_alive(u32::try_from(pid)?)?,
                            format!("{label}: {item}"),
                        )?;
                        let output = item["output_file"]
                            .as_str()
                            .ok_or_else(|| format!("{label}: no output file in {item}"))?;
                        ensure(!Path::new(output).exists(), format!("{label}: {item}"))?;
                    }
                }
                for name in ["workflow.idx", "workflow.idx-wal", "workflow.idx-shm"] {
                    let path = root.join(name);
                    settled(&path, || match fs::remove_file(&path) {
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                        other => other,
                    })?;
                }
            }
        }
        ensure(
            digest(&config)? == config_sha,
            "the plugin changed during the check",
        )?;
        ensure(
            digest(&workflow)? == workflow_sha,
            "the workflow script changed during the check",
        )?;
    }
    let report = steps.report;
    report["qualification_gaps"] = json!([]);
    report["unsupported_read_only_field_control"] =
        json!("all nine read-only searches are implemented");
    report["unsupported_commands_qualification"] =
        json!("raw persistent regression only; full editor behavior remains unqualified");
    report["full_editor_qualification"] = json!(false);
    ensure(
        digest(backend)? == report["backend_sha256"],
        "backend changed during check",
    )?;
    ensure(
        digest(vim)? == report["vim_sha256"],
        "Vim changed during check",
    )?;
    if let Some(dll) = vim_dll {
        ensure(
            digest(dll)? == report["vim_dll_sha256"],
            "Vim DLL changed during check",
        )?;
    }
    report["passed"] = json!(true);
    directory.close()?;
    Ok(())
}

/// A word quoted for a POSIX shell, unchanged when it needs no quoting.
fn shell_quote(word: &str) -> String {
    let safe = |ch: char| ch.is_ascii_alphanumeric() || "_@%+=:,./-".contains(ch);
    if !word.is_empty() && word.chars().all(safe) {
        word.to_owned()
    } else {
        format!("'{}'", word.replace('\'', "'\"'\"'"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(mode: &str) -> Command {
        let mut command = Command::new(env::current_exe().unwrap());
        command
            .args(["--exact", "tests::subprocess_fixture", "--nocapture"])
            .env("RIPSCOPE_VIM_DRIVER_FIXTURE", mode);
        command
    }

    #[test]
    fn subprocess_fixture() {
        let Ok(mode) = env::var("RIPSCOPE_VIM_DRIVER_FIXTURE") else {
            return;
        };
        match mode.as_str() {
            "output" => {
                println!("step output");
                eprintln!("step diagnostics");
                std::process::exit(3);
            }
            "hang" => std::thread::sleep(Duration::from_secs(60)),
            _ => panic!("unknown fixture mode"),
        }
    }

    #[test]
    fn captured_steps_keep_output_status_and_deadline() {
        let outcome = EditorProcess::start_captured(fixture("output"))
            .unwrap()
            .wait_captured(Duration::from_secs(20))
            .unwrap();
        assert_eq!(outcome.code, Some(3));
        assert!(String::from_utf8_lossy(&outcome.stdout).contains("step output"));
        assert!(String::from_utf8_lossy(&outcome.stderr).contains("step diagnostics"));
        let stopped = EditorProcess::start_captured(fixture("hang"))
            .unwrap()
            .wait_captured(Duration::from_millis(200))
            .unwrap();
        assert_eq!(stopped.code, None);
    }

    #[test]
    fn connection_listings_name_backend_processes() {
        let observations = json!([
            {"connection": " # pid    database name                       prepend path\n 0 4242  index  <none>\n 1 4343  copy/index  /tmp\n"},
            {"other": "no connection"},
        ]);
        assert_eq!(connection_pids(&observations), [4242, 4343]);
        assert!(process_alive(std::process::id()).unwrap());
        assert_eq!(shell_quote("/a b/'c"), "'/a b/'\"'\"'c'");
    }
}
