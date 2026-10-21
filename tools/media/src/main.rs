//! Capture real Vim terminal screens; playback timing belongs to the renderer.
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
const USAGE: &str = "usage: cargo run --manifest-path tools/media/Cargo.toml -- capture --binary EXE --output NEW-DIR [--repo ROOT]";
const MAIN: &str = include_str!("../fixture/main.c");
const GREET: &str = include_str!("../fixture/greet.c");
const HEADER: &str = include_str!("../fixture/include/greet.h");
const EXPECTED: &str = include_str!("../fixture/expected.json");

fn output(command: &mut Command) -> Result<String> {
    let result = command.output()?;
    if !result.status.success() {
        return Err(format!("{command:?}: {}", String::from_utf8_lossy(&result.stderr)).into());
    }
    Ok(String::from_utf8(result.stdout)?)
}
fn sha(path: &Path) -> Result<String> {
    Ok(base16ct::lower::encode_string(&Sha256::digest(fs::read(
        path,
    )?)))
}
fn ascii_path(path: &Path) -> Result<&str> {
    let value = path.to_str().ok_or("path is not UTF-8")?;
    if !value.starts_with('/')
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"/._-".contains(&byte))
    {
        return Err(
            "the Vim adapter requires absolute ASCII paths using letters, digits, /, ., _ or -"
                .into(),
        );
    }
    Ok(value)
}

struct Vim {
    socket: PathBuf,
}
impl Vim {
    fn tmux(&self, args: &[&str]) -> Result<String> {
        output(Command::new("tmux").arg("-S").arg(&self.socket).args(args))
    }
    fn ex(&self, command: &str) -> Result<()> {
        self.tmux(&["send-keys", "-t", "demo:0.0", "-l", &format!(":{command}")])?;
        self.tmux(&["send-keys", "-t", "demo:0.0", "Enter"])?;
        Ok(())
    }
    fn screen(&self) -> Result<String> {
        self.tmux(&["capture-pane", "-p", "-t", "demo:0.0"])
    }
    fn wait_screen(&self, needle: &str) -> Result<String> {
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            let screen = self.screen()?;
            if screen.contains(needle) {
                return Ok(screen);
            }
            if Instant::now() >= deadline {
                return Err(
                    format!("Vim did not display {needle:?} before deadline:\n{screen}").into(),
                );
            }
            thread::sleep(Duration::from_millis(30));
        }
    }
    fn wait_file(&self, path: &Path) -> Result<String> {
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            // writefile() ends its line with a newline, so a file without one is still being written.
            if let Ok(data) = fs::read_to_string(path)
                && data.ends_with('\n')
            {
                return Ok(data);
            }
            if Instant::now() >= deadline {
                return Err(format!("Vim did not write {}", path.display()).into());
            }
            thread::sleep(Duration::from_millis(30));
        }
    }
    fn capture(&self, dir: &Path, name: &str, needle: &str) -> Result<()> {
        let screen = self.wait_screen(needle)?;
        fs::write(dir.join(format!("{name}.txt")), &screen)?;
        let cursor = self.tmux(&[
            "display-message",
            "-p",
            "-t",
            "demo:0.0",
            "#{cursor_x} #{cursor_y}",
        ])?;
        fs::write(dir.join(format!("{name}.cursor")), cursor)?;
        Ok(())
    }
    fn evidence(&self, dir: &Path, phase: &str) -> Result<Value> {
        let file = dir.join(format!("{phase}.json"));
        self.ex(&format!("call writefile([json_encode({{'connection': execute('cs show'), 'quickfix': getqflist(), 'path': expand('%:t'), 'line': line('.')}})], '{}')", ascii_path(&file)?))?;
        Ok(serde_json::from_str(&self.wait_file(&file)?)?)
    }
}
impl Drop for Vim {
    fn drop(&mut self) {
        // This socket belongs only to this capture; no other tmux server is touched.
        let _ = self.tmux(&["kill-server"]);
        let _ = fs::remove_file(&self.socket);
    }
}

fn run() -> Result<()> {
    if !cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        return Err("actual editor capture currently requires native Linux x86-64".into());
    }
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() != Some("capture") {
        return Err(USAGE.into());
    }
    let mut binary = None;
    let mut destination = None;
    let mut root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    while let Some(flag) = args.next() {
        let value = PathBuf::from(args.next().ok_or(USAGE)?);
        match flag.as_str() {
            "--binary" => binary = Some(value.canonicalize()?),
            "--output" => destination = Some(value),
            "--repo" => root = value.canonicalize()?,
            _ => return Err(USAGE.into()),
        }
    }
    let binary = binary.ok_or(USAGE)?;
    let destination = destination.ok_or(USAGE)?;
    if destination.exists() {
        return Err("output directory must be new; existing evidence is never replaced".into());
    }
    let parent = destination
        .parent()
        .ok_or("output directory needs a parent")?
        .canonicalize()?;
    let destination = parent.join(destination.file_name().ok_or("missing output name")?);
    ascii_path(&binary)?;
    ascii_path(&destination)?;
    let adapter = root.join("plugin/ripscope.vim").canonicalize()?;
    ascii_path(&adapter)?;
    fs::create_dir(&destination)?;
    let project = destination.join("hello");
    fs::create_dir_all(project.join("include"))?;
    fs::write(project.join("main.c"), MAIN)?;
    fs::write(project.join("greet.c"), GREET)?;
    fs::write(project.join("include/greet.h"), HEADER)?;
    let expected: Value = serde_json::from_str(EXPECTED)?;
    let index = project.join("project.idx");
    let query = |field: &str| -> Result<String> {
        output(
            Command::new(&binary)
                .current_dir(&project)
                .args(["-dL", "-f"])
                .arg(&index)
                .args([field, "greet"]),
        )
    };
    let init = destination.join("init.vim");
    fs::write(
        &init,
        format!(
            "set nocompatible nomore noswapfile noerrorbells belloff=all\nset number laststatus=2 ruler noshowmode noshowcmd\nset statusline=%t\\ %m%=%l:%c\nset shortmess+=I\nsyntax off\nlet g:ripscope = {{'backend': '{}', 'root': '{}', 'index': '{}', 'sources': ['greet.c', 'main.c'], 'include_dirs': ['include']}}\nsource {}\ncd {}\nedit greet.c\n",
            ascii_path(&binary)?,
            ascii_path(&project)?,
            ascii_path(&index)?,
            ascii_path(&adapter)?,
            ascii_path(&project)?
        ),
    )?;
    let vim = Vim {
        socket: destination.join("tmux.sock"),
    };
    vim.tmux(&[
        "new-session",
        "-d",
        "-x",
        "100",
        "-y",
        "30",
        "-s",
        "demo",
        "vim",
        "-Nu",
        ascii_path(&init)?,
        "-i",
        "NONE",
    ])?;
    vim.tmux(&["set-option", "-g", "status", "off"])?;
    // Dismiss Vim's startup prompt before sending the actual demonstration commands.
    vim.wait_screen("greet.c")?;
    vim.tmux(&["send-keys", "-t", "demo:0.0", "Enter"])?;
    vim.ex("RipscopeBuild")?;
    vim.wait_screen("ripscope: build completed")?;
    let definition = query("-1")?;
    let before_rows = query("-3")?;
    if definition != expected["definition"].as_str().unwrap()
        || before_rows != expected["before"].as_str().unwrap()
    {
        return Err("current backend no longer matches the quick-start fixture; inspect before producing media".into());
    }
    vim.ex("cs find c greet")?;
    vim.wait_screen("(1 of 1)")?;
    let before = vim.evidence(&destination, "before")?;
    if before["path"] != "main.c"
        || before["line"] != 5
        || before["quickfix"].as_array().map(Vec::len) != Some(1)
    {
        return Err("initial Vim query did not navigate to main.c:5 with one caller".into());
    }
    vim.ex("copen 5")?;
    vim.capture(&destination, "01-first-caller", "main.c|5| <<main>>")?;
    vim.ex("cclose")?;
    vim.tmux(&["send-keys", "-t", "demo:0.0", "G", "o"])?;
    vim.tmux(&[
        "send-keys",
        "-t",
        "demo:0.0",
        "-l",
        expected["addition"].as_str().unwrap(),
    ])?;
    vim.tmux(&["send-keys", "-t", "demo:0.0", "Escape"])?;
    vim.capture(&destination, "02-edit", "void helper(void)")?;
    vim.ex("w")?;
    // Read the saved source before proceeding, rather than treating keystrokes as proof.
    let deadline = Instant::now() + Duration::from_secs(8);
    let after_source = format!("{MAIN}{}\n", expected["addition"].as_str().unwrap());
    while fs::read_to_string(project.join("main.c"))? != after_source {
        if Instant::now() >= deadline {
            return Err("Vim did not save the expected helper at main.c:8".into());
        }
        thread::sleep(Duration::from_millis(30));
    }
    vim.capture(&destination, "03-save", "written")?;
    // No index build here: the session refreshes before the first search after a save.
    vim.ex("cs find c greet")?;
    vim.wait_screen("(1 of 2)")?;
    let after = vim.evidence(&destination, "after")?;
    if before["connection"] != after["connection"]
        || after["quickfix"].as_array().map(Vec::len) != Some(2)
        || after["quickfix"][0]["lnum"] != 5
        || after["quickfix"][1]["lnum"] != 8
    {
        return Err(
            "Vim did not retain the same connection and find the two expected callers".into(),
        );
    }
    // Keep useful navigation evidence without publishing temporary paths or PIDs.
    let projection = |record: &Value| {
        json!({
            "path": record["path"], "line": record["line"],
            "quickfix": record["quickfix"].as_array().unwrap().iter().map(|row| {
                json!({"line": row["lnum"], "text": row["text"]})
            }).collect::<Vec<_>>()
        })
    };
    let observation = json!({
        "format": 1,
        "before": projection(&before), "after": projection(&after),
        "byte_identical_connection_records": true,
        "raw_connection_sha256": base16ct::lower::encode_string(&Sha256::digest(before["connection"].as_str().unwrap().as_bytes())),
        "raw_record_sha256": {"before.json": sha(&destination.join("before.json"))?, "after.json": sha(&destination.join("after.json"))?},
        "projection_note": "Quickfix locations and text are copied from raw Vim records. Temporary absolute paths, process IDs and internal buffer fields are omitted. The connection digest covers the exact original cs show bytes, which were compared before projection."
    });
    fs::write(
        destination.join("observation.json"),
        serde_json::to_string_pretty(&observation)? + "\n",
    )?;
    vim.ex("copen 5")?;
    vim.capture(&destination, "04-two-callers", "main.c|8| <<helper>>")?;
    let after_rows = query("-3")?;
    if after_rows != expected["after"].as_str().unwrap() {
        return Err("updated backend rows differ from fixture".into());
    }
    let mut screens = serde_json::Map::new();
    for name in ["01-first-caller", "02-edit", "03-save", "04-two-callers"] {
        for suffix in ["txt", "cursor"] {
            let file = format!("{name}.{suffix}");
            screens.insert(file.clone(), json!(sha(&destination.join(file))?));
        }
    }
    let manifest = json!({
        "format": 1, "platform": "Linux x86-64", "terminal": {"columns": 100, "rows": 30},
        "capture": "Actual Vim terminal screens; playback holds are editorial, not recorded timing.",
        "checkout_revision_at_capture": output(Command::new("git").arg("-C").arg(&root).args(["rev-parse", "HEAD"]))?.trim(),
        "backend": {"version": output(Command::new(&binary).arg("--version"))?.trim(), "sha256": sha(&binary)?, "build_receipt": null},
        "provenance_note": "Checkout HEAD is recorded separately; it is not a binary build receipt. Media demonstrates a bounded workflow, not performance.",
        "recording_toolchain": output(Command::new("rustc").arg("--version"))?.trim(),
        "screen_sha256": screens,
        "observation_sha256": sha(&destination.join("observation.json"))?,
        "commands": ["RipscopeBuild", "cs find c greet", "copen 5", "cclose", "Go; insert helper; Escape", "w", "cs find c greet", "copen 5"],
        "adapter_sha256": sha(&adapter)?,
        "fixture_sha256": {"main.c": base16ct::lower::encode_string(&Sha256::digest(MAIN.as_bytes())), "greet.c": base16ct::lower::encode_string(&Sha256::digest(GREET.as_bytes())), "include/greet.h": base16ct::lower::encode_string(&Sha256::digest(HEADER.as_bytes())), "main.c.after": base16ct::lower::encode_string(&Sha256::digest(after_source.as_bytes()))}, "vim_version": output(Command::new("vim").arg("--version"))?.lines().next(),
        "same_connection": true, "definition": definition, "callers_before": before_rows, "callers_after": after_rows,
        "sources": {"main.c": MAIN, "greet.c": GREET, "include/greet.h": HEADER, "main.c.after": after_source},
        "frames": [
            {"file": "01-first-caller.txt", "label": ":cs find c greet", "hold_seconds": 2.2},
            {"file": "02-edit.txt", "label": "Add a call", "hold_seconds": 1.6},
            {"file": "03-save.txt", "label": ":w", "hold_seconds": 1.2},
            {"file": "04-two-callers.txt", "label": ":cs find c greet", "hold_seconds": 2.8}
        ]
    });
    fs::write(
        destination.join("manifest.json"),
        serde_json::to_string_pretty(&manifest)? + "\n",
    )?;
    fs::write(destination.join("callers-before.txt"), &before_rows)?;
    fs::write(destination.join("callers-after.txt"), &after_rows)?;
    vim.ex("qa!")?;
    drop(vim);
    for suffix in ["", "-wal", "-shm"] {
        let _ = fs::remove_file(project.join(format!("project.idx{suffix}")));
    }
    println!("Actual Vim capture verified in {}", destination.display());
    Ok(())
}
fn main() {
    if let Err(error) = run() {
        eprintln!("media capture: {error}");
        std::process::exit(1);
    }
}
