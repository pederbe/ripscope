//! `-l`: a session answering requests from standard input until `q` or its end: in
//! cscope's line protocol as Vim's connection reads it (with Vim's RETURN prompt for
//! problems under `--vim-errors`), or in JSON frames.

use super::answer::{self, Unusable};
use super::{Updated, output_failed, prepare, update};
use crate::engine::Reads;
use crate::invocation::Invocation;
use crate::query::{IdentifierCase, LIMITS};
use crate::refresh::Trust;
use crate::revision::{FileIdentity, HeldIndex, Index, Revision, Units};
use crate::watch::Watch;
use std::io::{self, BufRead, Write};
use std::path::Path;

mod replies;
use replies::{AfterReply, Replies, Response};

/// What a session reads next.
#[derive(Debug, PartialEq, Eq)]
enum Request {
    Line(Vec<u8>),
    /// A line longer than any request, read to its end.
    TooLong,
    End,
}

/// Reads one request line, without its line ending. A line longer than a field digit
/// and the longest pattern is read to its end and refused whole, so its remainder never
/// becomes further requests (cscope's 252-byte buffer would run it as more commands).
fn read_request(input: &mut impl BufRead) -> io::Result<Request> {
    let mut line = Vec::new();
    let mut too_long = false;
    loop {
        let available = input.fill_buf()?;
        if available.is_empty() {
            if line.is_empty() && !too_long {
                return Ok(Request::End);
            }
            break;
        }
        let end = available.iter().position(|byte| *byte == b'\n');
        let taken = end.unwrap_or(available.len());
        if !too_long {
            line.extend_from_slice(&available[..taken]);
            // Room for a field digit, the pattern and the CR of a CR LF.
            too_long = line.len() > LIMITS.input_bytes + 2;
        }
        input.consume(taken + usize::from(end.is_some()));
        if end.is_some() {
            break;
        }
    }
    if too_long {
        return Ok(Request::TooLong);
    }
    while line.last().is_some_and(|byte| *byte == b'\r') {
        line.pop();
    }
    Ok(Request::Line(line))
}

/// Whether a session goes on after a step.
enum Flow {
    Continue,
    /// Input ended while Vim was to acknowledge a message.
    Stop,
}

pub(super) fn session(
    invocation: &Invocation,
    root: &Path,
    stdin: &mut impl BufRead,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> Result<(), String> {
    #[cfg(feature = "session-profile")]
    let startup = crate::profile::Operation::start("startup", false);
    let file = FileIdentity::of(&invocation.index);
    // An updating session watches what its index was made from, and refreshes before a
    // query only once something changed.
    let mut watch = invocation.update.then(|| Watch::new(&invocation.index));
    let refreshed = prepare(invocation, root, stderr, &mut watch)?;
    let index = HeldIndex::new(&invocation.index);
    let mut units = Units::with_budget(Units::SESSION);
    // A session starts only on an index it can read, as a one-shot query answers. The
    // paths it reads serve its file searches. They are read from the index the update
    // opened, if it did, before the session holds it.
    let opened = refreshed
        .as_ref()
        .map(|store| Index::Opened(&invocation.index, store));
    Revision::read(opened.unwrap_or((&index).into()), &Reads::Paths, &mut units)?;
    if let Some(store) = refreshed {
        index.adopt(store, file);
    }
    #[cfg(feature = "session-profile")]
    drop(startup);
    Session {
        invocation,
        root,
        input: stdin,
        replies: Replies::new(invocation, stdout, stderr),
        index,
        units,
        watch,
        case: super::case(invocation),
        pending: None,
    }
    .serve()
}

struct Session<'s, I, O, E> {
    invocation: &'s Invocation,
    root: &'s Path,
    input: &'s mut I,
    replies: Replies<'s, O, E>,
    /// The index, held open between queries. Each query reads its current revision.
    index: HeldIndex,
    /// The units its queries decoded, kept for the next ones up to a budget.
    units: Units,
    /// What tells an updating session when to refresh.
    watch: Option<Watch>,
    case: IdentifierCase,
    /// A request that came where Vim's acknowledgement was awaited.
    pending: Option<Vec<u8>>,
}

impl<I: BufRead, O: Write, E: Write> Session<'_, I, O, E> {
    fn serve(mut self) -> Result<(), String> {
        loop {
            self.replies.ready().map_err(output_failed)?;
            if let Some(watch) = &mut self.watch {
                watch.tidy();
            }
            let request = match self.pending.take() {
                Some(line) => Request::Line(line),
                None => read_request(&mut *self.input).map_err(input_failed)?,
            };
            // As cscope, a command is known by its first byte.
            let flow = match request {
                Request::End => return Ok(()),
                Request::TooLong => self.reply(Response::Unusable(&Unusable::new(
                    "the query request is too long",
                )))?,
                Request::Line(line) => match line.as_slice() {
                    [b'q' | b'\x04' | b'\x1a', ..] => return Ok(()),
                    [b'c' | b'\x03', ..] => {
                        self.case = match self.case {
                            IdentifierCase::Sensitive => IdentifierCase::AsciiInsensitive,
                            IdentifierCase::AsciiInsensitive => IdentifierCase::Sensitive,
                        };
                        Flow::Continue
                    }
                    [b'r' | b'R' | b'\x12', ..] => self.refresh()?,
                    [digit @ b'0'..=b'9', pattern @ ..] => self.query(digit - b'0', pattern)?,
                    // cscope's commands to clear and extend its file list; the selection
                    // rule decides the files here. cscope answers them with a blank line.
                    [b'C' | b'F', ..] => self.reply(Response::Unsupported)?,
                    _ => self.reply(Response::Unknown)?,
                },
            };
            if let Flow::Stop = flow {
                return Ok(());
            }
        }
    }

    fn query(&mut self, field: u8, pattern: &[u8]) -> Result<Flow, String> {
        #[cfg(feature = "session-profile")]
        let _request = crate::profile::Operation::start("request", true);
        let query = match answer::compile(field, pattern, self.case, self.invocation.transport) {
            Ok(query) => query,
            Err(unusable) => return self.reply(Response::Unusable(&unusable)),
        };
        let mut notes = Vec::new();
        if self.watch.as_mut().is_some_and(Watch::changed) {
            let (failure, warnings) = self.update(Trust::Metadata);
            notes = warnings;
            if let Some(failure) = failure {
                notes.push(format!(
                    "automatic Refresh did not complete ({failure}); answering from the last published revision, whose freshness is unconfirmed"
                ));
            }
        }
        notes.extend(self.watch.as_mut().and_then(Watch::note));
        let found = answer::find(
            &query,
            self.invocation,
            (&self.index).into(),
            &mut self.units,
        );
        let mut found = match found {
            Ok(found) => found,
            // The session answers from the index as it is now, or not at all. Vim is told why
            // at its prompt, in a line short enough to show.
            Err(answer::Failed::Index(reason)) => {
                return self.reply(Response::IndexUnavailable(&reason));
            }
            Err(answer::Failed::Output(error)) => return Err(output_failed(error)),
        };
        if let Some(watch) = &mut self.watch {
            watch.read(self.units.read());
        }
        found.add_warnings(notes);
        self.reply(Response::Answer(&mut found))
    }

    fn reply(&mut self, response: Response<'_>) -> Result<Flow, String> {
        match self.replies.send(response).map_err(output_failed)? {
            AfterReply::Continue => Ok(Flow::Continue),
            AfterReply::Acknowledge => self.acknowledge(),
        }
    }

    /// Vim shows what it read before this prompt as an error, then sends an empty line.
    /// Another reply is kept as the next request.
    fn acknowledge(&mut self) -> Result<Flow, String> {
        self.replies
            .acknowledgement_prompt()
            .map_err(output_failed)?;
        match read_request(&mut *self.input).map_err(input_failed)? {
            Request::End => Ok(Flow::Stop),
            Request::Line(line) if !line.is_empty() => {
                self.pending = Some(line);
                Ok(Flow::Continue)
            }
            Request::Line(_) | Request::TooLong => Ok(Flow::Continue),
        }
    }

    /// `r`: a refresh, then the index as it now is. A refresh that was skipped, because
    /// another process is updating the index or it is kept where it cannot be updated,
    /// rebuilt nothing: a JSON client is told why, as for one that failed.
    fn refresh(&mut self) -> Result<Flow, String> {
        let (failure, notes) = self.update(super::trust(self.invocation));
        self.reply(Response::Refresh {
            failure: failure.as_deref(),
            notes: &notes,
        })
    }

    fn update(&mut self, trust: Trust) -> (Option<String>, Vec<String>) {
        // A refresh may replace the index, which Windows refuses while it is held.
        self.index.release();
        let file = FileIdentity::of(&self.invocation.index);
        match update(self.invocation, self.root, trust, &mut self.watch) {
            Ok(Updated {
                skipped,
                warnings,
                store,
            }) => {
                if let Some(store) = store.and_then(|store| store.into_reader().ok()) {
                    self.index.adopt(store, file);
                }
                (skipped, warnings)
            }
            Err(failure) => (Some(failure.reason), failure.notes),
        }
    }
}

fn input_failed(error: io::Error) -> String {
    format!("input failed: {error}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    fn requests(input: &[u8]) -> Vec<Request> {
        let mut input = input;
        let mut found = Vec::new();
        loop {
            let request = read_request(&mut input).expect("read");
            if request == Request::End {
                return found;
            }
            found.push(request);
        }
    }

    #[test]
    fn a_request_is_one_line_and_an_over_long_one_is_refused_whole() {
        let longest = [b"1".as_slice(), &[b'x'; 4096], b"\r\n"].concat();
        let over = [b"1".as_slice(), &[b'x'; 4097], b"\r\n"].concat();
        let input = [&longest[..], &over, b"0a\r\r\n", b"q"].concat();
        assert_eq!(
            requests(&input),
            [
                Request::Line(longest[..4097].to_vec()),
                Request::TooLong,
                Request::Line(b"0a".to_vec()),
                Request::Line(b"q".to_vec()),
            ]
        );
    }

    struct FailingReply {
        bytes: Vec<u8>,
        limit: usize,
        flushes: usize,
        failed_flush: Option<usize>,
    }

    impl Write for FailingReply {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            let count = bytes.len().min(self.limit - self.bytes.len());
            if count == 0 {
                return Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed response"));
            }
            self.bytes.extend_from_slice(&bytes[..count]);
            Ok(count)
        }

        fn flush(&mut self) -> io::Result<()> {
            self.flushes += 1;
            if self.failed_flush == Some(self.flushes) {
                Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed response"))
            } else {
                Ok(())
            }
        }
    }

    #[test]
    fn later_response_failures_stop_before_consuming_acknowledgements_or_requests() {
        let work = tempfile::tempdir().unwrap();
        let source = work.path().join("a.c");
        let index = work.path().join("i.db");
        std::fs::write(&source, "int Alpha;\n").unwrap();
        let built = super::super::run(
            &[
                "-bk".into(),
                "-f".into(),
                index.clone().into(),
                source.clone().into(),
            ],
            &mut io::empty(),
            &mut Vec::new(),
            &mut Vec::new(),
        );
        assert_eq!(built, std::process::ExitCode::SUCCESS);
        let args: Vec<OsString> = vec![
            "--vim-errors".into(),
            "-dl".into(),
            "-f".into(),
            index.into(),
        ];
        let invocation = crate::invocation::parse(&args).unwrap();
        let refusal = b">> Unable to search database\nripscope: the query pattern cannot be used\n";
        let prompt = b"Press the RETURN key to continue:";
        for (limit, failed_flush, consumed) in [
            (refusal.len(), None, 3),
            (usize::MAX, Some(2), 3),
            (refusal.len() + prompt.len(), None, 4),
        ] {
            let mut output = FailingReply {
                bytes: Vec::new(),
                limit,
                flushes: 0,
                failed_flush,
            };
            let mut input = io::Cursor::new(b"1[\n\n1Alpha\nq\n");
            let result = session(
                &invocation,
                work.path(),
                &mut input,
                &mut output,
                &mut Vec::new(),
            );
            assert_eq!(result, Err("output failed: closed response".into()));
            assert_eq!(input.position(), consumed);
            assert!(output.bytes.starts_with(refusal));
        }
        // A warning notice fails after initial readiness and answer delivery succeed.
        std::fs::remove_file(source).unwrap();
        let mut output = FailingReply {
            bytes: Vec::new(),
            limit: b">> Unable to search database\n".len(),
            flushes: 0,
            failed_flush: None,
        };
        let mut input = io::Cursor::new(b"4Alpha\n\n1Alpha\nq\n");
        let mut errors = Vec::new();
        let result = session(
            &invocation,
            work.path(),
            &mut input,
            &mut output,
            &mut errors,
        );
        assert_eq!(result, Err("output failed: closed response".into()));
        assert_eq!(input.position(), 7);
        assert_eq!(output.bytes, b">> Unable to search database\n");
        assert!(String::from_utf8_lossy(&errors).contains("cannot read"));
    }
}
