//! `ripscope lsp`: the Language server, over standard input and output. It answers go
//! to definition, find references, call hierarchy and workspace symbols from the
//! definition, symbol, callers and callees searches, each in the Project of the file it
//! names (ADR 0016).
//!
//! The name under the cursor comes from the text the editor sent for that file, and
//! results from the saved files: the index keeps lines, not columns, so a result's range
//! is its name in that line as the file is now, or the whole line.

mod projects;
mod text;

use projects::{Projects, Row, Session};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::ffi::OsString;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};
use text::Encoding;

const FIELD_SYMBOL: u8 = 0;
const FIELD_DEFINITION: u8 = 1;
const FIELD_CALLEES: u8 = 2;
const FIELD_CALLERS: u8 = 3;

/// The protocol's error codes.
const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;
const SERVER_NOT_INITIALIZED: i64 = -32002;
const REQUEST_FAILED: i64 = -32803;

/// Symbol kinds.
const KIND_ENUM: u64 = 10;
const KIND_FUNCTION: u64 = 12;
const KIND_VARIABLE: u64 = 13;
const KIND_CONSTANT: u64 = 14;
const KIND_STRUCT: u64 = 23;

pub(super) fn run(
    args: &[OsString],
    stdin: &mut impl BufRead,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> ExitCode {
    // Editors' launchers commonly add `--stdio`, the only transport there is.
    if let Some(arg) = args.iter().find(|arg| *arg != "--stdio") {
        let _ = writeln!(
            stderr,
            "ripscope: lsp takes no argument but --stdio, not {}",
            arg.to_string_lossy()
        );
        return ExitCode::FAILURE;
    }
    let started = std::env::current_dir()
        .map_err(|error| format!("working directory: {error}"))
        .and_then(|cwd| Ok((crate::project::Places::from_environment(&cwd)?, cwd)));
    let (places, cwd) = match started {
        Ok(started) => started,
        Err(error) => {
            let _ = writeln!(stderr, "ripscope: {error}");
            return ExitCode::FAILURE;
        }
    };
    let mut server = Server {
        output: stdout,
        encoding: Encoding::Utf16,
        cwd,
        folders: Vec::new(),
        documents: HashMap::new(),
        projects: Projects::new(places),
        initialized: false,
        shut_down: false,
    };
    match server.serve(stdin) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(error) => {
            let _ = writeln!(stderr, "ripscope: {error}");
            ExitCode::FAILURE
        }
    }
}

/// One message's body, or the end of input.
fn read_message(input: &mut impl BufRead) -> Result<Option<Vec<u8>>, String> {
    let mut length = None;
    let mut first = true;
    loop {
        let mut header = String::new();
        let read = input
            .read_line(&mut header)
            .map_err(|error| format!("input failed: {error}"))?;
        if read == 0 {
            return if first {
                Ok(None)
            } else {
                Err("input ended inside a message".into())
            };
        }
        first = false;
        let header = header.trim_end_matches(['\r', '\n']);
        if header.is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':')
            && name.trim().eq_ignore_ascii_case("content-length")
        {
            length = value.trim().parse::<usize>().ok();
        }
    }
    let length = length.ok_or("a message has no Content-Length")?;
    let mut body = vec![0; length];
    input
        .read_exact(&mut body)
        .map_err(|error| format!("input failed: {error}"))?;
    Ok(Some(body))
}

/// A request's failure: an error code and message.
struct Refusal(i64, String);

impl From<String> for Refusal {
    fn from(message: String) -> Self {
        Self(REQUEST_FAILED, message)
    }
}

type Answered = Result<Value, Refusal>;

struct Server<'o, O> {
    output: &'o mut O,
    encoding: Encoding,
    /// The server's directory, where an editor names no workspace folder for a file.
    cwd: PathBuf,
    folders: Vec<PathBuf>,
    /// The text the editor sent for each open file, by URI.
    documents: HashMap<String, String>,
    projects: Projects,
    initialized: bool,
    shut_down: bool,
}

/// How long a request waits for its Project's first index build before it is refused,
/// within the time editors give a request (Helix gives 20 seconds).
const FIRST_BUILD_WAIT: Duration = Duration::from_secs(10);

fn send(output: &mut impl Write, message: &Value) -> io::Result<()> {
    let body = message.to_string();
    write!(output, "Content-Length: {}\r\n\r\n{body}", body.len())?;
    output.flush()
}

/// The Session of FILE in PROJECTS, started if it has none yet, telling OUTPUT about a
/// first index build.
fn open<'p>(
    projects: &'p mut Projects,
    output: &mut impl Write,
    file: &Path,
    cwd: &Path,
) -> Result<&'p mut Session, String> {
    let (session, started) = projects.of(file, cwd)?;
    if started && session.building {
        let note = format!("indexing {} for the first time", session.root.display());
        log(output, &note);
    }
    Ok(session)
}

/// Tells the editor's log MESSAGE.
fn log(output: &mut impl Write, message: &str) {
    let _ = send(
        output,
        &json!({
            "jsonrpc": "2.0",
            "method": "window/logMessage",
            "params": {"type": 3, "message": format!("ripscope: {message}")},
        }),
    );
}

impl<O: Write> Server<'_, O> {
    /// Serves until `exit`, then says whether `shutdown` came first.
    fn serve(&mut self, input: &mut impl BufRead) -> Result<bool, String> {
        while let Some(body) = read_message(input)? {
            let message: Value = match serde_json::from_slice(&body) {
                Ok(message) => message,
                Err(error) => {
                    self.respond(&Value::Null, Err(Refusal(PARSE_ERROR, error.to_string())))?;
                    continue;
                }
            };
            let method = message["method"].as_str().unwrap_or_default();
            let params = &message["params"];
            if method == "exit" {
                return Ok(self.shut_down);
            }
            match message.get("id") {
                Some(id) => {
                    let answered = self.request(method, params);
                    self.respond(id, answered)?;
                }
                None => self.notification(method, params),
            }
        }
        Ok(false)
    }

    fn respond(&mut self, id: &Value, answered: Answered) -> Result<(), String> {
        let message = match answered {
            Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
            Err(Refusal(code, message)) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {"code": code, "message": message},
            }),
        };
        send(self.output, &message).map_err(|error| format!("output failed: {error}"))
    }

    /// Tells the editor's log what a Project's first index build and answers' warnings
    /// say.
    fn note(&mut self, message: &str) {
        log(self.output, message);
    }

    fn request(&mut self, method: &str, params: &Value) -> Answered {
        if method == "initialize" {
            return Ok(self.initialize(params));
        }
        if !self.initialized {
            return Err(Refusal(SERVER_NOT_INITIALIZED, "not initialized".into()));
        }
        if self.shut_down {
            return Err(Refusal(INVALID_REQUEST, "shut down".into()));
        }
        match method {
            "shutdown" => {
                self.shut_down = true;
                Ok(Value::Null)
            }
            "textDocument/definition" => self.definition(params),
            "textDocument/references" => self.references(params),
            "textDocument/prepareCallHierarchy" => self.prepare_call_hierarchy(params),
            "callHierarchy/incomingCalls" => self.incoming_calls(params),
            "callHierarchy/outgoingCalls" => self.outgoing_calls(params),
            "workspace/symbol" => self.workspace_symbols(params),
            _ => Err(Refusal(METHOD_NOT_FOUND, format!("{method} is not served"))),
        }
    }

    fn notification(&mut self, method: &str, params: &Value) {
        let uri = params["textDocument"]["uri"].as_str();
        match (method, uri) {
            ("textDocument/didOpen", Some(uri)) => {
                let text = params["textDocument"]["text"].as_str().unwrap_or_default();
                self.documents.insert(uri.to_owned(), text.to_owned());
                // The Project's Session starts now, and its first index build with it.
                if let Some(file) = text::path_of(uri)
                    && let Err(error) = self.start(&file)
                {
                    self.note(&error);
                }
            }
            // The server asks for whole texts.
            ("textDocument/didChange", Some(uri)) => {
                let changes = params["contentChanges"].as_array();
                if let Some(text) = changes
                    .and_then(|changes| changes.last())
                    .and_then(|change| change["text"].as_str())
                {
                    self.documents.insert(uri.to_owned(), text.to_owned());
                }
            }
            ("textDocument/didClose", Some(uri)) => {
                self.documents.remove(uri);
            }
            ("workspace/didChangeWorkspaceFolders", _) => {
                let event = &params["event"];
                let removed = folders(&event["removed"]).unwrap_or_default();
                self.folders.retain(|folder| !removed.contains(folder));
                self.folders
                    .extend(folders(&event["added"]).unwrap_or_default());
            }
            _ => {}
        }
    }

    fn initialize(&mut self, params: &Value) -> Value {
        let offered = params["capabilities"]["general"]["positionEncodings"].as_array();
        self.encoding = Encoding::chosen(offered.into_iter().flatten().filter_map(Value::as_str));
        self.folders = folders(&params["workspaceFolders"]).unwrap_or_else(|| {
            let root = params["rootUri"].as_str().and_then(text::path_of);
            root.into_iter().collect()
        });
        self.initialized = true;
        json!({
            "capabilities": {
                "positionEncoding": self.encoding.name(),
                "textDocumentSync": {"openClose": true, "change": 1},
                "definitionProvider": true,
                "referencesProvider": true,
                "callHierarchyProvider": true,
                "workspaceSymbolProvider": true,
                "workspace": {"workspaceFolders": {"supported": true, "changeNotifications": true}},
            },
            "serverInfo": {"name": "ripscope", "version": env!("CARGO_PKG_VERSION")},
        })
    }

    /// The editor's directory for FILE: the innermost workspace folder holding it, else
    /// the server's own.
    fn cwd_of(&self, file: &Path) -> PathBuf {
        self.folders
            .iter()
            .filter(|folder| crate::project::inside(file, folder).unwrap_or(false))
            .max_by_key(|folder| folder.components().count())
            .unwrap_or(&self.cwd)
            .clone()
    }

    /// The file a request names, and the name under its cursor, if any.
    fn named(&self, params: &Value) -> Result<(PathBuf, Option<String>), Refusal> {
        let uri = params["textDocument"]["uri"]
            .as_str()
            .ok_or_else(|| Refusal(INVALID_PARAMS, "no textDocument.uri".into()))?;
        let file = text::path_of(uri)
            .ok_or_else(|| Refusal(INVALID_PARAMS, format!("not a file URI: {uri}")))?;
        let position = &params["position"];
        let (Some(line), Some(character)) =
            (position["line"].as_u64(), position["character"].as_u64())
        else {
            return Err(Refusal(INVALID_PARAMS, "no position".into()));
        };
        let saved;
        let document = match self.documents.get(uri) {
            Some(document) => document,
            None => {
                saved = read(&file)
                    .map_err(|error| format!("cannot read {}: {error}", file.display()))?;
                &saved
            }
        };
        let name = usize::try_from(line)
            .ok()
            .and_then(|line| text::line(document, line))
            .and_then(|line| {
                let character = u32::try_from(character).unwrap_or(u32::MAX);
                text::name_at(line, self.encoding.byte(line, character))
            })
            .map(str::to_owned);
        Ok((file, name))
    }

    /// Starts FILE's Session if it has none yet, telling the editor about a first index
    /// build.
    fn start(&mut self, file: &Path) -> Result<(), String> {
        let cwd = self.cwd_of(file);
        open(&mut self.projects, self.output, file, &cwd).map(|_| ())
    }

    /// FILE's Session once it is ready. A first index build that takes longer than
    /// editors wait refuses the request, and a later one is answered once it is done.
    fn session(&mut self, file: &Path) -> Result<&mut Session, Refusal> {
        let cwd = self.cwd_of(file);
        let session = open(&mut self.projects, self.output, file, &cwd)?;
        let ready = session.ready_by(Instant::now() + FIRST_BUILD_WAIT);
        for warning in session.warnings.drain(..) {
            log(self.output, &warning);
        }
        if ready? {
            return Ok(session);
        }
        Err(Refusal(
            REQUEST_FAILED,
            format!(
                "still indexing {} for the first time. Try again shortly.",
                session.root.display()
            ),
        ))
    }

    /// The rows of a query in FILE's Project, telling the editor its warnings.
    fn query(&mut self, file: &Path, field: u8, name: &str) -> Result<Vec<Row>, Refusal> {
        let answer = self.session(file)?.query(field, name, false)?;
        for warning in &answer.warnings {
            self.note(warning);
        }
        Ok(answer.rows)
    }

    fn definition(&mut self, params: &Value) -> Answered {
        let (file, Some(name)) = self.named(params)? else {
            return Ok(Value::Null);
        };
        let rows = self.query(&file, FIELD_DEFINITION, &name)?;
        let mut lines = Lines::new(self.encoding);
        Ok(rows.iter().map(|row| lines.location(row, &name)).collect())
    }

    fn references(&mut self, params: &Value) -> Answered {
        let (file, Some(name)) = self.named(params)? else {
            return Ok(Value::Null);
        };
        let mut rows = self.query(&file, FIELD_SYMBOL, &name)?;
        if params["context"]["includeDeclaration"] == false {
            let definitions = self.query(&file, FIELD_DEFINITION, &name)?;
            rows.retain(|row| {
                !definitions
                    .iter()
                    .any(|definition| definition.path == row.path && definition.line == row.line)
            });
        }
        let mut lines = Lines::new(self.encoding);
        Ok(rows.iter().map(|row| lines.location(row, &name)).collect())
    }

    fn prepare_call_hierarchy(&mut self, params: &Value) -> Answered {
        let (file, Some(name)) = self.named(params)? else {
            return Ok(Value::Null);
        };
        let rows = self.query(&file, FIELD_DEFINITION, &name)?;
        let mut lines = Lines::new(self.encoding);
        let mut items = Vec::new();
        for row in &rows {
            if lines.kind(row, &name) == KIND_FUNCTION {
                items.push(lines.item(row, &name));
            }
        }
        Ok(if items.is_empty() {
            Value::Null
        } else {
            items.into()
        })
    }

    /// The function a call hierarchy request's item names, and its file.
    fn item(params: &Value) -> Result<(PathBuf, String), Refusal> {
        let item = &params["item"];
        let file = item["uri"].as_str().and_then(text::path_of);
        let name = item["data"]["name"].as_str().or(item["name"].as_str());
        match (file, name) {
            (Some(file), Some(name)) => Ok((file, name.to_owned())),
            _ => Err(Refusal(INVALID_PARAMS, "not a call hierarchy item".into())),
        }
    }

    /// The call hierarchy item of function NAME, called or calling at ROW: its
    /// definition, the one in ROW's file if there are several, else ROW itself. A
    /// context such as `<global>` names no function to look for.
    fn definition_item(
        &mut self,
        lines: &mut Lines,
        definitions: &mut HashMap<String, Vec<Row>>,
        row: &Row,
        name: &str,
    ) -> Result<Value, Refusal> {
        if !definitions.contains_key(name) {
            let found = if name.starts_with('<') {
                Vec::new()
            } else {
                self.query(&row.path, FIELD_DEFINITION, name)?
            };
            definitions.insert(name.to_owned(), found);
        }
        let found = &definitions[name];
        let definition = found
            .iter()
            .find(|definition| definition.path == row.path)
            .or(found.first())
            .unwrap_or(row);
        Ok(lines.item(definition, name))
    }

    /// The calls of ROWS, one per function at their other end, with the ranges of the
    /// calls: in a caller's file for incoming calls, where NAME is called, and in the
    /// item's file for outgoing ones, where each callee is.
    fn calls(&mut self, rows: &[Row], direction: Direction, name: &str) -> Answered {
        let mut lines = Lines::new(self.encoding);
        let mut definitions = HashMap::new();
        let mut calls: Vec<(Value, Vec<Value>)> = Vec::new();
        let mut ends: Vec<(&Path, &str)> = Vec::new();
        for row in rows {
            let (end, called) = match direction {
                // Calls from one function, in one file, are one incoming call.
                Direction::Incoming => ((row.path.as_path(), row.context.as_str()), name),
                Direction::Outgoing => ((Path::new(""), row.context.as_str()), &*row.context),
            };
            let range = lines.range(row, called);
            match ends.iter().position(|known| *known == end) {
                Some(at) => calls[at].1.push(range),
                None => {
                    let item =
                        self.definition_item(&mut lines, &mut definitions, row, &row.context)?;
                    ends.push(end);
                    calls.push((item, vec![range]));
                }
            }
        }
        let key = match direction {
            Direction::Incoming => "from",
            Direction::Outgoing => "to",
        };
        Ok(calls
            .into_iter()
            .map(|(item, ranges)| json!({key: item, "fromRanges": ranges}))
            .collect())
    }

    fn incoming_calls(&mut self, params: &Value) -> Answered {
        let (file, name) = Self::item(params)?;
        let rows = self.query(&file, FIELD_CALLERS, &name)?;
        self.calls(&rows, Direction::Incoming, &name)
    }

    fn outgoing_calls(&mut self, params: &Value) -> Answered {
        let (file, name) = Self::item(params)?;
        let mut rows = self.query(&file, FIELD_CALLEES, &name)?;
        // Functions of one name in several files are told apart by the item's file.
        if rows.iter().any(|row| row.path == file) {
            rows.retain(|row| row.path == file);
        }
        self.calls(&rows, Direction::Outgoing, &name)
    }

    /// Definitions whose name holds the query, ignoring letter case, in every Project
    /// opened so far whose first index build is done in time. An empty query, or one no
    /// C name can hold, finds nothing.
    fn workspace_symbols(&mut self, params: &Value) -> Answered {
        let query = params["query"].as_str().unwrap_or_default();
        if !text::name_like(query) {
            return Ok(json!([]));
        }
        let pattern = format!(".*{query}.*");
        let lowered = query.to_ascii_lowercase();
        let deadline = Instant::now() + FIRST_BUILD_WAIT;
        let mut answers = Vec::new();
        let mut notes = Vec::new();
        for session in self.projects.opened() {
            let ready = session.ready_by(deadline);
            notes.append(&mut session.warnings);
            match ready {
                Ok(true) => answers.push(session.query(FIELD_DEFINITION, &pattern, true)?),
                Ok(false) => notes.push(format!(
                    "workspace symbols leave out {}, which is still being indexed",
                    session.root.display()
                )),
                Err(error) => notes.push(error),
            }
        }
        notes.extend(answers.iter().flat_map(|answer| answer.warnings.clone()));
        for note in &notes {
            self.note(note);
        }
        let mut lines = Lines::new(self.encoding);
        let mut symbols = Vec::new();
        for row in answers.iter().flat_map(|answer| &answer.rows) {
            let Some(name) = defined_name(&row.text, &lowered) else {
                continue;
            };
            symbols.push(json!({
                "name": name,
                "kind": lines.kind(row, name),
                "location": lines.location(row, name),
            }));
        }
        Ok(symbols.into())
    }
}

#[derive(Clone, Copy)]
enum Direction {
    Incoming,
    Outgoing,
}

/// The parsed folders of a list of workspace folders.
fn folders(list: &Value) -> Option<Vec<PathBuf>> {
    let list = list.as_array()?;
    Some(
        list.iter()
            .filter_map(|folder| text::path_of(folder["uri"].as_str()?))
            .collect(),
    )
}

/// FILE's text, as best it reads as UTF-8.
fn read(file: &Path) -> io::Result<String> {
    std::fs::read(file).map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
}

/// The C names in LINE, in order.
fn names(line: &str) -> impl Iterator<Item = &str> {
    line.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .filter(|name| {
            name.as_bytes()
                .first()
                .is_some_and(|first| !first.is_ascii_digit())
        })
}

/// The name a definition's LINE defines that holds LOWERED, ignoring case. A definition
/// row's context is the pattern, so the name is read from the line: the first such name
/// that a `(` follows, as a function's or a macro's, else the last before any `=`, `(`
/// or `{`, past type names such as `struct greeter`.
fn defined_name<'l>(line: &'l str, lowered: &str) -> Option<&'l str> {
    let declarator = line.split(['=', '(', '{']).next().unwrap_or(line);
    let matching = |name: &&str| name.to_ascii_lowercase().contains(lowered);
    let called = names(line).filter(matching).find(|name| {
        let (_, end) = text::find_name(line, name).unwrap_or_default();
        line[end..].trim_start().starts_with('(')
    });
    called.or_else(|| names(declarator).filter(matching).last())
}

/// The saved files' lines that results' ranges are found in, each file read once.
struct Lines {
    encoding: Encoding,
    files: HashMap<PathBuf, Option<String>>,
}

impl Lines {
    fn new(encoding: Encoding) -> Self {
        Self {
            encoding,
            files: HashMap::new(),
        }
    }

    /// ROW's line as the file is now.
    fn line(&mut self, row: &Row) -> Option<&str> {
        let file = self
            .files
            .entry(row.path.clone())
            .or_insert_with(|| read(&row.path).ok());
        text::line(file.as_deref()?, row.line.checked_sub(1)?)
    }

    /// The range of NAME in ROW's line, else the whole line.
    fn range(&mut self, row: &Row, name: &str) -> Value {
        let encoding = self.encoding;
        let line = self.line(row).unwrap_or_default();
        let (start, end) = text::find_name(line, name).unwrap_or((0, line.len()));
        let at = |byte: usize| json!({"line": row.line.saturating_sub(1), "character": encoding.character(line, byte)});
        json!({"start": at(start), "end": at(end)})
    }

    fn location(&mut self, row: &Row, name: &str) -> Value {
        json!({"uri": text::uri_of(&row.path), "range": self.range(row, name)})
    }

    /// What NAME's definition in ROW defines, as its line reads.
    fn kind(&mut self, row: &Row, name: &str) -> u64 {
        let line = self.line(row).unwrap_or_default();
        let Some((start, end)) = text::find_name(line, name) else {
            return KIND_VARIABLE;
        };
        let before: Vec<&str> = names(&line[..start]).collect();
        let after = line[end..].trim_start();
        if after.starts_with('(') {
            KIND_FUNCTION
        } else if line.trim_start().starts_with('#') {
            KIND_CONSTANT
        } else if before.last().is_some_and(|word| *word == "enum") {
            KIND_ENUM
        } else if before
            .iter()
            .any(|word| matches!(*word, "struct" | "union" | "typedef"))
        {
            KIND_STRUCT
        } else {
            KIND_VARIABLE
        }
    }

    /// The call hierarchy item of function NAME defined at ROW.
    fn item(&mut self, row: &Row, name: &str) -> Value {
        let selection = self.range(row, name);
        let encoding = self.encoding;
        let line = self.line(row).unwrap_or_default();
        let whole = json!({
            "start": {"line": row.line.saturating_sub(1), "character": 0},
            "end": {"line": row.line.saturating_sub(1), "character": encoding.character(line, line.len())},
        });
        json!({
            "name": name,
            "kind": KIND_FUNCTION,
            "uri": text::uri_of(&row.path),
            "range": whole,
            "selectionRange": selection,
            "data": {"name": name},
        })
    }
}

#[cfg(test)]
mod tests {
    use super::defined_name;

    #[test]
    fn a_definition_names_what_it_defines_not_its_type() {
        assert_eq!(
            defined_name("struct greeter *greet(void)", "gree"),
            Some("greet")
        );
        assert_eq!(
            defined_name("struct greeter greet_all = {", "gree"),
            Some("greet_all")
        );
        assert_eq!(defined_name("#define GREET(x) x", "gree"), Some("GREET"));
        assert_eq!(defined_name("#define GREETING 1", "gree"), Some("GREETING"));
        assert_eq!(defined_name("int count;", "gree"), None);
    }
}
