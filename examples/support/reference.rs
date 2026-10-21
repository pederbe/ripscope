//! Run the pinned reference cscope (ADR 0001) on native Linux, or on Windows through
//! Ubuntu WSL.

use sha2::{Digest, Sha256};
use std::env;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

pub const CSCOPE_SHA256: &str = "c2b68c8d216835b5b0637fcf41a40c59d6b73a81cc411136ec224cfcff01401c";

/// The pinned executable and how commands reach it.
pub struct Reference {
    pub cscope: PathBuf,
    pub sha256: String,
    /// Run through WSL, with Windows paths translated to `/mnt/<drive>`.
    pub windows: bool,
    /// Private narrow-ncurses libraries for the reference process on Linux.
    pub libraries: Option<String>,
}

/// What a reference command wrote.
pub struct Output {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

impl Reference {
    /// The package under `RIPSCOPE_REFERENCE_DIR`, or `.reference/cscope` in the checkout.
    pub fn pinned() -> Reference {
        let package = env::var_os("RIPSCOPE_REFERENCE_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join(".reference/cscope"));
        let windows = cfg!(windows);
        Reference {
            cscope: package.join("extracted/usr/bin/cscope"),
            sha256: CSCOPE_SHA256.to_owned(),
            windows,
            libraries: if windows {
                None
            } else {
                libraries(&package, env::var("RIPSCOPE_REFERENCE_LIBRARY_PATH").ok())
            },
        }
    }

    /// A local absolute path as the reference command sees it.
    pub fn path(&self, path: &Path) -> String {
        let text = path.to_string_lossy();
        if !self.windows {
            return text.into_owned();
        }
        match text.split_once(':') {
            Some((drive, rest)) => {
                format!("/mnt/{}{}", drive.to_lowercase(), rest.replace('\\', "/"))
            }
            None => text.into_owned(),
        }
    }

    /// The shell command for the pinned executable, with any private Linux libraries.
    pub fn command(&self) -> String {
        let mut words = vec![self.path(&self.cscope)];
        if let Some(libraries) = self.libraries.as_ref().filter(|value| !value.is_empty()) {
            words.splice(
                0..0,
                ["env".to_owned(), format!("LD_LIBRARY_PATH={libraries}")],
            );
        }
        words
            .iter()
            .map(|word| quote(word))
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// The process arguments for `args` under a group deadline of `seconds`.
    pub fn arguments(&self, args: &[&str], seconds: f64) -> Vec<String> {
        let mut command = Vec::new();
        if self.windows {
            command.extend(["wsl", "-d", "Ubuntu", "--exec"].map(str::to_owned));
        }
        command.extend([
            "timeout".to_owned(),
            "--signal=KILL".to_owned(),
            seconds_text(seconds),
        ]);
        command.extend(args.iter().map(|arg| (*arg).to_owned()));
        command
    }

    /// Checks the executable's identity, then runs `args` inside Linux. The inner deadline
    /// kills the whole group; commands under it must use `timeout --foreground`.
    pub fn run(&self, args: &[&str], stdin: Option<&[u8]>, seconds: f64) -> io::Result<Output> {
        let actual = base16ct::lower::encode_string(&Sha256::digest(std::fs::read(&self.cscope)?));
        if actual != self.sha256 {
            return Err(io::Error::other(
                "the reference cscope binary does not match ADR 0001",
            ));
        }
        let arguments = self.arguments(args, seconds);
        // Let Linux stop the process group before the outer transport deadline expires.
        let (status, stdout, stderr) =
            capture(&arguments, stdin, Duration::from_secs_f64(seconds + 10.0))?;
        if status != 0 {
            let detail = if [-9, 124, 137].contains(&status) {
                "timed out"
            } else {
                "failed"
            };
            let text = String::from_utf8_lossy(&stderr);
            let tail: String = {
                let chars: Vec<char> = text.chars().collect();
                chars[chars.len().saturating_sub(2000)..].iter().collect()
            };
            return Err(io::Error::other(format!(
                "reference command {detail} (status {status}): {tail}"
            )));
        }
        Ok(Output { stdout, stderr })
    }
}

/// The library directory the reference uses on Linux: the explicit setting, else the
/// private runtime beside the package when it exists.
pub fn libraries(package: &Path, explicit: Option<String>) -> Option<String> {
    explicit.or_else(|| {
        let bundled = package.join("runtime/usr/lib/x86_64-linux-gnu");
        bundled
            .is_dir()
            .then(|| bundled.to_string_lossy().into_owned())
    })
}

/// A word quoted for a POSIX shell, unchanged when it needs no quoting.
pub fn quote(word: &str) -> String {
    let safe = |ch: char| ch.is_ascii_alphanumeric() || "_@%+=:,./-".contains(ch);
    if word.is_empty() {
        "''".to_owned()
    } else if word.chars().all(safe) {
        word.to_owned()
    } else {
        format!("'{}'", word.replace('\'', "'\"'\"'"))
    }
}

fn seconds_text(seconds: f64) -> String {
    if seconds.fract() == 0.0 {
        format!("{}", seconds as u64)
    } else {
        format!("{seconds}")
    }
}

/// Runs a command with optional standard input, returning its status, where a signal
/// is reported as its negated number, and both outputs. A deadline kills the process.
fn capture(
    arguments: &[String],
    stdin: Option<&[u8]>,
    deadline: Duration,
) -> io::Result<(i32, Vec<u8>, Vec<u8>)> {
    let mut child = Command::new(&arguments[0])
        .args(&arguments[1..])
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let input = stdin.map(<[u8]>::to_vec);
    let mut writer = child.stdin.take();
    let feeder = thread::spawn(move || {
        if let (Some(mut pipe), Some(input)) = (writer.take(), input) {
            let _ = pipe.write_all(&input);
        }
    });
    let mut out = child.stdout.take().expect("piped stdout");
    let mut err = child.stderr.take().expect("piped stderr");
    let stdout = thread::spawn(move || {
        let mut bytes = Vec::new();
        out.read_to_end(&mut bytes).map(|_| bytes)
    });
    let stderr = thread::spawn(move || {
        let mut bytes = Vec::new();
        err.read_to_end(&mut bytes).map(|_| bytes)
    });
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if started.elapsed() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "reference transport deadline exceeded",
            ));
        }
        thread::sleep(Duration::from_millis(10));
    };
    let _ = feeder.join();
    let stdout = stdout
        .join()
        .map_err(|_| io::Error::other("stdout reader failed"))??;
    let stderr = stderr
        .join()
        .map_err(|_| io::Error::other("stderr reader failed"))??;
    Ok((code(status), stdout, stderr))
}

fn code(status: std::process::ExitStatus) -> i32 {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            return -signal;
        }
    }
    status.code().unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake(root: &Path, windows: bool) -> Reference {
        let cscope = root.join("cscope");
        std::fs::write(&cscope, b"pinned reference").unwrap();
        Reference {
            sha256: base16ct::lower::encode_string(&Sha256::digest(b"pinned reference")),
            cscope,
            windows,
            libraries: None,
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn timeout_stops_the_script_and_its_descendant() {
        let root = tempfile::tempdir().unwrap();
        let reference = fake(root.path(), false);
        let pid_file = root.path().join("child.pid");
        let child = format!(
            "trap '' TERM; sleep 30 & echo $! > {}; wait",
            quote(&pid_file.to_string_lossy())
        );
        let script = format!(
            "timeout --foreground 30 bash -c {} >/dev/null 2>&1",
            quote(&child)
        );
        let error = reference
            .run(&["bash", "-c", &script], None, 0.2)
            .err()
            .unwrap();
        assert!(error.to_string().contains("timed out"), "{error}");
        let pid: u32 = std::fs::read_to_string(&pid_file)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        // SIGKILL is delivered asynchronously, so the descendant can briefly still run.
        let state = || {
            std::fs::read_to_string(format!("/proc/{pid}/status"))
                .ok()
                .and_then(|status| {
                    status
                        .lines()
                        .find(|line| line.starts_with("State:"))
                        .map(str::to_owned)
                })
                .filter(|state| !state.contains("Z (zombie)"))
        };
        let stopping = Instant::now();
        while state().is_some() && stopping.elapsed() < Duration::from_secs(5) {
            thread::sleep(Duration::from_millis(10));
        }
        if let Some(state) = state() {
            panic!("timed-out descendant is still running: {state}");
        }
    }

    #[test]
    fn windows_paths_and_commands_keep_spaces_and_quotes() {
        let reference = Reference {
            cscope: PathBuf::from("D:\\source trees\\owner's project\\cscope"),
            sha256: String::new(),
            windows: true,
            libraries: None,
        };
        assert_eq!(
            reference.path(&reference.cscope),
            "/mnt/d/source trees/owner's project/cscope"
        );
        assert_eq!(
            reference.command(),
            "'/mnt/d/source trees/owner'\"'\"'s project/cscope'"
        );
    }

    #[test]
    fn private_libraries_are_shell_quoted_and_only_used_on_linux() {
        let reference = Reference {
            cscope: PathBuf::from("/pinned/cscope"),
            sha256: String::new(),
            windows: false,
            libraries: Some("/private libraries/owner's".to_owned()),
        };
        assert_eq!(
            reference.command(),
            "env 'LD_LIBRARY_PATH=/private libraries/owner'\"'\"'s' /pinned/cscope"
        );
        let package = tempfile::tempdir().unwrap();
        assert_eq!(libraries(package.path(), None), None);
        std::fs::create_dir_all(package.path().join("runtime/usr/lib/x86_64-linux-gnu")).unwrap();
        assert!(
            libraries(package.path(), None)
                .unwrap()
                .ends_with("x86_64-linux-gnu")
        );
        assert_eq!(
            libraries(package.path(), Some(String::new())),
            Some(String::new())
        );
    }

    #[test]
    fn reference_identity_is_checked_before_any_command() {
        let root = tempfile::tempdir().unwrap();
        let mut reference = fake(root.path(), false);
        reference.sha256 = base16ct::lower::encode_string(&Sha256::digest(b"another cscope build"));
        let marker = root.path().join("ran");
        let script = format!("touch {}", quote(&marker.to_string_lossy()));
        let error = reference
            .run(&["sh", "-c", &script], None, 10.0)
            .err()
            .unwrap();
        assert!(error.to_string().contains("ADR 0001"), "{error}");
        assert!(!marker.exists());
    }

    #[test]
    fn transports_wrap_the_group_deadline() {
        let root = tempfile::tempdir().unwrap();
        for windows in [false, true] {
            let reference = fake(root.path(), windows);
            let mut expected = if windows {
                vec!["wsl", "-d", "Ubuntu", "--exec"]
            } else {
                Vec::new()
            };
            expected.extend(["timeout", "--signal=KILL", "1900", "bash", "-c", "cat"]);
            assert_eq!(
                reference.arguments(&["bash", "-c", "cat"], 1900.0),
                expected
            );
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn native_transport_preserves_stdin_stdout_and_stderr() {
        let root = tempfile::tempdir().unwrap();
        let reference = fake(root.path(), false);
        let data = b"binary\x00\xff\n";
        let output = reference
            .run(
                &["bash", "-c", "cat; printf 'warning\\377' >&2"],
                Some(data),
                30.0,
            )
            .unwrap();
        assert_eq!(output.stdout, data);
        assert_eq!(output.stderr, b"warning\xff");
    }
}
