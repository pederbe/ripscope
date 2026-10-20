//! An editor or backend process with a deadline that stops its whole process tree, for
//! the real-editor checks.

use std::env;
use std::ffi::OsStr;
use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// The exit code a deadline reports, as `timeout` does.
pub const TIMED_OUT: i32 = 124;

/// A thread reading one captured output to its end.
type Reader = JoinHandle<io::Result<Vec<u8>>>;

pub struct EditorProcess {
    child: Child,
    running: bool,
    captured: Option<(Reader, Reader)>,
}

/// A captured process's outcome: its exit code, or `None` when its deadline stopped it.
pub struct Captured {
    pub code: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

impl EditorProcess {
    /// Starts `command` with both outputs going to `log`.
    pub fn start(mut command: Command, log: &Path) -> Result<Self> {
        let log = File::create(log)?;
        command
            .stdin(Stdio::null())
            .stdout(log.try_clone()?)
            .stderr(log);
        Self::spawn(command, false)
    }

    /// Starts `command` with both outputs captured.
    pub fn start_captured(mut command: Command) -> Result<Self> {
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        Self::spawn(command, true)
    }

    fn spawn(mut command: Command, captured: bool) -> Result<Self> {
        // Keep the editor's inherited Windows console and its encoding for paths.
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        let mut child = command.spawn()?;
        let captured = if captured {
            let mut stdout = child.stdout.take().ok_or("no captured stdout")?;
            let mut stderr = child.stderr.take().ok_or("no captured stderr")?;
            Some((
                thread::spawn(move || {
                    let mut bytes = Vec::new();
                    stdout.read_to_end(&mut bytes).map(|_| bytes)
                }),
                thread::spawn(move || {
                    let mut bytes = Vec::new();
                    stderr.read_to_end(&mut bytes).map(|_| bytes)
                }),
            ))
        } else {
            None
        };
        Ok(Self {
            child,
            running: true,
            captured,
        })
    }

    /// Waits up to `timeout`, then stops the process tree and reports [`TIMED_OUT`].
    pub fn wait(mut self, timeout: Duration) -> Result<i32> {
        Ok(self.finish(timeout)?.unwrap_or(TIMED_OUT))
    }

    /// Waits up to `timeout` for a captured process, stopping its tree at the deadline.
    pub fn wait_captured(mut self, timeout: Duration) -> Result<Captured> {
        let code = self.finish(timeout)?;
        let (stdout, stderr) = self.captured.take().ok_or("output was not captured")?;
        let join = |reader: JoinHandle<io::Result<Vec<u8>>>| -> Result<Vec<u8>> {
            Ok(reader.join().map_err(|_| "output reader failed")??)
        };
        Ok(Captured {
            code,
            stdout: join(stdout)?,
            stderr: join(stderr)?,
        })
    }

    fn finish(&mut self, timeout: Duration) -> Result<Option<i32>> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(status) = self.child.try_wait()? {
                self.running = false;
                return Ok(Some(exit_code(status)));
            }
            if Instant::now() >= deadline {
                self.terminate()?;
                return Ok(None);
            }
            thread::sleep(Duration::from_millis(20));
        }
    }

    fn terminate(&mut self) -> Result<()> {
        #[cfg(windows)]
        {
            let mut command = Command::new(system_command("taskkill.exe")?);
            command.args(["/PID", &self.child.id().to_string(), "/T", "/F"]);
            hide_window(&mut command);
            let output = command.output()?;
            if !output.status.success() {
                return Err(format!(
                    "process tree cleanup failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                )
                .into());
            }
        }
        #[cfg(unix)]
        {
            use rustix::process::{Pid, Signal, kill_process_group};
            let pid = Pid::from_raw(i32::try_from(self.child.id())?).ok_or("invalid process id")?;
            match kill_process_group(pid, Signal::KILL) {
                Ok(()) | Err(rustix::io::Errno::SRCH) => {}
                Err(error) => return Err(error.into()),
            }
        }
        self.child.wait()?;
        self.running = false;
        Ok(())
    }
}

impl Drop for EditorProcess {
    fn drop(&mut self) {
        if self.running {
            let _ = self.terminate();
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

/// A process's exit code, with a Unix signal reported as its negated number.
pub fn exit_code(status: ExitStatus) -> i32 {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        status
            .code()
            .unwrap_or_else(|| -status.signal().unwrap_or(1))
    }
    #[cfg(not(unix))]
    status.code().unwrap_or(1)
}

/// A program in the Windows system directory.
pub fn system_command(name: &str) -> Result<PathBuf> {
    Ok(system_directory()?.join(name))
}

/// The Windows system directory, `System32`.
pub fn system_directory() -> Result<PathBuf> {
    Ok(PathBuf::from(env::var_os("SystemRoot").ok_or("SystemRoot is not set")?).join("System32"))
}

/// An existing file's absolute path. Linux resolves links; Windows keeps the ordinary
/// drive form, which editors and their scripts accept.
pub fn real_path(path: &Path) -> Result<PathBuf> {
    if cfg!(windows) {
        let absolute = std::path::absolute(path)?;
        if !absolute.exists() {
            return Err(format!("not found: {}", absolute.display()).into());
        }
        Ok(absolute)
    } else {
        Ok(path.canonicalize()?)
    }
}

/// Keeps a helper console program from opening a window on Windows.
pub fn hide_window(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    #[cfg(not(windows))]
    let _ = command;
}

/// The first line a version command prints.
pub fn first_line(command: &mut Command) -> Result<String> {
    hide_window(command);
    let output = command.output()?;
    if !output.status.success() {
        return Err(format!("version command failed: {command:?}: {}", output.status).into());
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .trim()
        .lines()
        .next()
        .ok_or("empty version output")?
        .to_owned())
}

/// The operating system's release description.
pub fn os_release() -> Result<String> {
    if cfg!(windows) {
        first_line(Command::new(system_command("cmd.exe")?).args(["/d", "/c", "ver"]))
    } else {
        Ok(std::fs::read_to_string("/etc/os-release")?)
    }
}

/// Whether a process with this id is still running.
pub fn process_alive(pid: u32) -> Result<bool> {
    if cfg!(windows) {
        let mut command = Command::new(system_command("tasklist.exe")?);
        command.args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"]);
        hide_window(&mut command);
        let output = command.output()?;
        if !output.status.success() {
            return Err(format!("tasklist failed: {}", output.status).into());
        }
        let quoted = format!("\"{pid}\"");
        Ok(String::from_utf8_lossy(&output.stdout)
            .lines()
            .any(|line| line.split(',').nth(1) == Some(quoted.as_str())))
    } else {
        Ok(Path::new(&format!("/proc/{pid}")).exists())
    }
}

/// The program `name` names, searching PATH for a bare name.
pub fn executable(name: &OsStr) -> Result<PathBuf> {
    let path = Path::new(name);
    let candidates = if path.components().count() > 1 || path.is_absolute() {
        vec![path.to_owned()]
    } else {
        env::split_paths(&env::var_os("PATH").unwrap_or_default())
            .map(|directory| directory.join(path))
            .collect()
    };
    for path in candidates {
        let mut variants = vec![path.clone()];
        if cfg!(windows) && path.extension().is_none() {
            variants.push(path.with_extension("exe"));
        }
        for candidate in variants {
            if candidate.is_file() {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    if candidate.metadata()?.permissions().mode() & 0o111 == 0 {
                        continue;
                    }
                }
                // Ordinary absolute paths also work with native Windows editors.
                return Ok(std::path::absolute(candidate)?);
            }
        }
    }
    Err(format!("executable not found: {}", name.to_string_lossy()).into())
}
