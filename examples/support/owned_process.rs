//! Process ownership for guarded developer workflows, independent of parent exit.
#[cfg(windows)]
#[path = "owned_process_windows.rs"]
mod windows;
#[cfg(windows)]
pub(crate) use windows::OwnedProcess;

#[cfg(not(windows))]
pub(crate) use native::OwnedProcess;
#[cfg(not(windows))]
mod native {
    use serde_json::{Value, json};
    use std::fs::{self, File};
    use std::io;
    use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
    use std::time::{Duration, Instant};

    pub(crate) struct OwnedProcess {
        child: Child,
        unit: Option<String>,
        group: u32,
        finished: bool,
    }

    fn control(arguments: &[&str]) -> io::Result<std::process::Output> {
        let mut child = Command::new("systemctl")
            .args(arguments)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let started = Instant::now();
        loop {
            if child.try_wait()?.is_some() {
                return child.wait_with_output();
            }
            if started.elapsed() > Duration::from_secs(2) {
                let _ = child.kill();
                return Err(io::Error::other("scope control deadline"));
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn observation(output: &io::Result<std::process::Output>) -> Value {
        match output {
            Ok(output) => {
                json!({"status":output.status.code(),"stdout":String::from_utf8_lossy(&output.stdout),"stderr":String::from_utf8_lossy(&output.stderr),"error":null})
            }
            Err(error) => {
                json!({"status":null,"stdout":null,"stderr":null,"error":error.to_string()})
            }
        }
    }

    impl OwnedProcess {
        pub(crate) fn spawn(
            mut command: Command,
            stdout: File,
            stderr: File,
            piped: bool,
        ) -> io::Result<Self> {
            let mut unit = None;
            #[cfg(target_os = "linux")]
            if control(&["--user", "show-environment"]).is_ok_and(|output| output.status.success())
            {
                let name = format!(
                    "ripscope-workflow-{}-{}.scope",
                    std::process::id(),
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_nanos()
                );
                let mut scoped = Command::new("systemd-run");
                scoped
                    .args([
                        "--user",
                        "--quiet",
                        "--scope",
                        "-p",
                        "MemoryMax=8G",
                        "-p",
                        "MemorySwapMax=0",
                        "--unit",
                        &name,
                        "--",
                    ])
                    .arg(command.get_program())
                    .args(command.get_args());
                if let Some(directory) = command.get_current_dir() {
                    scoped.current_dir(directory);
                }
                for (key, value) in command.get_envs() {
                    if let Some(value) = value {
                        scoped.env(key, value);
                    } else {
                        scoped.env_remove(key);
                    }
                }
                command = scoped;
                unit = Some(name);
            }
            command
                .stdin(if piped { Stdio::piped() } else { Stdio::null() })
                .stdout(stdout)
                .stderr(stderr);
            #[cfg(unix)]
            {
                use std::os::unix::process::CommandExt;
                command.process_group(0);
            }
            let child = command.spawn()?;
            let group = child.id();
            Ok(Self {
                child,
                group,
                unit,
                finished: false,
            })
        }

        pub(crate) fn id(&self) -> u32 {
            self.group
        }
        pub(crate) fn take_stdin(&mut self) -> Option<ChildStdin> {
            self.child.stdin.take()
        }
        pub(crate) fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
            self.child.try_wait()
        }

        pub(crate) fn finish(&mut self) -> Value {
            let started = Instant::now();
            let mut attempts = Vec::new();
            #[cfg(unix)]
            if let Some(pid) = rustix::process::Pid::from_raw(self.group as i32) {
                attempts.push(json!({"operation":"kill_process_group", "error":rustix::process::kill_process_group(pid, rustix::process::Signal::KILL).err().map(|error|error.to_string())}));
            }
            attempts.push(json!({"operation":"kill_parent", "error":self.child.kill().err().map(|error|error.to_string())}));
            let mut scope = None;
            if let Some(unit) = &self.unit {
                let before =
                    control(&["--user", "show", "--property=ControlGroup", "--value", unit]);
                let group = before
                    .as_ref()
                    .ok()
                    .filter(|output| output.status.success())
                    .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned());
                attempts.push(json!({"operation":"kill_scope", "control":observation(&control(&["--user", "kill", "--kill-whom=all", "--signal=SIGKILL", unit]))}));
                attempts.push(json!({"operation":"stop_scope", "control":observation(&control(&["--user", "stop", unit]))}));
                let after = control(&["--user", "show", "--property=ActiveState", "--value", unit]);
                let state = after
                    .as_ref()
                    .ok()
                    .filter(|output| output.status.success())
                    .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned());
                let members = group
                    .as_deref()
                    .filter(|path| path.starts_with('/') && !path.contains(".."))
                    .map(|path| fs::read_to_string(format!("/sys/fs/cgroup{path}/cgroup.procs")));
                let discovered = scope_members(unit);
                let empty = discovered.as_ref().is_ok_and(|groups| {
                    groups.iter().all(|(_, members)| members.trim().is_empty())
                });
                scope = Some(
                    json!({"unit":unit,"control_group":group,"active_state":state,
                "control_group_observation":observation(&before),"active_state_observation":observation(&after),
                "members":members.map(|value| match value { Ok(text)=>json!({"text":text}),Err(error)=>json!({"error":error.to_string()}) }),
                "scope_cgroups":discovered.as_ref().ok(),"scope_cgroup_error":discovered.err().map(|error|error.to_string()),
                "confirmed":empty && matches!(state.as_deref(),Some("inactive"|"failed"))}),
                );
            }
            let (parent_terminal, live) = loop {
                let parent_terminal = self.child.try_wait().is_ok_and(|status| status.is_some());
                #[cfg(target_os = "linux")]
                let live = live_group(self.group)
                    .unwrap_or_else(|error| vec![format!("inspection failed: {error}")]);
                #[cfg(not(target_os = "linux"))]
                let live = vec!["full process inspection unsupported".to_owned()];
                if parent_terminal && live.is_empty() {
                    break (parent_terminal, live);
                }
                if started.elapsed() >= Duration::from_secs(5) {
                    break (parent_terminal, live);
                }
                std::thread::sleep(Duration::from_millis(10));
            };
            let confirmed = parent_terminal
                && live.is_empty()
                && scope
                    .as_ref()
                    .is_some_and(|value| value["confirmed"] == true);
            self.finished = parent_terminal && live.is_empty();
            json!({"state":if confirmed {"confirmed"} else {"unknown"},"method":if self.unit.is_some(){"linux-systemd-scope"}else{"process-group-observation"},
            "parent_pid":self.group,"parent_terminal":parent_terminal,"live_group_members":live,
            "scope":scope,"attempts":attempts,"seconds":started.elapsed().as_secs_f64()})
        }
    }

    #[cfg(target_os = "linux")]
    fn live_group(group: u32) -> io::Result<Vec<String>> {
        let mut members = Vec::new();
        for entry in fs::read_dir("/proc")? {
            let entry = entry?;
            if entry.file_name().to_string_lossy().parse::<u32>().is_err() {
                continue;
            }
            let text = match fs::read_to_string(entry.path().join("stat")) {
                Ok(text) => text,
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::NotFound | io::ErrorKind::PermissionDenied
                    ) =>
                {
                    continue;
                }
                Err(error) => return Err(error),
            };
            if let Some((_, tail)) = text.rsplit_once(") ") {
                let fields = tail.split_whitespace().collect::<Vec<_>>();
                if fields.get(2).and_then(|value| value.parse::<u32>().ok()) == Some(group)
                    && !matches!(fields.first(), Some(&"Z" | &"X"))
                {
                    members.push(entry.file_name().to_string_lossy().into_owned());
                }
            }
        }
        Ok(members)
    }

    impl Drop for OwnedProcess {
        fn drop(&mut self) {
            if !self.finished {
                let _ = self.finish();
            }
        }
    }

    #[cfg(target_os = "linux")]
    fn scope_members(unit: &str) -> io::Result<Vec<(String, String)>> {
        let membership = fs::read_to_string("/proc/self/cgroup")?;
        let hierarchy = membership
            .lines()
            .find_map(|line| line.strip_prefix("0::"))
            .ok_or_else(|| io::Error::other("unified cgroup membership unavailable"))?;
        let mut root = std::path::PathBuf::from("/sys/fs/cgroup");
        let mut manager = false;
        for part in std::path::Path::new(hierarchy).components() {
            if let std::path::Component::Normal(part) = part {
                root.push(part);
                if part.to_string_lossy().starts_with("user@")
                    && part.to_string_lossy().ends_with(".service")
                {
                    manager = true;
                    break;
                }
            }
        }
        if !manager {
            return Err(io::Error::other("user manager cgroup unavailable"));
        }
        let started = Instant::now();
        let mut stack = vec![(root, false)];
        let mut groups = Vec::new();
        while let Some((path, parent_scope)) = stack.pop() {
            if started.elapsed() > Duration::from_secs(2) {
                return Err(io::Error::other("cgroup inspection deadline"));
            }
            let in_scope = parent_scope || path.file_name().is_some_and(|name| name == unit);
            if in_scope {
                groups.push((
                    path.to_string_lossy().into_owned(),
                    match fs::read_to_string(path.join("cgroup.procs")) {
                        Ok(text) => text,
                        Err(error) if error.kind() == io::ErrorKind::NotFound => String::new(),
                        Err(error) => return Err(error),
                    },
                ));
            }
            let entries = match fs::read_dir(&path) {
                Ok(entries) => entries,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error),
            };
            for entry in entries {
                let entry = entry?;
                if entry.file_type()?.is_dir() {
                    stack.push((entry.path(), in_scope));
                }
            }
        }
        Ok(groups)
    }

    #[cfg(not(target_os = "linux"))]
    fn scope_members(_: &str) -> io::Result<Vec<(String, String)>> {
        Err(io::Error::other("Linux cgroup proof unavailable"))
    }
}
