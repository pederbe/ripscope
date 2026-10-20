//! Independent native subprocess fixture for workflow capture and CLI tests.
use std::io::{self, BufRead, Write};
use std::process::{Command, Stdio};
use std::time::Duration;

#[allow(clippy::zombie_processes)] // Deliberately leave descendants for the owning guard.
fn main() {
    if let Ok(report) = std::env::var("RIPSCOPE_WORKFLOW_REPORT") {
        let parent = std::path::Path::new(&report).parent().unwrap();
        std::fs::write(
            parent.join("editor-environment.txt"),
            if std::env::var("RIPSCOPE_WORKFLOW_VALIDATE_ONLY").as_deref() == Ok("1") {
                "bypassed"
            } else {
                "controlled"
            },
        )
        .unwrap();
        if std::env::var("WORKFLOW_FIXTURE_EDITOR").is_ok_and(|role| {
            role.starts_with("neovim-") || role == "vim-valid" || role == "emacs-valid"
        }) {
            use serde_json::json;
            let root = std::env::var("RIPSCOPE_WORKFLOW_ROOT").unwrap();
            let jump = |line| json!({"kind":"jump","detail":{"file":std::path::Path::new(&root).join("a.c"),"line":line}});
            let row = "[\"a.c\",\"renamed\",1,\"int renamed(void) { return neighbor(); }\"]\n";
            let live = "[\"a.c\",\"<unknown>\",1,\"int renamed(void) { return neighbor(); }\"]\n";
            let unrelated = "[\"unrelated.c\",\"unrelated\",77,\"int unrelated;\"]\n";
            let role = std::env::var("WORKFLOW_FIXTURE_EDITOR").unwrap();
            let operation = |field, pattern, stdout: &str, stderr: &str, message: &str, pid| {
                if role.starts_with("neovim-") {
                    json!({"kind":"operation","detail":{"kind":"query","field":field,"pattern":pattern,"stdout":stdout,"stderr":stderr,"message":message,"session_pid":pid}})
                } else {
                    let rows = stdout
                        .lines()
                        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
                        .collect::<Vec<_>>();
                    json!({"kind":"query","detail":{"field":field,"pattern":pattern,"text":format!("{stderr}\n{message}"),"observed_rows":rows}})
                }
            };
            let last_pid =
                if std::env::var("WORKFLOW_FIXTURE_EDITOR").as_deref() == Ok("neovim-pid-change") {
                    202
                } else {
                    101
                };
            let events = vec![
                jump(1),
                jump(1),
                jump(2),
                jump(1),
                operation(
                    1,
                    "__ripscope_workflow_absent_20261005__",
                    "",
                    "",
                    if role == "emacs-valid" {
                        "No matches were found"
                    } else if role == "vim-valid" {
                        "E259: no matches"
                    } else {
                        "No matches"
                    },
                    101,
                ),
                operation(1, "[", "", "invalid query pattern", "failed", 101),
                operation(4, "renamed", "", "cannot read a.c", "No matches", 101),
                operation(
                    4,
                    "include",
                    if role == "neovim-unrelated-readable" {
                        unrelated
                    } else {
                        "[\"b.h\",\"<unknown>\",1,\"// include lifecycle control\"]\n"
                    },
                    "cannot read a.c",
                    "matches",
                    101,
                ),
                operation(
                    1,
                    "renamed",
                    if role == "neovim-unrelated-stored" {
                        unrelated
                    } else {
                        row
                    },
                    "",
                    "matches",
                    101,
                ),
                operation(
                    4,
                    "renamed",
                    if role == "neovim-unrelated-repair" {
                        unrelated
                    } else {
                        live
                    },
                    "",
                    "matches",
                    last_pid,
                ),
            ];
            std::fs::write(std::path::Path::new(&root).join("a.c"),"int renamed(void) { return neighbor(); }\nint neighbor(void) { return 2; }\n#include \"b.h\"\n").unwrap();
            std::fs::write(
                report,
                serde_json::to_vec(&json!({"complete":true,"observations":events})).unwrap(),
            )
            .unwrap();
        } else if std::env::var("WORKFLOW_FIXTURE_EDITOR").as_deref() == Ok("wrong-hash") {
            use serde_json::json;
            let root = std::env::var("RIPSCOPE_WORKFLOW_ROOT").unwrap();
            let jump = |line| json!({"kind":"jump","detail":{"file":std::path::Path::new(&root).join("a.c"),"line":line}});
            let query =
                |pattern, text| json!({"kind":"query","detail":{"pattern":pattern,"text":text}});
            let events = vec![
                jump(1),
                jump(1),
                jump(2),
                jump(1),
                query(
                    "__ripscope_workflow_absent_20261005__",
                    "No matches were found",
                ),
                query("[", "invalid query pattern"),
                query("renamed", "cannot read a.c"),
                query("include", "cannot read a.c\n// include lifecycle control"),
                query("renamed", "renamed stored row"),
                query("renamed", "renamed live row"),
            ];
            std::fs::write(
                report,
                serde_json::to_vec(&json!({"complete":true,"observations":events})).unwrap(),
            )
            .unwrap();
        } else {
            std::fs::write(report, "{\"complete\":true,\"observations\":[]}").unwrap();
        }
        return;
    }
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    if arguments
        .iter()
        .any(|arg| arg.starts_with("-b") || arg == "-dl" || arg == "--verify")
    {
        backend(&arguments);
        return;
    }
    match arguments.first().map(String::as_str) {
        Some("environment") => {
            println!("{}", std::env::var("WORKFLOW_FIXTURE_VALUE").unwrap());
            eprintln!("diagnostic");
        }
        Some("early-exit") => {
            let child = Command::new(std::env::current_exe().unwrap())
                .arg("leaf")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            std::fs::write(&arguments[1], child.id().to_string()).unwrap();
        }
        Some("leaf") => std::thread::sleep(Duration::from_secs(60)),
        Some("detached-tree" | "early-tree") => {
            let mut child = Command::new(std::env::current_exe().unwrap());
            child
                .arg("branch")
                .arg(&arguments[1])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            #[cfg(unix)]
            {
                use std::os::unix::process::CommandExt;
                if arguments[0] == "detached-tree" {
                    child.process_group(0);
                }
            }
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                if arguments[0] == "detached-tree" {
                    child.creation_flags(0x00000208);
                }
            }
            child.spawn().unwrap();
            let started = std::time::Instant::now();
            while !std::path::Path::new(&arguments[1]).exists() {
                assert!(started.elapsed() < Duration::from_secs(2));
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        Some("branch") => {
            let child = Command::new(std::env::current_exe().unwrap())
                .arg("leaf")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            std::fs::write(
                &arguments[1],
                format!("{}\n{}\n", std::process::id(), child.id()),
            )
            .unwrap();
            std::thread::sleep(Duration::from_secs(60));
        }
        Some("protocol") => {
            println!("{{\"type\":\"ready\"}}");
            io::stdout().flush().unwrap();
            for line in io::stdin().lock().lines() {
                let line = line.unwrap();
                if line == "q" {
                    break;
                }
                let response = match line.as_str() {
                    "r" => {
                        "{\"type\":\"rebuilt\"}\n{\"type\":\"warning\",\"message\":\"refresh warning\"}\n{\"type\":\"ready\"}\n"
                    }
                    "1[" => {
                        "{\"type\":\"error\",\"message\":\"invalid query pattern\"}\n{\"type\":\"ready\"}\n"
                    }
                    "9bare" => "{\"type\":\"ready\"}\n",
                    "1count" => {
                        "{\"type\":\"result\",\"count\":2}\n[\"a.c\",\"alpha\",1,\"head\"]\n{\"type\":\"ready\"}\n"
                    }
                    "1truncated" => "{\"type\":\"result\",\"count\":0}",
                    _ => {
                        "{\"type\":\"result\",\"count\":1}\n[\"a.c\",\"alpha\",1,\"int alpha(void) {}\"]\n{\"type\":\"warning\",\"message\":\"fixture warning\"}\n{\"type\":\"ready\"}\n"
                    }
                };
                for fragment in response.as_bytes().chunks(7) {
                    io::stdout().write_all(fragment).unwrap();
                    io::stdout().flush().unwrap();
                    std::thread::sleep(Duration::from_millis(1));
                }
            }
        }
        _ => {
            io::stdout().flush().unwrap();
            eprintln!("unknown native fixture role");
            std::process::exit(2);
        }
    }
}

fn backend(arguments: &[String]) {
    use serde_json::{Value, json};
    let index = arguments
        .windows(2)
        .find(|args| args[0] == "-f")
        .map(|args| args[1].as_str())
        .unwrap_or("workflow.db");
    let load = || {
        std::fs::read(index)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            .unwrap_or(json!({"revision":0,"edited":false,"added":false}))
    };
    let update = || {
        let previous = load();
        let edited = std::fs::read_to_string("a.c").is_ok_and(|source| source.contains("renamed"));
        let added = std::path::Path::new("added.c").exists();
        let changed =
            previous["edited"] != edited || previous["added"] != added || previous["revision"] == 0;
        let state = json!({"edited":edited,"added":added,"revision":previous["revision"].as_u64().unwrap()+u64::from(changed)});
        std::fs::write(index, serde_json::to_vec(&state).unwrap()).unwrap();
        state
    };
    if arguments.iter().any(|arg| arg.starts_with("-b")) {
        update();
        return;
    }
    if arguments.iter().any(|arg| arg == "--verify") {
        let state = load();
        println!(
            "index verified: revision {}, {} files",
            state["revision"],
            if state["added"] == true { 4 } else { 2 }
        );
        return;
    }
    println!("{{\"type\":\"ready\"}}");
    io::stdout().flush().unwrap();
    for command in io::stdin().lock().lines() {
        let command = command.unwrap();
        if command == "q" {
            break;
        }
        if command == "r" {
            update();
            println!("{{\"type\":\"rebuilt\"}}\n{{\"type\":\"ready\"}}");
            io::stdout().flush().unwrap();
            continue;
        }
        let state = load();
        let field = command.as_bytes()[0] - b'0';
        let pattern = &command[1..];
        if pattern == "[" {
            println!(
                "{{\"type\":\"error\",\"message\":\"invalid query pattern\"}}\n{{\"type\":\"ready\"}}"
            );
            io::stdout().flush().unwrap();
            continue;
        }
        let mut rows = Vec::<Value>::new();
        let mut warnings = Vec::new();
        let source = std::fs::read_to_string("a.c").ok();
        let name = if state["edited"] == true {
            "renamed"
        } else {
            "alpha"
        };
        if [4, 5, 6].contains(&field) {
            if source.is_none() {
                warnings.push("cannot read a.c");
            }
            if state["added"] == true && !std::path::Path::new("added.c").exists() {
                warnings.push("cannot read added.c");
            }
        }
        match (field, pattern) {
            (7, ".") => {
                rows.push(json!(["a.c", "<unknown>", 1, ""]));
                rows.push(json!(["b.h", "<unknown>", 1, ""]));
                if state["added"] == true {
                    rows.push(json!(["added.c", "<unknown>", 1, ""]));
                    rows.push(json!(["added.inc", "<unknown>", 1, ""]));
                }
            }
            (1, "alpha" | "renamed") if pattern == name => rows.push(json!([
                "a.c",
                name,
                1,
                format!("int {name}(void) {{ return neighbor(); }}")
            ])),
            (1, "neighbor") => rows.push(json!([
                "a.c",
                "neighbor",
                2,
                "int neighbor(void) { return 2; }"
            ])),
            (2, "renamed") if state["edited"] == true => {
                rows.push(json!(["a.c", "neighbor", 1, ""]))
            }
            (3, "neighbor") => rows.push(json!(["a.c", name, 1, ""])),
            (4..=6, "renamed")
                if source
                    .as_ref()
                    .is_some_and(|source| source.contains("renamed")) =>
            {
                rows.push(json!([
                    "a.c",
                    "<unknown>",
                    1,
                    "int renamed(void) { return neighbor(); }"
                ]))
            }
            (4, "include") => rows.push(json!([
                "b.h",
                "<unknown>",
                1,
                "// include lifecycle control"
            ])),
            (1, "added") if state["added"] == true => rows.push(json!([
                "added.c",
                "added",
                2,
                "int added(void) { return extra(); }"
            ])),
            (1, "extra") if state["added"] == true => rows.push(json!([
                "added.inc",
                "extra",
                1,
                "int extra(void) { return 3; }"
            ])),
            (7, "added[.]") if state["added"] == true => {
                rows.push(json!(["added.c", "<unknown>", 1, ""]));
                rows.push(json!(["added.inc", "<unknown>", 1, ""]));
            }
            (4, "int") if state["added"] == true => {
                rows.push(json!([
                    "added.inc",
                    "<unknown>",
                    1,
                    "int extra(void) { return 3; }"
                ]));
            }
            _ => (),
        }
        println!("{}", json!({"type":"result","count":rows.len()}));
        for row in rows {
            println!("{row}");
        }
        for warning in warnings {
            println!("{}", json!({"type":"warning","message":warning}));
        }
        println!("{{\"type\":\"ready\"}}");
        io::stdout().flush().unwrap();
    }
}
