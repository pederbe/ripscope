//! Exercise exact Windows archive bytes with a minimal runtime environment.

use crate::archive::{entry_path, zip_entries, zip_read};
use crate::common::{
    Ended, Result, capture_command, command_failure, inventory, json_text, run_logged, sha,
    write_text,
};
use serde_json::{Map, Value, json};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

const USAGE: &str = "ripscope-release windows-smoke ARCHIVE.zip NEW_WORK_DIR";

/// Extracts `archive` under `install` after checking every entry, and checks the
/// unpacked files against the archive's manifest.
pub fn unpack(archive: &Path, install: &Path) -> Result<PathBuf> {
    let data = fs::read(archive)?;
    let entries = zip_entries(&data)?;
    let mut targets = Vec::new();
    for entry in &entries {
        let target =
            entry_path(install, &entry.name).ok_or("Archive entry escapes installation")?;
        if ![0, 0o100000, 0o040000].contains(&((entry.external >> 16) & 0o170000)) {
            return Err("Archive contains a linked or special entry".into());
        }
        targets.push(target);
    }
    for (entry, target) in entries.iter().zip(&targets) {
        if entry.name.ends_with('/') {
            fs::create_dir_all(target)?;
            continue;
        }
        fs::create_dir_all(target.parent().ok_or("entry without a parent")?)?;
        fs::write(target, zip_read(&data, entry)?)?;
    }
    let mut packages = fs::read_dir(install)?.collect::<std::io::Result<Vec<_>>>()?;
    if packages.len() != 1 {
        return Err("The archive must hold exactly one package directory".into());
    }
    let package = packages.remove(0).path();
    let manifest = fs::read_to_string(package.join("MANIFEST.sha256"))?.replace("\r\n", "\n");
    let mut expected = Map::new();
    for line in manifest.lines() {
        let (digest, name) = line.split_once("  ").ok_or("malformed manifest line")?;
        expected.insert(name.to_owned(), json!(digest));
    }
    let mut actual = inventory(&package)?;
    actual.remove("MANIFEST.sha256");
    if actual != expected {
        return Err("Unpacked file inventory/checksum mismatch".into());
    }
    let emacs = package.join("contrib/emacs/ripscope.el");
    if !emacs.is_file() || fs::metadata(&emacs)?.len() == 0 {
        return Err("Missing the Emacs package".into());
    }
    Ok(package)
}

/// Whether `tool` resolves in the directories of `path`, with Windows' executable
/// extensions.
fn visible(tool: &str, path: &str) -> bool {
    let extensions = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_owned());
    path.split(';')
        .filter(|directory| !directory.is_empty())
        .any(|directory| {
            let directory = Path::new(directory);
            directory.join(tool).is_file()
                || extensions
                    .split(';')
                    .filter(|extension| !extension.is_empty())
                    .any(|extension| directory.join(format!("{tool}{extension}")).is_file())
        })
}

struct Smoke {
    binary: PathBuf,
    env: Vec<(String, String)>,
    receipt: Map<String, Value>,
}

impl Smoke {
    fn command(&self, project: &Path, args: &[&str]) -> Command {
        let mut command = Command::new(&self.binary);
        command
            .args(args)
            .current_dir(project)
            .env_clear()
            .envs(self.env.iter().map(|(k, v)| (k, v)));
        command
    }

    fn record(&mut self, args: &[&str], exit: i32, stdout: &[u8], stderr: &[u8]) -> Value {
        let entry = json!({
            "args": args, "exit": exit,
            "stdout": String::from_utf8_lossy(stdout), "stderr": String::from_utf8_lossy(stderr),
        });
        if let Some(Value::Array(commands)) = self.receipt.get_mut("commands") {
            commands.push(entry.clone());
        }
        entry
    }

    /// Runs one command and checks its exit, output and warning.
    fn run(
        &mut self,
        project: &Path,
        args: &[&str],
        expected: Option<&[u8]>,
        warning: bool,
        data: Option<&[u8]>,
    ) -> Result<Vec<u8>> {
        let (exit, stdout, stderr) = capture_command(
            &mut self.command(project, args),
            data,
            Duration::from_secs(30),
        )?
        .ok_or_else(|| format!("smoke command exceeded 30 seconds: {args:?}"))?;
        let entry = self.record(args, exit, &stdout, &stderr);
        if exit != 0 || expected.is_some_and(|expected| stdout != expected) {
            return Err(format!("Unexpected smoke result: {entry}").into());
        }
        if warning {
            if !stderr.starts_with(b"ripscope: cannot read sample.c: ") {
                return Err("Missing live source warning".into());
            }
        } else if !stderr.is_empty() {
            return Err(format!("Unexpected warning: {entry}").into());
        }
        Ok(stdout)
    }
}

pub fn run(args: &[String]) -> Result<()> {
    let [archive, work] = args else {
        return Err(USAGE.into());
    };
    let (archive, work) = (std::path::absolute(archive)?, std::path::absolute(work)?);
    if work.exists() {
        return Err(format!("{} already exists", work.display()).into());
    }
    fs::create_dir_all(&work)?;
    let install = work.join("install");
    fs::create_dir(&install)?;
    let package = unpack(&archive, &install)?;
    let binary = package.join("bin").join("ripscope.exe");
    let system = PathBuf::from(std::env::var_os("SystemRoot").ok_or("SystemRoot is not set")?);
    let temporary = work.join("tmp");
    fs::create_dir(&temporary)?;
    let text = |path: &Path| path.to_string_lossy().into_owned();
    let env = vec![
        ("SystemRoot".to_owned(), text(&system)),
        ("WINDIR".to_owned(), text(&system)),
        ("PATH".to_owned(), text(&system.join("System32"))),
        ("TEMP".to_owned(), text(&temporary)),
        ("TMP".to_owned(), text(&temporary)),
        ("USERPROFILE".to_owned(), text(&work)),
    ];
    for tool in [
        "rustc",
        "cargo",
        "cl",
        "clang",
        "gcc",
        "sqlite3",
        "tree-sitter",
    ] {
        if visible(tool, &env[2].1) {
            return Err(format!("Development tool visible in runtime PATH: {tool}").into());
        }
    }
    let host = Command::new(system.join("System32").join("cmd.exe"))
        .args(["/d", "/c", "ver"])
        .output()?;
    let mut receipt = Map::new();
    receipt.insert("complete".into(), json!(false));
    receipt.insert("archive_sha256".into(), json!(sha(&archive)?));
    receipt.insert("binary_sha256".into(), json!(sha(&binary)?));
    receipt.insert(
        "host".into(),
        json!(String::from_utf8_lossy(&host.stdout).trim()),
    );
    receipt.insert(
        "runtime_environment".into(),
        Value::Object(env.iter().map(|(k, v)| (k.clone(), json!(v))).collect()),
    );
    receipt.insert("commands".into(), json!([]));
    receipt.insert("loaded_modules".into(), json!({}));
    let mut smoke = Smoke {
        binary,
        env,
        receipt,
    };
    let result = exercise(&mut smoke, &work, &system);
    if result.is_ok() {
        smoke.receipt.insert("complete".into(), json!(true));
        println!("Archive LF/CRLF stored/live/warning/Refresh smoke passed");
    }
    write_text(
        &work.join("smoke-receipt.json"),
        &(json_text(&Value::Object(smoke.receipt)) + "\n"),
    )?;
    result
}

fn exercise(smoke: &mut Smoke, work: &Path, system: &Path) -> Result<()> {
    for ending in [&b"\n"[..], &b"\r\n"[..]] {
        let label = if ending == b"\n" { "lf" } else { "crlf" };
        let project = work.join(label);
        fs::create_dir(&project)?;
        smoke.run(&project, &["--help"], None, false, None)?;
        let lines = |rows: &[&[u8]]| rows.join(ending);
        fs::write(
            project.join("sample.c"),
            lines(&[
                b"int target(void) { return 7; }",
                b"int caller(void) { return target(); }",
                b"",
            ]),
        )?;
        fs::write(
            project.join("extra.c"),
            [&b"int extra(void) { return 7; }"[..], ending].concat(),
        )?;
        let definition: &[u8] = b"sample.c target 1 int target(void ) { return 7; }\n";
        smoke.run(
            &project,
            &["-bk", "-f", "index", "sample.c", "extra.c"],
            None,
            false,
            None,
        )?;
        smoke.run(
            &project,
            &["-dL", "-f", "index", "-1", "target"],
            Some(definition),
            false,
            None,
        )?;
        let both = [
            &b"extra.c <unknown> 1 int extra(void) { return 7; }"[..],
            ending,
            b"sample.c <unknown> 1 int target(void) { return 7; }",
            ending,
        ]
        .concat();
        smoke.run(
            &project,
            &["-dL", "-f", "index", "-4", "return 7"],
            Some(&both),
            false,
            None,
        )?;
        fs::remove_file(project.join("sample.c"))?;
        smoke.run(
            &project,
            &["-dL", "-f", "index", "-1", "target"],
            Some(definition),
            false,
            None,
        )?;
        let extra = [
            &b"extra.c <unknown> 1 int extra(void) { return 7; }"[..],
            ending,
        ]
        .concat();
        smoke.run(
            &project,
            &["-dL", "-f", "index", "-4", "return 7"],
            Some(&extra),
            true,
            None,
        )?;
        fs::write(
            project.join("sample.c"),
            [&b"int replacement(void) { return 9; }"[..], ending].concat(),
        )?;
        persistent(smoke, &project, label, definition, system)?;
        let before = smoke.run(
            &project,
            &["--verify", "-d", "-f", "index"],
            None,
            false,
            None,
        )?;
        smoke.run(
            &project,
            &["-dl", "-f", "index"],
            Some(b">> \n>> "),
            false,
            Some(b"r\nq\n"),
        )?;
        if smoke.run(
            &project,
            &["--verify", "-d", "-f", "index"],
            None,
            false,
            None,
        )? != before
        {
            return Err("Unchanged Refresh changed the revision".into());
        }
    }
    Ok(())
}

/// The inspector is a host tool. Only the product Session gets the restricted
/// environment. Keep phase/output files if the host probe cannot finish.
fn module_inventory(system: &Path, pid: u32, work: &Path, label: &str) -> Result<Value> {
    let powershell = system.join("System32/WindowsPowerShell/v1.0/powershell.exe");
    let phase = work.join(format!("loaded-modules-{label}.stage"));
    let listing = work.join(format!("loaded-modules-{label}.json"));
    let log = work.join(format!("loaded-modules-{label}.log"));
    let quoted = |path: &Path| path.to_string_lossy().replace('\'', "''");
    let phase_arg = quoted(&phase);
    let listing_arg = quoted(&listing);
    let script = format!(
        "$ErrorActionPreference='Stop'; \
         [IO.File]::WriteAllText('{phase_arg}','entered'); \
         $p=Get-Process -Id {pid}; \
         [IO.File]::WriteAllText('{phase_arg}','process'); \
         $m=@($p.Modules | ForEach-Object {{$_.FileName}}); \
         [IO.File]::WriteAllText('{phase_arg}','modules'); \
         $json=ConvertTo-Json -InputObject $m -Compress; \
         [IO.File]::WriteAllText('{listing_arg}',$json); \
         [IO.File]::WriteAllText('{phase_arg}','complete')"
    );
    let arguments = [
        powershell.to_string_lossy().into_owned(),
        "-NoProfile".into(),
        "-NonInteractive".into(),
        "-Command".into(),
        script,
    ];
    let ended = run_logged(
        &arguments,
        None,
        None,
        &log,
        Some(Duration::from_secs(30)),
        None,
    )?;
    if ended != Ended::Exit(0) {
        let phase = fs::read_to_string(&phase).unwrap_or_else(|_| "before script entry".into());
        let reason = match ended {
            Ended::Deadline => "Loaded-module inspection timed out",
            _ => "Loaded-module inspection failed",
        };
        return Err(format!(
            "{reason} at phase {phase}: {}",
            command_failure("host module probe", &log)
        )
        .into());
    }
    Ok(serde_json::from_slice(&fs::read(listing)?)?)
}

/// A Session answers queries while its loaded modules are checked, then Refreshes.
fn persistent(
    smoke: &mut Smoke,
    project: &Path,
    label: &str,
    definition: &[u8],
    system: &Path,
) -> Result<()> {
    let expected = [
        &b">> cscope: 1 lines\n"[..],
        definition,
        b">> cscope: 1 lines\nsample.c caller 2 int caller(void ) { return target(); }\n>> ",
    ]
    .concat();
    let mut child = smoke
        .command(project, &["-dl", "-f", "index"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let outcome = (|| -> Result<()> {
        let mut input = child.stdin.take().ok_or("no session input")?;
        input.write_all(b"1target\n3target\n")?;
        input.flush()?;
        let mut stdout = child.stdout.take().ok_or("no session output")?;
        let mut stderr = child.stderr.take().ok_or("no session diagnostics")?;
        let (sender, receiver) = mpsc::channel();
        let length = expected.len();
        thread::spawn(move || {
            let mut first = vec![0; length];
            let read = stdout.read_exact(&mut first).map(|()| first);
            let _ = sender.send((read, stdout));
        });
        let (first, mut stdout) = receiver
            .recv_timeout(Duration::from_secs(30))
            .map_err(|_| "Persistent response timed out")?;
        if first? != expected {
            return Err("Persistent response mismatch".into());
        }
        let modules = module_inventory(
            system,
            child.id(),
            project
                .parent()
                .ok_or("smoke project has no work directory")?,
            label,
        )?;
        let names = modules
            .as_array()
            .filter(|names| !names.is_empty())
            .ok_or("Loaded-module inspection returned no inventory")?;
        let binary = smoke.binary.canonicalize()?;
        let allowed = [
            system.join("System32").canonicalize()?,
            system.join("WinSxS").canonicalize()?,
        ];
        for name in names {
            let path =
                PathBuf::from(name.as_str().ok_or("module path is not text")?).canonicalize()?;
            if path != binary && !allowed.iter().any(|directory| path.starts_with(directory)) {
                return Err(format!("Non-system runtime module loaded: {}", path.display()).into());
            }
        }
        if let Some(Value::Object(loaded)) = smoke.receipt.get_mut("loaded_modules") {
            loaded.insert(label.to_owned(), modules.clone());
        }
        input.write_all(b"r\n1replacement\nr\nq\n")?;
        drop(input);
        let (sender, receiver) = mpsc::channel();
        let diagnostics_sender = sender.clone();
        thread::spawn(move || {
            let mut rest = Vec::new();
            let _ = sender.send((true, stdout.read_to_end(&mut rest).map(|_| rest)));
        });
        thread::spawn(move || {
            let mut diagnostics = Vec::new();
            let _ = diagnostics_sender.send((
                false,
                stderr.read_to_end(&mut diagnostics).map(|_| diagnostics),
            ));
        });
        let (mut rest, mut diagnostics) = (Vec::new(), Vec::new());
        for _ in 0..2 {
            let (output, bytes) = receiver
                .recv_timeout(Duration::from_secs(30))
                .map_err(|_| "Persistent Refresh timed out")?;
            if output {
                rest = bytes?;
            } else {
                diagnostics = bytes?;
            }
        }
        let status = child.wait()?;
        let expected = b"\n>> cscope: 1 lines\nsample.c replacement 1 int replacement(void ) { return 9; }\n>> \n>> ";
        if !status.success() || rest != expected || !diagnostics.is_empty() {
            return Err(format!(
                "Persistent Refresh mismatch: {:?} {:?}",
                String::from_utf8_lossy(&rest),
                String::from_utf8_lossy(&diagnostics)
            )
            .into());
        }
        Ok(())
    })();
    if child.try_wait()?.is_none() {
        let _ = child.kill();
        let _ = child.wait();
    }
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive;

    #[cfg(windows)]
    #[test]
    fn host_module_probe_inspects_a_real_live_process() {
        let root = tempfile::tempdir().unwrap();
        let system = PathBuf::from(std::env::var_os("SystemRoot").unwrap());
        let modules = module_inventory(&system, std::process::id(), root.path(), "test").unwrap();
        let executable = std::env::current_exe().unwrap().canonicalize().unwrap();
        assert!(modules.as_array().unwrap().iter().any(|module| {
            PathBuf::from(module.as_str().unwrap())
                .canonicalize()
                .unwrap()
                == executable
        }));
        assert_eq!(
            fs::read_to_string(root.path().join("loaded-modules-test.stage")).unwrap(),
            "complete"
        );
    }

    #[test]
    fn unpacking_checks_the_manifest_and_the_emacs_package() {
        let root = tempfile::tempdir().unwrap();
        let package = root.path().join("ripscope-0.1.0-x86_64-windows-msvc");
        for (name, contents) in [
            ("bin/ripscope.exe", "binary"),
            ("contrib/emacs/ripscope.el", "package"),
        ] {
            let path = package.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, contents).unwrap();
        }
        let manifest: String = inventory(&package)
            .unwrap()
            .iter()
            .map(|(name, digest)| format!("{}  {name}\r\n", digest.as_str().unwrap()))
            .collect();
        fs::write(package.join("MANIFEST.sha256"), manifest).unwrap();
        archive::zip(&package, &root.path().join("good.zip"), 1_700_000_000).unwrap();
        let unpacked = unpack(&root.path().join("good.zip"), &root.path().join("good")).unwrap();
        assert_eq!(
            fs::read_to_string(unpacked.join("bin/ripscope.exe")).unwrap(),
            "binary"
        );
        fs::write(package.join("bin/ripscope.exe"), "changed").unwrap();
        archive::zip(&package, &root.path().join("bad.zip"), 1_700_000_000).unwrap();
        let error = unpack(&root.path().join("bad.zip"), &root.path().join("bad")).unwrap_err();
        assert!(error.to_string().contains("mismatch"), "{error}");
    }
}
