//! Reader-specific Session output. Input and acknowledgement state stay with Session.
use super::super::answer::Unusable;
use super::super::one_line;
use crate::invocation::Invocation;
use crate::sink::{Answer, Transport, WarningStyle};
use std::borrow::Cow;
use std::io::{self, Write};

const EMPTY: &[u8] = b"Unable to search database\n";

// Vim keeps about 1,000 bytes before its RETURN prompt. Full warnings precede the
// answer, where its reader passes over them; this short notice follows the rows.
const VIM_NOTICE: &[u8] =
    b"ripscope: this answer may be incomplete; a one-shot query (-L) shows why\n";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Client {
    Line,
    Vim,
    Json,
}

pub(super) enum Response<'a> {
    Answer(&'a mut Answer),
    Unusable(&'a Unusable),
    IndexUnavailable(&'a str),
    Refresh {
        failure: Option<&'a str>,
        notes: &'a [String],
    },
    Unsupported,
    Unknown,
}

pub(super) enum AfterReply {
    Continue,
    Acknowledge,
}

pub(super) struct Replies<'a, O, E> {
    client: Client,
    output: &'a mut O,
    errors: &'a mut E,
}

impl<'a, O: Write, E: Write> Replies<'a, O, E> {
    pub(super) fn new(invocation: &Invocation, output: &'a mut O, errors: &'a mut E) -> Self {
        let client = match (invocation.transport, invocation.vim_errors) {
            (Transport::Json, _) => Client::Json,
            (Transport::Line, true) => Client::Vim,
            (Transport::Line, false) => Client::Line,
        };
        Self {
            client,
            output,
            errors,
        }
    }

    pub(super) fn ready(&mut self) -> io::Result<()> {
        let ready: &[u8] = match self.client {
            Client::Json => b"{\"type\":\"ready\"}\n",
            Client::Line | Client::Vim => b">> ",
        };
        self.output
            .write_all(ready)
            .and_then(|()| self.output.flush())
    }

    pub(super) fn send(&mut self, response: Response<'_>) -> io::Result<AfterReply> {
        match response {
            Response::Answer(answer) => return self.answer(answer),
            Response::Unusable(unusable) => return self.refuse(unusable),
            Response::IndexUnavailable(reason) => {
                let shown: String = vim_safe(Client::Vim, &one_line(reason))
                    .chars()
                    .take(200)
                    .collect();
                return self.refuse(&Unusable {
                    message: Cow::Owned(format!("the index cannot be read: {shown}")),
                    detail: Some(reason.to_owned()),
                });
            }
            Response::Refresh { failure, notes } => match self.client {
                Client::Json => {
                    match failure {
                        Some(error) => self.frame("error", error)?,
                        None => self.output.write_all(b"{\"type\":\"rebuilt\"}\n")?,
                    }
                    for note in notes {
                        self.frame("warning", note)?;
                    }
                }
                Client::Line | Client::Vim => {
                    // cscope puts Refresh diagnostics before its blank response.
                    for note in notes.iter().map(String::as_str).chain(failure) {
                        let _ = writeln!(
                            self.errors,
                            "ripscope: {}",
                            vim_safe(self.client, &one_line(note))
                        );
                    }
                    let _ = self.errors.flush();
                    self.output
                        .write_all(b"\n")
                        .and_then(|()| self.output.flush())?;
                }
            },
            Response::Unsupported => {
                if self.client == Client::Json {
                    self.frame("error", "the C and F commands are not supported")?;
                } else {
                    self.output.write_all(b"\n")?;
                    let _ = writeln!(
                        self.errors,
                        "ripscope: the C and F commands are not supported"
                    );
                }
            }
            Response::Unknown => {
                if self.client == Client::Json {
                    self.frame("error", "unknown command")?;
                } else {
                    let _ = writeln!(self.errors, "ripscope: unknown command");
                }
            }
        }
        Ok(AfterReply::Continue)
    }

    pub(super) fn acknowledgement_prompt(&mut self) -> io::Result<()> {
        self.output
            .write_all(b"Press the RETURN key to continue:")
            .and_then(|()| self.output.flush())
    }

    fn answer(&mut self, answer: &mut Answer) -> io::Result<AfterReply> {
        profile_span!("deliver");
        let warnings = match self.client {
            Client::Line => WarningStyle::Stderr,
            Client::Vim => WarningStyle::Vim,
            Client::Json => WarningStyle::JsonSession,
        };
        answer.deliver(self.output, self.errors, warnings, |out, rows| {
            match self.client {
                Client::Json => writeln!(out, "{{\"type\":\"result\",\"count\":{rows}}}"),
                Client::Line | Client::Vim if rows == 0 => out.write_all(EMPTY),
                Client::Line | Client::Vim => writeln!(out, "cscope: {rows} lines"),
            }
        })?;
        if self.client == Client::Json {
            return Ok(AfterReply::Continue);
        }
        self.output.flush()?;
        if self.client == Client::Vim && answer.has_warnings() {
            self.output.write_all(VIM_NOTICE)?;
            return Ok(AfterReply::Acknowledge);
        }
        Ok(AfterReply::Continue)
    }

    fn refuse(&mut self, unusable: &Unusable) -> io::Result<AfterReply> {
        match self.client {
            Client::Json => self.frame("error", &unusable.reason())?,
            Client::Line => {
                self.output
                    .write_all(EMPTY)
                    .and_then(|()| self.output.flush())?;
                let _ = writeln!(self.errors, "ripscope: {}", one_line(&unusable.reason()));
                let _ = self.errors.flush();
            }
            Client::Vim => {
                self.output.write_all(EMPTY)?;
                writeln!(self.output, "ripscope: {}", unusable.message)?;
                return Ok(AfterReply::Acknowledge);
            }
        }
        Ok(AfterReply::Continue)
    }

    fn frame(&mut self, kind: &str, message: &str) -> io::Result<()> {
        let frame = serde_json::json!({"type": kind, "message": message});
        writeln!(self.output, "{frame}")
    }
}

/// A `>` in text before Vim's prompt could be mistaken for the prompt itself.
fn vim_safe(client: Client, text: &str) -> Cow<'_, str> {
    if client == Client::Vim && text.contains('>') {
        Cow::Owned(text.replace('>', "?"))
    } else {
        Cow::Borrowed(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_failure_keeps_full_detail_but_gives_vim_a_short_safe_message() {
        let reason = format!("first > detail\n{}", "界".repeat(220));
        for option in [None, Some("--vim-errors"), Some("--json")] {
            let mut args = vec!["-dl".into()];
            if let Some(option) = option {
                args.push(std::ffi::OsString::from(option));
            }
            let invocation = crate::invocation::parse(&args).unwrap();
            let mut out = Vec::new();
            let mut errors = Vec::new();
            let after = Replies::new(&invocation, &mut out, &mut errors)
                .send(Response::IndexUnavailable(&reason))
                .unwrap();
            match option {
                Some("--json") => {
                    assert!(matches!(after, AfterReply::Continue));
                    let frame: serde_json::Value = serde_json::from_slice(&out).unwrap();
                    assert_eq!(frame, serde_json::json!({"type":"error","message":reason}));
                    assert!(errors.is_empty());
                }
                Some("--vim-errors") => {
                    assert!(matches!(after, AfterReply::Acknowledge));
                    let text = String::from_utf8(out).unwrap();
                    let shown = text
                        .strip_prefix(
                            "Unable to search database\nripscope: the index cannot be read: ",
                        )
                        .unwrap()
                        .strip_suffix('\n')
                        .unwrap();
                    assert_eq!(shown.chars().count(), 200);
                    assert!(shown.starts_with("first ? detail\\x0a"));
                    assert!(!shown.contains('>'));
                    assert!(errors.is_empty());
                }
                _ => {
                    assert!(matches!(after, AfterReply::Continue));
                    assert_eq!(out, EMPTY);
                    assert_eq!(
                        String::from_utf8(errors).unwrap(),
                        format!("ripscope: first > detail\\x0a{}\n", "界".repeat(220))
                    );
                }
            }
        }
    }
}
