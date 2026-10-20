//! File-backed command capture with deadlines and explicit incomplete outcomes.

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{self, Read};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[path = "owned_process.rs"]
#[allow(dead_code)]
pub(crate) mod owned_process;

pub const OUTPUT_LIMIT: u64 = 2 << 30;

/// Shared stream/tree guard for the command and open-session interfaces.
#[allow(dead_code)]
pub(crate) fn exceeded(
    prefix: &Path,
    limits: &[(std::path::PathBuf, u64)],
) -> io::Result<Option<&'static str>> {
    if fs::metadata(prefix.with_extension("stdout"))?.len()
        + fs::metadata(prefix.with_extension("stderr"))?.len()
        > OUTPUT_LIMIT
    {
        return Ok(Some("output_limit"));
    }
    for (tree, limit) in limits {
        if tree_size(tree)? > *limit {
            return Ok(Some("tree_artifact_limit"));
        }
    }
    Ok(None)
}

pub fn digest(path: &Path) -> io::Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hash = Sha256::new();
    let mut bytes = [0; 65536];
    loop {
        let count = file.read(&mut bytes)?;
        if count == 0 {
            break;
        }
        hash.update(&bytes[..count]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

/// Hash only the bounded byte snapshot; an unknown producer may still append.
#[allow(dead_code)]
pub(crate) fn snapshot(path: &Path) -> io::Result<Value> {
    let observed = fs::metadata(path)?.len();
    let mut file = fs::File::open(path)?.take(observed.min(OUTPUT_LIMIT));
    let mut hash = Sha256::new();
    let mut total = 0_u64;
    let mut bytes = [0; 65536];
    loop {
        let count = file.read(&mut bytes)?;
        if count == 0 {
            break;
        }
        total += count as u64;
        hash.update(&bytes[..count]);
    }
    Ok(
        json!({"observed_bytes":observed,"hash_bytes":total,"sha256":format!("{:x}",hash.finalize())}),
    )
}

pub fn save(path: &Path, value: &Value) -> io::Result<()> {
    fs::write(path, serde_json::to_vec_pretty(value)?)
}

/// Guard a prepared command and report descendant cleanup independently of exit.
#[allow(dead_code)] // Shared by examples; only workflow clients need this guard.
pub fn run_command(
    command: Command,
    prefix: &Path,
    seconds: u64,
    limits: &[(std::path::PathBuf, u64)],
) -> io::Result<Value> {
    let identity = format!("{command:?}");
    let stdout = prefix.with_extension("stdout");
    let stderr = prefix.with_extension("stderr");
    let started = Instant::now();
    let launched = owned_process::OwnedProcess::spawn(
        command,
        fs::File::create(&stdout)?,
        fs::File::create(&stderr)?,
        false,
    );
    let mut child = match launched {
        Ok(child) => child,
        Err(error) => {
            let result = json!({"command":identity,"status":null,"complete":false,"stopped":"spawn_failure","monitor_error":error.to_string(),
                "cleanup":{"state":"confirmed","method":"no-native-process-created"},"seconds":started.elapsed().as_secs_f64(),
                "stdout_path":stdout,"stderr_path":stderr,"stdout_sha256":digest(&stdout)?,"stderr_sha256":digest(&stderr)?});
            save(&prefix.with_extension("json"), &result)?;
            return Ok(result);
        }
    };
    let mut stopped = None;
    let mut status = None;
    let monitored = (|| -> io::Result<()> {
        loop {
            status = child.try_wait()?;
            if started.elapsed() >= Duration::from_secs(seconds) {
                stopped = Some("timeout");
            }
            if let Some(limit) = exceeded(prefix, limits)? {
                stopped = Some(limit);
            }
            if status.is_some() || stopped.is_some() {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    })();
    let cleanup = child.finish();
    let stdout_snapshot = snapshot(&stdout)?;
    let stderr_snapshot = snapshot(&stderr)?;
    let result = json!({"command":identity,"status":status.and_then(|status|status.code()),
        "complete":status.is_some_and(|status|status.success()) && stopped.is_none() && monitored.is_ok() && cleanup["state"]=="confirmed",
        "stopped":stopped,"monitor_error":monitored.err().map(|error|error.to_string()),"cleanup":cleanup,
        "seconds":started.elapsed().as_secs_f64(),"stdout_path":stdout,"stderr_path":stderr,
        "stdout_sha256":stdout_snapshot["sha256"],"stderr_sha256":stderr_snapshot["sha256"],"stdout_snapshot":stdout_snapshot,"stderr_snapshot":stderr_snapshot,
        "capture_hash_scope":if cleanup["state"]=="confirmed" {"closed-file-snapshot"}else{"observed-prefix"}});
    save(&prefix.with_extension("json"), &result)?;
    Ok(result)
}

/// The optional scope makes the kernel enforce memory limits for all descendants.
pub fn run(
    program: &Path,
    args: &[String],
    cwd: &Path,
    prefix: &Path,
    seconds: u64,
    memory_scope: bool,
) -> io::Result<Value> {
    run_guarded(program, args, cwd, prefix, seconds, memory_scope, &[])
}

pub fn tree_size(path: &Path) -> io::Result<u64> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(error),
    };
    if metadata.is_symlink() {
        return Ok(0);
    }
    if metadata.is_file() {
        return Ok(metadata.len());
    }
    let mut entries = match fs::read_dir(path) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(error),
    };
    entries.try_fold(0, |total, item| Ok(total + tree_size(&item?.path())?))
}

struct ManagedChild {
    child: std::process::Child,
    unit: Option<String>,
}

impl ManagedChild {
    fn finish_scope(&mut self) -> io::Result<()> {
        if let Some(unit) = self.unit.take() {
            let _ = control(&[
                "--user",
                "kill",
                "--kill-whom=all",
                "--signal=SIGKILL",
                &unit,
            ]);
            let _ = control(&["--user", "stop", &unit]);
            let state = control(&["--user", "show", "--property=ActiveState", "--value", &unit])?;
            if !state.status.success()
                || !matches!(
                    String::from_utf8_lossy(&state.stdout).trim(),
                    "inactive" | "failed"
                )
            {
                return Err(io::Error::other(format!(
                    "could not confirm scope termination: {unit}"
                )));
            }
        }
        Ok(())
    }
}

fn control(args: &[&str]) -> io::Result<std::process::Output> {
    let mut child = Command::new("systemctl")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let started = Instant::now();
    while child.try_wait()?.is_none() {
        if started.elapsed() >= Duration::from_secs(10) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(io::Error::other("scope-control command timed out"));
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    child.wait_with_output()
}

impl std::ops::Deref for ManagedChild {
    type Target = std::process::Child;
    fn deref(&self) -> &Self::Target {
        &self.child
    }
}
impl std::ops::DerefMut for ManagedChild {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.child
    }
}
impl Drop for ManagedChild {
    fn drop(&mut self) {
        let _ = self.finish_scope();
        if self.child.try_wait().ok().flatten().is_none() {
            #[cfg(unix)]
            if let Some(pid) = rustix::process::Pid::from_raw(self.child.id() as i32) {
                let _ = rustix::process::kill_process_group(pid, rustix::process::Signal::KILL);
            }
            #[cfg(windows)]
            let _ = Command::new("taskkill")
                .args(["/PID", &self.child.id().to_string(), "/T", "/F"])
                .output();
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

/// Additional tree limits include archive writes and nested evidence directories.
pub fn run_guarded(
    program: &Path,
    args: &[String],
    cwd: &Path,
    prefix: &Path,
    seconds: u64,
    memory_scope: bool,
    limits: &[(std::path::PathBuf, u64)],
) -> io::Result<Value> {
    let stdout = prefix.with_extension("stdout");
    let stderr = prefix.with_extension("stderr");
    let unit = memory_scope.then(|| {
        format!(
            "ripscope-capture-{}-{}.scope",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        )
    });
    let mut command = if memory_scope {
        let mut command = Command::new("systemd-run");
        command.args([
            "--user",
            "--quiet",
            "--scope",
            "-p",
            "MemoryMax=8G",
            "-p",
            "MemorySwapMax=0",
        ]);
        command.args(["--unit", unit.as_ref().unwrap()]);
        command.arg("--").arg(program);
        command
    } else {
        Command::new(program)
    };
    command
        .args(args)
        .current_dir(cwd)
        .env_remove("INCDIR")
        .env_remove("INCLUDEDIRS")
        .env_remove("RIPSCOPE_BLOCK_SIZE")
        .env("LC_ALL", "C")
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .stdout(fs::File::create(&stdout)?)
        .stderr(fs::File::create(&stderr)?);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let started = Instant::now();
    let mut child = ManagedChild {
        child: command.spawn()?,
        unit: unit.clone(),
    };
    let mut stopped = None;
    let mut monitor_error = None;
    let mut last_tree_check = Instant::now() - Duration::from_secs(1);
    let status = loop {
        let status = child.try_wait()?;
        if started.elapsed() >= Duration::from_secs(seconds) {
            stopped = Some("timeout");
        } else if fs::metadata(&stdout)?.len() + fs::metadata(&stderr)?.len() > OUTPUT_LIMIT {
            stopped = Some("output_limit");
        } else if fs::read_dir(
            prefix
                .parent()
                .ok_or_else(|| io::Error::other("capture has no directory"))?,
        )?
        .try_fold(0u64, |size, item| {
            let metadata = item?.metadata()?;
            Ok::<_, io::Error>(
                size + if metadata.is_file() {
                    metadata.len()
                } else {
                    0
                },
            )
        })? > 20 << 30
        {
            stopped = Some("project_artifact_limit");
        }
        if status.is_some() || last_tree_check.elapsed() >= Duration::from_secs(1) {
            for (tree, limit) in limits {
                match tree_size(tree) {
                    Ok(bytes) if bytes > *limit => stopped = Some("tree_artifact_limit"),
                    Ok(_) => {}
                    Err(error) => {
                        stopped = Some("artifact_monitor_error");
                        monitor_error = Some(error.to_string());
                    }
                }
            }
            last_tree_check = Instant::now();
        }
        if let Some(status) = status {
            break status;
        }
        if stopped.is_some() {
            #[cfg(unix)]
            if let Some(pid) = rustix::process::Pid::from_raw(child.id() as i32) {
                let _ = rustix::process::kill_process_group(pid, rustix::process::Signal::KILL);
            }
            #[cfg(windows)]
            {
                let _ = Command::new("taskkill")
                    .args(["/PID", &child.id().to_string(), "/T", "/F"])
                    .output();
            }
            let _ = child.kill();
            break child.wait()?;
        }
        std::thread::sleep(Duration::from_millis(25));
    };
    let cleanup_error = child.finish_scope().err().map(|error| error.to_string());
    if cleanup_error.is_some() {
        stopped = Some("scope_cleanup_failed");
    }
    let result = json!({
        "program": program, "args": args, "cwd": cwd, "status": status.code(),
        "complete": status.success() && stopped.is_none(), "stopped": stopped,
        "monitor_error":monitor_error,
        "cleanup_error":cleanup_error,
        "memory_unit":unit,
        "seconds": started.elapsed().as_secs_f64(), "memory_scope": memory_scope,
        "stdout_path": stdout, "stderr_path": stderr,
        "stdout_bytes": fs::metadata(&stdout)?.len(), "stderr_bytes": fs::metadata(&stderr)?.len(),
        "stdout_sha256": digest(&stdout)?, "stderr_sha256": digest(&stderr)?,
    });
    save(&prefix.with_extension("json"), &result)?;
    Ok(result)
}

#[cfg(all(test, unix))]
mod tests {
    #[test]
    #[ignore = "requires a Linux user systemd manager"]
    fn scope_cancellation_reaps_a_detached_descendant() {
        let directory = tempfile::tempdir().unwrap();
        let result = super::run(
            std::path::Path::new("/bin/sh"),
            &[
                "-c".into(),
                "setsid /bin/sh -c 'echo $$; sleep 30' & wait".into(),
            ],
            directory.path(),
            &directory.path().join("scope"),
            1,
            true,
        )
        .unwrap();
        assert_eq!(result["complete"], false);
        assert_eq!(result["stopped"], "timeout");
        let pid = std::fs::read_to_string(result["stdout_path"].as_str().unwrap()).unwrap();
        let status = std::process::Command::new("ps")
            .args(["-o", "stat=", "-p", pid.trim()])
            .output()
            .unwrap();
        let state = String::from_utf8_lossy(&status.stdout);
        assert!(
            state.trim().is_empty() || state.trim().starts_with('Z'),
            "descendant still running: {state}"
        );
    }

    #[test]
    fn a_deadline_retains_partial_capture_and_is_never_complete() {
        let directory = tempfile::tempdir().unwrap();
        let result = super::run(
            std::path::Path::new("/bin/sh"),
            &["-c".into(), "printf partial; sleep 5".into()],
            directory.path(),
            &directory.path().join("deadline"),
            0,
            false,
        )
        .unwrap();
        assert_eq!(result["complete"], false);
        assert_eq!(result["stopped"], "timeout");
        assert!(directory.path().join("deadline.stdout").exists());
        assert!(directory.path().join("deadline.json").exists());
    }
}
