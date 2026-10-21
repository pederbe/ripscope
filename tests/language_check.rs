//! All of the project's code, including tests, tools and measurement harnesses, is Rust.
//!
//! This test lists the tracked files (`git ls-files`, or every file under the checkout
//! when it is not a Git repository, as in a release build's source copy) and fails on
//! code in any other language outside the named exceptions below. Each exception names
//! the languages it allows, so other code placed there still fails. Configuration (TOML,
//! JSON, YAML), documentation, web pages and patches are not code here.

use std::path::Path;
use std::process::Command;

/// File extensions of code in a language other than Rust.
const OTHER_LANGUAGES: &[(&str, &str)] = &[
    ("py", "Python"),
    ("pyw", "Python"),
    ("pyi", "Python"),
    ("ipynb", "Jupyter notebook"),
    ("sh", "shell"),
    ("bash", "shell"),
    ("zsh", "shell"),
    ("fish", "shell"),
    ("ksh", "shell"),
    ("csh", "shell"),
    ("tcsh", "shell"),
    ("bats", "shell"),
    ("ps1", "PowerShell"),
    ("psm1", "PowerShell"),
    ("bat", "batch"),
    ("cmd", "batch"),
    ("js", "JavaScript"),
    ("mjs", "JavaScript"),
    ("cjs", "JavaScript"),
    ("jsx", "JavaScript"),
    ("ts", "TypeScript"),
    ("tsx", "TypeScript"),
    ("mts", "TypeScript"),
    ("cts", "TypeScript"),
    ("lua", "Lua"),
    ("vim", "Vim script"),
    ("el", "Emacs Lisp"),
    ("rb", "Ruby"),
    ("pl", "Perl"),
    ("pm", "Perl"),
    ("php", "PHP"),
    ("go", "Go"),
    ("java", "Java"),
    ("kt", "Kotlin"),
    ("cs", "C#"),
    ("swift", "Swift"),
    ("c", "C"),
    ("h", "C"),
    ("inc", "C"),
    ("cc", "C++"),
    ("cpp", "C++"),
    ("cxx", "C++"),
    ("hh", "C++"),
    ("hpp", "C++"),
    ("l", "lex"),
    ("y", "yacc"),
    ("nix", "Nix"),
    ("awk", "awk"),
    ("tcl", "Tcl"),
    ("zig", "Zig"),
    ("mk", "make"),
    ("just", "just"),
    ("cmake", "CMake"),
];

/// Files whose name alone says their language.
const NAMED_FILES: &[(&str, &str)] = &[
    ("Makefile", "make"),
    ("makefile", "make"),
    ("GNUmakefile", "make"),
    ("justfile", "just"),
    ("Justfile", "just"),
    (".justfile", "just"),
    ("PKGBUILD", "shell"),
    ("CMakeLists.txt", "CMake"),
];

/// Interpreters a script's `#!` line may name, by program name without any version.
const INTERPRETERS: &[(&str, &str)] = &[
    ("python", "Python"),
    ("pypy", "Python"),
    ("sh", "shell"),
    ("bash", "shell"),
    ("dash", "shell"),
    ("ash", "shell"),
    ("zsh", "shell"),
    ("ksh", "shell"),
    ("mksh", "shell"),
    ("csh", "shell"),
    ("tcsh", "shell"),
    ("bats", "shell"),
    ("fish", "shell"),
    ("node", "JavaScript"),
    ("nodejs", "JavaScript"),
    ("deno", "JavaScript"),
    ("bun", "JavaScript"),
    ("pwsh", "PowerShell"),
    ("powershell", "PowerShell"),
    ("perl", "Perl"),
    ("ruby", "Ruby"),
    ("php", "PHP"),
    ("lua", "Lua"),
    ("luajit", "Lua"),
    ("nvim", "Lua"),
    ("awk", "awk"),
    ("gawk", "awk"),
    ("tclsh", "Tcl"),
    ("make", "make"),
    ("just", "just"),
];

/// A place code in another language may live, the languages it allows, and why. A path
/// ending in `/` covers a directory.
#[derive(Clone, Copy)]
struct Exception {
    path: &'static str,
    languages: Languages,
    reason: &'static str,
}

#[derive(Clone, Copy)]
enum Languages {
    Only(&'static [&'static str]),
    /// Any language, for working material that is not published.
    Any,
}

const fn only(
    path: &'static str,
    languages: &'static [&'static str],
    reason: &'static str,
) -> Exception {
    Exception {
        path,
        languages: Languages::Only(languages),
        reason,
    }
}

/// The only places code in another language may live.
const EXCEPTIONS: &[Exception] = &[
    only(
        ".github/scripts/renovate-sync.sh",
        &["shell"],
        "dependency updater glue synchronizes download metadata before CI",
    ),
    only(
        "install.sh",
        &["shell"],
        "the Linux installer runs before Ripscope or Rust is present",
    ),
    only(
        "install.ps1",
        &["PowerShell"],
        "the Windows installer runs before Ripscope or Rust is present",
    ),
    only(
        "plugin/",
        &["Vim script", "Lua"],
        "the Vim and Neovim plugin, in the editors' own languages",
    ),
    only(
        "lua/",
        &["Lua"],
        "the Neovim adapter, in the editor's own language",
    ),
    only(
        "contrib/emacs/",
        &["Emacs Lisp"],
        "the Emacs package, in the editor's own language",
    ),
    only(
        "tests/editor/setup.vim",
        &["Vim script"],
        "runs inside Vim for the one-step setup check",
    ),
    only(
        "tests/editor/setup.lua",
        &["Lua"],
        "runs inside Neovim for the one-step setup check",
    ),
    only(
        "tests/editor/setup.el",
        &["Emacs Lisp"],
        "runs inside Emacs for the one-step setup check",
    ),
    only(
        "tests/editor/persistent.vim",
        &["Vim script"],
        "runs inside Vim for the Vim check",
    ),
    only(
        "tests/editor/workflow.vim",
        &["Vim script"],
        "runs inside Vim for the Vim workflow check",
    ),
    only(
        "tests/editor/neovim-workflow.lua",
        &["Lua"],
        "runs inside Neovim for the Neovim check",
    ),
    only(
        "tests/editor/emacs-workflow.el",
        &["Emacs Lisp"],
        "runs inside Emacs for the Emacs check",
    ),
    only(
        "tests/editor/emacs-windows-workflow.el",
        &["Emacs Lisp"],
        "runs inside Emacs for the Windows Emacs check",
    ),
    only(
        "tests/fixtures/",
        &["C", "lex", "yacc"],
        "the C, lex and yacc inputs the analyzer's tests read",
    ),
    only(
        "evidence/captures/",
        &["C", "lex", "yacc"],
        "copies of those test inputs that the published evidence links to",
    ),
    only(
        "tools/media/fixture/",
        &["C"],
        "the C project the demo recording indexes",
    ),
    only("site/", &["JavaScript"], "the website's own scripts"),
    only(
        "docs/theme/",
        &["JavaScript"],
        "the documentation website's scripts",
    ),
    only(
        "scripts/test-landing-page.mjs",
        &["JavaScript"],
        "the landing page's browser check, run by Node",
    ),
    only(
        "scripts/demo/",
        &["JavaScript"],
        "renders the website's demo recording, run by Node",
    ),
    only(
        "scripts/build-site.sh",
        &["shell"],
        "short build glue that starts the website build",
    ),
    only(
        "scripts/linux-smoke.sh",
        &["shell"],
        "runs inside the release runtime container, which has no Rust",
    ),
    only(
        "scripts/package-linux.sh",
        &["shell"],
        "short packaging glue around dpkg-deb and rpmbuild",
    ),
    only(
        "packaging/check-installed.sh",
        &["shell"],
        "the installed-package check the package recipes run",
    ),
    only(
        "packaging/templates/",
        &["shell", "Ruby", "Nix"],
        "package-manager recipes in their managers' own formats",
    ),
];

/// A list of further exceptions for working material that is not published. A checkout
/// without it has only the exceptions above.
const LOCAL_EXCEPTIONS: &str = ".scratch/language-exceptions.txt";

/// The exceptions above, and those the local list at `root` adds where it exists.
fn exceptions(root: &Path) -> &'static [Exception] {
    let text = match std::fs::read_to_string(root.join(LOCAL_EXCEPTIONS)) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => panic!("{LOCAL_EXCEPTIONS}: {error}"),
    };
    let mut all = EXCEPTIONS.to_vec();
    all.extend(parse_exceptions(Box::leak(text.into_boxed_str())));
    all.leak()
}

/// Exceptions written one per line: a path, the languages it allows separated by commas
/// (`*` for any) and the reason, separated by tabs. Blank lines and lines starting with
/// `#` are skipped.
fn parse_exceptions(text: &'static str) -> Vec<Exception> {
    text.lines()
        .map(str::trim_end)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| {
            let mut fields = line.splitn(3, '\t');
            let path = fields.next().unwrap_or_default();
            let (Some(languages), Some(reason)) = (fields.next(), fields.next()) else {
                panic!("{LOCAL_EXCEPTIONS}: {path} needs its languages and a reason");
            };
            let languages = if languages == "*" {
                Languages::Any
            } else {
                Languages::Only(
                    languages
                        .split(',')
                        .map(str::trim)
                        .collect::<Vec<_>>()
                        .leak(),
                )
            };
            Exception {
                path,
                languages,
                reason,
            }
        })
        .collect()
}

/// The language of a file's code, when it is not Rust: by its name or extension, or by
/// the interpreter its `#!` line names.
fn other_language(name: &str, first_line: &[u8]) -> Option<&'static str> {
    let file = name.rsplit('/').next().unwrap_or(name);
    if let Some((_, language)) = NAMED_FILES.iter().find(|(named, _)| *named == file) {
        return Some(language);
    }
    if let Some((stem, extension)) = file.rsplit_once('.')
        && !stem.is_empty()
        && let Some((_, language)) = OTHER_LANGUAGES
            .iter()
            .find(|(known, _)| extension.eq_ignore_ascii_case(known))
    {
        return Some(language);
    }
    // Rust's inner attributes also start with `#!`, as `#![...]`.
    let line = first_line.strip_prefix(b"#!")?;
    if line.starts_with(b"[") {
        return None;
    }
    interpreter_language(&String::from_utf8_lossy(line))
}

/// The language of the interpreter a `#!` line names, looking through `env` and its
/// options, or "script" for one not listed. A Rust script that Cargo runs is not another
/// language.
fn interpreter_language(line: &str) -> Option<&'static str> {
    let mut words = line.split_whitespace();
    let mut program = words.next().unwrap_or_default();
    if program.rsplit('/').next() == Some("env") {
        program = words
            .find(|word| !word.starts_with('-') && !word.contains('='))
            .unwrap_or_default();
    }
    let program = program.rsplit('/').next().unwrap_or_default();
    // python3.12 and python3 are both Python.
    let program = program.trim_end_matches(|c: char| c.is_ascii_digit() || c == '.');
    // A Rust script, which Cargo runs.
    if matches!(program, "cargo" | "rust-script") {
        return None;
    }
    Some(
        INTERPRETERS
            .iter()
            .find(|(known, _)| *known == program)
            .map_or("script", |(_, language)| language),
    )
}

/// The exception in `exceptions` covering `name`, if any.
fn covering(exceptions: &'static [Exception], name: &str) -> Option<&'static Exception> {
    exceptions.iter().find(|exception| {
        if exception.path.ends_with('/') {
            name.starts_with(exception.path)
        } else {
            name == exception.path
        }
    })
}

/// The exception in `exceptions` that allows `language` at `name`, if any.
fn excepted(exceptions: &'static [Exception], name: &str, language: &str) -> Option<&'static str> {
    let exception = covering(exceptions, name)?;
    let allowed = match exception.languages {
        Languages::Any => true,
        Languages::Only(languages) => languages.contains(&language),
    };
    allowed.then_some(exception.path)
}

/// The tracked files, and whether Git listed them. Without a repository at the
/// checkout, every file outside build output and hidden directories counts.
fn tracked(root: &Path) -> (Vec<String>, bool) {
    let toplevel = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(root)
        .output();
    let at_root = toplevel.is_ok_and(|output| {
        output.status.success()
            && Path::new(String::from_utf8_lossy(&output.stdout).trim())
                .canonicalize()
                .is_ok_and(|top| root.canonicalize().is_ok_and(|root| top == root))
    });
    if at_root {
        let listed = Command::new("git")
            .args(["ls-files", "-z"])
            .current_dir(root)
            .output()
            .expect("git ls-files");
        assert!(listed.status.success(), "git ls-files failed");
        let names = listed
            .stdout
            .split(|byte| *byte == 0)
            .filter(|name| !name.is_empty())
            .map(|name| String::from_utf8_lossy(name).into_owned())
            .collect();
        return (names, true);
    }
    let mut names = Vec::new();
    let mut pending = vec![root.to_owned()];
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(&directory).expect("readable checkout") {
            let entry = entry.expect("directory entry");
            let file_name = entry.file_name().to_string_lossy().into_owned();
            let kind = entry.file_type().expect("file type");
            if kind.is_dir() {
                if !file_name.starts_with('.')
                    && file_name != "target"
                    && file_name != "node_modules"
                {
                    pending.push(entry.path());
                }
            } else {
                let relative = entry
                    .path()
                    .strip_prefix(root)
                    .expect("below the root")
                    .to_owned();
                let parts: Vec<String> = relative
                    .components()
                    .map(|part| part.as_os_str().to_string_lossy().into_owned())
                    .collect();
                names.push(parts.join("/"));
            }
        }
    }
    (names, false)
}

/// The start of a file's first line, enough to read a `#!` line.
fn first_line(path: &Path) -> Vec<u8> {
    let mut start = Vec::new();
    if let Ok(file) = std::fs::File::open(path) {
        use std::io::Read;
        let _ = file.take(256).read_to_end(&mut start);
    }
    let end = start
        .iter()
        .position(|byte| *byte == b'\n')
        .unwrap_or(start.len());
    start.truncate(end);
    start
}

/// The tracked files in other languages that no exception allows, and the stale
/// exceptions.
fn violations(root: &Path, names: &[String]) -> (Vec<String>, Vec<&'static str>) {
    let exceptions = exceptions(root);
    let mut unexcused = Vec::new();
    let mut used = std::collections::BTreeSet::new();
    for name in names {
        let path = root.join(name);
        // A tracked file deleted from the working tree is no longer code here.
        if std::fs::symlink_metadata(&path).is_err() {
            continue;
        }
        let Some(language) = other_language(name, &first_line(&path)) else {
            continue;
        };
        match excepted(exceptions, name, language) {
            Some(path) => {
                used.insert(path);
            }
            None => unexcused.push(match covering(exceptions, name) {
                Some(exception) => format!(
                    "{name} ({language}, which {} does not allow: {})",
                    exception.path, exception.reason
                ),
                None => format!("{name} ({language})"),
            }),
        }
    }
    // An exception is stale once the place it names is still there but holds no code
    // it excuses: a directory with no such code left, or a missing file whose directory
    // remains. Places a checkout leaves out entirely, such as private material in a
    // public export, are not stale.
    let stale = exceptions
        .iter()
        .map(|exception| exception.path)
        .filter(|path| !used.contains(path))
        .filter(|path| {
            let directory = match path.strip_suffix('/') {
                Some(directory) => directory,
                None => path.rsplit_once('/').map_or("", |(directory, _)| directory),
            };
            directory.is_empty()
                || names
                    .iter()
                    .any(|name| name.starts_with(&format!("{directory}/")))
        })
        .collect();
    (unexcused, stale)
}

#[test]
fn code_outside_the_named_exceptions_is_rust() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let (names, from_git) = tracked(root);
    assert!(!names.is_empty(), "no files found under {}", root.display());
    let (unexcused, stale) = violations(root, &names);
    assert!(
        unexcused.is_empty(),
        "code in another language outside the named exceptions; port it to Rust:\n{}",
        unexcused.join("\n")
    );
    if from_git {
        assert!(
            stale.is_empty(),
            "exceptions that no longer cover any file; remove them: {stale:?}"
        );
    }
}

#[test]
fn a_python_file_or_script_anywhere_else_fails() {
    for (name, first) in [
        ("scripts/new_tool.py", &b""[..]),
        ("tests/editor/check_vim.py", b""),
        ("tools/release/helper", b"#!/usr/bin/env python3"),
        ("examples/support/run.sh", b""),
        ("plugin/helper.py", b""),
        ("lua/ripscope/run.sh", b""),
        ("contrib/emacs/helper", b"#!/bin/sh"),
        ("tests/fixtures/generate.py", b""),
        ("site/build.py", b""),
        ("packaging/templates/helper.py", b""),
        ("tests/editor/setup.vim", b""),
    ] {
        let language = other_language(name, first).unwrap_or_else(|| panic!("{name}"));
        if name == "tests/editor/setup.vim" {
            assert_eq!(excepted(EXCEPTIONS, name, language), Some(name));
            assert_eq!(excepted(EXCEPTIONS, name, "Python"), None);
        } else {
            assert!(
                excepted(EXCEPTIONS, name, language).is_none(),
                "{name} must not be excepted"
            );
        }
    }
    assert_eq!(
        other_language("src/main.rs", b"#![forbid(unsafe_code)]"),
        None
    );
    assert_eq!(other_language("Cargo.toml", b""), None);
    assert_eq!(other_language("LICENSE-MIT", b"Permission"), None);
    assert_eq!(other_language(".gitignore", b"# Generated"), None);
    assert_eq!(
        excepted(EXCEPTIONS, "plugin/ripscope.vim", "Vim script"),
        Some("plugin/")
    );
    assert_eq!(
        excepted(EXCEPTIONS, "plugin/ripscope.lua", "Lua"),
        Some("plugin/")
    );
}

#[test]
fn a_local_list_adds_exceptions() {
    let local = parse_exceptions(
        "# Local exceptions\n\nnotes/\t*\tworking notes\ntools/vm.ps1\tPowerShell, shell\tVM glue\n",
    )
    .leak();
    assert_eq!(local.len(), 2);
    assert_eq!(excepted(local, "notes/draft.py", "Python"), Some("notes/"));
    assert_eq!(
        excepted(local, "tools/vm.ps1", "shell"),
        Some("tools/vm.ps1")
    );
    assert_eq!(excepted(local, "tools/vm.ps1", "Python"), None);
    assert_eq!(excepted(local, "src/run.py", "Python"), None);
}

#[test]
fn build_recipes_and_script_interpreters_are_recognised() {
    for (name, first, language) in [
        ("Makefile", &b""[..], "make"),
        ("tools/GNUmakefile", b"", "make"),
        ("rules.mk", b"", "make"),
        ("justfile", b"", "just"),
        ("tools/.justfile", b"", "just"),
        ("deploy.just", b"", "just"),
        ("bin/run", b"#!/usr/bin/env python3.12", "Python"),
        ("bin/run", b"#!/usr/bin/env -S python3 -u", "Python"),
        ("bin/run", b"#!/usr/bin/env PYTHONPATH=x python", "Python"),
        ("bin/run", b"#!/bin/bash -eu", "shell"),
        ("bin/run", b"#! /bin/sh", "shell"),
        ("bin/run", b"#!/usr/bin/env node", "JavaScript"),
        ("bin/run", b"#!/usr/bin/perl -w", "Perl"),
        ("bin/run", b"#!/usr/bin/env pwsh", "PowerShell"),
        ("bin/run.txt", b"#!/bin/sh", "shell"),
        ("bin/run", b"#!/opt/odd/interpreter", "script"),
        ("analysis.ipynb", b"{", "Jupyter notebook"),
        ("CMakeLists.txt", b"", "CMake"),
        ("cmake/deps.cmake", b"", "CMake"),
        ("tests/cli.bats", b"", "shell"),
        ("bin/setup.csh", b"", "shell"),
        ("bin/run", b"#!/bin/tcsh -f", "shell"),
        ("site/module.mts", b"", "TypeScript"),
        ("site/common.cts", b"", "TypeScript"),
    ] {
        assert_eq!(
            other_language(name, first),
            Some(language),
            "{name} {first:?}"
        );
    }
    // Rust scripts that Cargo runs are Rust.
    for first in [
        &b"#!/usr/bin/env -S cargo +nightly -Zscript"[..],
        b"#!/usr/bin/env cargo",
        b"#!/home/user/.cargo/bin/rust-script",
    ] {
        assert_eq!(other_language("tools/check.rs", first), None, "{first:?}");
        assert_eq!(other_language("bin/check", first), None, "{first:?}");
    }
}
