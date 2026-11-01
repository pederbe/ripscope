//! The Language server's Projects: one JSON Session per Project, run on a thread of its
//! own over a pipe, so that each request is answered as an editor integration's session
//! answers it, from the same index and kept up to date the same way (ADR 0014).

use crate::cli::pipeline;
use crate::project::{self, Places, Project};
use serde_json::Value;
use std::ffi::OsString;
use std::io::{self, BufRead, BufReader, PipeReader, PipeWriter, Write};
use std::path::{Path, PathBuf};
use std::thread::JoinHandle;

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

/// What the server tells the editor while opening a Project.
pub(super) trait Progress {
    fn note(&mut self, message: &str);
}

impl Projects {
    pub(super) fn new(places: Places) -> Self {
        Self {
            home: places.home,
            cache: places.cache,
            sessions: Vec::new(),
        }
    }

    /// The Session of FILE's Project, as an editor in CWD finds it: started, after the
    /// Project's first index build if nothing is published yet, when this is its first
    /// request.
    pub(super) fn of(
        &mut self,
        file: &Path,
        cwd: &Path,
        progress: &mut dyn Progress,
    ) -> Result<&mut Session, String> {
        let places = Places {
            cwd: cwd.to_owned(),
            home: self.home.clone(),
            cache: self.cache.clone(),
        };
        let found = project::find(file, &places)?;
        // A Session that ended, as when its index could no longer be read, starts again.
        self.sessions.retain(|session| !session.ended());
        let at = match self
            .sessions
            .iter()
            .position(|session| session.index == found.index)
        {
            Some(at) => at,
            None => {
                self.sessions
                    .push(Session::start(&found, &self.cache, progress)?);
                self.sessions.len() - 1
            }
        };
        Ok(&mut self.sessions[at])
    }

    /// Every Session started so far.
    pub(super) fn opened(&mut self) -> impl Iterator<Item = &mut Session> {
        self.sessions.iter_mut()
    }
}

pub(super) struct Session {
    root: PathBuf,
    index: PathBuf,
    requests: PipeWriter,
    replies: BufReader<PipeReader>,
    /// What the Session wrote on standard error, once it ends.
    thread: Option<JoinHandle<String>>,
    caseless: bool,
}

impl Session {
    fn start(found: &Project, cache: &Path, progress: &mut dyn Progress) -> Result<Self, String> {
        std::fs::create_dir_all(cache)
            .map_err(|error| format!("cannot make {}: {error}", cache.display()))?;
        let mut selection: Vec<OsString> = found.selection.iter().map(OsString::from).collect();
        selection.extend(["-f".into(), found.index.clone().into()]);
        if !pipeline::published(found) {
            progress.note(&format!(
                "indexing {} for the first time",
                found.root.display()
            ));
            let mut errors = Vec::new();
            let args: Vec<OsString> = std::iter::once("-b".into())
                .chain(selection.iter().cloned())
                .collect();
            let status = pipeline::run_in(
                &args,
                &found.root,
                &mut io::empty(),
                &mut io::sink(),
                &mut errors,
            );
            let errors = String::from_utf8_lossy(&errors);
            if status != std::process::ExitCode::SUCCESS {
                return Err(last_line(&errors));
            }
            for warning in errors.lines() {
                progress.note(warning);
            }
        }
        let pipe = |error: io::Error| format!("cannot start a session: {error}");
        let (session_input, requests) = io::pipe().map_err(pipe)?;
        let (replies, mut session_output) = io::pipe().map_err(pipe)?;
        let args: Vec<OsString> = ["--json".into(), "-l".into()]
            .into_iter()
            .chain(selection)
            .collect();
        let root = found.root.clone();
        let thread = std::thread::spawn(move || {
            let mut errors = Vec::new();
            pipeline::run_in(
                &args,
                &root,
                &mut BufReader::new(session_input),
                &mut session_output,
                &mut errors,
            );
            String::from_utf8_lossy(&errors).into_owned()
        });
        let mut session = Self {
            root: found.root.clone(),
            index: found.index.clone(),
            requests,
            replies: BufReader::new(replies),
            thread: Some(thread),
            caseless: false,
        };
        session.ready(&mut Vec::new())?;
        Ok(session)
    }

    fn ended(&self) -> bool {
        self.thread.as_ref().is_none_or(JoinHandle::is_finished)
    }

    /// The rows of a query of FIELD for PATTERN, ignoring letter case if CASELESS.
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
        let mut line = String::new();
        match self.replies.read_line(&mut line) {
            Ok(0) | Err(_) => Err(self.end()),
            Ok(_) => serde_json::from_str(&line)
                .map_err(|error| format!("unexpected session output: {error}")),
        }
    }

    /// Why the Session ended, from what it wrote last on standard error.
    fn end(&mut self) -> String {
        let errors = self
            .thread
            .take()
            .and_then(|thread| thread.join().ok())
            .unwrap_or_default();
        let reason = last_line(&errors);
        if reason.is_empty() {
            "the session ended".to_owned()
        } else {
            reason
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

fn message(frame: &Value) -> Option<String> {
    frame["message"].as_str().map(str::to_owned)
}

fn last_line(errors: &str) -> String {
    let line = errors.lines().last().unwrap_or_default();
    line.strip_prefix("ripscope: ").unwrap_or(line).to_owned()
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
