//! Run frozen navigation workflows on disposable native source copies.

use serde_json::{Value, json};
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

#[allow(dead_code)]
#[path = "support/capture.rs"]
mod capture;
#[path = "support/workflow_session.rs"]
mod workflow_session;

fn main() {
    match run(std::env::args_os().skip(1)) {
        Ok(true) => (),
        Ok(false) => std::process::exit(1),
        Err(error) => {
            eprintln!("workflow assessment failed: {error}");
            std::process::exit(1);
        }
    }
}

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
fn bad(message: &str) -> io::Error {
    io::Error::other(message)
}
fn text(value: &Value) -> Result<&str> {
    value.as_str().ok_or_else(|| bad("expected string").into())
}
fn items(value: &Value) -> Result<&Vec<Value>> {
    value.as_array().ok_or_else(|| bad("expected array").into())
}
fn read(path: &Path) -> Result<Value> {
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}
fn relative(root: &Path, name: &str) -> Result<PathBuf> {
    let path = Path::new(name);
    if name.is_empty()
        || name.contains(['\n', '\r'])
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(bad("input path must remain relative to its declared root").into());
    }
    Ok(root.join(path))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Cli,
    Vim,
    Neovim,
    Emacs,
}

struct Editor {
    extension: &'static str,
    adapter: &'static str,
    dependencies: &'static [&'static str],
    args: &'static [&'static str],
    seconds: u64,
}

impl Mode {
    fn name(self) -> &'static str {
        match self {
            Self::Cli => "cli",
            Self::Vim => "vim",
            Self::Neovim => "neovim",
            Self::Emacs => "emacs",
        }
    }
    fn parse(value: &str) -> Result<Self> {
        match value {
            "cli" => Ok(Self::Cli),
            "vim" => Ok(Self::Vim),
            "neovim" => Ok(Self::Neovim),
            "emacs" => Ok(Self::Emacs),
            _ => Err(bad("unsupported workflow mode").into()),
        }
    }
    fn editor(self) -> Option<Editor> {
        Some(match self {
            Self::Cli => return None,
            Self::Vim => Editor {
                extension: "vim",
                adapter: "plugin/ripscope.vim",
                dependencies: &[],
                args: &[
                    "-Nu", "NONE", "-U", "NONE", "-i", "NONE", "-n", "-N", "-es", "-S",
                ],
                seconds: 30,
            },
            Self::Neovim => Editor {
                extension: "lua",
                adapter: "lua/ripscope.lua",
                dependencies: &["lua/ripscope/session.lua", "lua/ripscope/project.lua"],
                args: &["--headless", "-u", "NONE", "-i", "NONE", "-l"],
                seconds: 30,
            },
            Self::Emacs => Editor {
                extension: "el",
                adapter: "contrib/emacs/ripscope.el",
                dependencies: &[],
                args: &[
                    "-Q",
                    "--batch",
                    "--eval",
                    "(setq make-backup-files nil auto-save-default nil create-lockfiles nil)",
                    "-l",
                ],
                seconds: 90,
            },
        })
    }
}

struct Options {
    frame: PathBuf,
    project: String,
    backend: PathBuf,
    repo: PathBuf,
    work: PathBuf,
    output: PathBuf,
    mode: Mode,
    editor: Option<PathBuf>,
    expires: Instant,
}

fn run(arguments: impl Iterator<Item = std::ffi::OsString>) -> Result<bool> {
    let mut options = std::collections::BTreeMap::new();
    let mut arguments = arguments;
    while let Some(key) = arguments.next() {
        if key == "--help" || key == "-h" {
            println!(
                "assess-workflows --frame FRAME --project NAME --backend EXE --repo REPO --work-root DIR --output NEW_DIR --mode cli|vim|neovim|emacs [--editor EXE]"
            );
            return Ok(true);
        }
        let key = key.into_string().map_err(|_| bad("invalid option"))?;
        if ![
            "--frame",
            "--project",
            "--backend",
            "--repo",
            "--work-root",
            "--output",
            "--mode",
            "--editor",
        ]
        .contains(&key.as_str())
            || options.contains_key(&key)
        {
            return Err(bad("unknown or duplicate option").into());
        }
        options.insert(
            key,
            arguments
                .next()
                .ok_or_else(|| bad("missing option value"))?,
        );
    }
    let required = |key: &str| {
        options
            .get(key)
            .cloned()
            .ok_or_else(|| bad(&format!("{key} is required")))
    };
    let options = Options {
        frame: std::path::absolute(required("--frame")?)?,
        project: required("--project")?
            .into_string()
            .map_err(|_| bad("invalid project"))?,
        backend: std::path::absolute(required("--backend")?)?,
        repo: std::path::absolute(required("--repo")?)?,
        work: std::path::absolute(required("--work-root")?)?,
        output: std::path::absolute(required("--output")?)?,
        mode: Mode::parse(
            &required("--mode")?
                .into_string()
                .map_err(|_| bad("invalid mode"))?,
        )?,
        editor: options
            .get("--editor")
            .map(std::path::absolute)
            .transpose()?,
        expires: Instant::now() + Duration::from_secs(3600),
    };
    #[cfg(all(test, windows))]
    let options = {
        let mut options = options;
        if let Ok(seconds) = std::env::var("WORKFLOW_FIXTURE_PLATFORM_SECONDS")
            && let Ok(seconds) = seconds.parse::<u64>()
        {
            options.expires = Instant::now() + Duration::from_secs(seconds.min(3600));
        }
        options
    };
    if options.output.exists() {
        return Err(bad("output already exists").into());
    }
    fs::create_dir(&options.output)?;
    let mut report = json!({"complete":false,"platform":format!("{}-{}",std::env::consts::OS,std::env::consts::ARCH),"frame":options.frame,"project":options.project,"mode":options.mode.name(),"runs":[],"limits":{"platform_seconds":3600,"query_seconds":120,"index_seconds":1800,"capture_bytes":2_u64<<30,"runtime_bytes":2_u64<<30,"minimum_free_bytes":4_u64<<30,"memory_enforcement":if cfg!(target_os="linux"){"8GiB systemd scope when available; see each cleanup record"}else{"no per-process memory cap; record platform memory separately"}}});
    let outcome = execute(&options, &mut report);
    report["failure"] = json!(outcome.as_ref().err().map(|error| error.to_string()));
    if outcome.is_ok() {
        report["complete"] = json!(true);
    }
    capture::save(&options.output.join("report.json"), &report)?;
    if let Err(error) = outcome {
        eprintln!("{error}");
    }
    Ok(report["complete"] == true)
}

fn verify_frame(frame: &Path) -> Result<Value> {
    let seal = read(&frame.join("SEAL.json"))?;
    if seal["sealed"] != true || seal["workflow_inventory_version"] != 3 {
        return Err(bad("frame is not a sealed workflow inventory version3").into());
    }
    let list = relative(frame, text(&seal["input_hash_list"]["path"])?)?;
    if capture::digest(&list)? != text(&seal["input_hash_list"]["sha256"])? {
        return Err(bad("frozen input hash list changed").into());
    }
    let contents = fs::read_to_string(list)?;
    let original_root = contents
        .lines()
        .filter_map(|line| line.split_once("  "))
        .find_map(|(_, path)| path.strip_suffix("/HANDOFF.md"));
    for line in contents.lines() {
        let (hash, name) = line
            .split_once("  ")
            .ok_or_else(|| bad("invalid input hash list"))?;
        let name = if let Some(original) = original_root {
            name.strip_prefix(&format!("{original}/"))
                .ok_or_else(|| bad("sealed input outside original frame"))?
        } else {
            name
        };
        if capture::digest(&relative(frame, name)?)? != hash {
            return Err(bad(&format!("frozen workflow input changed: {name}")).into());
        }
    }
    let inventory = read(&frame.join("workflow-inventory.json"))?;
    if inventory["workflow_inventory_version"] != 3 {
        return Err(bad("unsupported workflow inventory").into());
    }
    Ok(inventory)
}

fn verify_sources(root: &Path, project: &Value, edit: Option<&Value>) -> Result<()> {
    let canonical = fs::canonicalize(root)?;
    for source in items(&project["sources"])? {
        let name = text(&source["path"])?;
        let path = relative(root, name)?;
        let target = fs::canonicalize(&path)?;
        if !target.starts_with(&canonical)
            || target
                .strip_prefix(&canonical)?
                .components()
                .collect::<Vec<_>>()
                != Path::new(name).components().collect::<Vec<_>>()
            || fs::symlink_metadata(&path)?.is_symlink()
            || !items(&source["links"])?.is_empty()
            || Path::new(text(&source["target"])?)
                .components()
                .collect::<Vec<_>>()
                != Path::new(name).components().collect::<Vec<_>>()
        {
            return Err(bad(&format!("frozen source target changed: {name}")).into());
        }
        let expected = if let Some(edit) = edit.filter(|edit| edit["file"] == name) {
            text(&edit["edited_sha256"])?
        } else {
            text(&source["sha256"])?
        };
        if capture::digest(&path)? != expected {
            return Err(bad(&format!("frozen source bytes changed: {name}")).into());
        }
    }
    Ok(())
}

fn inventory_paths(root: &Path, directory: &Path, paths: &mut Vec<String>) -> Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        let kind = entry.file_type()?;
        if kind.is_symlink() || (!kind.is_dir() && !kind.is_file()) {
            return Err(bad("unsupported source alias or file type").into());
        }
        if kind.is_dir() {
            inventory_paths(root, &path, paths)?;
        } else {
            paths.push(
                path.strip_prefix(root)?
                    .to_string_lossy()
                    .replace('\\', "/"),
            );
        }
    }
    Ok(())
}

fn verify_inventory(root: &Path, project: &Value, runtime: bool) -> Result<()> {
    let mut actual = Vec::new();
    inventory_paths(root, root, &mut actual)?;
    let expected = items(&project["sources"])?
        .iter()
        .map(|source| text(&source["path"]))
        .collect::<Result<std::collections::BTreeSet<_>>>()?;
    if actual.iter().any(|name| {
        !expected.contains(name.as_str())
            && !(runtime
                && [
                    "workflow.files",
                    "workflow.db",
                    "workflow.db-wal",
                    "workflow.db-shm",
                    "workflow-editor.db",
                    "workflow-editor.db-wal",
                    "workflow-editor.db-shm",
                ]
                .contains(&name.as_str()))
    }) {
        return Err(bad("new input outside frozen inventory").into());
    }
    Ok(())
}

fn budget(options: &Options, requested: u64) -> Result<u64> {
    let left = options
        .expires
        .saturating_duration_since(Instant::now())
        .as_secs();
    if left == 0 {
        return Err(bad("platform workflow deadline").into());
    }
    Ok(left.min(requested))
}

fn clean_command(program: &Path, root: &Path) -> Command {
    let mut command = Command::new(program);
    command
        .current_dir(root)
        .env_remove("INCDIR")
        .env_remove("INCLUDEDIRS")
        .env_remove("RIPSCOPE_BLOCK_SIZE")
        .env_remove("RIPSCOPE_WORKFLOW_VALIDATE_ONLY")
        .env("LC_ALL", "C")
        .env("GIT_TERMINAL_PROMPT", "0");
    #[cfg(all(test, windows))]
    if std::env::var_os("WORKFLOW_FIXTURE_DELAY_PROGRAM")
        .is_some_and(|selected| Path::new(&selected) == program)
        && let Some(delay) = std::env::var_os("WORKFLOW_FIXTURE_LAUNCH_DELAY_MS")
    {
        command.env("WORKFLOW_FIXTURE_LAUNCH_DELAY_MS", delay);
    }
    command
}

fn execute(options: &Options, report: &mut Value) -> Result<()> {
    let inventory = verify_frame(&options.frame)?;
    let project = items(&inventory["projects"])?
        .iter()
        .find(|project| project["name"] == options.project)
        .ok_or_else(|| bad("unknown frozen project"))?;
    let plan = read(&relative(&options.frame, text(&project["editor_plan"])?)?)?;
    let input = relative(&options.frame, &format!("inputs/{}", options.project))?;
    verify_sources(&input, project, None)?;
    verify_inventory(&input, project, false)?;
    if project
        .get("unsupported_inputs")
        .is_some_and(|value| value.as_array().is_none_or(|items| !items.is_empty()))
    {
        return Err(bad("frozen frame contains unsupported inputs").into());
    }
    if project.get("include_directories").is_some()
        && project["include_directories"] != plan["selection"]["include_directories"]
    {
        return Err(bad("frozen include order differs from project inventory").into());
    }
    let canonical = fs::canonicalize(&input)?;
    for directory in items(&plan["selection"]["include_directories"])? {
        let name = text(directory)?;
        let path = if name == "." {
            input.clone()
        } else {
            relative(&input, name)?
        };
        if !path.is_dir() || !fs::canonicalize(path)?.starts_with(&canonical) {
            return Err(bad("include directory escaped frozen source root").into());
        }
    }
    if !options.backend.is_file() || !options.repo.is_dir() || !options.work.is_dir() {
        return Err(bad("backend/repository/work root unavailable").into());
    }
    #[cfg(unix)]
    {
        let filesystem = rustix::fs::statvfs(&options.work)?;
        if filesystem.f_bavail.saturating_mul(filesystem.f_frsize) < 4 << 30 {
            return Err(bad("insufficient free local storage").into());
        }
        report["storage"] = json!({"work_root":fs::canonicalize(&options.work)?,"available_bytes":filesystem.f_bavail.saturating_mul(filesystem.f_frsize)});
    }
    #[cfg(target_os = "linux")]
    {
        let filesystem = rustix::fs::statfs(&options.work)?;
        // Linux filesystem magic values for tmpfs, NFS and CIFS/SMB.
        if [0x01021994, 0x6969, 0xff534d42, 0xfe534d42].contains(&(filesystem.f_type as u64)) {
            return Err(bad("work root requires disk-backed local storage").into());
        }
        report["storage"]["filesystem_magic"] = json!(filesystem.f_type);
        report["user"] = json!({"uid":rustix::process::getuid().as_raw(),"unprivileged":!rustix::process::getuid().is_root()});
        if rustix::process::getuid().is_root() {
            return Err(bad("run as an unprivileged user").into());
        }
    }
    #[cfg(windows)]
    {
        let powershell = PathBuf::from(
            std::env::var_os("SystemRoot").ok_or_else(|| bad("SystemRoot unavailable"))?,
        )
        .join("System32/WindowsPowerShell/v1.0/powershell.exe");
        let mut command = clean_command(&powershell, &options.work);
        command.env("RIPSCOPE_STORAGE_ROOT",&options.work).args(["-NoProfile","-NonInteractive","-Command","$ErrorActionPreference='Stop'; $p=$env:RIPSCOPE_STORAGE_ROOT; if ($p.StartsWith('\\\\')) { throw 'local drive required' }; $d=[IO.DriveInfo]::new([IO.Path]::GetPathRoot($p)); $u=[Security.Principal.WindowsIdentity]::GetCurrent(); $admin=([Security.Principal.WindowsPrincipal]::new($u)).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator); @{format=$d.DriveFormat; available_bytes=$d.AvailableFreeSpace; user=$u.Name; administrator=$admin; work_root=$p} | ConvertTo-Json -Compress"]);
        let captured = capture::run_command(
            command,
            &options.output.join("storage"),
            budget(options, 15)?,
            &[(options.output.clone(), 2 << 30)],
        )?;
        report["storage_capture"] = captured.clone();
        if captured["complete"] != true {
            return Err(bad("native storage/user identity capture incomplete").into());
        }
        let storage = read(&options.output.join("storage.stdout"))?;
        if storage["format"] != "NTFS"
            || storage["administrator"] != false
            || storage["available_bytes"].as_u64().unwrap_or(0) < 4 << 30
        {
            return Err(bad("ordinary-user local NTFS storage with4GiB free is required").into());
        }
        report["storage"] = storage;
    }
    report["identities"] = json!({"backend_sha256":capture::digest(&options.backend)?,"driver_sha256":capture::digest(&std::env::current_exe()?)?,"seal_sha256":capture::digest(&options.frame.join("SEAL.json"))?,"inventory_sha256":capture::digest(&options.frame.join("workflow-inventory.json"))?});
    report["identities"]["repo"] = json!(options.repo);
    report["identities"]["input_root"] = json!(input);
    report["identities"]["plan_sha256"] = json!(capture::digest(&relative(
        &options.frame,
        text(&project["editor_plan"])?
    )?)?);
    let selections = if options.mode == Mode::Cli {
        vec!["explicit-file-list", "recursive-discovery"]
    } else {
        vec!["editor"]
    };
    for selection in selections {
        budget(options, 1)?;
        let runtime = tempfile::Builder::new()
            .prefix("ripscope-workflow-")
            .tempdir_in(&options.work)?
            .keep();
        let output = if selection == "editor" {
            options.output.clone()
        } else {
            let output = options.output.join(selection);
            fs::create_dir(&output)?;
            output
        };
        let mut run =
            json!({"selection":selection,"runtime_root":runtime,"complete":false,"stages":[]});
        let outcome = (|| -> Result<()> {
            for source in items(&project["sources"])? {
                let name = text(&source["path"])?;
                let destination = relative(&runtime, name)?;
                fs::create_dir_all(
                    destination
                        .parent()
                        .ok_or_else(|| bad("source has no parent"))?,
                )?;
                fs::copy(relative(&input, name)?, destination)?;
            }
            file_list(&runtime, &plan, None)?;
            verify_sources(&runtime, project, None)?;
            if selection == "editor" {
                editor(options, &runtime, &output, &plan, &mut run)?;
            } else {
                cli(options, &runtime, &output, &plan, selection, &mut run)?;
            }
            verify_sources(&runtime, project, Some(&plan["edit"]))?;
            verify_inventory(&runtime, project, true)?;
            verify_sources(&input, project, None)?;
            verify_inventory(&input, project, false)?;
            verify_frame(&options.frame)?;
            if capture::digest(&options.backend)? != text(&report["identities"]["backend_sha256"])?
                || capture::digest(&std::env::current_exe()?)?
                    != text(&report["identities"]["driver_sha256"])?
            {
                return Err(bad("backend or driver identity changed during replay").into());
            }
            let mut indexes = Vec::new();
            closed_indexes(&runtime, &mut indexes)?;
            run["closed_indexes"] = json!(indexes);
            run["complete"] = json!(true);
            Ok(())
        })();
        let outcome = outcome.and_then(|()| {
            fs::remove_dir_all(&runtime)?;
            Ok(())
        });
        run["complete"] = json!(outcome.is_ok());
        run["failure"] = json!(outcome.as_ref().err().map(|error| error.to_string()));
        run["runtime_preserved"] = json!(runtime.exists());
        report["runs"].as_array_mut().unwrap().push(run);
        outcome?;
    }
    Ok(())
}

fn file_list(root: &Path, plan: &Value, added: Option<&str>) -> Result<()> {
    let mut names = items(&plan["selection"]["index_files"])?
        .iter()
        .map(text)
        .collect::<Result<Vec<_>>>()?;
    if let Some(added) = added {
        names.push(added);
    }
    let contents = names
        .into_iter()
        .map(|name| format!("\"{name}\"\n"))
        .collect::<String>();
    fs::write(root.join("workflow.files"), contents)?;
    Ok(())
}

fn closed_indexes(root: &Path, identities: &mut Vec<Value>) -> Result<()> {
    for entry in fs::read_dir(root)? {
        let path = entry?.path();
        if path.is_dir() {
            closed_indexes(&path, identities)?;
        } else if path
            .file_name()
            .is_some_and(|name| name.to_string_lossy().contains(".db"))
        {
            identities.push(json!({"path":path,"sha256":capture::digest(&path)?}));
        }
    }
    Ok(())
}

fn editor(
    options: &Options,
    root: &Path,
    output: &Path,
    plan: &Value,
    run: &mut Value,
) -> Result<()> {
    let executable = options
        .editor
        .as_ref()
        .ok_or_else(|| bad("--editor is required for editor mode"))?;
    let editor = options
        .mode
        .editor()
        .ok_or_else(|| bad("editor mode required"))?;
    let script = options
        .frame
        .join(format!("editor-scripts/project.{}", editor.extension));
    let adapter = options.repo.join(editor.adapter);
    if !executable.is_file() || !adapter.is_file() {
        return Err(bad("editor or adapter missing").into());
    }
    let report_path = output.join("editor.json");
    let mut command = clean_command(executable, root);
    let home = output.join("editor-home");
    fs::create_dir(&home)?;
    command
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env("APPDATA", &home)
        .env("LOCALAPPDATA", &home)
        .env("XDG_CONFIG_HOME", &home)
        .env("XDG_DATA_HOME", &home)
        .env("XDG_STATE_HOME", &home)
        .env("XDG_CACHE_HOME", &home);
    command.args(editor.args).arg(&script);
    command
        .env("RIPSCOPE_WORKFLOW_ROOT", root)
        .env(
            "RIPSCOPE_WORKFLOW_PLAN",
            options
                .frame
                .join(format!("plans/{}.json", options.project)),
        )
        .env("RIPSCOPE_WORKFLOW_BACKEND", &options.backend)
        .env("RIPSCOPE_WORKFLOW_ADAPTER", &adapter)
        .env("RIPSCOPE_WORKFLOW_REPORT", &report_path);
    run["editor_inputs"] = json!({"editor_sha256":capture::digest(executable)?,"script_sha256":capture::digest(&script)?,"adapter_sha256":capture::digest(&adapter)?,"validate_only":"removed"});
    let dependencies = editor.dependencies.iter().map(|name| Ok(json!({"path":options.repo.join(name),"sha256":capture::digest(&options.repo.join(name))?}))).collect::<Result<Vec<_>>>()?;
    run["editor_inputs"]["dependencies"] = json!(dependencies);
    let capture = capture::run_command(
        command,
        &output.join("editor-process"),
        budget(options, editor.seconds)?,
        &[(root.to_owned(), 2 << 30), (output.to_owned(), 2 << 30)],
    )?;
    run["process"] = capture.clone();
    let report = read(&report_path)?;
    validate_editor(&report, plan, root, options.mode)?;
    for (path, expected) in [
        (
            executable.as_path(),
            text(&run["editor_inputs"]["editor_sha256"])?,
        ),
        (&script, text(&run["editor_inputs"]["script_sha256"])?),
        (&adapter, text(&run["editor_inputs"]["adapter_sha256"])?),
    ] {
        if capture::digest(path)? != expected {
            return Err(bad("editor input changed during replay").into());
        }
    }
    for dependency in dependencies {
        if capture::digest(Path::new(text(&dependency["path"])?))? != text(&dependency["sha256"])? {
            return Err(bad("editor dependency changed during replay").into());
        }
    }
    if capture["complete"] != true {
        return Err(bad("editor process or cleanup incomplete").into());
    }
    Ok(())
}

fn validate_editor(report: &Value, plan: &Value, root: &Path, mode: Mode) -> Result<()> {
    let events = items(&report["observations"])?;
    if report["complete"] != true || events.is_empty() {
        return Err(bad("expected editor observations are missing").into());
    }
    let jump = |file: &Value, line: &Value| json!([file, line]);
    let edit = jump(&plan["edit"]["file"], &plan["edit"]["line"]);
    let mut expected = vec![edit.clone(), edit.clone()];
    for neighbor in items(&plan["source_expectations"]["neighbor_unchanged"])? {
        expected.push(jump(&neighbor[0], &neighbor[1]));
    }
    expected.push(edit);
    let jumps = events
        .iter()
        .filter(|event| event["kind"] == "jump")
        .map(|event| -> Result<Value> {
            let path = PathBuf::from(text(&event["detail"]["file"])?);
            let path = path
                .strip_prefix(root)
                .map_err(|_| bad("editor jump escaped runtime root"))?;
            Ok(json!([
                path.to_string_lossy().replace('\\', "/"),
                event["detail"]["line"]
            ]))
        })
        .collect::<Result<Vec<_>>>()?;
    if jumps.len() < expected.len() || jumps[..expected.len()] != expected {
        return Err(bad("expected editor jump observations are missing or wrong").into());
    }
    if capture::digest(&relative(root, text(&plan["edit"]["file"])?)?)?
        != text(&plan["edit"]["edited_sha256"])?
    {
        return Err(bad("edited source hash does not match frozen plan").into());
    }
    let diagnostics = events
        .iter()
        .map(|event| &event["detail"])
        .collect::<Vec<_>>();
    let new = text(&plan["edit"]["new"])?;
    let rows = |value: &Value| -> Result<Vec<Value>> {
        let rows = if mode == Mode::Neovim {
            text(&value["stdout"])?
                .lines()
                .map(serde_json::from_str::<Value>)
                .collect::<std::result::Result<Vec<_>, _>>()?
        } else {
            items(&value["observed_rows"])?.clone()
        };
        if rows.iter().any(|row| {
            !row.as_array().is_some_and(|row| {
                row.len() == 4
                    && row[0].is_string()
                    && row[1].is_string()
                    && row[2].as_u64().is_some_and(|line| line > 0)
                    && row[3].is_string()
            })
        }) {
            return Err(bad("malformed editor row payload").into());
        }
        rows.into_iter()
            .map(|mut row| {
                let file = row[0].as_str().unwrap().replace('\\', "/");
                let root_name = root.to_string_lossy().replace('\\', "/");
                let file = if let Some(file) = file.strip_prefix(&format!("{root_name}/")) {
                    file.to_owned()
                } else {
                    if Path::new(&file).is_absolute() {
                        return Err(bad("editor row escaped runtime root").into());
                    }
                    file
                };
                relative(root, &file)?;
                row[0] = json!(file);
                Ok(row)
            })
            .collect()
    };
    let message = |value: &Value| {
        value["messages"]
            .as_str()
            .or_else(|| value["stderr"].as_str())
            .or_else(|| value["text"].as_str())
            .unwrap_or("")
            .to_owned()
    };
    let query = |value: &Value, field, pattern: &str| {
        value["field"] == field && value["pattern"] == pattern
    };
    let warning = |value: &Value| {
        let diagnostic = message(value).replace('\\', "/");
        (diagnostic.contains("cannot read")
            && diagnostic.contains(text(&plan["edit"]["file"]).unwrap()))
            || (mode == Mode::Vim && diagnostic.contains("E609"))
    };
    let empty = diagnostics.iter().any(|value| {
        query(value, 1, "__ripscope_workflow_absent_20261005__")
            && rows(value).is_ok_and(|rows| rows.is_empty())
            && (message(value).contains("No matches were found")
                || message(value).contains("E259")
                || value["message"] == "No matches")
    });
    let invalid = diagnostics.iter().any(|value| {
        query(value, 1, "[")
            && rows(value).is_ok_and(|rows| rows.is_empty())
            && (message(value).contains("invalid query pattern")
                || (mode == Mode::Vim
                    && message(value).contains("E609")
                    && message(value).contains("pattern")))
    });
    let warning_empty = diagnostics.iter().any(|value| {
        query(value, 4, new) && warning(value) && rows(value).is_ok_and(|rows| rows.is_empty())
    });
    let warning_rows = diagnostics.iter().any(|value| {
        query(value, 4, "include")
            && warning(value)
            && rows(value).is_ok_and(|rows| {
                rows.iter().any(|row| {
                    let file = row[0].as_str().unwrap().replace('\\', "/");
                    file != plan["edit"]["file"]
                        && items(&plan["selection"]["closure_paths"])
                            .is_ok_and(|paths| paths.contains(&json!(file)))
                        && row[1] == "<unknown>"
                        && physical_line(root, &file, row[2].as_u64().unwrap())
                            .is_ok_and(|line| line == row[3] && line.contains("include"))
                })
            })
    });
    let source_line = physical_line(
        root,
        text(&plan["edit"]["file"])?,
        plan["edit"]["line"]
            .as_u64()
            .ok_or_else(|| bad("invalid edited physical line"))?,
    )?;
    let compact = |value: &str| {
        value
            .chars()
            .filter(|character| !character.is_ascii_whitespace())
            .collect::<String>()
    };
    let exact_head = |rows: &[Value], context: &str, live: bool| {
        rows.len() == 1
            && rows[0][0]
                .as_str()
                .is_some_and(|path| path.replace('\\', "/") == plan["edit"]["file"])
            && rows[0][1] == context
            && rows[0][2] == plan["edit"]["line"]
            && if live {
                rows[0][3] == source_line
            } else {
                compact(rows[0][3].as_str().unwrap()) == compact(&source_line)
            }
    };
    let repair = diagnostics.last().is_some_and(|value| {
        query(value, 4, new)
            && !warning(value)
            && rows(value).is_ok_and(|rows| exact_head(&rows, "<unknown>", true))
    });
    let stored = diagnostics.iter().rev().nth(1).is_some_and(|value| {
        query(value, 1, new)
            && !warning(value)
            && rows(value).is_ok_and(|rows| exact_head(&rows, new, false))
    });
    if !(empty && invalid && warning_empty && warning_rows && repair && stored) {
        return Err(bad(
            "expected editor diagnostic/stored/repair payload observations are missing or wrong",
        )
        .into());
    }
    if mode == Mode::Neovim {
        let pids = diagnostics
            .iter()
            .filter_map(|value| value["session_pid"].as_u64())
            .collect::<std::collections::BTreeSet<_>>();
        if pids.len() != 1 || pids.contains(&0) {
            return Err(bad("Neovim same-session identity missing or changed").into());
        }
    }
    Ok(())
}

fn physical_line(root: &Path, file: &str, line: u64) -> Result<String> {
    fs::read_to_string(relative(root, file)?)?
        .split('\n')
        .nth(
            line.checked_sub(1)
                .ok_or_else(|| bad("invalid physical line"))? as usize,
        )
        .map(str::to_owned)
        .ok_or_else(|| bad("physical line outside source").into())
}

fn record(run: &mut Value, stage: &str, request: &str, response: &Value, matches: bool) {
    run["stages"].as_array_mut().unwrap().push(
        json!({"stage":stage,"request":request,"response":response,"expectation_match":matches}),
    );
}

#[allow(clippy::too_many_arguments)] // Keep stage, request and frozen checks explicit.
fn query(
    session: &mut workflow_session::Session,
    run: &mut Value,
    stage: &str,
    field: u64,
    pattern: &str,
    expected: &Value,
    exact: bool,
    warnings: Option<&str>,
) -> Result<Value> {
    let request = format!("{field}{pattern}");
    let response = session.request(&request)?;
    let actual = items(&response["rows"])?;
    let expected = items(expected)?;
    let found = expected.iter().all(|row| {
        actual.iter().any(|actual| {
            row.as_array().is_some_and(|prefix| {
                actual.as_array().is_some_and(|actual| {
                    actual.len() >= prefix.len()
                        && prefix.iter().enumerate().all(|(index, value)| {
                            if index == 0 {
                                actual[index].as_str().zip(value.as_str()).is_some_and(
                                    |(actual, expected)| {
                                        actual.replace('\\', "/") == expected.replace('\\', "/")
                                    },
                                )
                            } else {
                                &actual[index] == value
                            }
                        })
                })
            })
        })
    });
    let diagnostics = if let Some(path) = warnings {
        items(&response["warnings"])?.iter().any(|warning| {
            warning.as_str().is_some_and(|warning| {
                warning.contains("cannot read")
                    && warning
                        .replace('\\', "/")
                        .contains(&path.replace('\\', "/"))
            })
        })
    } else {
        items(&response["warnings"])?.is_empty()
    };
    record(
        run,
        stage,
        &request,
        &response,
        found
            && (!exact || actual.len() == expected.len())
            && response["error"].is_null()
            && diagnostics,
    );
    Ok(response)
}

fn revision(
    options: &Options,
    root: &Path,
    output: &Path,
    name: &str,
    run: &mut Value,
) -> Result<u64> {
    let mut command = clean_command(&options.backend, root);
    command.args(["--verify", "-d", "-f", "workflow.db"]);
    let captured = capture::run_command(
        command,
        &output.join(name),
        budget(options, 120)?,
        &[(root.to_owned(), 2 << 30), (output.to_owned(), 2 << 30)],
    )?;
    run["stages"]
        .as_array_mut()
        .unwrap()
        .push(json!({"stage":name,"capture":captured}));
    if captured["complete"] != true {
        return Err(bad("revision observation command or cleanup incomplete").into());
    }
    let result = fs::read_to_string(output.join(name).with_extension("stdout"))?;
    let expression = regex::Regex::new(r"index verified: revision ([0-9]+), ([0-9]+) files")?;
    Ok(expression
        .captures(&result)
        .ok_or_else(|| bad("missing public verified revision observation"))?[1]
        .parse()?)
}

fn refresh(session: &mut workflow_session::Session, run: &mut Value, stage: &str) -> Result<()> {
    let response = session.request("r")?;
    record(
        run,
        stage,
        "r",
        &response,
        response["rebuilt"] == true
            && response["error"].is_null()
            && items(&response["warnings"])?.is_empty(),
    );
    Ok(())
}

fn cli(
    options: &Options,
    root: &Path,
    output: &Path,
    plan: &Value,
    selection: &str,
    run: &mut Value,
) -> Result<()> {
    let mut command = clean_command(&options.backend, root);
    command.args([
        if selection == "explicit-file-list" {
            "-bk"
        } else {
            "-bkR"
        },
        "-f",
        "workflow.db",
    ]);
    if selection == "explicit-file-list" {
        command.args(["-i", "workflow.files"]);
    }
    for directory in items(&plan["selection"]["include_directories"])? {
        command.arg("-I").arg(text(directory)?);
    }
    let captured = capture::run_command(
        command,
        &output.join("build"),
        budget(options, 1800)?,
        &[(root.to_owned(), 2 << 30), (output.to_owned(), 2 << 30)],
    )?;
    run["build"] = captured.clone();
    if captured["complete"] != true {
        return Err(bad("index build or cleanup incomplete").into());
    }
    let initial_revision = revision(options, root, output, "initial-revision", run)?;
    let mut command = clean_command(&options.backend, root);
    command.args(["-dl", "--json", "-f", "workflow.db"]);
    let mut session = workflow_session::Session::start(
        command,
        &output.join("session"),
        Duration::from_secs(budget(options, 120)?),
        &[(root.to_owned(), 2 << 30), (output.to_owned(), 2 << 30)],
    )?;
    session.limit_until(options.expires);
    let outcome = (|| -> Result<()> {
        let empty = json!([]);
        let membership = items(&plan["selection"]["closure_paths"])?
            .iter()
            .map(|name| Ok(json!([text(name)?, "<unknown>", 1])))
            .collect::<Result<Vec<_>>>()?;
        query(
            &mut session,
            run,
            "baseline-membership",
            7,
            ".",
            &json!(membership),
            true,
            None,
        )?;
        for neighbor in items(&plan["source_expectations"]["neighbor_unchanged"])? {
            query(
                &mut session,
                run,
                "original-neighbor",
                1,
                text(&neighbor[2])?,
                &json!([[neighbor[0], neighbor[2], neighbor[1]]]),
                false,
                None,
            )?;
        }
        let edit = &plan["edit"];
        let file = text(&edit["file"])?;
        let old = text(&edit["old"])?;
        let new = text(&edit["new"])?;
        let line = edit["line"]
            .as_u64()
            .ok_or_else(|| bad("missing edit line"))?;
        let original = fs::read(relative(root, file)?)?;
        if capture::digest(&relative(root, file)?)? != text(&edit["original_sha256"])? {
            return Err(bad("wrong source edit input hash").into());
        }
        query(
            &mut session,
            run,
            "original-definition",
            1,
            old,
            &json!([[file, old, line]]),
            false,
            None,
        )?;
        query(
            &mut session,
            run,
            "unique-name-before-edit",
            1,
            new,
            &empty,
            true,
            None,
        )?;
        let source = std::str::from_utf8(&original)?;
        let mut lines = source
            .split_inclusive('\n')
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let head = lines
            .get_mut(line as usize - 1)
            .ok_or_else(|| bad("edit line outside source"))?;
        if head.matches(old).count() != 1 {
            return Err(bad("edit head name is not unique on frozen line").into());
        }
        *head = head.replacen(old, new, 1);
        fs::write(relative(root, file)?, lines.concat())?;
        if capture::digest(&relative(root, file)?)? != text(&edit["edited_sha256"])? {
            return Err(bad("edited source hash differs from frozen expectation").into());
        }
        let edited = fs::read(relative(root, file)?)?;
        query(
            &mut session,
            run,
            "edited-before-refresh-stored",
            1,
            new,
            &empty,
            true,
            None,
        )?;
        query(
            &mut session,
            run,
            "edited-before-refresh-old",
            1,
            old,
            &json!([[file, old, line]]),
            false,
            None,
        )?;
        let line_text = lines[line as usize - 1].trim_end_matches(['\r', '\n']);
        for field in [4, 5, 6] {
            query(
                &mut session,
                run,
                "edited-before-refresh-live",
                field,
                new,
                &json!([[file, "<unknown>", line, line_text]]),
                true,
                None,
            )?;
        }
        refresh(&mut session, run, "Refresh-edited")?;
        let edited_revision = revision(options, root, output, "edited-revision", run)?;
        run["revision_observations"] = json!({"initial":initial_revision,"edited":edited_revision});
        let updated = query(
            &mut session,
            run,
            "edited-definition",
            1,
            new,
            &json!([[file, new, line]]),
            true,
            None,
        )?;
        let old_response = session.request(&format!("1{old}"))?;
        let stale = items(&old_response["rows"])?.iter().any(|row| {
            row[0]
                .as_str()
                .is_some_and(|path| path.replace('\\', "/") == file)
                && row[1] == old
                && row[2] == line
        });
        record(
            run,
            "old-head-removed",
            &format!("1{old}"),
            &old_response,
            !stale
                && old_response["error"].is_null()
                && items(&old_response["warnings"])?.is_empty(),
        );
        for neighbor in items(&plan["source_expectations"]["neighbor_unchanged"])? {
            query(
                &mut session,
                run,
                "unchanged-neighbor",
                1,
                text(&neighbor[2])?,
                &json!([[neighbor[0], neighbor[2], neighbor[1]]]),
                false,
                None,
            )?;
        }
        for call in items(&plan["source_expectations"]["calls_after"])? {
            query(
                &mut session,
                run,
                "edited-callees",
                2,
                text(&call[2])?,
                &json!([[call[0], call[3], call[1]]]),
                false,
                None,
            )?;
            query(
                &mut session,
                run,
                "edited-callers",
                3,
                text(&call[3])?,
                &json!([[call[0], call[2], call[1]]]),
                false,
                None,
            )?;
        }
        refresh(&mut session, run, "Refresh-no-change")?;
        let unchanged_revision = revision(options, root, output, "unchanged-revision", run)?;
        let no_change = session.request(&format!("1{new}"))?;
        record(
            run,
            "no-change-answer",
            &format!("1{new}"),
            &no_change,
            no_change == updated,
        );
        run["revision_observations"]["no_change"] = json!(unchanged_revision);
        run["revision_observations"]["no_change_equal"] =
            json!(unchanged_revision == edited_revision);
        for field in 0..=9 {
            query(
                &mut session,
                run,
                "clean-empty",
                field,
                "__ripscope_workflow_absent_20261005__",
                &empty,
                true,
                None,
            )?;
        }
        for field in [1, 6] {
            let request = format!("{field}[");
            let response = session.request(&request)?;
            record(
                run,
                "invalid-pattern",
                &request,
                &response,
                response["error"]
                    .as_str()
                    .is_some_and(|error| error.contains("invalid query pattern"))
                    && items(&response["rows"])?.is_empty(),
            );
            query(
                &mut session,
                run,
                "same-session-recovery",
                1,
                new,
                &json!([[file, new, line]]),
                true,
                None,
            )?;
        }
        fs::remove_file(relative(root, file)?)?;
        query(
            &mut session,
            run,
            "deleted-original-live-empty",
            4,
            new,
            &empty,
            true,
            Some(file),
        )?;
        let available = session.request("4include")?;
        record(
            run,
            "deleted-original-live-available",
            "4include",
            &available,
            !items(&available["rows"])?.is_empty()
                && items(&available["warnings"])?.iter().any(|warning| {
                    warning.as_str().is_some_and(|warning| {
                        warning.contains("cannot read") && warning.contains(file)
                    })
                }),
        );
        query(
            &mut session,
            run,
            "deleted-original-stored",
            1,
            new,
            &json!([[file, new, line]]),
            true,
            None,
        )?;
        fs::write(relative(root, file)?, &edited)?;
        query(
            &mut session,
            run,
            "same-session-live-repair",
            4,
            new,
            &json!([[file, "<unknown>", line, line_text]]),
            true,
            None,
        )?;
        let addition = read(&options.frame.join("plans/added-files.json"))?;
        let queries = items(&addition["queries"])?;
        for stage in ["before-add", "added-before-Refresh"] {
            if stage == "added-before-Refresh" {
                for (name, bytes) in addition["files"]
                    .as_object()
                    .ok_or_else(|| bad("missing added files"))?
                {
                    fs::write(relative(root, name)?, text(bytes)?)?;
                }
                if selection == "explicit-file-list" {
                    file_list(root, plan, Some(text(&addition["seed"])?))?;
                }
            }
            for request in queries {
                query(
                    &mut session,
                    run,
                    stage,
                    request["field"]
                        .as_u64()
                        .ok_or_else(|| bad("missing added query field"))?,
                    text(&request["pattern"])?,
                    &empty,
                    true,
                    None,
                )?;
            }
        }
        refresh(&mut session, run, "Refresh-added")?;
        run["revision_observations"]["added"] =
            json!(revision(options, root, output, "added-revision", run)?);
        for request in queries {
            query(
                &mut session,
                run,
                "added-after-Refresh",
                request["field"].as_u64().unwrap(),
                text(&request["pattern"])?,
                &request["expected"],
                true,
                None,
            )?;
        }
        let seed = text(&addition["seed"])?;
        fs::remove_file(relative(root, seed)?)?;
        for request in queries {
            let field = request["field"].as_u64().unwrap();
            let live = [4, 5, 6].contains(&field);
            query(
                &mut session,
                run,
                "deleted-added-before-Refresh",
                field,
                text(&request["pattern"])?,
                if live { &empty } else { &request["expected"] },
                true,
                live.then_some(seed),
            )?;
        }
        let available = session.request("4int")?;
        record(
            run,
            "deleted-added-readable-include",
            "4int",
            &available,
            items(&available["rows"])?.iter().any(|row| {
                row[0].as_str().is_some_and(|path| {
                    path.replace('\\', "/") == addition["include_only"].as_str().unwrap_or("")
                })
            }) && items(&available["warnings"])?.iter().any(|warning| {
                warning.as_str().is_some_and(|warning| {
                    warning.contains("cannot read") && warning.contains(seed)
                })
            }),
        );
        if selection == "explicit-file-list" {
            file_list(root, plan, None)?;
        }
        refresh(&mut session, run, "Refresh-deleted")?;
        let deleted_revision = revision(options, root, output, "deleted-revision", run)?;
        run["revision_observations"]["deleted"] = json!(deleted_revision);
        for request in queries {
            query(
                &mut session,
                run,
                "deleted-added-after-Refresh",
                request["field"].as_u64().unwrap(),
                text(&request["pattern"])?,
                &empty,
                true,
                None,
            )?;
        }
        query(
            &mut session,
            run,
            "final-membership",
            7,
            ".",
            &json!(membership),
            true,
            None,
        )?;
        refresh(&mut session, run, "Refresh-final-no-change")?;
        let final_revision = revision(options, root, output, "final-no-change-revision", run)?;
        run["revision_observations"]["final_no_change"] = json!(final_revision);
        run["revision_observations"]["final_no_change_equal"] =
            json!(final_revision == deleted_revision);
        query(
            &mut session,
            run,
            "final-no-change-membership",
            7,
            ".",
            &json!(membership),
            true,
            None,
        )?;
        fs::remove_file(relative(root, text(&addition["include_only"])?)?)?;
        Ok(())
    })();
    let cleanup = session.finish()?;
    run["session_cleanup"] = cleanup.clone();
    outcome?;
    if cleanup["state"] != "confirmed" {
        return Err(bad("session cleanup incomplete; runtime preserved").into());
    }
    run["expectations_match"] = json!(
        items(&run["stages"])?
            .iter()
            .all(|stage| stage["expectation_match"] != false)
            && run["revision_observations"]["no_change_equal"] == true
            && run["revision_observations"]["final_no_change_equal"] == true
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{Path, Value, capture, fs};
    use std::path::PathBuf;
    use std::process::Command;

    fn fixture() -> PathBuf {
        static BUILT: std::sync::OnceLock<()> = std::sync::OnceLock::new();
        let executable = std::env::current_exe().unwrap();
        let directory = executable.parent().unwrap();
        let directory = if directory.file_name().unwrap() == "deps" {
            directory.parent().unwrap().join("examples")
        } else {
            directory.to_owned()
        };
        let fixture = directory.join(format!("workflow-fixture{}", std::env::consts::EXE_SUFFIX));
        BUILT.get_or_init(|| {
            // Cross-built native tests reuse the exported binaries without guest Rust.
            if Path::new(env!("CARGO")).is_file() {
                let built = Command::new("cargo")
                    .current_dir(env!("CARGO_MANIFEST_DIR"))
                    .args([
                        "build",
                        "--locked",
                        "--example",
                        "workflow-fixture",
                        "--example",
                        "assess-workflows",
                    ])
                    .output()
                    .unwrap();
                assert!(
                    built.status.success(),
                    "{}",
                    String::from_utf8_lossy(&built.stderr)
                );
            }
            // Microsoft Defender checks a newly linked executable on its first launch,
            // in about half a second, but ten seconds or more when its cloud lookup is
            // slow. Launch the fixture once here, untimed, so that check never counts
            // against a test's deadline.
            let launched = Command::new(&fixture)
                .arg("environment")
                .env("WORKFLOW_FIXTURE_VALUE", "first launch")
                .output()
                .unwrap();
            assert!(
                launched.status.success(),
                "{}",
                String::from_utf8_lossy(&launched.stderr)
            );
        });
        fixture
    }

    #[test]
    fn prepared_capture_keeps_environment_and_streams() {
        let directory = fixture_directory();
        let mut command = Command::new(fixture());
        command
            .arg("environment")
            .env("WORKFLOW_FIXTURE_VALUE", "private value");
        let prefix = directory.path().join("command");
        let result = capture::run_command(command, &prefix, 5, &[]).unwrap();
        assert_eq!(result["status"], 0);
        assert_eq!(
            std::fs::read_to_string(prefix.with_extension("stdout")).unwrap(),
            "private value\n"
        );
        assert_eq!(
            std::fs::read_to_string(prefix.with_extension("stderr")).unwrap(),
            "diagnostic\n"
        );
        assert!(result["cleanup"]["state"].is_string());
    }

    #[test]
    fn successful_parent_does_not_hide_a_lingering_descendant() {
        let directory = fixture_directory();
        let marker = directory.path().join("descendant.pid");
        let mut command = Command::new(fixture());
        command.arg("early-exit").arg(&marker);
        let result = capture::run_command(command, &directory.path().join("tree"), 5, &[]).unwrap();
        // A launcher failure (systemd-run exits 1) names its cause on standard error.
        assert_eq!(
            result["status"],
            0,
            "{result} {}",
            fs::read_to_string(directory.path().join("tree.stderr")).unwrap_or_default()
        );
        assert!(marker.exists());
        assert_ne!(result["cleanup"]["state"], serde_json::Value::Null);
        #[cfg(windows)]
        {
            let pid = fs::read_to_string(&marker)
                .unwrap()
                .trim()
                .parse::<u64>()
                .unwrap();
            assert!(
                result["cleanup"]["members_before_termination"]
                    .as_array()
                    .unwrap()
                    .contains(&serde_json::json!(pid)),
                "{result}"
            );
            assert_eq!(result["cleanup"]["state"], "confirmed", "{result}");
            assert_eq!(result["cleanup"]["members_after"], serde_json::json!([]));
            println!("early-parent-exit={result}");
        }
        #[cfg(target_os = "linux")]
        {
            let pid = std::fs::read_to_string(marker).unwrap();
            let stat = std::fs::read_to_string(format!("/proc/{}/stat", pid.trim()));
            assert!(stat.is_err() || stat.unwrap().split(") ").nth(1).unwrap().starts_with('Z'));
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_proof_contains_early_exit_and_detached_grandchildren() {
        for role in ["early-tree", "detached-tree"] {
            let directory = fixture_directory();
            let marker = directory.path().join("descendants");
            let mut command = Command::new(fixture());
            command.arg(role).arg(&marker);
            let result =
                capture::run_command(command, &directory.path().join("tree"), 5, &[]).unwrap();
            assert_eq!(result["status"], 0, "{result}");
            let before = result["cleanup"]["members_before_termination"]
                .as_array()
                .unwrap();
            for pid in fs::read_to_string(marker).unwrap().lines() {
                assert!(
                    before.contains(&serde_json::json!(pid.parse::<u64>().unwrap())),
                    "{role}: {result}"
                );
            }
            assert_eq!(result["cleanup"]["state"], "confirmed", "{result}");
            assert_eq!(result["cleanup"]["members_after"], serde_json::json!([]));
            println!("{role}={result}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_pending_launch_is_unknown_even_with_an_empty_proof_job() {
        for delay in [
            "WORKFLOW_FIXTURE_LAUNCH_DELAY_MS",
            "WORKFLOW_FIXTURE_ASSIGN_DELAY_MS",
        ] {
            let directory = fixture_directory();
            let runtime = directory.path().join("runtime-copy");
            fs::create_dir(&runtime).unwrap();
            fs::write(runtime.join("source.c"), "int retained;\n").unwrap();
            let mut command = Command::new(fixture());
            command.arg("leaf").env(delay, "6500");
            let mut child = capture::owned_process::OwnedProcess::spawn(
                command,
                fs::File::create(directory.path().join("stdout")).unwrap(),
                fs::File::create(directory.path().join("stderr")).unwrap(),
                false,
            )
            .unwrap();
            std::thread::sleep(std::time::Duration::from_millis(150));
            let started = std::time::Instant::now();
            let first = child.finish();
            assert!(started.elapsed() < std::time::Duration::from_secs(6));
            assert_eq!(first["state"], "unknown", "{first}");
            assert_eq!(first["launch_terminal"], false, "{first}");
            assert_eq!(first["members_after"], serde_json::json!([]));
            if delay == "WORKFLOW_FIXTURE_ASSIGN_DELAY_MS" {
                assert!(
                    !first["native_pids"].as_array().unwrap().is_empty(),
                    "{first}"
                );
            }
            assert!(runtime.join("source.c").exists());
            capture::save(&directory.path().join("first-unknown.json"), &first).unwrap();
            println!("{delay}-first={first}");
            std::thread::sleep(std::time::Duration::from_secs(2));
            let terminal = child.finish();
            assert_eq!(terminal["state"], "confirmed", "{terminal}");
            assert!(terminal["launch_error"].is_string());
            assert!(runtime.join("source.c").exists());
            assert_eq!(
                super::read(&directory.path().join("first-unknown.json")).unwrap()["state"],
                "unknown"
            );
            println!("{delay}-terminal={terminal}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_unknown_launch_preserves_runtime_at_driver_interface() {
        if let Some(root) = std::env::var_os("WORKFLOW_FIXTURE_DRIVER_CHILD_ROOT") {
            let root = Path::new(&root);
            let frame = tiny_frame(root);
            let command = editor_command_mode(root, &frame, "cli");
            assert!(
                !super::run(command.get_args().map(|argument| argument.to_os_string())).unwrap()
            );
            let report = super::read(&root.join("out/report.json")).unwrap();
            assert_eq!(report["complete"], false);
            assert_eq!(
                report["runs"][0]["build"]["cleanup"]["state"], "unknown",
                "{report}"
            );
            assert_eq!(report["runs"][0]["runtime_preserved"], true, "{report}");
            let runtime = Path::new(report["runs"][0]["runtime_root"].as_str().unwrap());
            assert!(runtime.join("a.c").is_file());
            println!("pending-driver-runtime={report}");
            std::thread::sleep(std::time::Duration::from_secs(5));
            assert_eq!(super::read(&root.join("out/report.json")).unwrap(), report);
            return;
        }
        let directory = fixture_directory();
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "tests::windows_unknown_launch_preserves_runtime_at_driver_interface",
                "--nocapture",
            ])
            .env("WORKFLOW_FIXTURE_DRIVER_CHILD_ROOT", directory.path())
            .env("WORKFLOW_FIXTURE_PLATFORM_SECONDS", "4")
            .env("WORKFLOW_FIXTURE_DELAY_PROGRAM", fixture())
            .env("WORKFLOW_FIXTURE_LAUNCH_DELAY_MS", "12000")
            .output()
            .unwrap();
        println!("{}", String::from_utf8_lossy(&output.stdout));
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let report = super::read(&directory.path().join("out/report.json")).unwrap();
        assert!(
            Path::new(report["runs"][0]["runtime_root"].as_str().unwrap())
                .join("a.c")
                .exists()
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_startup_failure_and_membership_error_never_hide_owned_children() {
        for failure in ["assignment", "resume"] {
            let directory = fixture_directory();
            let mut command = Command::new(fixture());
            command
                .arg("leaf")
                .env("WORKFLOW_FIXTURE_STARTUP_FAILURE", failure);
            let result =
                capture::run_command(command, &directory.path().join("failure"), 5, &[]).unwrap();
            assert_eq!(result["complete"], false, "{result}");
            assert!(result["monitor_error"].is_string(), "{result}");
            assert_eq!(result["cleanup"]["state"], "confirmed", "{result}");
            assert_eq!(result["cleanup"]["native_startup_accounted"], true);
            assert!(
                !result["cleanup"]["native_pids"]
                    .as_array()
                    .unwrap()
                    .is_empty()
            );
            println!("{failure}-failure={result}");
        }
        let directory = fixture_directory();
        let mut command = Command::new(fixture());
        command
            .arg("environment")
            .env("WORKFLOW_FIXTURE_VALUE", "captured")
            .env("WORKFLOW_FIXTURE_QUERY_FAILURE", "1");
        let result =
            capture::run_command(command, &directory.path().join("query-failure"), 5, &[]).unwrap();
        assert_eq!(result["complete"], false);
        assert_eq!(result["cleanup"]["state"], "unknown");
        assert!(result["cleanup"]["membership_error"].is_string());
        println!("membership-query-failure={result}");
    }

    #[cfg(windows)]
    #[test]
    fn windows_deadline_terminates_a_populated_proof_job() {
        let directory = fixture_directory();
        let mut command = Command::new(fixture());
        command.arg("leaf");
        let result =
            capture::run_command(command, &directory.path().join("deadline"), 1, &[]).unwrap();
        assert_eq!(result["complete"], false, "{result}");
        assert_eq!(result["stopped"], "timeout", "{result}");
        assert!(
            !result["cleanup"]["members_before_termination"]
                .as_array()
                .unwrap()
                .is_empty(),
            "{result}"
        );
        assert_eq!(result["cleanup"]["state"], "confirmed", "{result}");
        assert_eq!(result["cleanup"]["members_after"], serde_json::json!([]));
        println!("populated-job-deadline={result}");
    }

    #[test]
    fn session_handles_fragmented_rows_warnings_errors_and_refresh() {
        let directory = fixture_directory();
        let mut command = Command::new(fixture());
        command.arg("protocol");
        let mut session = super::workflow_session::Session::start(
            command,
            &directory.path().join("session"),
            std::time::Duration::from_secs(2),
            &[],
        )
        .unwrap();
        let result = session.request("1alpha").unwrap();
        assert_eq!(
            result["rows"],
            serde_json::json!([["a.c", "alpha", 1, "int alpha(void) {}"]])
        );
        assert_eq!(result["warnings"], serde_json::json!(["fixture warning"]));
        assert_eq!(
            session.request("1[").unwrap()["error"],
            "invalid query pattern"
        );
        assert_eq!(
            session.request("1alpha").unwrap()["rows"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        let refresh = session.request("r").unwrap();
        assert_eq!(refresh["rebuilt"], true);
        assert_eq!(refresh["warnings"], serde_json::json!(["refresh warning"]));
        session.finish().unwrap();
    }

    #[test]
    fn bare_ready_is_not_an_empty_query() {
        let directory = fixture_directory();
        let mut command = Command::new(fixture());
        command.arg("protocol");
        let mut session = super::workflow_session::Session::start(
            command,
            &directory.path().join("session"),
            std::time::Duration::from_secs(2),
            &[],
        )
        .unwrap();
        assert!(session.request("9bare").is_err());
        assert!(directory.path().join("session.stdout").exists());
        session.finish().unwrap();
    }

    #[test]
    fn malformed_count_and_truncated_frames_preserve_partial_capture() {
        for request in ["1count", "1truncated"] {
            let directory = fixture_directory();
            let mut command = Command::new(fixture());
            command.arg("protocol");
            let mut session = super::workflow_session::Session::start(
                command,
                &directory.path().join("session"),
                std::time::Duration::from_millis(150),
                &[],
            )
            .unwrap();
            assert!(session.request(request).is_err());
            session.finish().unwrap();
            assert!(
                !fs::read(directory.path().join("session.stdout"))
                    .unwrap()
                    .is_empty()
            );
            assert!(directory.path().join("session.cleanup.json").exists());
        }
    }

    #[test]
    fn capture_deadline_and_tree_cap_are_incomplete() {
        for limit in [None, Some(0)] {
            let directory = fixture_directory();
            let mut command = Command::new(fixture());
            command
                .arg("environment")
                .env("WORKFLOW_FIXTURE_VALUE", "captured");
            let prefix = directory.path().join("capture");
            let limits = limit
                .map(|limit| vec![(directory.path().to_owned(), limit)])
                .unwrap_or_default();
            let result = capture::run_command(
                command,
                &prefix,
                if limit.is_none() { 0 } else { 5 },
                &limits,
            )
            .unwrap();
            assert_eq!(result["complete"], false);
            assert!(prefix.with_extension("json").exists());
            assert_eq!(
                result["stopped"],
                if limit.is_none() {
                    "timeout"
                } else {
                    "tree_artifact_limit"
                }
            );
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn scope_cleanup_contains_a_detached_child_and_grandchild() {
        let directory = fixture_directory();
        let marker = directory.path().join("descendants");
        let mut command = Command::new(fixture());
        command.arg("detached-tree").arg(&marker);
        let result = capture::run_command(command, &directory.path().join("tree"), 5, &[]).unwrap();
        let pids = fs::read_to_string(marker).unwrap();
        for pid in pids.lines() {
            let stat = fs::read_to_string(format!("/proc/{pid}/stat"));
            assert!(stat.is_err() || stat.unwrap().split(") ").nth(1).unwrap().starts_with('Z'));
        }
        assert_eq!(result["cleanup"]["state"], "confirmed", "{result}");
    }

    fn driver() -> PathBuf {
        fixture().with_file_name(format!("assess-workflows{}", std::env::consts::EXE_SUFFIX))
    }

    fn tiny_frame(root: &std::path::Path) -> PathBuf {
        let frame = root.join("frame");
        for directory in [
            "inputs/tiny",
            "plans",
            "sealed",
            "validation",
            "editor-scripts",
        ] {
            std::fs::create_dir_all(frame.join(directory)).unwrap();
        }
        let original = "int alpha(void) { return neighbor(); }\nint neighbor(void) { return 2; }\n#include \"b.h\"\n";
        let edited = "int renamed(void) { return neighbor(); }\nint neighbor(void) { return 2; }\n#include \"b.h\"\n";
        std::fs::write(frame.join("inputs/tiny/a.c"), original).unwrap();
        std::fs::write(
            frame.join("inputs/tiny/b.h"),
            "// include lifecycle control\n",
        )
        .unwrap();
        let hash = |bytes: &str| {
            use sha2::Digest;
            base16ct::lower::encode_string(&sha2::Sha256::digest(bytes.as_bytes()))
        };
        let plan = serde_json::json!({"name":"tiny","workflow_inventory_version":3,"edit":{"file":"a.c","line":1,"old":"alpha","new":"renamed","original_sha256":hash(original),"edited_sha256":hash(edited)},
            "selection":{"include_directories":[],"index_files":["a.c","b.h"],"closure_paths":["a.c","b.h"]},"source_expectations":{"neighbor_unchanged":[["a.c",2,"neighbor","function"]],"calls_after":[["a.c",1,"renamed","neighbor","call"]]}});
        capture::save(&frame.join("plans/tiny.json"), &plan).unwrap();
        capture::save(&frame.join("plans/added-files.json"),&serde_json::json!({"files":{"added.c":"#include \"added.inc\"\nint added(void) { return extra(); }\n","added.inc":"int extra(void) { return 3; }\n"},"queries":[{"field":1,"pattern":"added","expected":[["added.c","added",2]]},{"field":1,"pattern":"extra","expected":[["added.inc","extra",1]]},{"field":7,"pattern":"added[.]","expected":[["added.c","<unknown>",1],["added.inc","<unknown>",1]]}],"seed":"added.c","include_only":"added.inc"})).unwrap();
        capture::save(
            &frame.join("plans/workflow-contract.json"),
            &serde_json::json!({"workflow_inventory_version":3}),
        )
        .unwrap();
        capture::save(&frame.join("workflow-inventory.json"),&serde_json::json!({"workflow_inventory_version":3,"projects":[{"name":"tiny","editor_plan":"plans/tiny.json","sources":[{"path":"a.c","target":"a.c","links":[],"bytes":original.len(),"sha256":hash(original)},{"path":"b.h","target":"b.h","links":[],"bytes":28,"sha256":hash("// include lifecycle control\n")}]}]})).unwrap();
        for extension in ["vim", "lua", "el"] {
            std::fs::write(
                frame.join(format!("editor-scripts/project.{extension}")),
                "independent fixture",
            )
            .unwrap();
        }
        let paths = [
            "plans/tiny.json",
            "plans/added-files.json",
            "plans/workflow-contract.json",
            "workflow-inventory.json",
            "editor-scripts/project.vim",
            "editor-scripts/project.lua",
            "editor-scripts/project.el",
        ];
        let list = paths
            .iter()
            .map(|path| format!("{}  {path}\n", capture::digest(&frame.join(path)).unwrap()))
            .collect::<String>();
        std::fs::write(frame.join("validation/workflow-inputs.sha256"), &list).unwrap();
        capture::save(&frame.join("SEAL.json"),&serde_json::json!({"sealed":true,"workflow_inventory_version":3,"input_hash_list":{"path":"validation/workflow-inputs.sha256","sha256":hash(&list)}})).unwrap();
        frame
    }

    fn editor_command(directory: &std::path::Path, frame: &std::path::Path) -> Command {
        editor_command_mode(directory, frame, "emacs")
    }

    fn editor_command_mode(
        directory: &std::path::Path,
        frame: &std::path::Path,
        mode: &str,
    ) -> Command {
        let mut command = Command::new(driver());
        command.args([
            "--frame",
            frame.to_str().unwrap(),
            "--project",
            "tiny",
            "--backend",
            fixture().to_str().unwrap(),
            "--repo",
            &std::env::var("WORKFLOW_FIXTURE_REPO")
                .unwrap_or_else(|_| env!("CARGO_MANIFEST_DIR").to_owned()),
            "--work-root",
            directory.to_str().unwrap(),
            "--output",
            directory.join("out").to_str().unwrap(),
            "--mode",
            mode,
            "--editor",
            fixture().to_str().unwrap(),
        ]);
        command
    }

    #[test]
    fn neovim_raw_row_observations_require_one_session_identity() {
        for (role, complete) in [("neovim-valid", true), ("neovim-pid-change", false)] {
            let directory = fixture_directory();
            let frame = tiny_frame(directory.path());
            let output = editor_command_mode(directory.path(), &frame, "neovim")
                .env("WORKFLOW_FIXTURE_EDITOR", role)
                .output()
                .unwrap();
            let report = super::read(&directory.path().join("out/report.json")).unwrap();
            assert_eq!(
                output.status.success(),
                complete,
                "{report}\n{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(report["complete"], complete, "{report}");
            if !complete {
                assert!(
                    report["failure"]
                        .as_str()
                        .unwrap()
                        .contains("same-session identity"),
                    "{report}"
                );
            }
        }
    }

    #[test]
    fn recorded_native_editor_rows_cross_the_same_driver_interface() {
        for mode in ["vim", "emacs"] {
            let directory = fixture_directory();
            let frame = tiny_frame(directory.path());
            let output = editor_command_mode(directory.path(), &frame, mode)
                .env("WORKFLOW_FIXTURE_EDITOR", format!("{mode}-valid"))
                .output()
                .unwrap();
            let report = super::read(&directory.path().join("out/report.json")).unwrap();
            assert!(output.status.success(), "{mode}: {report}");
            assert_eq!(report["complete"], true, "{mode}: {report}");
        }
    }

    #[test]
    fn unrelated_editor_rows_fail_at_the_driver_cli() {
        for role in [
            "neovim-unrelated-stored",
            "neovim-unrelated-repair",
            "neovim-unrelated-readable",
        ] {
            let directory = fixture_directory();
            let frame = tiny_frame(directory.path());
            let output = editor_command_mode(directory.path(), &frame, "neovim")
                .env("WORKFLOW_FIXTURE_EDITOR", role)
                .output()
                .unwrap();
            let report = super::read(&directory.path().join("out/report.json")).unwrap();
            assert!(!output.status.success(), "{role}: {report}");
            assert_eq!(report["complete"], false, "{role}: {report}");
            assert!(
                report["failure"].as_str().unwrap().contains("payload"),
                "{role}: {report}"
            );
        }
    }

    #[test]
    fn successful_empty_editor_report_fails_at_the_driver_cli() {
        let directory = fixture_directory();
        let frame = tiny_frame(directory.path());
        let output = editor_command(directory.path(), &frame)
            .env("RIPSCOPE_WORKFLOW_VALIDATE_ONLY", "1")
            .output()
            .unwrap();
        assert!(!output.status.success());
        let report: serde_json::Value = serde_json::from_slice(
            &std::fs::read(directory.path().join("out/report.json")).unwrap(),
        )
        .unwrap();
        assert!(
            report["failure"].as_str().unwrap().contains("observations"),
            "{report}"
        );
        assert_eq!(
            std::fs::read_to_string(directory.path().join("out/editor-environment.txt")).unwrap(),
            "controlled"
        );
    }

    #[test]
    fn dirty_source_stops_before_backend_or_editor_launch() {
        let directory = fixture_directory();
        let frame = tiny_frame(directory.path());
        std::fs::write(frame.join("inputs/tiny/a.c"), "changed").unwrap();
        let output = editor_command(directory.path(), &frame).output().unwrap();
        assert!(!output.status.success());
        assert!(!directory.path().join("out/editor-environment.txt").exists());
        let report: serde_json::Value = serde_json::from_slice(
            &std::fs::read(directory.path().join("out/report.json")).unwrap(),
        )
        .unwrap();
        assert!(
            report["failure"]
                .as_str()
                .unwrap()
                .contains("frozen source")
        );
    }

    #[test]
    fn unlisted_input_stops_before_backend_or_editor_launch() {
        let directory = fixture_directory();
        let frame = tiny_frame(directory.path());
        fs::write(frame.join("inputs/tiny/unlisted.c"), "int surprise;\n").unwrap();
        let output = editor_command(directory.path(), &frame).output().unwrap();
        assert!(!output.status.success());
        assert!(!directory.path().join("out/editor-environment.txt").exists());
        let report: Value =
            serde_json::from_slice(&fs::read(directory.path().join("out/report.json")).unwrap())
                .unwrap();
        assert!(
            report["failure"]
                .as_str()
                .unwrap()
                .contains("outside frozen inventory"),
            "{report}"
        );
    }

    #[test]
    fn failed_native_launch_retains_capture_and_session_outcomes() {
        let directory = fixture_directory();
        let missing = directory.path().join("missing-native-executable");
        let result = capture::run_command(
            Command::new(&missing),
            &directory.path().join("capture"),
            1,
            &[],
        )
        .unwrap();
        assert_eq!(result["complete"], false);
        assert!(directory.path().join("capture.json").exists());
        assert!(
            super::workflow_session::Session::start(
                Command::new(missing),
                &directory.path().join("session"),
                std::time::Duration::from_secs(1),
                &[]
            )
            .is_err()
        );
        let cleanup: Value = serde_json::from_slice(
            &fs::read(directory.path().join("session.cleanup.json")).unwrap(),
        )
        .unwrap();
        assert!(cleanup["startup_error"].is_string());
        assert!(cleanup["cleanup"]["state"].is_string());
    }

    struct FixtureDirectory(Option<tempfile::TempDir>);
    impl FixtureDirectory {
        fn path(&self) -> &Path {
            self.0.as_ref().unwrap().path()
        }
    }
    impl Drop for FixtureDirectory {
        fn drop(&mut self) {
            if std::thread::panicking() {
                let retained = self.0.take().unwrap().keep();
                eprintln!("failed fixture evidence retained at {}", retained.display());
            }
        }
    }
    fn fixture_directory() -> FixtureDirectory {
        #[cfg(target_os = "linux")]
        return FixtureDirectory(Some(tempfile::tempdir_in("/var/tmp").unwrap()));
        #[cfg(not(target_os = "linux"))]
        FixtureDirectory(Some(tempfile::tempdir().unwrap()))
    }

    #[test]
    fn plausible_editor_observations_do_not_override_the_frozen_edit_hash() {
        let directory = fixture_directory();
        let frame = tiny_frame(directory.path());
        let output = editor_command(directory.path(), &frame)
            .env("WORKFLOW_FIXTURE_EDITOR", "wrong-hash")
            .output()
            .unwrap();
        assert!(!output.status.success());
        let report: Value =
            serde_json::from_slice(&fs::read(directory.path().join("out/report.json")).unwrap())
                .unwrap();
        assert!(
            report["failure"]
                .as_str()
                .unwrap()
                .contains("edited source hash"),
            "{report}"
        );
        let runtime = Path::new(report["runs"][0]["runtime_root"].as_str().unwrap());
        assert!(runtime.exists());
    }

    #[test]
    fn reused_output_is_rejected_without_mutation() {
        let directory = fixture_directory();
        let frame = tiny_frame(directory.path());
        fs::create_dir(directory.path().join("out")).unwrap();
        fs::write(directory.path().join("out/report.json"), "preserve").unwrap();
        assert!(
            !editor_command(directory.path(), &frame)
                .output()
                .unwrap()
                .status
                .success()
        );
        assert_eq!(
            fs::read_to_string(directory.path().join("out/report.json")).unwrap(),
            "preserve"
        );
    }

    #[test]
    fn cli_runs_the_frozen_lifecycle_in_one_session_per_selection() {
        let directory = fixture_directory();
        let frame = tiny_frame(directory.path());
        let editor = editor_command(directory.path(), &frame);
        let arguments = editor
            .get_args()
            .map(|arg| arg.to_os_string())
            .collect::<Vec<_>>();
        let mut command = Command::new(driver());
        for pair in arguments.chunks(2) {
            if pair[0] == "--editor" {
                continue;
            }
            if pair[0] == "--mode" {
                command.args(["--mode", "cli"]);
            } else {
                command.args(pair);
            }
        }
        let output = command.output().unwrap();
        let report: Value =
            serde_json::from_slice(&fs::read(directory.path().join("out/report.json")).unwrap())
                .unwrap();
        assert!(
            output.status.success(),
            "{}\n{report}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(report["runs"].as_array().unwrap().len(), 2);
        for run in report["runs"].as_array().unwrap() {
            assert_eq!(run["expectations_match"], true, "{run}");
            assert_eq!(run["session_cleanup"]["state"], "confirmed");
            assert!(!Path::new(run["runtime_root"].as_str().unwrap()).exists());
            let transcript: Value = serde_json::from_slice(
                &fs::read(
                    directory
                        .path()
                        .join("out")
                        .join(run["selection"].as_str().unwrap())
                        .join("session.transcript.json"),
                )
                .unwrap(),
            )
            .unwrap();
            let pids = transcript
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|entry| entry["pid"].as_u64())
                .collect::<std::collections::BTreeSet<_>>();
            assert_eq!(pids.len(), 1);
        }
    }
}
