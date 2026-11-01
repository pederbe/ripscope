//! `ripscope lsp`, the Language server: go to definition, find references, call
//! hierarchy and workspace symbols over the Language Server Protocol, each answered by a
//! Session of the file's Project (ADR 0016).
use serde_json::{Value, json};
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

struct Server {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    next: i64,
    /// The `window/logMessage` texts seen so far.
    logs: Vec<String>,
}

impl Server {
    /// The server started in CWD, with a profile of its own at HOME.
    fn start(cwd: &Path, home: &Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_ripscope"))
            .arg("lsp")
            .current_dir(cwd)
            .env_remove("RIPSCOPE_CACHE_DIR")
            .env("HOME", home)
            .env("USERPROFILE", home)
            .env("XDG_CACHE_HOME", home.join("cache"))
            .env("LOCALAPPDATA", home.join("local"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        Self {
            input: child.stdin.take().unwrap(),
            output: BufReader::new(child.stdout.take().unwrap()),
            child,
            next: 1,
            logs: Vec::new(),
        }
    }

    fn send(&mut self, message: &Value) {
        let body = message.to_string();
        write!(self.input, "Content-Length: {}\r\n\r\n{body}", body.len()).unwrap();
        self.input.flush().unwrap();
    }

    fn receive(&mut self) -> Value {
        let mut length = None;
        loop {
            let mut header = String::new();
            assert_ne!(
                self.output.read_line(&mut header).unwrap(),
                0,
                "server ended"
            );
            let header = header.trim_end();
            if header.is_empty() {
                break;
            }
            if let Some(value) = header.strip_prefix("Content-Length: ") {
                length = Some(value.parse::<usize>().unwrap());
            }
        }
        let mut body = vec![0; length.expect("Content-Length")];
        self.output.read_exact(&mut body).unwrap();
        serde_json::from_slice(&body).unwrap()
    }

    fn notify(&mut self, method: &str, params: Value) {
        self.send(&json!({"jsonrpc": "2.0", "method": method, "params": params}));
    }

    /// The response to a request: its `result`, or its `error` as `Err`.
    fn call(&mut self, method: &str, params: Value) -> Result<Value, Value> {
        let id = self.next;
        self.next += 1;
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        loop {
            let message = self.receive();
            if message["id"] == json!(id) {
                return match message.get("error") {
                    Some(error) => Err(error.clone()),
                    None => Ok(message["result"].clone()),
                };
            }
            if message["method"] == "window/logMessage" {
                self.logs
                    .push(message["params"]["message"].as_str().unwrap().to_owned());
            }
        }
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        self.call(method, params)
            .unwrap_or_else(|error| panic!("{method} failed: {error}"))
    }

    fn initialize(&mut self, capabilities: Value, folders: &[&Path]) -> Value {
        let folders: Vec<Value> = folders
            .iter()
            .map(|folder| json!({"uri": uri(folder), "name": "folder"}))
            .collect();
        let result = self.request(
            "initialize",
            json!({"processId": null, "capabilities": capabilities, "workspaceFolders": folders}),
        );
        self.notify("initialized", json!({}));
        result
    }

    fn open(&mut self, path: &Path, text: &str) {
        self.notify(
            "textDocument/didOpen",
            json!({"textDocument": {"uri": uri(path), "languageId": "c", "version": 1, "text": text}}),
        );
    }

    fn stop(mut self) {
        self.request("shutdown", Value::Null);
        self.notify("exit", Value::Null);
        assert!(self.child.wait().unwrap().success());
    }
}

/// A `file:` URI for PATH, as editors send them.
fn uri(path: &Path) -> String {
    let text = path.to_str().unwrap().replace('\\', "/");
    let text = if text.starts_with('/') {
        text
    } else {
        format!("/{text}")
    };
    let mut encoded = String::from("file://");
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"/-._~".contains(&byte) {
            encoded.push(char::from(byte));
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

fn at(path: &Path, line: u32, character: u32) -> Value {
    json!({"textDocument": {"uri": uri(path)}, "position": {"line": line, "character": character}})
}

fn range(line: u32, start: u32, end: u32) -> Value {
    json!({"start": {"line": line, "character": start}, "end": {"line": line, "character": end}})
}

fn location(path: &Path, line: u32, start: u32, end: u32) -> Value {
    json!({"uri": uri(path), "range": range(line, start, end)})
}

fn sorted(mut locations: Value) -> Value {
    locations
        .as_array_mut()
        .unwrap()
        .sort_by_key(|location| location.to_string());
    locations
}

const GREET: &str = "\
#include \"greet.h\"

static int count;

void greet(const char *name)
{
\tcount++;
\tshout(name);
}
";

const MAIN: &str = "\
#include \"greet.h\"

int main(void)
{
\tgreet(\"world\");
\treturn 0;
}
";

const SHOUT: &str = "\
void shout(const char *text)
{
}
";

/// A Git repository of three C files, with the server's home beside it, both under a
/// canonical temporary directory so the paths the server reports compare equal.
struct Fixture {
    _work: tempfile::TempDir,
    home: PathBuf,
    root: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let work = tempfile::tempdir().unwrap();
        // Without Windows's verbatim `\\?\` prefix, which editors do not send.
        let base = fs::canonicalize(work.path()).unwrap();
        let base = match base.to_str().and_then(|base| base.strip_prefix(r"\\?\")) {
            Some(plain) => PathBuf::from(plain),
            None => base,
        };
        let home = base.join("home");
        let root = base.join("project");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::write(root.join("greet.c"), GREET).unwrap();
        fs::write(root.join("main.c"), MAIN).unwrap();
        fs::write(root.join("shout.c"), SHOUT).unwrap();
        Self {
            _work: work,
            home,
            root,
        }
    }

    fn file(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    fn server(&self) -> Server {
        let mut server = Server::start(&self.root, &self.home);
        server.initialize(json!({}), &[&self.root]);
        server
    }
}

#[test]
fn initialize_announces_the_four_requests() {
    let fixture = Fixture::new();
    let mut server = Server::start(&fixture.root, &fixture.home);
    let result = server.initialize(json!({}), &[]);
    let capabilities = &result["capabilities"];
    assert_eq!(capabilities["definitionProvider"], true);
    assert_eq!(capabilities["referencesProvider"], true);
    assert_eq!(capabilities["callHierarchyProvider"], true);
    assert_eq!(capabilities["workspaceSymbolProvider"], true);
    assert_eq!(capabilities["positionEncoding"], "utf-16");
    assert_eq!(result["serverInfo"]["name"], "ripscope");
    server.stop();
}

#[test]
fn definition_builds_the_index_on_first_use_and_finds_the_name_under_the_cursor() {
    let fixture = Fixture::new();
    let mut server = fixture.server();
    let main = fixture.file("main.c");
    server.open(&main, MAIN);
    let found = server.request("textDocument/definition", at(&main, 4, 3));
    assert_eq!(found, json!([location(&fixture.file("greet.c"), 4, 5, 10)]));
    // The index lives in the user's cache, shared with the editor integrations.
    let cache = fixture.home.join("cache").join("ripscope");
    assert!(fs::read_dir(&cache).unwrap().any(|entry| {
        entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with("project-")
    }));
    server.stop();
}

#[test]
fn the_name_under_the_cursor_comes_from_the_text_the_editor_sent() {
    let fixture = Fixture::new();
    let mut server = fixture.server();
    let main = fixture.file("main.c");
    // Unsaved: the call moved down a line, and only the editor knows.
    server.open(&main, &MAIN.replace("{\n", "{\n\n"));
    let found = server.request("textDocument/definition", at(&main, 5, 2));
    assert_eq!(found, json!([location(&fixture.file("greet.c"), 4, 5, 10)]));
    server.notify(
        "textDocument/didChange",
        json!({
            "textDocument": {"uri": uri(&main), "version": 2},
            "contentChanges": [{"text": "int x = count;\n"}],
        }),
    );
    let found = server.request("textDocument/definition", at(&main, 0, 9));
    assert_eq!(
        found,
        json!([location(&fixture.file("greet.c"), 2, 11, 16)])
    );
    // Off any name, there is nothing to find.
    assert_eq!(
        server.request("textDocument/definition", at(&main, 0, 7)),
        Value::Null
    );
    server.stop();
}

#[test]
fn references_come_from_the_symbol_search() {
    let fixture = Fixture::new();
    let mut server = fixture.server();
    let main = fixture.file("main.c");
    server.open(&main, MAIN);
    let mut params = at(&main, 4, 1);
    params["context"] = json!({"includeDeclaration": true});
    assert_eq!(
        sorted(server.request("textDocument/references", params.clone())),
        sorted(json!([
            location(&fixture.file("greet.c"), 4, 5, 10),
            location(&main, 4, 1, 6),
        ]))
    );
    params["context"] = json!({"includeDeclaration": false});
    assert_eq!(
        server.request("textDocument/references", params),
        json!([location(&main, 4, 1, 6)])
    );
    server.stop();
}

#[test]
fn call_hierarchy_comes_from_the_callers_and_callees_searches() {
    let fixture = Fixture::new();
    let mut server = fixture.server();
    let main = fixture.file("main.c");
    let greet = fixture.file("greet.c");
    server.open(&main, MAIN);
    let items = server.request("textDocument/prepareCallHierarchy", at(&main, 4, 1));
    let items = items.as_array().unwrap();
    assert_eq!(items.len(), 1);
    let item = &items[0];
    assert_eq!(item["name"], "greet");
    assert_eq!(item["uri"], uri(&greet));
    assert_eq!(item["selectionRange"], range(4, 5, 10));

    let incoming = server.request("callHierarchy/incomingCalls", json!({"item": item}));
    let incoming = incoming.as_array().unwrap();
    assert_eq!(incoming.len(), 1);
    assert_eq!(incoming[0]["from"]["name"], "main");
    assert_eq!(incoming[0]["from"]["uri"], uri(&main));
    assert_eq!(incoming[0]["from"]["selectionRange"], range(2, 4, 8));
    assert_eq!(incoming[0]["fromRanges"], json!([range(4, 1, 6)]));

    let outgoing = server.request("callHierarchy/outgoingCalls", json!({"item": item}));
    let outgoing = outgoing.as_array().unwrap();
    assert_eq!(outgoing.len(), 1);
    assert_eq!(outgoing[0]["to"]["name"], "shout");
    assert_eq!(outgoing[0]["to"]["uri"], uri(&fixture.file("shout.c")));
    assert_eq!(outgoing[0]["to"]["selectionRange"], range(0, 5, 10));
    assert_eq!(outgoing[0]["fromRanges"], json!([range(7, 1, 6)]));
    server.stop();
}

#[test]
fn workspace_symbols_search_the_projects_opened_so_far_and_see_saved_edits() {
    let fixture = Fixture::new();
    let mut server = fixture.server();
    // No file opened: no Project to search.
    assert_eq!(
        server.request("workspace/symbol", json!({"query": "gree"})),
        json!([])
    );
    let main = fixture.file("main.c");
    server.open(&main, MAIN);
    server.request("textDocument/definition", at(&main, 4, 1));
    let found = server.request("workspace/symbol", json!({"query": "GREE"}));
    let found = found.as_array().unwrap();
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0]["name"], "greet");
    assert_eq!(found[0]["kind"], 12);
    assert_eq!(
        found[0]["location"],
        location(&fixture.file("greet.c"), 4, 5, 10)
    );
    // An empty query lists nothing rather than every definition.
    assert_eq!(
        server.request("workspace/symbol", json!({"query": ""})),
        json!([])
    );

    // A saved edit shows in the next request.
    fs::write(
        fixture.file("shout.c"),
        format!("{SHOUT}\nvoid whisper(void)\n{{\n}}\n"),
    )
    .unwrap();
    let found = server.request("workspace/symbol", json!({"query": "whisper"}));
    assert_eq!(
        found[0]["location"],
        location(&fixture.file("shout.c"), 4, 5, 12)
    );
    server.stop();
}

#[test]
fn utf8_positions_are_used_when_the_editor_offers_them() {
    let fixture = Fixture::new();
    fs::write(
        fixture.file("greet.c"),
        GREET.replace("void greet(", "/* é */ void greet("),
    )
    .unwrap();
    let main = fixture.file("main.c");
    for (offered, encoding, start) in [
        (json!(["utf-8", "utf-16"]), "utf-8", 14),
        (json!(["utf-16"]), "utf-16", 13),
    ] {
        let mut server = Server::start(&fixture.root, &fixture.home);
        let result = server.initialize(
            json!({"general": {"positionEncodings": offered}}),
            &[&fixture.root],
        );
        assert_eq!(result["capabilities"]["positionEncoding"], encoding);
        server.open(&main, MAIN);
        let found = server.request("textDocument/definition", at(&main, 4, 1));
        assert_eq!(
            found,
            json!([location(&fixture.file("greet.c"), 4, start, start + 5)])
        );
        server.stop();
    }
}

#[test]
fn unknown_requests_are_refused_and_the_server_keeps_going() {
    let fixture = Fixture::new();
    let mut server = fixture.server();
    let error = server
        .call("textDocument/hover", at(&fixture.file("main.c"), 0, 0))
        .unwrap_err();
    assert_eq!(error["code"], -32601);
    // A file the editor never opened is read from disk.
    let found = server.request("textDocument/definition", at(&fixture.file("main.c"), 4, 1));
    assert_eq!(found, json!([location(&fixture.file("greet.c"), 4, 5, 10)]));
    server.stop();
}
