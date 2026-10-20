//! Safe Windows Job composition; launcher completion and membership are separate.
use process_wrap::std::{ChildWrapper, CommandWrap, JobObject};
use serde_json::{Value, json};
use std::fs::File;
use std::io;
use std::os::windows::io::AsRawHandle;
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};
use win32job::{ExtendedLimitInfo, Job};

type Launched = io::Result<Box<dyn ChildWrapper>>;

pub(crate) struct OwnedProcess {
    proof: Arc<Job>,
    cancelled: Arc<AtomicBool>,
    accounted: Arc<AtomicBool>,
    native_pids: Arc<Mutex<Vec<u32>>>,
    launcher: mpsc::Receiver<Launched>,
    launch_terminal: bool,
    launch_error: Option<String>,
    child: Option<Box<dyn ChildWrapper>>,
    finished: bool,
    #[cfg(test)]
    query_failure: bool,
}

fn terminate_native(child: &mut Child) -> bool {
    let _ = child.kill();
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(2) {
        if child.try_wait().is_ok_and(|status| status.is_some()) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}

impl OwnedProcess {
    pub(crate) fn spawn(
        mut command: Command,
        stdout: File,
        stderr: File,
        piped: bool,
    ) -> io::Result<Self> {
        #[cfg(test)]
        let query_failure = command.get_envs().any(|(key, value)| {
            key == "WORKFLOW_FIXTURE_QUERY_FAILURE" && value.is_some_and(|value| value == "1")
        });
        command
            .stdin(if piped { Stdio::piped() } else { Stdio::null() })
            .stdout(stdout)
            .stderr(stderr);
        let mut limits = ExtendedLimitInfo::new();
        limits.limit_kill_on_job_close();
        let proof = Arc::new(Job::create_with_limit_info(&limits).map_err(io::Error::other)?);
        let cancelled = Arc::new(AtomicBool::new(false));
        let accounted = Arc::new(AtomicBool::new(true));
        let native_pids = Arc::new(Mutex::new(Vec::new()));
        let (sent, launcher) = mpsc::channel();
        let owned_proof = proof.clone();
        let cancellation = cancelled.clone();
        let ownership = accounted.clone();
        let pids = native_pids.clone();
        std::thread::spawn(move || {
            let mut wrapped = CommandWrap::from(command);
            wrapped.wrap(JobObject);
            let result = wrapped.spawn_with(|command| {
                #[cfg(test)]
                if let Some((_, Some(delay))) = command
                    .get_envs()
                    .find(|(key, _)| *key == "WORKFLOW_FIXTURE_LAUNCH_DELAY_MS")
                    && let Ok(delay) = delay.to_string_lossy().parse::<u64>()
                {
                    std::thread::sleep(Duration::from_millis(delay));
                }
                if cancellation.load(Ordering::Acquire) {
                    return Err(io::Error::new(
                        io::ErrorKind::Interrupted,
                        "launch cancelled before native spawn",
                    ));
                }
                let mut child = command.spawn()?;
                if let Ok(mut pids) = pids.lock() {
                    pids.push(child.id());
                } else {
                    ownership.store(false, Ordering::Release);
                }
                #[cfg(test)]
                if let Some((_, Some(delay))) = command
                    .get_envs()
                    .find(|(key, _)| *key == "WORKFLOW_FIXTURE_ASSIGN_DELAY_MS")
                    && let Ok(delay) = delay.to_string_lossy().parse::<u64>()
                {
                    std::thread::sleep(Duration::from_millis(delay));
                }
                #[cfg(test)]
                if command.get_envs().any(|(key, value)| {
                    key == "WORKFLOW_FIXTURE_STARTUP_FAILURE"
                        && value.is_some_and(|value| value == "assignment")
                }) {
                    ownership.fetch_and(terminate_native(&mut child), Ordering::AcqRel);
                    return Err(io::Error::other(
                        "fixture assignment failure with owned suspended child",
                    ));
                }
                if let Err(error) = owned_proof.assign_process(child.as_raw_handle() as isize) {
                    ownership.fetch_and(terminate_native(&mut child), Ordering::AcqRel);
                    return Err(io::Error::other(error));
                }
                if cancellation.load(Ordering::Acquire) {
                    ownership.fetch_and(terminate_native(&mut child), Ordering::AcqRel);
                    return Err(io::Error::new(
                        io::ErrorKind::Interrupted,
                        "late native child cancelled",
                    ));
                }
                #[cfg(test)]
                if command.get_envs().any(|(key, value)| {
                    key == "WORKFLOW_FIXTURE_STARTUP_FAILURE"
                        && value.is_some_and(|value| value == "resume")
                }) {
                    // Exercise the released wrapper's assignment/resume failure path
                    // using a terminated native child, never an unowned live handle.
                    ownership.fetch_and(terminate_native(&mut child), Ordering::AcqRel);
                }
                Ok(child)
            });
            let mut result = result;
            if cancellation.load(Ordering::Acquire)
                && let Ok(child) = &mut result
            {
                let _ = child.start_kill();
            }
            if let Err(unsent) = sent.send(result)
                && let Ok(mut child) = unsent.0
            {
                let _ = child.start_kill();
            }
            // The last proof handle's kill-on-close covers unresolved late processes.
            // It does not turn an already reported unknown outcome into confirmation.
        });
        Ok(Self {
            proof,
            cancelled,
            accounted,
            native_pids,
            launcher,
            launch_terminal: false,
            launch_error: None,
            child: None,
            finished: false,
            #[cfg(test)]
            query_failure,
        })
    }
    fn members(&self) -> io::Result<Vec<usize>> {
        #[cfg(test)]
        if self.query_failure {
            return Err(io::Error::other("fixture proof membership query failure"));
        }
        self.proof.query_process_id_list().map_err(io::Error::other)
    }

    fn poll_launch(&mut self) {
        if self.launch_terminal {
            return;
        }
        match self.launcher.try_recv() {
            Ok(Ok(mut child)) => {
                if self.cancelled.load(Ordering::Acquire) {
                    let _ = child.start_kill();
                }
                self.child = Some(child);
                self.launch_terminal = true;
            }
            Ok(Err(error)) => {
                self.launch_error = Some(error.to_string());
                self.launch_terminal = true;
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.launch_error =
                    Some("launcher disconnected without an ownership result".into());
                self.accounted.store(false, Ordering::Release);
                self.launch_terminal = true;
            }
            Err(mpsc::TryRecvError::Empty) => (),
        }
    }
    pub(crate) fn id(&self) -> u32 {
        self.native_pids
            .lock()
            .ok()
            .and_then(|pids| pids.first().copied())
            .unwrap_or(0)
    }
    pub(crate) fn take_stdin(&mut self) -> Option<ChildStdin> {
        self.poll_launch();
        self.child.as_mut().and_then(|child| child.stdin().take())
    }
    pub(crate) fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        self.poll_launch();
        if let Some(error) = &self.launch_error {
            return Err(io::Error::other(error.clone()));
        }
        self.child.as_mut().map_or(Ok(None), |child| {
            // JobObject is the only wrapper; its inner child reads native parent
            // status without consulting the wrapper's completion notifications.
            child.inner_mut().try_wait()
        })
    }
    pub(crate) fn finish(&mut self) -> Value {
        self.cancelled.store(true, Ordering::Release);
        let started = Instant::now();
        self.poll_launch();
        let before = self.members();
        let mut kill_error = None;
        let mut last_members = None;
        let mut query_error = None;
        let confirmed = loop {
            self.poll_launch();
            if let Some(child) = &mut self.child
                && let Err(error) = child.start_kill()
            {
                kill_error = Some(error.to_string());
            }
            match self.members() {
                Ok(members) => {
                    let empty = members.is_empty();
                    last_members = Some(members);
                    if self.launch_terminal && self.accounted.load(Ordering::Acquire) && empty {
                        break true;
                    }
                }
                Err(error) => {
                    query_error = Some(error.to_string());
                    break false;
                }
            }
            if started.elapsed() >= Duration::from_secs(5) {
                break false;
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        self.finished = true;
        json!({"state":if confirmed{"confirmed"}else{"unknown"},"method":"windows-proof-job","launch_terminal":self.launch_terminal,
            "native_startup_accounted":self.accounted.load(Ordering::Acquire),"native_pids":self.native_pids.lock().map(|pids|pids.clone()).unwrap_or_default(),
            "launch_error":self.launch_error,"members_before_termination":before.as_ref().ok(),"membership_before_error":before.err().map(|error|error.to_string()),
            "members_after":last_members,"membership_error":query_error,"kill_error":kill_error,"seconds":started.elapsed().as_secs_f64()})
    }
}

impl Drop for OwnedProcess {
    fn drop(&mut self) {
        if !self.finished {
            let _ = self.finish();
        }
    }
}
