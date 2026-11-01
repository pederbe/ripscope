//! The Language server's Projects: one JSON Session per Project, run on a thread of its
//! own over a pipe, so that each request is answered as an editor integration's session
//! answers it, from the same index and kept up to date the same way (ADR 0016).
//!
//! A Project's first index build runs on that thread before its Session starts, so
//! the server goes on serving other Projects meanwhile.

use crate::cli::pipeline;
use crate::project::{self, Places, Project};
use serde_json::Value;
use std::ffi::OsString;
use std::io::{self, BufRead, BufReader, PipeWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread::JoinHandle;
use std::time::Instant;

/// One row of an answer.
pub(super) struct Row {
    /// The file, absolute.
    pub path: PathBuf,
    pub context: String,
    /// From 1, as the index counts lines.
    pub line: usize,
    pub text: String,
}

/// A query's rows, with the warnings that came with them.
pub(super) struct Answer {
    pub rows: Vec<Row>,
    pub warnings: Vec<String>,
}

pub(super) struct Projects {
    home: PathBuf,
    cache: PathBuf,
    sessions: Vec<Session>,
}

impl Projects {
    pub(super) fn new(places: Places) -> Self {
        Self {
            home: places.home,
            cache: places.cache,
            sessions: Vec::new(),
        }
    }

    /// The Session of FILE's Project, as an editor in CWD finds it, and whether it was
    /// started for this call, with a first index build if nothing is published yet.
    pub(super) fn of(&mut self, file: &Path, cwd: &Path) -> Result<(&mut Session, bool), String> {
        let places = Places {
            cwd: cwd.to_owned(),
            home: self.home.clone(),
            cache: self.cache.clone(),
        };
        let found = project::find(file, &places)?;
        // A Session that ended, as when its index could no longer be read, starts again.
        self.sessions.retain(|session| !session.ended());
        match self
            .sessions
            .iter()
            .position(|session| session.index == found.index)
        {
            Some(at) => Ok((&mut self.sessions[at], false)),
            None => {
                self.sessions.push(Session::start(&found, &self.cache)?);
                Ok((self.sessions.last_mut().expect("just pushed"), true))
            }
        }
    }

    /// Every Session started so far.
    pub(super) fn opened(&mut self) -> impl Iterator<Item = &mut Session> {
        self.sessions.iter_mut()
    }
}

pub(super) struct Session {
    pub root: PathBuf,
    index: PathBuf,
    /// Whether its Project's first index build runs before it starts.
    pub building: bool,
    /// What its first index build and its start had to tell, until the server tells it.
    pub warnings: Vec<String>,
    requests: PipeWriter,
    /// Its lines, read on a thread of their own so that the server can wait for the
    /// first one with a deadline.
    replies: Receiver<String>,
    /// Whether its first `ready` came.
    started: bool,
    /// What the index build or the Session wrote on standard error, once it ends.
    thread: Option<JoinHandle<String>>,
    caseless: bool,
}

impl Session {
    fn start(found: &Project, cache: &Path) -> Result<Self, String> {
        std::fs::create_dir_all(cache)
            .map_err(|error| format!("cannot make {}: {error}", cache.display()))?;
        let mut selection: Vec<OsString> = found.selection.iter().map(OsString::from).collect();
        selection.extend(["-f".into(), found.index.clone().into()]);
        let building = !pipeline::published(found);
        let pipe = |error: io::Error| format!("cannot start a session: {error}");
        let (session_input, requests) = io::pipe().map_err(pipe)?;
        let (replies, mut session_output) = io::pipe().map_err(pipe)?;
        let root = found.root.clone();
        let (lines, received) = mpsc::channel();
        let built = lines.clone();
        let thread = std::thread::spawn(move || {
            let mut errors = Vec::new();
            if building {
                let build: Vec<OsString> = std::iter::once("-b".into())
                    .chain(selection.iter().cloned())
                    .collect();
                let status = pipeline::run_in(
                    &build,
                    &root,
                    &mut io::empty(),
                    &mut io::sink(),
                    &mut errors,
                );
                if status != std::process::ExitCode::SUCCESS {
                    return String::from_utf8_lossy(&errors).into_owned();
                }
                // Files the build could not use are told as the Session tells warnings: the
                // process holds standard error.
                for warning in String::from_utf8_lossy(&errors).lines() {
                    let warning = warning.strip_prefix("ripscope: ").unwrap_or(warning);
                    let _ = built.send(
                        serde_json::json!({"type": "warning", "message": warning}).to_string(),
                    );
                }
                errors.clear();
            }
            let args: Vec<OsString> = ["--json".into(), "-l".into()]
                .into_iter()
                .chain(selection)
                .collect();
            pipeline::run_in(
                &args,
                &root,
                &mut BufReader::new(session_input),
                &mut session_output,
                &mut errors,
            );
            String::from_utf8_lossy(&errors).into_owned()
        });
        std::thread::spawn(move || {
            for line in BufReader::new(replies).lines() {
                let Ok(line) = line else { break };
                if lines.send(line).is_err() {
                    break;
                }
            }
        });
        Ok(Self {
            root: found.root.clone(),
            index: found.index.clone(),
            building,
            warnings: Vec::new(),
            requests,
            replies: received,
            started: false,
            thread: Some(thread),
            caseless: false,
        })
    }

    fn ended(&self) -> bool {
        self.thread.as_ref().is_none_or(JoinHandle::is_finished)
    }

    /// Whether the Session is ready for queries by DEADLINE: its first index build, if
    /// any, is done, and it has started. Its failure to start is an error.
    pub(super) fn ready_by(&mut self, deadline: Instant) -> Result<bool, String> {
        while !self.started {
            let wait = deadline.saturating_duration_since(Instant::now());
            match self.replies.recv_timeout(wait) {
                Ok(line) => {
                    let frame = frame(&line)?;
                    self.started = frame["type"] == "ready";
                    self.warnings
                        .extend(message(&frame).filter(|_| !self.started));
                }
                Err(RecvTimeoutError::Timeout) => return Ok(false),
                Err(RecvTimeoutError::Disconnected) => return Err(self.end()),
            }
        }
        Ok(true)
    }

    /// The rows of a query of FIELD for PATTERN, ignoring letter case if CASELESS, once
    /// the Session is ready.
    pub(super) fn query(
        &mut self,
        field: u8,
        pattern: &str,
        caseless: bool,
    ) -> Result<Answer, String> {
        let mut warnings = Vec::new();
        if caseless != self.caseless {
            self.send("c")?;
            self.ready(&mut warnings)?;
            self.caseless = caseless;
        }
        self.send(&format!("{field}{pattern}"))?;
        let mut rows = Vec::new();
        let mut failure = None;
        loop {
            let frame = self.frame()?;
            match frame["type"].as_str() {
                Some("ready") => break,
                Some("result") => {
                    let count = frame["count"].as_u64().unwrap_or(0);
                    for _ in 0..count {
                        let row = self.frame()?;
                        rows.push(self.row(&row)?);
                    }
                }
                Some("warning") => warnings.extend(message(&frame)),
                Some("error") => failure = message(&frame),
                _ => {}
            }
        }
        match failure {
            Some(failure) => Err(failure),
            None => Ok(Answer { rows, warnings }),
        }
    }

    fn send(&mut self, request: &str) -> Result<(), String> {
        let sent = writeln!(self.requests, "{request}").and_then(|()| self.requests.flush());
        sent.map_err(|_| self.end())
    }

    /// Reads up to the next `ready`, keeping warnings.
    fn ready(&mut self, warnings: &mut Vec<String>) -> Result<(), String> {
        loop {
            let frame = self.frame()?;
            match frame["type"].as_str() {
                Some("ready") => return Ok(()),
                Some("warning") => warnings.extend(message(&frame)),
                _ => {}
            }
        }
    }

    fn frame(&mut self) -> Result<Value, String> {
        match self.replies.recv() {
            Ok(line) => frame(&line),
            Err(_) => Err(self.end()),
        }
    }

    /// Why the Session ended, from what it wrote last on standard error.
    fn end(&mut self) -> String {
        let errors = self
            .thread
            .take()
            .and_then(|thread| thread.join().ok())
            .unwrap_or_default();
        let line = errors.lines().last().unwrap_or_default();
        let reason = line.strip_prefix("ripscope: ").unwrap_or(line);
        if reason.is_empty() {
            "the session ended".to_owned()
        } else {
            reason.to_owned()
        }
    }

    fn row(&self, row: &Value) -> Result<Row, String> {
        let unexpected = || format!("unexpected session row: {row}");
        let [path, context, line, text] = row.as_array().map(Vec::as_slice).unwrap_or(&[]) else {
            return Err(unexpected());
        };
        Ok(Row {
            path: self.root.join(bytes_path(path).ok_or_else(unexpected)?),
            context: context.as_str().ok_or_else(unexpected)?.to_owned(),
            line: line
                .as_u64()
                .and_then(|line| usize::try_from(line).ok())
                .ok_or_else(unexpected)?,
            text: match text {
                Value::String(text) => text.clone(),
                bytes => {
                    String::from_utf8_lossy(&bytes_of(bytes).ok_or_else(unexpected)?).into_owned()
                }
            },
        })
    }
}

fn frame(line: &str) -> Result<Value, String> {
    serde_json::from_str(line).map_err(|error| format!("unexpected session output: {error}"))
}

fn message(frame: &Value) -> Option<String> {
    frame["message"].as_str().map(str::to_owned)
}

/// A JSON row's byte array.
fn bytes_of(value: &Value) -> Option<Vec<u8>> {
    value
        .as_array()?
        .iter()
        .map(|byte| byte.as_u64().and_then(|byte| u8::try_from(byte).ok()))
        .collect()
}

/// A JSON row's path: a string, or an array of bytes where it is not UTF-8.
fn bytes_path(value: &Value) -> Option<PathBuf> {
    if let Some(text) = value.as_str() {
        return Some(PathBuf::from(text));
    }
    let bytes = bytes_of(value)?;
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        Some(PathBuf::from(OsString::from_vec(bytes)))
    }
    #[cfg(not(unix))]
    {
        String::from_utf8(bytes).ok().map(PathBuf::from)
    }
}
