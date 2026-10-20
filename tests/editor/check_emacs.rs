//! Explicitly run the real Emacs workflows; ordinary tests only check this driver.
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::env;
use std::error::Error;
use std::ffi::{OsStr, OsString};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::Duration;

#[path = "../../examples/support/editor_process.rs"]
#[allow(dead_code)]
mod editor_process;
use editor_process::{EditorProcess, executable, first_line, os_release};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

fn main() -> ExitCode {
    match run() {
        Ok(code) => ExitCode::from(u8::try_from(code).unwrap_or(1)),
        Err(error) => {
            eprintln!("Emacs check failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<i32> {
    let mut backend = None;
    let mut emacs = OsString::from("emacs");
    let mut output = None;
    let mut repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut args = env::args_os().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--help" || arg == "-h" {
            println!("check-emacs --backend EXE [--emacs EXE] [--repo DIR] --output REPORT.json");
            return Ok(0);
        }
        let value = args.next().ok_or("missing option value")?;
        match arg.to_str() {
            Some("--backend") => backend = Some(PathBuf::from(value)),
            Some("--emacs") => emacs = value,
            Some("--output") => output = Some(PathBuf::from(value)),
            Some("--repo") => repo = PathBuf::from(value),
            _ => return Err(format!("unknown option: {}", arg.to_string_lossy()).into()),
        }
    }
    let repo = std::path::absolute(repo)?;
    if !repo.is_dir() {
        return Err(format!("repository unavailable: {}", repo.display()).into());
    }
    let backend = std::path::absolute(backend.ok_or("--backend is required")?)?;
    if !backend.is_file() {
        return Err(format!("backend not found: {}", backend.display()).into());
    }
    let emacs = executable(&emacs)?;
    let output = std::path::absolute(output.ok_or("--output is required")?)?;
    if !cfg!(any(target_os = "linux", windows)) {
        return Err("run on Linux or native Windows".into());
    }
    #[cfg(unix)]
    if rustix::process::getuid().is_root() {
        return Err("run as an unprivileged user for permission checks".into());
    }
    fs::create_dir_all(output.parent().ok_or("report has no parent directory")?)?;
    let adapter = repo.join("contrib/emacs/ripscope-xcscope.el");
    let workflow = repo.join("tests/editor/emacs-workflow.el");
    let mut input_paths = vec![
        backend.clone(),
        emacs.clone(),
        adapter.clone(),
        workflow.clone(),
        repo.join("contrib/emacs/vendor/xcscope.el"),
        repo.join("tests/editor/check_emacs.rs"),
        repo.join("tests/editor/emacs-windows-workflow.el"),
        repo.join("Cargo.toml"),
        repo.join("Cargo.lock"),
        env::current_exe()?,
    ];
    let mut helpers = BTreeMap::new();
    if cfg!(target_os = "linux") {
        for name in ["bash", "find", "sed"] {
            let path = executable(OsStr::new(name))?;
            helpers.insert(
                name,
                json!({
                    "path": path, "sha256": sha(&path)?,
                    "version": first_line(Command::new(&path).arg("--version"))?,
                }),
            );
            input_paths.push(path);
        }
    }
    let inputs = input_paths
        .iter()
        .map(|path| Ok((path.clone(), sha(path)?)))
        .collect::<Result<BTreeMap<_, _>>>()?;
    let mut report = json!({
        "inputs": inputs,
        "emacs_version": first_line(Command::new(&emacs).arg("--version"))?,
        "platform": format!("{}-{}", env::consts::OS, env::consts::ARCH),
        "os_release": os_release()?,
        "helpers": helpers,
    });
    let home = tempfile::Builder::new()
        .prefix("ripscope-emacs-home-")
        .tempdir()?;
    let staged = home
        .path()
        .join("backend & percent% é")
        .join(backend.file_name().ok_or("backend has no file name")?);
    fs::create_dir_all(staged.parent().ok_or("backend has no directory")?)?;
    fs::copy(&backend, &staged)?;
    let staged_hash = sha(&staged)?;
    if staged_hash != inputs[&backend] {
        return Err("staged backend differs from the recorded input".into());
    }
    report["staged_backend_sha256"] = json!(staged_hash);
    let mut command = Command::new(&emacs);
    command
        .args([
            OsStr::new("-Q"),
            OsStr::new("--batch"),
            OsStr::new("-l"),
            workflow.as_os_str(),
        ])
        .env("HOME", home.path())
        .env("LC_ALL", "C.UTF-8")
        .env("RIPSCOPE_EMACS_ADAPTER", &adapter)
        .env("RIPSCOPE_EMACS_BACKEND", &staged);
    if cfg!(windows) {
        command
            .env("USERPROFILE", home.path())
            .env("APPDATA", home.path().join("appdata"))
            .env("LOCALAPPDATA", home.path().join("localappdata"))
            .env("PATH", emacs.parent().ok_or("Emacs has no directory")?);
    }
    report["command"] = json!(
        std::iter::once(command.get_program())
            .chain(command.get_args())
            .map(|arg| arg.to_string_lossy())
            .collect::<Vec<_>>()
    );
    let log = output.with_extension("log");
    let code = EditorProcess::start(command, &log)?.wait(Duration::from_secs(90))?;
    report["exit_code"] = json!(code);
    report["log"] = json!(log);
    home.close()?;
    let unchanged = inputs
        .iter()
        .all(|(path, hash)| sha(path).is_ok_and(|now| now == *hash));
    report["inputs_unchanged"] = json!(unchanged);
    fs::write(
        &output,
        format!("{}\n", serde_json::to_string_pretty(&report)?),
    )?;
    if !unchanged {
        return Err("validation inputs changed during the run".into());
    }
    println!("Emacs check exit {code}; report: {}", output.display());
    Ok(code)
}

fn sha(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

#[cfg(test)]
mod tests {
    use super::editor_process::hide_window;
    use super::*;
    use std::thread;
    use std::time::Instant;

    fn fixture(mode: &str, marker: &Path) -> Command {
        let mut command = Command::new(env::current_exe().unwrap());
        command
            .args(["--exact", "tests::subprocess_fixture", "--nocapture"])
            .env("RIPSCOPE_EMACS_DRIVER_FIXTURE", mode)
            .env("RIPSCOPE_EMACS_DRIVER_MARKER", marker);
        command
    }

    #[test]
    fn subprocess_fixture() {
        let Ok(mode) = env::var("RIPSCOPE_EMACS_DRIVER_FIXTURE") else {
            return;
        };
        let marker = PathBuf::from(env::var_os("RIPSCOPE_EMACS_DRIVER_MARKER").unwrap());
        match mode.as_str() {
            "success" => println!("editor output"),
            "failure" => std::process::exit(7),
            "tree" => {
                let mut command = fixture("leaf", &marker);
                hide_window(&mut command);
                let mut leaf = command.spawn().unwrap();
                leaf.wait().unwrap();
            }
            "leaf" => loop {
                use std::io::Write;
                let mut file = fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&marker)
                    .unwrap();
                file.write_all(b"alive\n").unwrap();
                thread::sleep(Duration::from_millis(20));
            },
            _ => panic!("unknown fixture mode"),
        }
    }

    #[test]
    fn preserves_editor_exit_status_and_output() {
        let temp = tempfile::tempdir().unwrap();
        let log = temp.path().join("editor.log");
        for (mode, expected) in [("success", 0), ("failure", 7)] {
            let process = EditorProcess::start(fixture(mode, &log), &log).unwrap();
            assert_eq!(process.wait(Duration::from_secs(10)).unwrap(), expected);
            if expected == 0 {
                assert!(fs::read_to_string(&log).unwrap().contains("editor output"));
            }
        }
    }

    #[test]
    fn cli_accepts_a_runtime_repository_location() {
        if Path::new(env!("CARGO")).is_file() {
            assert!(
                Command::new("cargo")
                    .args(["build", "--locked", "--example", "check-emacs"])
                    .status()
                    .unwrap()
                    .success()
            );
        }
        let temporary = tempfile::tempdir().unwrap();
        let current = env::current_exe().unwrap();
        let binary = current.with_file_name(format!("check-emacs{}", env::consts::EXE_SUFFIX));
        let output = Command::new(binary)
            .arg("--repo")
            .arg(temporary.path().join("missing-repository"))
            .arg("--backend")
            .arg(&current)
            .arg("--emacs")
            .arg(&current)
            .arg("--output")
            .arg(temporary.path().join("report.json"))
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("repository unavailable"),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn timeout_stops_the_editor_and_its_descendants() {
        let temp = tempfile::tempdir().unwrap();
        let marker = temp.path().join("heartbeat");
        let process =
            EditorProcess::start(fixture("tree", &marker), &temp.path().join("editor.log"))
                .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !marker.exists() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(20));
        }
        assert!(marker.exists(), "descendant did not start");
        assert_eq!(process.wait(Duration::from_millis(50)).unwrap(), 124);
        thread::sleep(Duration::from_millis(100));
        let stopped = fs::metadata(&marker).unwrap().len();
        thread::sleep(Duration::from_millis(100));
        assert_eq!(
            fs::metadata(&marker).unwrap().len(),
            stopped,
            "descendant is still running"
        );
    }
}
