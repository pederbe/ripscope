//! The invocation: what a command line asks
//! for, read the way cscope reads its options, so the commands Vim, Neovim, Emacs and
//! `cscope -Rbqk`-style scripts send work unchanged (docs/spec.md, "Command line").
//!
//! Single-letter options may be grouped (`-Rbqk`, `-dL`); an option that takes a value
//! takes the rest of its argument or the next one (`-fINDEX`, `-f INDEX`, `-0name`,
//! `-0 name`), and `--` ends the options. Options that change nothing in cscope's line
//! and one-shot output are read and ignored. Options ripscope cannot honour faithfully
//! are refused, never ignored, and so are combinations no editor or script sends.

use crate::sink::Transport;
use std::ffi::OsString;
use std::path::PathBuf;

/// The index name used when `-f` is not given.
pub(crate) const DEFAULT_INDEX: &str = "ripscope.db";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Action {
    /// `-h`, `--help`: say how to use ripscope.
    Help,
    /// `-V`, `--version`.
    Version,
    /// `-b`: build or update the index, then stop.
    Build,
    /// `-L -<field> PATTERN`: one query. Fields 4 and 5 search text and 6 an egrep
    /// pattern in the files as they are now; the others are answered from the index.
    Query { field: u8, pattern: Vec<u8> },
    /// `-l`: a session answering queries from standard input.
    Session,
    /// `--verify`: check the index, then bring it up to date reading every file.
    Verify,
    /// `--project PATH`: report the Project of a file or directory, for editor
    /// integrations (ADR 0014).
    Project(PathBuf),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Invocation {
    pub action: Action,
    pub index: PathBuf,
    /// Bring the index up to date first and before Session Stored queries. Explicit
    /// automatic-refresh controls override `-d`, independent of argument order.
    pub update: bool,
    /// `-u`: build the index unconditionally, as cscope says, assuming every file
    /// changed: every file is analysed again and nothing the index held is kept (the way
    /// to recover a damaged index).
    pub unconditional: bool,
    /// `-C`: ignore letter case in queries.
    pub caseless: bool,
    /// `-R`: search subdirectories when discovering sources.
    pub recursive: bool,
    /// `-k`: leave out the system include directory.
    pub kernel: bool,
    pub include_dirs: Vec<Vec<u8>>,
    /// `-i LIST`: the file list (`-` is not supported).
    pub list: Option<Vec<u8>>,
    pub files: Vec<Vec<u8>>,
    pub transport: Transport,
    /// `-P PATH`: cscope's prepend path, put before relative names where text searches
    /// read files and in their rows.
    pub prefix: Option<Vec<u8>>,
    /// `--vim-errors`: a session in the line protocol shows problems through Vim's RETURN
    /// prompt.
    pub vim_errors: bool,
}

/// Reads a command line (without the program name).
pub(crate) fn parse(args: &[OsString]) -> Result<Invocation, String> {
    let mut build = false;
    let mut one_shot = false;
    let mut session = false;
    let mut query: Option<(u8, Vec<u8>)> = None;
    let mut index = None;
    let mut update = true;
    let mut automatic_refresh = None;
    let mut unconditional = false;
    let mut caseless = false;
    let mut recursive = false;
    let mut kernel = false;
    let mut include_dirs = Vec::new();
    let mut list = None;
    let mut files = Vec::new();
    let mut transport = Transport::Line;
    let mut verify = false;
    let mut project = None;
    let mut prefix = None;
    let mut vim_errors = false;
    // As cscope reads them, the first `-h` or `-V` ends the command line: what follows it
    // is not read, and nothing else is checked.
    let mut stop = None;
    // Options given besides `-f` and `-d`, which `--verify` does not take.
    let mut others = Vec::new();
    let mut options = true;
    let mut args = args.iter();
    'args: while let Some(arg) = args.next() {
        let bytes = arg.as_encoded_bytes();
        if !options {
            files.push(bytes.to_vec());
            continue;
        }
        if bytes == b"--" {
            options = false;
            continue;
        }
        if bytes == b"--vim-errors" {
            vim_errors = true;
            others.push("--vim-errors".to_owned());
            continue;
        }
        if bytes == b"--json" {
            transport = Transport::Json;
            others.push("--json".to_owned());
            continue;
        }
        if matches!(bytes, b"--auto-refresh" | b"--no-auto-refresh") {
            let enabled = bytes == b"--auto-refresh";
            if automatic_refresh.is_some_and(|previous| previous != enabled) {
                return Err("--auto-refresh and --no-auto-refresh cannot be combined".into());
            }
            automatic_refresh = Some(enabled);
            others.push(arg.to_string_lossy().into_owned());
            continue;
        }
        if bytes == b"--help" {
            stop = Some(Action::Help);
            break;
        }
        if bytes == b"--version" {
            stop = Some(Action::Version);
            break;
        }
        if bytes == b"--verify" {
            verify = true;
            continue;
        }
        if bytes == b"--project" {
            let path = args.next().ok_or("--project needs a file or directory")?;
            project = Some(PathBuf::from(path));
            continue;
        }
        if bytes.starts_with(b"--") {
            return Err(format!("unsupported option: {}", arg.to_string_lossy()));
        }
        if bytes.first() != Some(&b'-') || bytes.len() == 1 {
            files.push(bytes.to_vec());
            continue;
        }
        let mut at = 1;
        while at < bytes.len() {
            let letter = bytes[at];
            at += 1;
            // The option's value: the rest of this argument, or the next one.
            let mut value = |name: &str| -> Result<Vec<u8>, String> {
                let rest = &bytes[at..];
                at = bytes.len();
                if !rest.is_empty() {
                    return Ok(rest.to_vec());
                }
                args.next()
                    .map(|next| next.as_encoded_bytes().to_vec())
                    .ok_or_else(|| format!("-{name} needs a value"))
            };
            if !matches!(letter, b'f' | b'd') {
                others.push(format!("-{}", char::from(letter)));
            }
            match letter {
                b'h' => {
                    stop = Some(Action::Help);
                    break 'args;
                }
                b'V' => {
                    stop = Some(Action::Version);
                    break 'args;
                }
                b'b' => build = true,
                b'L' => one_shot = true,
                b'l' => session = true,
                b'd' => update = false,
                b'u' => unconditional = true,
                b'C' => caseless = true,
                b'R' => recursive = true,
                b'k' => kernel = true,
                // A fast-search index is what ripscope always has; `-e`, `-U` and `-v`
                // concern cscope's own screen, timestamps and chatter, and `-c` only stops
                // cscope compressing its own database.
                b'q' | b'e' | b'U' | b'v' | b'c' => {}
                // Only cscope's full-screen display reads these: how many path components
                // it shows, and a file of symbol reference lines to show.
                b'p' | b'F' => {
                    value(&char::from(letter).to_string())?;
                }
                b'f' => index = Some(PathBuf::from(os(value("f")?))),
                b'i' => {
                    let name = value("i")?;
                    if name == b"-" {
                        return Err("-i - (a file list on standard input) is not supported".into());
                    }
                    list = Some(name);
                }
                b'I' => include_dirs.push(value("I")?),
                b'P' => prefix = Some(value("P")?),
                b'0'..=b'9' => {
                    let pattern = value(&char::from(letter).to_string())?;
                    if query.replace((letter - b'0', pattern)).is_some() {
                        return Err("only one query can be given".into());
                    }
                }
                b'T' => {
                    return Err(
                        "-T changes how cscope stores symbols, which ripscope does not follow"
                            .into(),
                    );
                }
                b's' => return Err("-s is not supported".into()),
                other => return Err(format!("unsupported option: -{}", char::from(other))),
            }
        }
    }
    let json = transport == Transport::Json;
    let stopped = stop.is_some();
    let action = if let Some(action) = stop {
        action
    } else if let Some(path) = project {
        if verify || index.is_some() || !update || !others.is_empty() || !files.is_empty() {
            return Err("--project takes only a file or directory".into());
        }
        Action::Project(path)
    } else if verify {
        if let Some(other) = others.first() {
            return Err(format!("--verify takes only -f INDEX and -d, not {other}"));
        }
        if !files.is_empty() {
            return Err("--verify takes no file names".into());
        }
        Action::Verify
    } else if build {
        if one_shot || session || query.is_some() {
            return Err("-b cannot be combined with -L, -l or a query".into());
        }
        if json {
            return Err("--json is for queries and sessions".into());
        }
        Action::Build
    } else {
        match (one_shot, session, query) {
            (true, true, _) => return Err("-L and -l cannot be combined".into()),
            (true, false, Some((field, pattern))) => Action::Query { field, pattern },
            (true, false, None) => return Err("-L needs a query such as -0 NAME".into()),
            (false, true, None) => Action::Session,
            (false, _, Some(_)) => return Err("a query needs -L".into()),
            (false, false, None) => {
                return Err(
                    "cscope's full-screen interface is not supported; use -b, -L or -l".into(),
                );
            }
        }
    };
    if !files.is_empty() && matches!(action, Action::Query { .. } | Action::Session) {
        return Err("file names can only be given to an index build (-b)".into());
    }
    if vim_errors && !stopped && (action != Action::Session || json) {
        return Err("--vim-errors is for sessions (-l) in the line protocol".into());
    }
    if automatic_refresh.is_some()
        && !stopped
        && !matches!(action, Action::Query { .. } | Action::Session)
    {
        return Err("automatic-refresh controls are for queries (-L) and sessions (-l)".into());
    }
    Ok(Invocation {
        action,
        index: index.unwrap_or_else(|| PathBuf::from(DEFAULT_INDEX)),
        update: automatic_refresh.unwrap_or(update),
        unconditional,
        caseless,
        recursive,
        kernel,
        include_dirs,
        list,
        files,
        transport,
        prefix,
        vim_errors,
    })
}

fn os(bytes: Vec<u8>) -> OsString {
    crate::refresh::path_of(&bytes).into_os_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(args: &[&str]) -> Result<Invocation, String> {
        parse(&args.iter().map(OsString::from).collect::<Vec<_>>())
    }

    #[test]
    fn the_first_help_or_version_ends_the_command_line() {
        for (args, action) in [
            (&["-h", "-z"][..], Action::Help),
            (&["--help", "--vim-errors"], Action::Help),
            (&["--vim-errors", "-V"], Action::Version),
            (&["-V", "-h"], Action::Version),
            (&["-dLV", "-f"], Action::Version),
            (&["-L", "-1", "x", "--help"], Action::Help),
        ] {
            assert_eq!(read(args).expect("stops").action, action, "{args:?}");
        }
        // What comes before is read as usual.
        assert_eq!(
            read(&["-z", "-h"]).map(|_| ()),
            Err("unsupported option: -z".into())
        );
    }

    #[test]
    fn project_takes_one_path_and_nothing_else() {
        assert_eq!(
            read(&["--project", "src/main.c"]).expect("project").action,
            Action::Project(PathBuf::from("src/main.c"))
        );
        assert!(read(&["--project"]).is_err());
        for args in [
            &["--project", "a.c", "-R"][..],
            &["--project", "a.c", "-f", "x.db"],
            &["-l", "--project", "a.c"],
            &["--project", "a.c", "b.c"],
        ] {
            assert_eq!(
                read(args).map(|_| ()),
                Err("--project takes only a file or directory".into()),
                "{args:?}"
            );
        }
    }

    #[test]
    fn grouped_options_and_attached_values_read_as_cscope_reads_them() {
        let build = read(&["-Rbqk", "-Iinclude", "-I", "other", "-f", "tags.db"]).expect("build");
        assert_eq!(build.action, Action::Build);
        assert!(build.recursive && build.kernel && build.update && !build.unconditional);
        assert!(read(&["-bu"]).expect("unconditional build").unconditional);
        let verify = read(&["--verify", "-d", "-f", "x.db"]).expect("verify");
        assert_eq!((verify.action, verify.update), (Action::Verify, false));
        assert_eq!(build.include_dirs, [b"include".to_vec(), b"other".to_vec()]);
        assert_eq!(build.index, PathBuf::from("tags.db"));

        let query = read(&["-dLC", "-0main", "-fx.db"]).expect("query");
        assert_eq!(
            query.action,
            Action::Query {
                field: 0,
                pattern: b"main".to_vec()
            }
        );
        assert!(!query.update && query.caseless);
        assert_eq!(query.index, PathBuf::from("x.db"));

        let spaced = read(&["-L", "-3", "a b"]).expect("spaced pattern");
        assert_eq!(
            spaced.action,
            Action::Query {
                field: 3,
                pattern: b"a b".to_vec()
            }
        );
        assert_eq!(spaced.index, PathBuf::from(DEFAULT_INDEX));

        let session = read(&["-dl", "-f", "i.db", "--json"]).expect("session");
        assert_eq!(
            (session.action, session.transport),
            (Action::Session, Transport::Json)
        );

        let listed = read(&["-b", "-i", "files.txt", "extra.c"]).expect("list");
        assert_eq!(listed.list, Some(b"files.txt".to_vec()));
        assert_eq!(listed.files, [b"extra.c".to_vec()]);

        // As Vim starts a connection, and as the owned adapters end their options.
        let vim = read(&["--vim-errors", "-dl", "-f", "db", "-P/src/project"]).expect("vim");
        assert!(vim.vim_errors && !vim.update);
        assert_eq!(vim.prefix, Some(b"/src/project".to_vec()));
        let ended = read(&["-bk", "--", "-odd.c", "--json"]).expect("ended");
        assert_eq!(ended.files, [b"-odd.c".to_vec(), b"--json".to_vec()]);
        assert_eq!(ended.transport, Transport::Line);
        let text = read(&["-dL", "-5", "needle"]).expect("field 5");
        assert_eq!(
            text.action,
            Action::Query {
                field: 5,
                pattern: b"needle".to_vec()
            }
        );
    }

    #[test]
    fn options_that_change_no_row_are_read_and_ignored() {
        let plain = read(&["-dL", "-f", "i.db", "-1", "main"]).expect("plain");
        for args in [
            &["-dLc", "-f", "i.db", "-1", "main"][..],
            &["-dL", "-p", "3", "-f", "i.db", "-1", "main"],
            &["-dLp3", "-f", "i.db", "-1", "main"],
            &["-dL", "-F", "syms", "-f", "i.db", "-1", "main"],
            &["-dLFsyms", "-c", "-p", "-1", "-f", "i.db", "-1", "main"],
        ] {
            assert_eq!(read(args).as_ref(), Ok(&plain), "{args:?}");
        }
        let build = read(&["-bk", "-f", "i.db", "a.c"]).expect("build");
        let quiet = read(&["-bkc", "-p3", "-F", "syms", "-f", "i.db", "a.c"]).expect("quiet");
        assert_eq!(quiet, build);
        for (args, missing) in [(&["-dL0x", "-p"][..], "-p"), (&["-dL0x", "-F"], "-F")] {
            assert_eq!(
                read(args),
                Err(format!("{missing} needs a value")),
                "{args:?}"
            );
        }
    }

    #[test]
    fn options_ripscope_cannot_honour_are_refused() {
        for args in [
            &["-bT"][..],
            &["-bs", "src"],
            &["-L0x", "-1y"],
            &["-L"],
            &["-0x"],
            &[],
            &["-b", "-i", "-"],
            &["-bz"],
            &["--strict", "-b"],
            &["--json", "-b"],
            &["--vim-errors", "-dL", "-0x"],
            &["--vim-errors", "--json", "-dl"],
            &["-dL", "-0x", "a.c"],
            &["-dl", "--", "a.c"],
            &["-P"],
        ] {
            assert!(read(args).is_err(), "{args:?}");
        }
        for (args, refused) in [
            (&["--verify", "-b"][..], "-b"),
            (&["--verify", "-L", "-0x"], "-L"),
            (&["--verify", "--json"], "--json"),
            (&["--verify", "-u"], "-u"),
            (&["--verify", "-C"], "-C"),
            (&["--verify", "-k"], "-k"),
            (&["--verify", "-I", "include"], "-I"),
        ] {
            assert_eq!(
                read(args),
                Err(format!(
                    "--verify takes only -f INDEX and -d, not {refused}"
                )),
                "{args:?}"
            );
        }
        assert_eq!(
            read(&["--verify", "a.c"]),
            Err("--verify takes no file names".into())
        );
    }
}
