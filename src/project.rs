//! An editor's Project (ADR 0014): the directory a search covers when no setting names
//! one, with the index Ripscope keeps for it in the user's cache directory and the
//! selection arguments that build it. `ripscope --project PATH` reports it, so that Vim,
//! Neovim and Emacs share one way of finding it and so one index per project.

use crate::refresh::{AUTOMATIC_LIST, AUTOMATIC_LIST_NAME, Sources};
use sha2::{Digest, Sha256};
use std::path::{Component, Path, PathBuf};

/// Where a search's project and index are looked for: the editor's current directory,
/// the user's home directory and Ripscope's cache directory.
pub(crate) struct Places {
    pub cwd: PathBuf,
    pub home: PathBuf,
    pub cache: PathBuf,
}

impl Places {
    /// The places this process's environment names: `$HOME` and `$XDG_CACHE_HOME` (else
    /// `~/.cache`) on Unix, `%USERPROFILE%` and `%LOCALAPPDATA%` on Windows. An absolute
    /// `RIPSCOPE_CACHE_DIR` is the cache directory itself on both, for a user whose usual
    /// cache path an editor cannot pass on, such as Vim with a space in it on Windows.
    pub(crate) fn from_environment(cwd: &Path) -> Result<Self, String> {
        let variable = |name: &str| {
            std::env::var_os(name)
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
        };
        let (home, cache) = if cfg!(windows) {
            let home = variable("USERPROFILE").ok_or("USERPROFILE is not set")?;
            let local = variable("LOCALAPPDATA").ok_or("LOCALAPPDATA is not set")?;
            (home, local.join("ripscope"))
        } else {
            let home = variable("HOME").ok_or("HOME is not set")?;
            let cache = variable("XDG_CACHE_HOME")
                .filter(|cache| cache.is_absolute())
                .unwrap_or_else(|| home.join(".cache"));
            (home, cache.join("ripscope"))
        };
        let cache = variable("RIPSCOPE_CACHE_DIR")
            .filter(|cache| cache.is_absolute())
            .unwrap_or(cache);
        Ok(Self {
            cwd: cwd.to_owned(),
            home: absolute(&home, cwd),
            cache: absolute(&cache, cwd),
        })
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Project {
    pub root: PathBuf,
    pub index: PathBuf,
    /// The arguments an index build or a session in `root` takes to select the
    /// project's sources: its `cscope.files`, or discovery with or without its
    /// subdirectories.
    pub selection: Vec<&'static str>,
    /// The selection rule an index build with `selection` stores.
    pub sources: Sources,
}

/// The project of PATH, a file or a directory: the nearest directory upward holding
/// `cscope.files`, else the nearest Git repository, else the current directory when PATH
/// is inside it, else the nearest existing directory holding PATH. A Git repository or a
/// current directory at
/// the home directory or a file system root is no project, and a project there is read
/// without its subdirectories.
pub(crate) fn find(path: &Path, places: &Places) -> Result<Project, String> {
    let path = absolute(path, &places.cwd);
    let start = if path.is_dir() {
        path
    } else {
        path.parent().map_or(path.clone(), Path::to_owned)
    };
    // A new file's directory need not exist yet: its nearest existing ancestor's does.
    let start = start
        .ancestors()
        .find(|directory| directory.is_dir())
        .map_or(start.clone(), Path::to_owned);
    // Home reached through a link, such as /home for /var/home, is still home.
    let home = physical(&places.home);
    let broad =
        |directory: &Path| same(&physical(directory), &home) || directory.parent().is_none();
    let listed = start
        .ancestors()
        .find(|directory| directory.join(AUTOMATIC_LIST_NAME).is_file())
        .map(Path::to_owned);
    let root = match &listed {
        Some(listed) => listed.clone(),
        None => {
            let cwd = absolute(&places.cwd, &places.cwd);
            let repository = start
                .ancestors()
                .find(|directory| directory.join(".git").exists() && !broad(directory));
            match repository {
                Some(repository) => repository.to_owned(),
                None if inside(&start, &cwd)? && !broad(&cwd) => cwd,
                None => start.clone(),
            }
        }
    };
    let (selection, sources) = if listed.is_some() {
        (
            vec!["-k", "-i", AUTOMATIC_LIST_NAME],
            Sources::List(AUTOMATIC_LIST.to_vec()),
        )
    } else {
        let recursive = !broad(&root);
        let selection = if recursive {
            vec!["-k", "-R"]
        } else {
            vec!["-k"]
        };
        (selection, Sources::Automatic { recursive })
    };
    // One index for the directory, however it is reached: through a link or not.
    let index = places.cache.join(index_name(&physical(&root))?);
    Ok(Project {
        root,
        index,
        selection,
        sources,
    })
}

/// The index file name of the project at ROOT: its directory's name, with every
/// character other than ASCII letters, digits, `.`, `_` and `-` made `_`, at most 40 of
/// them, then the first 16 hexadecimal digits of the SHA-256 of `key(ROOT)`.
pub(crate) fn index_name(root: &Path) -> Result<String, String> {
    let name = root
        .file_name()
        .map(|name| {
            name.to_string_lossy()
                .chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() || "._-".contains(c) {
                        c
                    } else {
                        '_'
                    }
                })
                .take(40)
                .collect::<String>()
        })
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "root".to_owned());
    let digest = Sha256::digest(key(root)?.as_bytes());
    let hex = digest
        .iter()
        .take(8)
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Ok(format!("{name}-{hex}.db"))
}

/// PATH as the editors are given it: with forward slashes on Windows, where a verbatim
/// `\\?\` prefix is dropped too.
pub(crate) fn text(path: &Path) -> Result<String, String> {
    let text = path
        .to_str()
        .ok_or_else(|| format!("{} is not valid Unicode", path.display()))?;
    if cfg!(windows) {
        let text = text.replace('\\', "/");
        return Ok(if let Some(rest) = text.strip_prefix("//?/UNC/") {
            format!("//{rest}")
        } else if let Some(rest) = text.strip_prefix("//?/") {
            rest.to_owned()
        } else {
            text
        });
    }
    Ok(text.to_owned())
}

/// What names a directory: its text, and on Windows, where names ignore case, that text
/// in lower case (Unicode's full lowercase mapping).
fn key(path: &Path) -> Result<String, String> {
    let text = text(path)?;
    Ok(if cfg!(windows) {
        text.to_lowercase()
    } else {
        text
    })
}

/// PATH with its links resolved, or as it is when that fails.
fn physical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_owned())
}

fn same(left: &Path, right: &Path) -> bool {
    match (key(left), key(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => left == right,
    }
}

fn inside(path: &Path, directory: &Path) -> Result<bool, String> {
    let (path, directory) = (key(path)?, key(directory)?);
    let prefix = format!("{}/", directory.trim_end_matches('/'));
    Ok(path == directory || path.starts_with(&prefix))
}

/// PATH made absolute against CWD, with `.` and `..` resolved by name and no link
/// followed, as editors name files.
pub(crate) fn absolute(path: &Path, cwd: &Path) -> PathBuf {
    let joined = if path.is_absolute() {
        path.to_owned()
    } else {
        cwd.join(path)
    };
    let mut resolved = PathBuf::new();
    for component in joined.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                resolved.pop();
            }
            other => resolved.push(other.as_os_str()),
        }
    }
    resolved
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    struct Fixture {
        _temporary: tempfile::TempDir,
        top: PathBuf,
        places: Places,
    }

    /// A home directory and a cache inside a temporary directory, with the editor's
    /// current directory at `cwd` below it.
    fn fixture(cwd: &str) -> Fixture {
        let temporary = tempfile::tempdir().unwrap();
        let top = absolute(temporary.path(), Path::new("/"));
        let home = top.join("home");
        fs::create_dir_all(home.join(cwd)).unwrap();
        Fixture {
            places: Places {
                cwd: home.join(cwd),
                home: home.clone(),
                cache: top.join("cache"),
            },
            top,
            _temporary: temporary,
        }
    }

    fn file(path: &Path) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "int x;\n").unwrap();
    }

    #[test]
    fn a_file_list_wins_over_a_repository_and_names_the_sources() {
        let f = fixture("");
        let root = f.places.home.join("kernel");
        file(&root.join("cscope.files"));
        fs::create_dir_all(root.join("drivers/net/.git")).unwrap();
        let source = root.join("drivers/net/card.c");
        file(&source);
        let project = find(&source, &f.places).unwrap();
        assert_eq!(project.root, root);
        assert_eq!(project.selection, ["-k", "-i", "cscope.files"]);
        assert_eq!(project.sources, Sources::List(b"cscope.files".to_vec()));
        assert_eq!(project.index.parent(), Some(f.places.cache.as_path()));
    }

    #[test]
    fn a_repository_is_read_with_its_subdirectories() {
        let f = fixture("elsewhere");
        let root = f.places.home.join("src/tool");
        fs::create_dir_all(root.join(".git")).unwrap();
        let source = root.join("lib/deep/greet.c");
        file(&source);
        let project = find(&source, &f.places).unwrap();
        assert_eq!(project.sources, Sources::Automatic { recursive: true });
        assert_eq!((project.root, project.selection), (root, vec!["-k", "-R"]));
        // A worktree or submodule marks itself with a .git file.
        let worktree = f.places.home.join("worktree");
        file(&worktree.join(".git"));
        file(&worktree.join("a.c"));
        assert_eq!(
            find(&worktree.join("a.c"), &f.places).unwrap().root,
            worktree
        );
    }

    #[test]
    fn a_home_repository_is_no_project() {
        let f = fixture("");
        fs::create_dir_all(f.places.home.join(".git")).unwrap();
        let notes = f.places.home.join("notes");
        file(&notes.join("scratch.c"));
        let project = find(&notes.join("scratch.c"), &f.places).unwrap();
        // The current directory is home, too broad: the file's own directory instead.
        assert_eq!((project.root, project.selection), (notes, vec!["-k", "-R"]));
        // A file directly in the home directory is read without subdirectories.
        file(&f.places.home.join("top.c"));
        let project = find(&f.places.home.join("top.c"), &f.places).unwrap();
        assert_eq!(project.sources, Sources::Automatic { recursive: false });
        assert_eq!(
            (project.root, project.selection),
            (f.places.home.clone(), vec!["-k"])
        );
    }

    #[cfg(unix)]
    #[test]
    fn home_reached_through_a_link_is_still_home() {
        let f = fixture("");
        let linked = f.top.join("linked-home");
        std::os::unix::fs::symlink(&f.places.home, &linked).unwrap();
        fs::create_dir_all(f.places.home.join(".git")).unwrap();
        file(&f.places.home.join("top.c"));
        let project = find(&linked.join("top.c"), &f.places).unwrap();
        assert_eq!((project.root, project.selection), (linked, vec!["-k"]));
    }

    #[test]
    fn an_unmarked_file_takes_the_current_directory_only_from_inside_it() {
        let f = fixture("work/project");
        let inside = f.places.cwd.join("src/main.c");
        file(&inside);
        assert_eq!(find(&inside, &f.places).unwrap().root, f.places.cwd);
        let outside = f.top.join("tmp/stray.c");
        file(&outside);
        let project = find(&outside, &f.places).unwrap();
        assert_eq!(project.root, f.top.join("tmp"));
        // A directory is its own start, and a new file in directories that do not exist
        // yet belongs to the nearest one that does.
        assert_eq!(
            find(&f.places.cwd.join("src"), &f.places).unwrap().root,
            f.places.cwd
        );
        let new = f.top.join("tmp/fresh/deeper/new.c");
        assert_eq!(find(&new, &f.places).unwrap().root, f.top.join("tmp"));
    }

    #[test]
    fn relative_paths_and_dots_resolve_against_the_current_directory() {
        let f = fixture("work");
        let root = f.places.cwd.join("project");
        fs::create_dir_all(root.join(".git")).unwrap();
        file(&root.join("a.c"));
        let project = find(Path::new("project/../project/./a.c"), &f.places).unwrap();
        assert_eq!(project.root, root);
    }

    #[test]
    fn index_names_keep_safe_characters_and_hash_the_whole_path() {
        let root = if cfg!(windows) {
            PathBuf::from(r"C:\Projects\My Café")
        } else {
            PathBuf::from("/home/me/My Café")
        };
        let key = if cfg!(windows) {
            "c:/projects/my café"
        } else {
            "/home/me/My Café"
        };
        let digest = format!("{:x}", Sha256::digest(key.as_bytes()));
        assert_eq!(
            index_name(&root).unwrap(),
            format!("My_Caf_-{}.db", &digest[..16])
        );
        let long = Path::new("/a").join("x".repeat(60));
        assert_eq!(index_name(&long).unwrap().len(), 40 + 1 + 16 + 3);
        let top = if cfg!(windows) { r"C:\" } else { "/" };
        assert!(index_name(Path::new(top)).unwrap().starts_with("root-"));
    }

    #[cfg(windows)]
    #[test]
    fn windows_names_ignore_case_and_verbatim_prefixes() {
        // The name keeps the directory's own case; the hash folds all of it.
        let hash = |path: &str| {
            let name = index_name(Path::new(path)).unwrap();
            name[name.len() - 19..].to_owned()
        };
        assert_eq!(hash(r"C:\Proj\ÄBC"), hash(r"c:\proj\äbc"));
        assert_eq!(
            index_name(Path::new(r"C:\Proj\Tool")).unwrap(),
            index_name(Path::new(r"c:/PROJ/Tool")).unwrap()
        );
        assert_eq!(text(Path::new(r"\\?\C:\proj")).unwrap(), "C:/proj");
        assert_eq!(
            text(Path::new(r"\\?\UNC\server\share\proj")).unwrap(),
            "//server/share/proj"
        );
    }
}
