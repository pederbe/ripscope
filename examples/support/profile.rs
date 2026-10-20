//! A fresh user profile for the installer and editor setup checks: a home directory,
//! configuration, data and cache directories of its own, and the environment a new
//! user's editors and installers see there.
use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Windows PowerShell 5.1, which has a fixed home: checks may run with a PATH that names
/// System32 but not this directory, and Rust does not search beneath System32.
pub fn powershell() -> PathBuf {
    let root = std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into());
    Path::new(&root).join(r"System32\WindowsPowerShell\v1.0\powershell.exe")
}

pub struct Profile {
    pub root: PathBuf,
    pub home: PathBuf,
    pub env: Vec<(String, OsString)>,
}

impl Profile {
    /// A profile under ROOT, with no `ripscope` on PATH unless it is in EXTRA_PATH.
    pub fn new(root: PathBuf, extra_path: Option<&Path>) -> io::Result<Self> {
        let home = root.join("home");
        for name in [
            "home",
            "config",
            "data",
            "cache",
            "state",
            "localappdata",
            "appdata",
        ] {
            fs::create_dir_all(root.join(name))?;
        }
        let mut profile = Self {
            root,
            home,
            env: Vec::new(),
        };
        let home = profile.home.clone();
        profile.set("HOME", &home);
        if cfg!(windows) {
            profile.set("USERPROFILE", &home);
            profile.set("LOCALAPPDATA", profile.root.join("localappdata"));
            profile.set("APPDATA", profile.root.join("appdata"));
        } else {
            for (name, directory) in [
                ("XDG_CONFIG_HOME", "config"),
                ("XDG_DATA_HOME", "data"),
                ("XDG_CACHE_HOME", "cache"),
                ("XDG_STATE_HOME", "state"),
            ] {
                profile.set(name, profile.root.join(directory));
            }
        }
        let executable = if cfg!(windows) {
            "ripscope.exe"
        } else {
            "ripscope"
        };
        let mut path = extra_path
            .map(Path::to_owned)
            .into_iter()
            .collect::<Vec<_>>();
        path.extend(
            std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
                .filter(|directory| !directory.join(executable).exists()),
        );
        profile.set(
            "PATH",
            Path::new(&std::env::join_paths(path).map_err(io::Error::other)?),
        );
        Ok(profile)
    }

    pub fn set(&mut self, name: &str, value: impl AsRef<std::ffi::OsStr>) {
        self.env.retain(|(existing, _)| existing != name);
        self.env.push((name.to_owned(), value.as_ref().to_owned()));
    }

    /// Gives COMMAND this profile's environment, without the settings a user's own
    /// environment could carry into it.
    pub fn apply(&self, command: &mut Command) {
        for name in [
            "XDG_CONFIG_HOME",
            "XDG_DATA_HOME",
            "XDG_CACHE_HOME",
            "XDG_STATE_HOME",
            "RIPSCOPE_INSTALL_DIR",
            "RIPSCOPE_EDITORS",
            "RIPSCOPE_CACHE_DIR",
            "VIMINIT",
            "EXINIT",
            "VIM",
            "VIMRUNTIME",
            "MYVIMRC",
            "NVIM",
            "NVIM_APPNAME",
            "EMACSLOADPATH",
        ] {
            command.env_remove(name);
        }
        for (name, value) in &self.env {
            command.env(name, value);
        }
    }

    /// PROGRAM, run in the home directory with this profile's environment.
    pub fn command(&self, program: &Path) -> Command {
        let mut command = Command::new(program);
        self.apply(&mut command);
        command.current_dir(&self.home);
        command
    }

    /// Where the editors keep their indexes.
    pub fn cache(&self) -> PathBuf {
        if cfg!(windows) {
            self.root.join("localappdata/ripscope")
        } else {
            self.root.join("cache/ripscope")
        }
    }

    /// Where Neovim keeps its data, such as lazy.nvim's plugins.
    pub fn neovim_data(&self) -> PathBuf {
        if cfg!(windows) {
            self.root.join("localappdata/nvim-data")
        } else {
            self.root.join("data/nvim")
        }
    }

    /// Where Neovim reads its configuration.
    pub fn neovim_config(&self) -> PathBuf {
        if cfg!(windows) {
            self.root.join("localappdata/nvim")
        } else {
            self.root.join("config/nvim")
        }
    }

    /// Where the installers put the executable by default.
    pub fn installed_executable(&self) -> PathBuf {
        self.home.join(".local/bin").join(if cfg!(windows) {
            "ripscope.exe"
        } else {
            "ripscope"
        })
    }

    /// The editor integrations the installers place, with the archive directory each
    /// copies: Vim's and Neovim's packages and the Emacs integration.
    pub fn editor_directories(&self) -> Vec<(&'static str, PathBuf)> {
        if cfg!(windows) {
            vec![
                ("contrib/vim", self.home.join("vimfiles/pack/ripscope")),
                ("contrib/vim", self.neovim_data().join("site/pack/ripscope")),
                (
                    "contrib/emacs",
                    self.root.join("localappdata/ripscope/emacs"),
                ),
            ]
        } else {
            vec![
                ("contrib/vim", self.home.join(".vim/pack/ripscope")),
                ("contrib/vim", self.neovim_data().join("site/pack/ripscope")),
                ("contrib/emacs", self.root.join("data/ripscope/emacs")),
            ]
        }
    }
}
