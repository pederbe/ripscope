//! Run the owned Neovim adapter in headless Neovim with isolated configuration and
//! retained evidence. Ordinary tests only check this driver.
use serde_json::{Value, json};
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

const USAGE: &str = "check-neovim --backend EXE --output REPORT.json [--nvim EXE] [--repo DIR]";

fn main() -> ExitCode {
    match run() {
        Ok(code) => ExitCode::from(u8::try_from(code).unwrap_or(1)),
        Err(error) => {
            eprintln!("Neovim check failed: {error}");
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
    let repo = std::path::absolute(repo)?;
    let backend = real_path(&backend.ok_or("--backend is required")?)?;
    let output = std::path::absolute(output.ok_or("--output is required")?)?;
    let nvim = executable(&nvim)
        .ok()
        .filter(|_| cfg!(any(target_os = "linux", windows)));
    let Some(nvim) = nvim else {
        return Err("run on Linux or native Windows with Neovim 0.10+".into());
    };
    #[cfg(unix)]
    if rustix::process::geteuid().is_root() {
        return Err("run Linux permission checks as an unprivileged user".into());
    }
    fs::create_dir_all(output.parent().ok_or("report has no parent directory")?)?;
    let workflow = repo.join("tests/editor/neovim-workflow.lua");
    let adapter = repo.join("lua/ripscope.lua");
    let mut inputs = vec![
        backend.clone(),
        nvim.clone(),
        adapter.clone(),
        repo.join("lua/ripscope/session.lua"),
        repo.join("lua/ripscope/project.lua"),
        workflow.clone(),
        repo.join("tests/editor/check_neovim.rs"),
        env::current_exe()?,
    ];
    if cfg!(windows) {
        let mut libraries: Vec<PathBuf> =
            fs::read_dir(nvim.parent().ok_or("Neovim has no directory")?)?
                .map(|entry| entry.map(|entry| entry.path()))
                .collect::<std::io::Result<_>>()?;
        libraries.retain(|path| {
            path.extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("dll"))
        });
        libraries.sort();
        inputs.extend(libraries);
    }
    let recorded = inputs
        .iter()
        .map(|path| Ok((path.to_string_lossy().into_owned(), sha(path)?)))
        .collect::<Result<BTreeMap<_, _>>>()?;
    let version = Command::new(&nvim).arg("--version").output()?;
    if !version.status.success() {
        return Err(format!("nvim --version failed: {}", version.status).into());
    }
    let mut report = json!({
        "inputs": recorded,
        "editor_version": String::from_utf8_lossy(&version.stdout),
        "platform": format!("{}-{}", env::consts::OS, env::consts::ARCH),
        "os_release": os_release()?,
        "uid": uid(),
    });
    let home = tempfile::Builder::new()
        .prefix("ripscope-neovim-home-")
        .tempdir_in(if cfg!(windows) {
            env::temp_dir()
        } else {
            PathBuf::from("/var/tmp")
        })?;
    let home_text = home.path().to_string_lossy().into_owned();
    let code = with_platform_paths(home.path(), &mut report, |extra, report| {
        let staged = home
            .path()
            .join("backend & percent% ü")
            .join(backend.file_name().ok_or("backend has no file name")?);
        fs::create_dir(staged.parent().ok_or("staged backend has no directory")?)?;
        fs::copy(&backend, &staged)?;
        let staged_sha = sha(&staged)?;
        report["staged_backend_sha256"] = json!(staged_sha);
        if staged_sha != sha(&backend)? {
            return Err("staged backend differs from the recorded input".into());
        }
        let mut command = Command::new(&nvim);
        command
            .args(["--headless", "-u", "NONE", "-i", "NONE", "-l"])
            .arg(&workflow)
            .env("HOME", &home_text)
            .env("XDG_CONFIG_HOME", format!("{home_text}/config"))
            .env("XDG_DATA_HOME", format!("{home_text}/data"))
            .env("XDG_STATE_HOME", format!("{home_text}/state"))
            .env("XDG_CACHE_HOME", format!("{home_text}/cache"))
            .env("LC_ALL", "C.UTF-8")
            .env("RIPSCOPE_NVIM_ADAPTER", &adapter)
            .env("RIPSCOPE_NVIM_BACKEND", &staged)
            .env("RIPSCOPE_NVIM_TEST_ROOT", &home_text)
            .envs(extra.iter().map(|(k, v)| (k, v)))
            .env_remove("VIM")
            .env_remove("VIMRUNTIME");
        if cfg!(windows) {
            command
                .env("USERPROFILE", &home_text)
                .env("APPDATA", format!("{home_text}/appdata"))
                .env("LOCALAPPDATA", format!("{home_text}/localappdata"));
        }
        report["command"] = json!(
            std::iter::once(command.get_program())
                .chain(command.get_args())
                .map(|arg| arg.to_string_lossy())
                .collect::<Vec<_>>()
        );
        let log = output.with_extension("log");
        let mut code = EditorProcess::start(command, &log)?.wait(Duration::from_secs(120))?;
        report["log"] = json!(log);
        let text = String::from_utf8_lossy(&fs::read(&log)?).into_owned();
        let async_errors = text.contains("Error executing ");
        report["async_errors"] = json!(async_errors);
        if async_errors && code == 0 {
            code = 1;
        }
        Ok(code)
    })?;
    report["exit_code"] = json!(code);
    home.close()?;
    let unchanged = inputs.iter().all(|path| {
        sha(path).is_ok_and(|now| {
            Some(now.as_str()) == report["inputs"][path.to_string_lossy().as_ref()].as_str()
        })
    });
    report["inputs_unchanged"] = json!(unchanged);
    fs::write(
        &output,
        format!("{}\n", serde_json::to_string_pretty(&report)?),
    )?;
    if !unchanged {
        return Err("validation inputs changed".into());
    }
    println!("Neovim check exit {code}; report: {}", output.display());
    Ok(code)
}

fn uid() -> Value {
    #[cfg(unix)]
    return json!(rustix::process::geteuid().as_raw());
    #[cfg(not(unix))]
    Value::Null
}

/// Runs `body` with the platform's extra environment: on Windows a deny-write directory
/// and an unused temporary drive mapping it owns and restores, on Linux another working
/// directory.
fn with_platform_paths(
    home: &Path,
    report: &mut Value,
    body: impl FnOnce(&[(String, String)], &mut Value) -> Result<i32>,
) -> Result<i32> {
    if !cfg!(windows) {
        return body(
            &[("RIPSCOPE_NVIM_OTHER_CWD".to_owned(), "/tmp".to_owned())],
            report,
        );
    }
    let system = editor_process::system_directory()?;
    let denied = home.join("denied-output");
    fs::create_dir(&denied)?;
    let who =
        checked(Command::new(system.join("whoami.exe")).args(["/user", "/fo", "csv", "/nh"]))?;
    let sid = csv_fields(String::from_utf8_lossy(&who).trim())
        .get(1)
        .cloned()
        .ok_or("whoami printed no SID")?;
    let icacls = system.join("icacls.exe");
    let mut acl = Command::new(&icacls);
    acl.arg(&denied)
        .args(["/deny", &format!("*{sid}:(OI)(CI)(W)")]);
    checked(&mut acl)?;
    let mut drive = None;
    let result = (|| -> Result<i32> {
        match fs::write(denied.join("control"), b"must be refused") {
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
                report["deny_write_control"] = json!("PermissionError");
            }
            Err(error) => return Err(error.into()),
            Ok(()) => return Err("deny-write control unexpectedly succeeded".into()),
        }
        let letter = ('D'..='Z')
            .rev()
            .find(|letter| fs::metadata(format!("{letter}:\\")).is_err())
            .ok_or("no unused drive letter for the cross-drive cwd check")?;
        let name = format!("{letter}:");
        checked(Command::new(system.join("subst.exe")).arg(&name).arg(home))?;
        drive = Some(name.clone());
        report["temporary_drive"] = json!(name);
        let extra = [
            ("PATH".to_owned(), system.to_string_lossy().into_owned()),
            (
                "RIPSCOPE_NVIM_DENIED".to_owned(),
                denied.to_string_lossy().into_owned(),
            ),
            (
                "RIPSCOPE_NVIM_ICACLS".to_owned(),
                icacls.to_string_lossy().into_owned(),
            ),
            ("RIPSCOPE_NVIM_SID".to_owned(), sid.clone()),
            ("RIPSCOPE_NVIM_OTHER_CWD".to_owned(), format!("{name}/")),
        ];
        body(&extra, report)
    })();
    let unmapped = match &drive {
        Some(name) => checked(Command::new(system.join("subst.exe")).args([name.as_str(), "/D"]))
            .map(|_| {
                report["temporary_drive_removed"] = json!(true);
            }),
        None => Ok(()),
    };
    checked(
        Command::new(&icacls)
            .arg(&denied)
            .args(["/remove:d", &format!("*{sid}")]),
    )?;
    report["test_acl_restored"] = json!(true);
    unmapped?;
    result
}

fn checked(command: &mut Command) -> Result<Vec<u8>> {
    editor_process::hide_window(command);
    let output = command.output()?;
    if !output.status.success() {
        return Err(format!(
            "{command:?} failed: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    Ok(output.stdout)
}

/// The fields of one CSV line.
fn csv_fields(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '"' if quoted && chars.peek() == Some(&'"') => {
                field.push('"');
                chars.next();
            }
            '"' => quoted = !quoted,
            ',' if !quoted => fields.push(std::mem::take(&mut field)),
            ch => field.push(ch),
        }
    }
    fields.push(field);
    fields
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whoami_lines_give_the_account_and_sid() {
        assert_eq!(
            csv_fields("\"windows-test\\tester\",\"S-1-5-21-1-2-3-1001\""),
            ["windows-test\\tester", "S-1-5-21-1-2-3-1001"]
        );
        assert_eq!(csv_fields("\"a \"\"b\"\"\",c"), ["a \"b\"", "c"]);
    }

    #[cfg(unix)]
    #[test]
    fn linux_runs_take_another_working_directory() {
        let mut report = json!({});
        let home = tempfile::tempdir().unwrap();
        let code = with_platform_paths(home.path(), &mut report, |extra, _| {
            assert_eq!(
                extra,
                [("RIPSCOPE_NVIM_OTHER_CWD".to_owned(), "/tmp".to_owned())]
            );
            Ok(5)
        })
        .unwrap();
        assert_eq!(code, 5);
    }
}
