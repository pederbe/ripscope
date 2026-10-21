//! Check the one-step editor setup in fresh user profiles: Vim, Neovim and Emacs find
//! the executable, index a project in the cache directory on the first search, change
//! nothing in the project but the saved edit and show that edit in the next search.
//!
//! Routes set each editor up:
//! - `path`: the plugin as Vim's and Neovim's own packages, or on Emacs's load path,
//!   with ripscope on PATH, and for Vim on Linux a shell whose startup prints;
//! - `plugin`: a repository copy whose install command installs ripscope, as plugin
//!   managers' build hooks do, over an older executable on Linux;
//! - `installer`: the installer, which places the integrations itself, then
//!   `--uninstall`, with a decoy installer around the Emacs copy it placed;
//! - `installer-lazy`: the installer where lazy.nvim is set up, with the line it prints;
//! - `interrupted`: as `path`, after a first index build that published nothing;
//! - `home`: as `path`, with the editor started in the home directory;
//! - `spaces`: as `path`, in a profile whose paths hold spaces, with Vim's cache
//!   where `RIPSCOPE_CACHE_DIR` puts it;
//! - `vim-plug`, `lazy`, `use-package` and `package-vc`: the documented plugin manager
//!   lines, following the `release` branch of a Git repository of this checkout.
//!
//! The installing routes use a development archive of the given executable, served by a
//! loopback mirror. `ripscope --project` names each project's index.
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

#[allow(dead_code)]
#[path = "../../examples/support/capture.rs"]
mod capture;
#[path = "../../examples/support/mirror.rs"]
mod mirror;
#[path = "../../examples/support/profile.rs"]
mod profile;
use profile::{Profile, powershell};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const WINDOWS: bool = cfg!(windows);
const ROUTES: [&str; 11] = [
    "path",
    "plugin",
    "installer",
    "installer-lazy",
    "interrupted",
    "home",
    "spaces",
    "vim-plug",
    "lazy",
    "use-package",
    "package-vc",
];
/// The routes that install from a Git repository, through a plugin manager.
const MANAGERS: [&str; 4] = ["vim-plug", "lazy", "use-package", "package-vc"];
const FRESH: &str = "int fresh_symbol(void) { return 3; }";

fn main() -> ExitCode {
    match run() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(error) => {
            eprintln!("editor setup check failed: {error}");
            ExitCode::FAILURE
        }
    }
}

struct Options {
    backend: PathBuf,
    repo: PathBuf,
    work: PathBuf,
    editors: Vec<(&'static str, PathBuf)>,
    routes: Vec<String>,
    vim_plug: Option<PathBuf>,
    lazy: Option<PathBuf>,
}

fn options() -> Result<Options> {
    let mut values = BTreeMap::new();
    let mut args = std::env::args_os().skip(1);
    while let Some(key) = args.next() {
        if key == "--help" || key == "-h" {
            println!(
                "check-editor-setup --backend EXE --work NEW_DIR [--vim EXE] [--nvim EXE] [--emacs EXE] [--vim-plug DIR] [--lazy DIR] [--route ROUTE]... [--repo DIR]"
            );
            std::process::exit(0);
        }
        let key = key.to_string_lossy().into_owned();
        let value = args.next().ok_or("missing option value")?;
        values.entry(key).or_insert_with(Vec::new).push(value);
    }
    let one = |name: &str| {
        values
            .get(name)
            .and_then(|all| all.last())
            .map(PathBuf::from)
    };
    let mut editors = Vec::new();
    for name in ["vim", "nvim", "emacs"] {
        if let Some(path) = one(&format!("--{name}")) {
            editors.push((name, std::path::absolute(path)?));
        }
    }
    if editors.is_empty() {
        return Err("name at least one of --vim, --nvim and --emacs".into());
    }
    let routes: Vec<String> = values.get("--route").map_or_else(
        || ROUTES.iter().map(|route| (*route).to_owned()).collect(),
        |all| {
            all.iter()
                .map(|route| route.to_string_lossy().into_owned())
                .collect()
        },
    );
    if let Some(route) = routes
        .iter()
        .find(|route| !ROUTES.contains(&route.as_str()))
    {
        return Err(format!("unknown route: {route}").into());
    }
    Ok(Options {
        backend: std::path::absolute(one("--backend").ok_or("--backend is required")?)?,
        repo: std::path::absolute(
            one("--repo").unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR"))),
        )?,
        work: std::path::absolute(one("--work").ok_or("--work is required")?)?,
        editors,
        routes,
        vim_plug: one("--vim-plug").map(std::path::absolute).transpose()?,
        lazy: one("--lazy").map(std::path::absolute).transpose()?,
    })
}

fn run() -> Result<bool> {
    let options = options()?;
    if options.work.exists() {
        return Err("the work directory must be new".into());
    }
    fs::create_dir_all(&options.work)?;
    let version = reported_version(&options.backend).ok_or("unexpected --version output")?;
    let (archive_name, archive) = development_archive(&options, &version)?;
    let mirror = mirror::Mirror::start(&version)?;
    mirror.archive(&version, &archive_name, &archive, true);
    let mut runs = Vec::new();
    let mut passed = true;
    let mut repository = None;
    for (editor, executable) in &options.editors {
        for route in &options.routes {
            let label = format!("{editor}-{route}");
            if let Some(reason) = skipped(&options, editor, executable, route) {
                println!("SKIP {label}: {reason}");
                runs.push(json!({"editor": editor, "route": route, "skipped": reason}));
                continue;
            }
            if MANAGERS.contains(&route.as_str()) && repository.is_none() {
                repository = Some(git_repository(&options)?);
            }
            let check = Route {
                options: &options,
                editor,
                executable,
                route,
                mirror: &mirror.url,
                version: &version,
                url: repository.as_deref().unwrap_or_default(),
            };
            let result = check
                .run()
                .unwrap_or_else(|error| json!({"passed": false, "error": error.to_string()}));
            passed &= result["passed"] == true;
            println!(
                "{} {label}",
                if result["passed"] == true {
                    "PASS"
                } else {
                    "FAIL"
                }
            );
            runs.push(json!({"editor": editor, "route": route, "result": result}));
            capture::save(
                &options.work.join("receipt.json"),
                &json!({"passed": passed, "version": version, "backend": options.backend,
                    "backend_sha256": capture::digest(&options.backend)?, "archive": archive_name,
                    "archive_sha256": hash(&archive), "repo": options.repo, "runs": runs}),
            )?;
        }
    }
    println!(
        "{}; evidence: {}",
        if passed { "PASS" } else { "FAIL" },
        options.work.join("receipt.json").display()
    );
    Ok(passed)
}

fn hash(bytes: &[u8]) -> String {
    base16ct::lower::encode_string(&Sha256::digest(bytes))
}

fn reported_version(executable: &Path) -> Option<String> {
    let output = Command::new(executable).arg("--version").output().ok()?;
    String::from_utf8(output.stdout)
        .ok()?
        .trim()
        .strip_prefix("ripscope ")
        .map(str::to_owned)
}

/// Why ROUTE does not apply to EDITOR here, if it does not.
fn skipped(options: &Options, editor: &str, executable: &Path, route: &str) -> Option<String> {
    let only = |name: &str| (editor != name).then(|| format!("{route} is for {name} only"));
    match route {
        "vim-plug" => only("vim").or_else(|| {
            options
                .vim_plug
                .is_none()
                .then(|| "no --vim-plug checkout given".to_owned())
        }),
        "lazy" | "installer-lazy" => only("nvim").or_else(|| {
            options
                .lazy
                .is_none()
                .then(|| "no --lazy checkout given".to_owned())
        }),
        "spaces" if editor == "vim" && !WINDOWS => {
            Some("Vim's cscope support takes no spaces in paths on Linux".to_owned())
        }
        "use-package" | "package-vc" => only("emacs").or_else(|| {
            let version = Command::new(executable)
                .args(["-Q", "--batch", "--eval", "(princ emacs-major-version)"])
                .output()
                .ok()
                .and_then(|output| String::from_utf8(output.stdout).ok())
                .and_then(|text| text.trim().parse::<u32>().ok())
                .unwrap_or(0);
            let needed = if route == "use-package" { 30 } else { 29 };
            (version < needed).then(|| format!("{route} needs Emacs {needed} or newer"))
        }),
        _ => None,
    }
}

fn copy_tree(from: &Path, to: &Path) -> Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

/// Every regular file below ROOT, by forward-slash relative path, with its SHA-256.
fn inventory(root: &Path) -> Result<BTreeMap<String, String>> {
    fn walk(root: &Path, directory: &Path, files: &mut BTreeMap<String, String>) -> Result<()> {
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                walk(root, &entry.path(), files)?;
            } else {
                let relative = entry
                    .path()
                    .strip_prefix(root)?
                    .to_string_lossy()
                    .into_owned();
                files.insert(relative.replace('\\', "/"), capture::digest(&entry.path())?);
            }
        }
        Ok(())
    }
    let mut files = BTreeMap::new();
    if root.exists() {
        walk(root, root, &mut files)?;
    }
    Ok(files)
}

/// The editor files of this checkout as the archive's `contrib/vim` holds them.
fn plugin_inventory(repo: &Path) -> Result<BTreeMap<String, String>> {
    let mut files = BTreeMap::new();
    for directory in ["plugin", "lua"] {
        files.extend(
            inventory(&repo.join(directory))?
                .into_iter()
                .map(|(file, digest)| (format!("{directory}/{file}"), digest)),
        );
    }
    Ok(files)
}

/// An archive laid out as a release archive is, around the given executable and this
/// checkout's editor files, with placeholder notices: enough for the installers.
fn development_archive(options: &Options, version: &str) -> Result<(String, Vec<u8>)> {
    let target = if WINDOWS {
        "x86_64-windows-msvc"
    } else {
        "x86_64-linux-gnu"
    };
    let name = format!("ripscope-{version}-{target}");
    let root = options.work.join("archive");
    let package = root.join(&name);
    let executable = if WINDOWS { "ripscope.exe" } else { "ripscope" };
    fs::create_dir_all(package.join("bin"))?;
    fs::copy(&options.backend, package.join("bin").join(executable))?;
    for file in ["LICENSE-MIT", "LICENSE-APACHE", "README.md"] {
        fs::copy(options.repo.join(file), package.join(file))?;
    }
    copy_tree(
        &options.repo.join("contrib/emacs"),
        &package.join("contrib/emacs"),
    )?;
    for directory in ["plugin", "lua"] {
        copy_tree(
            &options.repo.join(directory),
            &package.join("contrib/vim").join(directory),
        )?;
    }
    let placeholder = "Development archive for the editor setup check.\n";
    for file in [
        "licenses/SQLITE-NOTICE.txt",
        "licenses/rust/COPYRIGHT-library.html",
        "licenses/rust/LICENSE.txt",
        "licenses/dependencies/placeholder/LICENSE",
    ] {
        let path = package.join(file);
        fs::create_dir_all(path.parent().ok_or("no parent")?)?;
        fs::write(path, placeholder)?;
    }
    fs::write(package.join("DEPENDENCIES.json"), "[]\n")?;
    let binary = capture::digest(&package.join("bin").join(executable))?;
    fs::write(
        package.join("BUILD.json"),
        format!("{{\n  \"binary_sha256\": \"{binary}\",\n  \"development\": true\n}}\n"),
    )?;
    let manifest = inventory(&package)?
        .into_iter()
        .map(|(file, digest)| format!("{digest}  {file}\n"))
        .collect::<String>();
    fs::write(package.join("MANIFEST.sha256"), manifest)?;
    let dist = options.work.join("dist");
    fs::create_dir_all(&dist)?;
    let (file, status) = if WINDOWS {
        let file = format!("{name}.zip");
        // Entry by entry: Windows PowerShell's CreateFromDirectory names them with
        // backslashes, which the installer rightly refuses.
        let script = format!(
            "$ErrorActionPreference = 'Stop'; Add-Type -AssemblyName System.IO.Compression.FileSystem; \
             $root = '{}'; $zip = [IO.Compression.ZipFile]::Open('{}', 'Create'); \
             try {{ Get-ChildItem -LiteralPath (Join-Path $root '{name}') -Recurse -File -Force | ForEach-Object {{ \
             $entry = $_.FullName.Substring($root.Length + 1).Replace('\\', '/'); \
             [void][IO.Compression.ZipFileExtensions]::CreateEntryFromFile($zip, $_.FullName, $entry) }} }} \
             finally {{ $zip.Dispose() }}",
            root.display().to_string().replace('\'', "''"),
            dist.join(&file).display().to_string().replace('\'', "''")
        );
        let status = Command::new(powershell())
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .status()?;
        (file, status)
    } else {
        let file = format!("{name}.tar.gz");
        let status = Command::new("tar")
            .arg("-czf")
            .arg(dist.join(&file))
            .arg("-C")
            .arg(&root)
            .arg(&name)
            .status()?;
        (file, status)
    };
    if !status.success() {
        return Err("could not pack the development archive".into());
    }
    let bytes = fs::read(dist.join(&file))?;
    Ok((file, bytes))
}

/// A Git repository of this checkout's editor files, as the public repository holds
/// them, with a `release` branch the plugin manager lines follow and a later commit on
/// `main` they must not see: its `file:` URL.
fn git_repository(options: &Options) -> Result<String> {
    let repository = options.work.join("repository");
    for file in [
        "install.sh",
        "install.ps1",
        "README.md",
        "LICENSE-MIT",
        "LICENSE-APACHE",
        "Cargo.toml",
    ] {
        fs::create_dir_all(&repository)?;
        fs::copy(options.repo.join(file), repository.join(file))?;
    }
    for directory in ["plugin", "lua", "contrib", "tests/editor"] {
        copy_tree(&options.repo.join(directory), &repository.join(directory))?;
    }
    let git = |args: &[&str]| -> Result<()> {
        let status = Command::new("git")
            .args([
                "-c",
                "user.name=Ripscope check",
                "-c",
                "user.email=check@example.invalid",
            ])
            .args(args)
            .current_dir(&repository)
            .status()?;
        if !status.success() {
            return Err(format!("git {} failed", args.join(" ")).into());
        }
        Ok(())
    };
    git(&["init", "-q", "-b", "main"])?;
    git(&["add", "-A"])?;
    git(&["commit", "-q", "-m", "Editor setup check"])?;
    git(&["branch", "release"])?;
    fs::write(repository.join("main-only.txt"), "not released\n")?;
    git(&["add", "main-only.txt"])?;
    git(&["commit", "-q", "-m", "Unreleased"])?;
    let path = slashes(&repository);
    Ok(if path.starts_with('/') {
        format!("file://{path}")
    } else {
        format!("file:///{path}")
    })
}

/// TEXT quoted for Lua and Emacs Lisp alike.
fn quoted(text: &str) -> Result<String> {
    Ok(serde_json::to_string(text)?)
}

/// Runs COMMAND to completion within SECONDS, keeping its output under LABEL.
fn execute(directory: &Path, label: &str, command: Command, seconds: u64) -> Result<Value> {
    capture::run_command(command, &directory.join(label), seconds, &[]).map_err(Into::into)
}

/// Whether a capture exited with status 0 and left no process behind. Where no user
/// manager can prove the cleanup (an SSH login, say), its process group must at least be
/// empty: an editor closed its sessions.
fn finished(capture: &Value) -> bool {
    capture["status"] == 0
        && (capture["cleanup"]["state"] == "confirmed"
            || capture["cleanup"]["live_group_members"]
                .as_array()
                .is_some_and(Vec::is_empty))
}

fn text(path: &Value) -> String {
    path.as_str()
        .and_then(|path| fs::read(path).ok())
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .unwrap_or_default()
}

fn slashes(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn same_file(reported: &Value, expected: &Path) -> bool {
    let reported = reported.as_str().unwrap_or_default();
    let expected = slashes(expected);
    if WINDOWS {
        reported.eq_ignore_ascii_case(&expected)
    } else {
        reported == expected
    }
}

/// One editor set up by one route, in a profile of its own.
struct Route<'a> {
    options: &'a Options,
    editor: &'a str,
    executable: &'a Path,
    route: &'a str,
    mirror: &'a str,
    version: &'a str,
    url: &'a str,
}

impl Route<'_> {
    fn run(&self) -> Result<Value> {
        let options = self.options;
        let directory = options.work.join(format!("{}-{}", self.editor, self.route));
        let mut profile = Profile::new(
            directory.join(if self.route == "spaces" {
                "profile with spaces"
            } else {
                "profile"
            }),
            None,
        )?;
        // ripscope on PATH for the routes that do not install it.
        let on_path = profile.root.join("bin");
        if ["path", "interrupted", "home", "spaces"].contains(&self.route) {
            fs::create_dir_all(&on_path)?;
            fs::copy(
                &options.backend,
                on_path.join(if WINDOWS { "ripscope.exe" } else { "ripscope" }),
            )?;
            let mut path = vec![on_path.clone()];
            path.extend(std::env::split_paths(
                &profile
                    .env
                    .iter()
                    .find(|(name, _)| name == "PATH")
                    .map(|(_, value)| value.clone())
                    .unwrap_or_default(),
            ));
            profile.set("PATH", std::env::join_paths(path)?);
        }
        if self.route == "path" && self.editor == "vim" && !WINDOWS {
            // A shell whose startup files print, as Vim runs its commands through it.
            let rc = profile.root.join("noisy-shell-rc");
            fs::write(&rc, "echo 'shell startup noise'\n")?;
            profile.set("SHELL", "/bin/bash");
            profile.set("BASH_ENV", &rc);
        }
        if self.vim_spaces() {
            // Vim takes a path with spaces by its short 8.3 name, which it expands again
            // for an index, so its cache is where the documented variable puts it.
            profile.set("RIPSCOPE_CACHE_DIR", self.cache(&profile));
        }
        profile.set("RIPSCOPE_RELEASES", self.mirror);
        profile.set("RIPSCOPE_VERSION", self.version);
        let mut steps = Vec::new();
        let mut passed = true;
        let emacs_init = self.prepare(&profile, &directory, &mut steps, &mut passed)?;
        let decoy = if self.route == "installer" && self.editor == "emacs" {
            Some(decoy_checkout(&profile, &directory)?)
        } else {
            None
        };
        let projects = match self.route {
            "home" => profile.home.join("projects"),
            "spaces" => profile.root.join("projects"),
            _ => directory.join("projects"),
        };
        for kind in ["listed", "git", "plain"] {
            let project = Project::create(&projects.join(kind), kind)?;
            let result = self.search(&profile, &directory, &project, &emacs_init)?;
            passed &= result["passed"] == true;
            steps.push(json!({"step": format!("search-{kind}"), "result": result}));
        }
        if !["path", "interrupted", "home", "spaces"].contains(&self.route) {
            // The executable the route installed, of this checkout's version.
            let installed = profile.installed_executable();
            let version = reported_version(&installed);
            let current = version.as_deref() == Some(self.version);
            passed &= current;
            steps.push(json!({"step": "installed-executable", "path": installed,
                "version": version, "current": current}));
        }
        if MANAGERS.contains(&self.route) {
            // The plugin manager followed the release branch.
            let checkouts = [
                profile.home.join(".vim/plugged/repository"),
                profile.home.join("vimfiles/plugged/repository"),
                profile.neovim_data().join("lazy/ripscope"),
                profile.home.join(".emacs.d/elpa/ripscope"),
            ];
            let checkout = checkouts.iter().find(|path| path.is_dir());
            let released = checkout.is_some_and(|path| !path.join("main-only.txt").exists());
            passed &= released;
            steps.push(json!({"step": "release-branch", "checkout": checkout,
                "released": released}));
        }
        if let Some(marker) = &decoy {
            // A copy the installer placed runs no installer that happens to sit nearby.
            let ran = marker.exists();
            passed &= !ran;
            steps.push(json!({"step": "decoy-installer", "marker": marker, "ran": ran}));
        }
        if self.route == "installer" {
            let (program, args) = installer(options, &["uninstall"]);
            let mut command = profile.command(&program);
            command.args(args);
            let result = execute(&directory, "uninstall", command, 60)?;
            let mut remaining = profile
                .editor_directories()
                .into_iter()
                .map(|(_, path)| path)
                .filter(|path| path.exists())
                .collect::<Vec<_>>();
            if profile.installed_executable().exists() {
                remaining.push(profile.installed_executable());
            }
            let cache_kept = profile.cache().is_dir();
            passed &= finished(&result) && remaining.is_empty() && cache_kept;
            steps.push(
                json!({"step": "uninstall", "capture": result, "remaining": remaining,
                "cache_kept": cache_kept}),
            );
        }
        Ok(json!({"passed": passed, "steps": steps}))
    }

    /// Sets the editor up by this route, returning the init forms Emacs evaluates.
    fn prepare(
        &self,
        profile: &Profile,
        directory: &Path,
        steps: &mut Vec<Value>,
        passed: &mut bool,
    ) -> Result<String> {
        let options = self.options;
        let editor = self.editor;
        let url = self.url;
        match self.route {
            "installer" | "installer-lazy" => {
                if self.route == "installer-lazy" {
                    // As lazy.nvim's own setup leaves it, before the installer runs.
                    let lazy = options.lazy.as_ref().ok_or("no lazy.nvim")?;
                    copy_tree(lazy, &profile.neovim_data().join("lazy/lazy.nvim"))?;
                }
                let (program, args) = installer(options, &[]);
                let mut command = profile.command(&program);
                command.args(args);
                let result = execute(directory, "install", command, 120)?;
                let output = text(&result["stdout_path"]);
                let line = |start: &str| {
                    output
                        .lines()
                        .map(str::trim)
                        .find(|line| line.starts_with(start))
                        .unwrap_or_default()
                        .to_owned()
                };
                let emacs = line("(when (load ");
                let lazy = line("{ dir = ");
                let placed = placed_editor_files(options, profile)?;
                *passed &= finished(&result) && !emacs.is_empty() && placed["matches"] == true;
                if self.route == "installer-lazy" {
                    *passed &= !lazy.is_empty();
                    fs::create_dir_all(profile.neovim_config())?;
                    fs::write(
                        profile.neovim_config().join("init.lua"),
                        format!(
                            "vim.opt.rtp:prepend(vim.fn.stdpath('data') .. '/lazy/lazy.nvim')\nrequire('lazy').setup({{\n  {lazy}\n}})\n"
                        ),
                    )?;
                }
                steps.push(
                    json!({"step": "install", "capture": result, "emacs_line": emacs,
                    "lazy_line": lazy, "placed": placed}),
                );
                Ok(emacs)
            }
            "vim-plug" => {
                let vim = profile.home.join(if WINDOWS { "vimfiles" } else { ".vim" });
                let source = options.vim_plug.as_ref().ok_or("no vim-plug")?;
                let plug = if source.is_dir() {
                    source.join("plug.vim")
                } else {
                    source.clone()
                };
                fs::create_dir_all(vim.join("autoload"))?;
                fs::copy(plug, vim.join("autoload/plug.vim"))?;
                // The documented line, with this checkout's repository for the public one.
                fs::write(
                    profile.home.join(if WINDOWS { "_vimrc" } else { ".vimrc" }),
                    format!(
                        "set nocompatible\ncall plug#begin()\nPlug '{url}', {{ 'branch': 'release', 'do': ':RipscopeInstall' }}\ncall plug#end()\n"
                    ),
                )?;
                let mut command = self.editor_command(profile)?;
                command.args(["-c", "PlugInstall --sync", "-c", "qa!"]);
                let result = execute(directory, "plug-install", command, 300)?;
                *passed &= finished(&result);
                steps.push(json!({"step": "plug-install", "capture": result}));
                Ok(String::new())
            }
            "lazy" => {
                let lazy = options.lazy.as_ref().ok_or("no lazy.nvim")?;
                fs::create_dir_all(profile.neovim_config())?;
                // The documented spec, with this checkout's repository for the public one.
                fs::write(
                    profile.neovim_config().join("init.lua"),
                    format!(
                        "vim.opt.rtp:prepend({})\nrequire('lazy').setup({{ {{ url = {}, name = 'ripscope', branch = 'release', build = ':RipscopeInstall' }} }})\n",
                        quoted(&slashes(lazy))?,
                        quoted(url)?
                    ),
                )?;
                let mut command = self.editor_command(profile)?;
                command.args(["-c", "Lazy! sync", "-c", "qa"]);
                let result = execute(directory, "lazy-sync", command, 300)?;
                *passed &= finished(&result);
                steps.push(json!({"step": "lazy-sync", "capture": result}));
                Ok(String::new())
            }
            // As at Emacs's startup, without package archives so that the check stays
            // local, then the documented line.
            "use-package" => Ok(format!(
                "(setq package-archives nil) (package-initialize) (use-package ripscope :vc (:url {} :branch \"release\" :lisp-dir \"contrib/emacs\" :rev :newest) :config (ripscope-setup))",
                quoted(url)?
            )),
            "package-vc" => Ok(format!(
                "(setq package-archives nil) (package-initialize) (unless (package-installed-p 'ripscope) (package-vc-install '(ripscope :url {} :branch \"release\" :lisp-dir \"contrib/emacs\"))) (ripscope-setup)",
                quoted(url)?
            )),
            route => {
                // A repository copy, as a plugin manager clones it.
                let clone = directory.join("clone");
                for file in ["install.sh", "install.ps1", "Cargo.toml"] {
                    fs::create_dir_all(&clone)?;
                    fs::copy(options.repo.join(file), clone.join(file))?;
                }
                for name in ["plugin", "lua"] {
                    copy_tree(&options.repo.join(name), &clone.join(name))?;
                }
                copy_tree(
                    &options.repo.join("contrib/emacs"),
                    &clone.join("contrib/emacs"),
                )?;
                let packages = match editor {
                    "vim" => Some(profile.home.join(if WINDOWS { "vimfiles" } else { ".vim" })),
                    "nvim" => Some(profile.neovim_data().join("site")),
                    _ => None,
                };
                if let Some(packages) = packages {
                    copy_tree(&clone, &packages.join("pack/plugins/start/ripscope"))?;
                }
                if route == "plugin" {
                    #[cfg(unix)]
                    {
                        // An older executable the installer put there, which the build
                        // hook upgrades.
                        use std::os::unix::fs::PermissionsExt;
                        let old = profile.installed_executable();
                        fs::create_dir_all(old.parent().ok_or("no parent")?)?;
                        fs::write(&old, "#!/bin/sh\necho 'ripscope 0.0.1'\n")?;
                        fs::set_permissions(&old, fs::Permissions::from_mode(0o755))?;
                    }
                    if editor != "emacs" {
                        // The build hook: lazy.nvim's build and vim-plug's do run this.
                        let mut command = self.editor_command(profile)?;
                        command.args(["-c", "RipscopeInstall", "-c", "qa!"]);
                        let result = execute(directory, "build-hook", command, 120)?;
                        *passed &= finished(&result);
                        steps.push(json!({"step": "build-hook", "capture": result}));
                    }
                }
                Ok(format!(
                    "(add-to-list 'load-path {}) (require 'ripscope) (ripscope-setup)",
                    quoted(&clone.join("contrib/emacs").to_string_lossy())?
                ))
            }
        }
    }

    fn vim_spaces(&self) -> bool {
        self.route == "spaces" && self.editor == "vim"
    }

    /// Where this route's editor keeps its indexes.
    fn cache(&self, profile: &Profile) -> PathBuf {
        if self.vim_spaces() {
            self.options.work.join("vim-spaces-cache")
        } else {
            profile.cache()
        }
    }

    fn editor_command(&self, profile: &Profile) -> Result<Command> {
        let mut command = profile.command(self.executable);
        match self.editor {
            "vim" => {
                // As a user's Vim starts, with a vimrc and the packages it loads.
                let vimrc = profile.home.join(if WINDOWS { "_vimrc" } else { ".vimrc" });
                if !vimrc.exists() {
                    fs::write(&vimrc, "set nocompatible\n")?;
                }
                command
                    .args(["-N", "-n", "-i", "NONE", "-es", "-u"])
                    .arg(vimrc);
            }
            "nvim" => {
                command.args(["--headless", "-i", "NONE"]);
            }
            _ => {
                command.args(["-Q", "--batch"]);
            }
        }
        Ok(command)
    }

    /// The directory the editor starts in for PROJECT.
    fn cwd(&self, profile: &Profile, project: &Project) -> PathBuf {
        if self.route == "home" {
            profile.home.clone()
        } else if project.kind == "plain" {
            project.root.clone()
        } else {
            profile.home.clone()
        }
    }

    /// The project `ripscope --project` reports for PROJECT's file, from CWD.
    fn reported(&self, profile: &Profile, project: &Project, cwd: &Path) -> Result<Value> {
        let mut command = Command::new(&self.options.backend);
        profile.apply(&mut command);
        let output = command
            .current_dir(cwd)
            .arg("--project")
            .arg(&project.file)
            .output()?;
        if !output.status.success() {
            return Err(String::from_utf8_lossy(&output.stderr).into_owned().into());
        }
        Ok(serde_json::from_slice(&output.stdout)?)
    }

    /// A first index build that never published, of the wrong sources: the opened file
    /// alone.
    fn interrupt(&self, reported: &Value, project: &Project) -> Result<()> {
        let index = reported["index"].as_str().ok_or("no index")?;
        let status = Command::new(&self.options.backend)
            .current_dir(&project.root)
            .args(["-bk", "-f", index, "--"])
            .arg(&project.file)
            .status()?;
        if !status.success() {
            return Err("could not prepare the interrupted index".into());
        }
        rusqlite::Connection::open(index)?.execute_batch(
            "UPDATE meta SET value = 0 WHERE key = 'revision';
             DELETE FROM files; DELETE FROM names; DELETE FROM units;",
        )?;
        Ok(())
    }

    fn search(
        &self,
        profile: &Profile,
        directory: &Path,
        project: &Project,
        emacs_init: &str,
    ) -> Result<Value> {
        let label = format!("{}-{}", self.editor, project.kind);
        let report = directory.join(format!("{}-report.json", project.kind));
        let cwd = self.cwd(profile, project);
        let expected = self.reported(profile, project, &cwd)?;
        if self.route == "interrupted" {
            self.interrupt(&expected, project)?;
        }
        let before = inventory(&project.root)?;
        let script = self
            .options
            .repo
            .join("tests/editor")
            .join(match self.editor {
                "vim" => "setup.vim",
                "nvim" => "setup.lua",
                _ => "setup.el",
            });
        let mut command = self.editor_command(profile)?;
        match self.editor {
            "vim" => command.arg("-S").arg(&script),
            "nvim" => command.arg("-c").arg(format!(
                "lua dofile({})",
                quoted(&script.to_string_lossy())?
            )),
            _ => command.arg("-l").arg(&script),
        };
        command
            .current_dir(&cwd)
            .env("RIPSCOPE_SETUP_FILE", &project.file)
            .env("RIPSCOPE_SETUP_REPORT", &report)
            .env("RIPSCOPE_SETUP_INIT", emacs_init)
            .env(
                "RIPSCOPE_SETUP_ABSENT",
                if project.kind == "listed" {
                    "ignored_symbol"
                } else {
                    ""
                },
            );
        let capture = execute(directory, &label, command, 180)?;
        let observed: Value = fs::read(&report)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or(Value::Null);
        let searches = observed["searches"].as_array().cloned().unwrap_or_default();
        let found = |pattern: &str| {
            searches
                .iter()
                .find(|search| search["pattern"] == pattern)
                .cloned()
                .unwrap_or(Value::Null)
        };
        let greet = found("greet");
        let fresh = found("fresh_symbol");
        let rows = |search: &Value| search["rows"].as_array().map_or(0, Vec::len);
        let mut checks = BTreeMap::new();
        checks.insert("editor_exit", finished(&capture));
        checks.insert(
            "no_errors",
            observed["errors"].as_array().is_some_and(Vec::is_empty),
        );
        checks.insert("project_root", same_file(&expected["root"], &project.root));
        checks.insert(
            "definition_row",
            rows(&greet) == 1
                && same_file(&greet["rows"][0]["file"], &project.definition)
                && greet["rows"][0]["line"] == 1,
        );
        checks.insert(
            "definition_jump",
            same_file(&greet["jump"]["file"], &project.definition) && greet["jump"]["line"] == 1,
        );
        if project.kind == "listed" {
            checks.insert("file_list_used", rows(&found("ignored_symbol")) == 0);
        } else if self.editor == "emacs" {
            // The first start checked the executable: later ones run no installer.
            checks.insert(
                "no_installer_after_first_start",
                observed["installer_started"] == false,
            );
        }
        if self.editor == "nvim" {
            // Searched from the quickfix window, where no file names the project.
            let main = found("main");
            checks.insert(
                "search_from_results",
                rows(&main) == 1
                    && same_file(&main["rows"][0]["file"], &project.file)
                    && main["rows"][0]["line"] == 2,
            );
        }
        checks.insert(
            "saved_edit_found",
            rows(&fresh) == 1
                && same_file(&fresh["rows"][0]["file"], &project.file)
                && fresh["rows"][0]["line"] == 3
                && same_file(&fresh["jump"]["file"], &project.file)
                && fresh["jump"]["text"] == FRESH,
        );
        // Nothing changed in the project but the saved edit: no index, list or marker.
        let after = inventory(&project.root)?;
        let main = slashes(project.file.strip_prefix(&project.root)?);
        let mut unchanged = before.clone();
        let edited =
            format!("#include \"greet.h\"\nint main(void) {{ return greet(); }}\n{FRESH}\n");
        let main_edited = [edited.clone(), edited.replace('\n', "\r\n")]
            .iter()
            .any(|text| after.get(&main) == Some(&hash(text.as_bytes())));
        unchanged.insert(main.clone(), after.get(&main).cloned().unwrap_or_default());
        checks.insert("project_unchanged", after == unchanged && main_edited);
        // The index `ripscope --project` names, published by the search.
        let index = expected["index"].as_str().unwrap_or_default();
        let indexed = self.reported(profile, project, &cwd)?;
        let cache = self.cache(profile);
        checks.insert(
            "index_in_cache",
            Path::new(index).starts_with(&cache) || {
                let cache = slashes(&cache);
                if WINDOWS {
                    index.to_lowercase().starts_with(&cache.to_lowercase())
                } else {
                    index.starts_with(&cache)
                }
            },
        );
        checks.insert("index_published", indexed["published"] == true);
        let passed = checks.values().all(|ok| *ok);
        Ok(
            json!({"passed": passed, "checks": checks, "project": project.root, "cwd": cwd,
            "reported": expected, "capture": capture, "report": observed,
            "files_before": before, "files_after": after}),
        )
    }
}

/// A decoy checkout around the Emacs copy the installer placed, two directories up as a
/// checkout's `contrib/emacs` is, whose installers leave the marker it returns.
fn decoy_checkout(profile: &Profile, directory: &Path) -> Result<PathBuf> {
    let emacs = profile
        .editor_directories()
        .into_iter()
        .find(|(source, _)| *source == "contrib/emacs")
        .map(|(_, path)| path)
        .ok_or("no Emacs directory")?;
    let root = emacs
        .parent()
        .and_then(Path::parent)
        .ok_or("no decoy root")?;
    let marker = directory.join("decoy-installer-ran");
    let shown = marker.to_string_lossy().replace('\'', "''");
    fs::create_dir_all(root.join("contrib/emacs"))?;
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"decoy\"\nversion = \"9.9.9\"\n",
    )?;
    fs::write(root.join("install.sh"), format!("touch '{shown}'\n"))?;
    fs::write(
        root.join("install.ps1"),
        format!("New-Item -ItemType File -Force -Path '{shown}' | Out-Null\n"),
    )?;
    Ok(marker)
}

/// The installer's command line, with FLAGS such as `uninstall`.
fn installer(options: &Options, flags: &[&str]) -> (PathBuf, Vec<OsString>) {
    if WINDOWS {
        let mut args: Vec<OsString> = [
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ]
        .map(OsString::from)
        .to_vec();
        args.push(options.repo.join("install.ps1").into());
        args.extend(flags.iter().map(|flag| OsString::from(format!("-{flag}"))));
        (powershell(), args)
    } else {
        let mut args = vec![OsString::from(options.repo.join("install.sh"))];
        args.extend(flags.iter().map(|flag| OsString::from(format!("--{flag}"))));
        (PathBuf::from("sh"), args)
    }
}

/// Whether the installer placed exactly this checkout's editor files.
fn placed_editor_files(options: &Options, profile: &Profile) -> Result<Value> {
    let vim = plugin_inventory(&options.repo)?;
    let emacs = inventory(&options.repo.join("contrib/emacs"))?;
    let mut matches = true;
    let mut placed = Vec::new();
    for (source, directory) in profile.editor_directories() {
        let (expected, directory) = if source == "contrib/vim" {
            (&vim, directory.join("start/ripscope"))
        } else {
            (&emacs, directory)
        };
        let actual = inventory(&directory)?;
        matches &= &actual == expected;
        placed.push(
            json!({"directory": directory, "files": actual.len(), "matches": &actual == expected}),
        );
    }
    Ok(json!({"matches": matches, "directories": placed}))
}

/// A sample project of one KIND: `listed` names its sources in cscope.files, `git` is a
/// Git repository discovered recursively, and `plain` has neither, so the editor's
/// current directory, or the file's own directory, is the project.
struct Project {
    kind: &'static str,
    root: PathBuf,
    file: PathBuf,
    definition: PathBuf,
}

impl Project {
    fn create(root: &Path, kind: &'static str) -> Result<Self> {
        let write = |relative: &str, text: &str| -> Result<()> {
            let path = root.join(relative);
            fs::create_dir_all(path.parent().ok_or("no parent")?)?;
            fs::write(path, text)?;
            Ok(())
        };
        let (file, definition) = match kind {
            "listed" => {
                write("cscope.files", "src/main.c\nlib/greet.c\ninclude/greet.h\n")?;
                write(
                    "extra/ignored.c",
                    "int ignored_symbol(void) { return 2; }\n",
                )?;
                ("src/main.c", "lib/greet.c")
            }
            "git" => {
                write(".git/HEAD", "ref: refs/heads/main\n")?;
                ("src/main.c", "lib/deep/greet.c")
            }
            _ => ("main.c", "sub/greet.c"),
        };
        write(
            file,
            "#include \"greet.h\"\nint main(void) { return greet(); }\n",
        )?;
        write("include/greet.h", "int greet(void);\n")?;
        write(definition, "int greet(void) { return 1; }\n")?;
        Ok(Self {
            kind,
            root: root.to_owned(),
            file: root.join(file),
            definition: root.join(definition),
        })
    }
}
