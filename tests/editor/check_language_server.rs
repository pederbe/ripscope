//! Run `ripscope lsp` through headless Neovim's built-in client, on a fresh project
//! and with isolated configuration, and retain the evidence.
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::Duration;

#[path = "../../examples/support/editor_process.rs"]
#[allow(dead_code)]
mod editor_process;
use editor_process::{EditorProcess, executable, os_release, real_path};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const USAGE: &str =
    "check-language-server --backend EXE --output REPORT.json [--nvim EXE] [--repo DIR]";

fn main() -> ExitCode {
    match run() {
        Ok(code) => ExitCode::from(u8::try_from(code).unwrap_or(1)),
        Err(error) => {
            eprintln!("Language server check failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn sha(path: &Path) -> Result<String> {
    Ok(base16ct::lower::encode_string(&Sha256::digest(fs::read(
        path,
    )?)))
}

fn run() -> Result<i32> {
    let mut backend = None;
    let mut nvim = OsString::from("nvim");
    let mut output = None;
    let mut repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut args = env::args_os().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--help" || arg == "-h" {
            println!("{USAGE}");
            return Ok(0);
        }
        let value = args.next().ok_or("missing option value")?;
        match arg.to_str() {
            Some("--backend") => backend = Some(PathBuf::from(value)),
            Some("--nvim") => nvim = value,
            Some("--output") => output = Some(PathBuf::from(value)),
            Some("--repo") => repo = PathBuf::from(value),
            _ => return Err(format!("unknown option: {}\n{USAGE}", arg.to_string_lossy()).into()),
        }
    }
    let backend = real_path(&backend.ok_or("--backend is required")?)?;
    let output = std::path::absolute(output.ok_or("--output is required")?)?;
    let nvim = executable(&nvim).map_err(|_| "Neovim 0.11 or newer is required")?;
    fs::create_dir_all(output.parent().ok_or("report has no parent directory")?)?;
    let script = std::path::absolute(repo)?.join("tests/editor/neovim-lsp.lua");
    let inputs = [
        backend.clone(),
        nvim.clone(),
        script.clone(),
        env::current_exe()?,
    ];
    let recorded = inputs
        .iter()
        .map(|path| Ok((path.to_string_lossy().into_owned(), sha(path)?)))
        .collect::<Result<BTreeMap<_, _>>>()?;
    let version = Command::new(&nvim).arg("--version").output()?;
    if !version.status.success() {
        return Err(format!("nvim --version failed: {}", version.status).into());
    }

    let home = tempfile::Builder::new()
        .prefix("ripscope-lsp-home-")
        .tempdir_in(if cfg!(windows) {
            env::temp_dir()
        } else {
            PathBuf::from("/var/tmp")
        })?;
    let project = home.path().join("project");
    fs::create_dir(&project)?;
    let mut command = Command::new(&nvim);
    command
        .args(["--headless", "-u", "NONE", "-i", "NONE", "-l"])
        .arg(&script)
        .current_dir(&project)
        .env("RIPSCOPE_LSP_BACKEND", &backend)
        .env("RIPSCOPE_LSP_PROJECT", &project)
        .env_remove("RIPSCOPE_CACHE_DIR")
        .env_remove("VIM")
        .env_remove("VIMRUNTIME");
    for (name, directory) in [
        ("HOME", ""),
        ("XDG_CONFIG_HOME", "config"),
        ("XDG_DATA_HOME", "data"),
        ("XDG_STATE_HOME", "state"),
        ("XDG_CACHE_HOME", "cache"),
        ("USERPROFILE", ""),
        ("APPDATA", "appdata"),
        ("LOCALAPPDATA", "localappdata"),
    ] {
        command.env(name, home.path().join(directory));
    }
    let shown = std::iter::once(command.get_program())
        .chain(command.get_args())
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    let captured =
        EditorProcess::start_captured(command)?.wait_captured(Duration::from_secs(120))?;
    let code = captured.code.unwrap_or(editor_process::TIMED_OUT);
    let stdout = String::from_utf8_lossy(&captured.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&captured.stderr).into_owned();
    println!("{}", stdout.trim_end());
    eprint!("{stderr}");
    home.close()?;

    let unchanged = inputs.iter().all(|path| {
        sha(path).is_ok_and(|now| recorded.get(path.to_string_lossy().as_ref()) == Some(&now))
    });
    let report = json!({
        "inputs": recorded,
        "inputs_unchanged": unchanged,
        "editor_version": String::from_utf8_lossy(&version.stdout),
        "platform": format!("{}-{}", env::consts::OS, env::consts::ARCH),
        "os_release": os_release()?,
        "command": shown,
        "stdout": stdout,
        "stderr": stderr,
        "exit_code": code,
    });
    fs::write(
        &output,
        format!("{}\n", serde_json::to_string_pretty(&report)?),
    )?;
    if !unchanged {
        return Err("validation inputs changed".into());
    }
    println!(
        "Language server check exit {}; report: {}",
        code,
        output.display()
    );
    Ok(code)
}
