//! One owned JSON session, with file-backed framing and retained partial answers.
use crate::capture::{self, owned_process::OwnedProcess};
use serde_json::{Value, json};
use std::fs::File;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{self, Sender};
use std::time::{Duration, Instant};

type WriteRequest = (Vec<u8>, Sender<io::Result<()>>);

pub(crate) struct Session {
    process: OwnedProcess,
    input: Option<Sender<WriteRequest>>,
    writer: std::thread::JoinHandle<()>,
    output: File,
    pending: Vec<u8>,
    prefix: PathBuf,
    deadline: Duration,
    limits: Vec<(PathBuf, u64)>,
    transcript: Vec<Value>,
    cleanup: Option<Value>,
    startup_error: Option<String>,
    expires: Instant,
}

#[derive(Clone, Copy)]
enum Response {
    Startup,
    Query,
    Refresh,
}

impl Session {
    pub(crate) fn start(
        command: Command,
        prefix: &Path,
        deadline: Duration,
        limits: &[(PathBuf, u64)],
    ) -> io::Result<Self> {
        let launched = OwnedProcess::spawn(
            command,
            File::create(prefix.with_extension("stdout"))?,
            File::create(prefix.with_extension("stderr"))?,
            true,
        );
        let mut process = match launched {
            Ok(process) => process,
            Err(error) => {
                capture::save(
                    &prefix.with_extension("cleanup.json"),
                    &json!({"startup_error":error.to_string(),"cleanup":{"state":"confirmed","method":"no-native-process-created"},"stdout_sha256":capture::digest(&prefix.with_extension("stdout"))?,"stderr_sha256":capture::digest(&prefix.with_extension("stderr"))?}),
                )?;
                return Err(error);
            }
        };
        let launched = Instant::now();
        let stdin = (|| -> io::Result<_> {
            loop {
                if let Some(stdin) = process.take_stdin() {
                    return Ok(stdin);
                }
                if process.try_wait()?.is_some() || launched.elapsed() >= deadline {
                    return Err(io::Error::other(
                        "session launch failed or deadline elapsed",
                    ));
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        })();
        let mut stdin = match stdin {
            Ok(stdin) => stdin,
            Err(error) => {
                let cleanup = process.finish();
                capture::save(
                    &prefix.with_extension("cleanup.json"),
                    &json!({"startup_error":error.to_string(),"cleanup":cleanup,"stdout_sha256":capture::digest(&prefix.with_extension("stdout"))?,"stderr_sha256":capture::digest(&prefix.with_extension("stderr"))?}),
                )?;
                return Err(error);
            }
        };
        let (input, receiver) = mpsc::channel::<WriteRequest>();
        let writer = std::thread::spawn(move || {
            while let Ok((bytes, outcome)) = receiver.recv() {
                let result = stdin.write_all(&bytes).and_then(|()| stdin.flush());
                let failed = result.is_err();
                let _ = outcome.send(result);
                if failed {
                    break;
                }
            }
        });
        let mut session = Self {
            process,
            input: Some(input),
            writer,
            output: File::open(prefix.with_extension("stdout"))?,
            pending: Vec::new(),
            prefix: prefix.to_owned(),
            deadline,
            limits: limits.to_vec(),
            transcript: Vec::new(),
            cleanup: None,
            startup_error: None,
            expires: Instant::now() + Duration::from_secs(3600),
        };
        if let Err(error) = session.response(Response::Startup, launched) {
            session.startup_error = Some(error.to_string());
            session
                .transcript
                .push(json!({"startup_error":error.to_string()}));
            session.finish()?;
            return Err(error);
        }
        Ok(session)
    }

    pub(crate) fn limit_until(&mut self, expires: Instant) {
        self.expires = expires;
    }

    fn send(&mut self, command: &str) -> io::Result<()> {
        self.send_with_timeout(command, self.deadline)
    }

    fn send_with_timeout(&mut self, command: &str, timeout: Duration) -> io::Result<()> {
        if command.len() > 4096 || command.contains(['\r', '\n']) {
            return Err(io::Error::other("invalid session command"));
        }
        let (sent, completed) = mpsc::channel();
        self.input
            .as_ref()
            .ok_or_else(|| io::Error::other("session closed"))?
            .send((format!("{command}\n").into_bytes(), sent))
            .map_err(|_| io::Error::other("session writer closed"))?;
        completed
            .recv_timeout(timeout.min(self.expires.saturating_duration_since(Instant::now())))
            .map_err(|_| io::Error::other("session write deadline"))??;
        Ok(())
    }

    pub(crate) fn request(&mut self, command: &str) -> io::Result<Value> {
        let started = Instant::now();
        let result = self.send(command).and_then(|()| {
            self.response(
                if command == "r" {
                    Response::Refresh
                } else {
                    Response::Query
                },
                started,
            )
        });
        self.transcript.push(json!({"command":command,"pid":self.process.id(),"seconds":started.elapsed().as_secs_f64(),
            "response":result.as_ref().ok(),"failure":result.as_ref().err().map(|error|error.to_string())}));
        capture::save(
            &self.prefix.with_extension("transcript.json"),
            &json!(self.transcript),
        )?;
        result
    }

    fn line(&mut self, started: Instant) -> io::Result<Value> {
        loop {
            if let Some(at) = self.pending.iter().position(|byte| *byte == b'\n') {
                let line = self.pending.drain(..=at).collect::<Vec<_>>();
                return serde_json::from_slice(&line).map_err(io::Error::other);
            }
            if started.elapsed() >= self.deadline || Instant::now() >= self.expires {
                return Err(io::Error::other("session response deadline"));
            }
            if let Some(limit) = capture::exceeded(&self.prefix, &self.limits)? {
                return Err(io::Error::other(format!("session {limit}")));
            }
            let mut bytes = [0; 65536];
            let count = self.output.read(&mut bytes)?;
            if count > 0 {
                self.pending.extend_from_slice(&bytes[..count]);
                if self.pending.len() > 8 << 20 {
                    return Err(io::Error::other("session frame limit"));
                }
            } else {
                if self.process.try_wait()?.is_some() {
                    return Err(io::Error::other(
                        "session exited with an incomplete response",
                    ));
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    }

    fn response(&mut self, response: Response, started: Instant) -> io::Result<Value> {
        let mut header = false;
        let mut remaining = 0;
        let mut rows = Vec::new();
        let mut warnings = Vec::new();
        let mut error = None;
        let mut rebuilt = false;
        loop {
            let value = self.line(started)?;
            if value.is_array() {
                if !header || remaining == 0 || !matches!(response, Response::Query) {
                    return Err(io::Error::other("unexpected session row"));
                }
                let row = value.as_array().unwrap();
                if row.len() != 4
                    || !row[0].is_string()
                    || !row[1].is_string()
                    || !row[2].as_u64().is_some_and(|line| line > 0)
                    || !row[3].is_string()
                {
                    return Err(io::Error::other("malformed session row"));
                }
                rows.push(value);
                remaining -= 1;
                continue;
            }
            match value["type"].as_str() {
                Some("ready")
                    if remaining == 0 && (header || matches!(response, Response::Startup)) =>
                {
                    return Ok(
                        json!({"rows":rows,"warnings":warnings,"error":error,"rebuilt":rebuilt}),
                    );
                }
                Some("result") if !header && matches!(response, Response::Query) => {
                    remaining = value["count"]
                        .as_u64()
                        .filter(|count| *count <= 1_000_000)
                        .ok_or_else(|| io::Error::other("invalid session result count"))?;
                    header = true;
                }
                Some("rebuilt") if !header && matches!(response, Response::Refresh) => {
                    header = true;
                    rebuilt = true;
                }
                Some("error") if !header && !matches!(response, Response::Startup) => {
                    error = Some(
                        value["message"]
                            .as_str()
                            .ok_or_else(|| io::Error::other("invalid session error"))?
                            .to_owned(),
                    );
                    header = true;
                }
                Some("warning") if header && remaining == 0 => warnings.push(
                    value["message"]
                        .as_str()
                        .ok_or_else(|| io::Error::other("invalid session warning"))?
                        .to_owned(),
                ),
                _ => return Err(io::Error::other("unexpected session frame")),
            }
        }
    }

    pub(crate) fn finish(&mut self) -> io::Result<Value> {
        if let Some(cleanup) = &self.cleanup {
            return Ok(cleanup.clone());
        }
        let _ = self.send_with_timeout("q", Duration::from_millis(250));
        self.input.take();
        let cleanup = self.process.finish();
        self.cleanup = Some(cleanup.clone());
        capture::save(
            &self.prefix.with_extension("transcript.json"),
            &json!(self.transcript),
        )?;
        capture::save(
            &self.prefix.with_extension("cleanup.json"),
            &json!({"cleanup":cleanup,"startup_error":self.startup_error,"writer_terminal":self.writer.is_finished(),
            "stdout_snapshot":capture::snapshot(&self.prefix.with_extension("stdout"))?,"stderr_snapshot":capture::snapshot(&self.prefix.with_extension("stderr"))?}),
        )?;
        Ok(cleanup)
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}
