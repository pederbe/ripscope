//! Exercise the installers through a loopback release mirror using exact archive bytes.
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

#[allow(dead_code)]
#[path = "support/capture.rs"]
mod capture;
#[path = "support/mirror.rs"]
mod mirror;
#[allow(dead_code)]
#[path = "support/profile.rs"]
mod profile;
use mirror::Mirror;
use profile::{Profile, powershell};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
const NOTICE_FIXTURES: &[&str] = &[
    "missing-rust-tree",
    "empty-rust-tree",
    "missing-copyright",
    "empty-copyright",
    "empty-rust-payload",
    "missing-dependency-tree",
    "empty-dependency-tree",
    "empty-dependency-payload",
];
fn ensure(ok: bool, message: &str) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(io::Error::other(message).into())
    }
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn script_path(path: &Path) -> Result<PathBuf> {
    let canonical = path.canonicalize()?;
    #[cfg(windows)]
    return powershell_path(canonical);
    #[cfg(not(windows))]
    Ok(canonical)
}
#[cfg(windows)]
fn powershell_path(canonical: PathBuf) -> Result<PathBuf> {
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use std::path::{Component, Prefix};
    // PowerShell 5.1 providers need ordinary DOS/UNC paths, retaining all UTF-16.
    let wide = canonical.as_os_str().encode_wide().collect::<Vec<_>>();
    let normal = match canonical.components().next() {
        Some(Component::Prefix(prefix)) => match prefix.kind() {
            Prefix::VerbatimDisk(_) => wide[4..].to_vec(),
            Prefix::VerbatimUNC(_, _) => {
                let mut normal = vec![u16::from(b'\\'); 2];
                normal.extend_from_slice(&wide[8..]);
                normal
            }
            Prefix::Verbatim(_) | Prefix::DeviceNS(_) => {
                return Err("PowerShell checks do not support Windows device paths".into());
            }
            _ => return Ok(canonical),
        },
        _ => return Ok(canonical),
    };
    Ok(PathBuf::from(std::ffi::OsString::from_wide(&normal)))
}

#[cfg(all(test, windows))]
mod path_tests {
    use super::*;
    use std::os::windows::ffi::{OsStrExt, OsStringExt};

    #[test]
    fn dos_unc_and_non_ascii_paths_cross_the_powershell_provider_boundary() {
        for (canonical, ordinary) in [
            (r"\\?\C:\install\bin", r"C:\install\bin"),
            (r"\\?\UNC\server\share\bin", r"\\server\share\bin"),
            (r"\\?\C:\café\航空", r"C:\café\航空"),
            (r"\\?\UNC\server\share\café", r"\\server\share\café"),
        ] {
            let actual = powershell_path(PathBuf::from(canonical)).unwrap();
            assert_eq!(actual, PathBuf::from(ordinary));
            let output = Command::new(powershell())
                .args(["-NoProfile", "-NonInteractive", "-Command",
                    "[Console]::OutputEncoding = New-Object Text.UTF8Encoding $false; Join-Path -Path $env:RIPSCOPE_PATH_FIXTURE -ChildPath 'ripscope.exe'"])
                .env("RIPSCOPE_PATH_FIXTURE", &actual)
                .output().unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(
                String::from_utf8(output.stdout).unwrap().trim(),
                format!("{ordinary}\\ripscope.exe")
            );
        }
    }

    #[test]
    fn canonical_path_conversion_preserves_unpaired_utf16() {
        let mut canonical = r"\\?\C:\bin\".encode_utf16().collect::<Vec<_>>();
        canonical.push(0xd800);
        let path = PathBuf::from(std::ffi::OsString::from_wide(&canonical));
        let actual = powershell_path(path).unwrap();
        assert_eq!(
            actual.as_os_str().encode_wide().collect::<Vec<_>>(),
            canonical[4..]
        );
    }

    #[test]
    fn device_paths_are_explicitly_unsupported() {
        for path in [
            r"\\?\Volume{12345678-1234-1234-1234-123456789abc}\bin",
            r"\\.\pipe\ripscope",
        ] {
            assert!(powershell_path(PathBuf::from(path)).is_err());
        }
    }
}
struct Checks {
    work: PathBuf,
    installer: PathBuf,
    mirror: Mirror,
    version: String,
    receipts: Vec<Value>,
    /// A user profile of its own, since the installer places the editor integrations.
    profile: Profile,
}

/// What an installer's refusal of a fixture says, the same on both platforms.
fn refusal(kind: &str) -> &'static str {
    match kind {
        "traversal" => "unsafe archive path",
        "symlink-entry" | "hardlink-entry" | "linked-entry" => "linked or special",
        "missing-copyright" | "empty-copyright" | "missing-rust-tree" | "empty-rust-tree" => {
            "mandatory archive file: licenses/rust/COPYRIGHT-library.html"
        }
        "empty-rust-payload" => "nonempty rust licence payloads",
        "missing-dependency-tree" => "missing dependencies licence notices",
        "empty-dependency-tree" | "empty-dependency-payload" => {
            "nonempty dependencies licence payloads"
        }
        _ => "",
    }
}
impl Checks {
    fn command(&mut self, label: &str, command: Command, success: bool) -> Result<()> {
        let result = capture::run_command(command, &self.work.join(label), 90, &[])?;
        if !success && !refusal(label).is_empty() {
            // The refusal must be for the reason the fixture tests.
            let said = [&result["stdout_path"], &result["stderr_path"]]
                .iter()
                .filter_map(|path| path.as_str())
                .filter_map(|path| fs::read(path).ok())
                .map(|bytes| String::from_utf8_lossy(&bytes).to_lowercase())
                .collect::<String>();
            ensure(
                said.contains(&refusal(label).to_lowercase()),
                &format!("{label}: refused without saying {:?}", refusal(label)),
            )?;
        }
        let passed = if success {
            result["complete"] == true
        } else {
            result["status"].as_i64().is_some_and(|code| code != 0)
                && result["cleanup"]["state"] == "confirmed"
        };
        self.receipts
            .push(json!({"check":label,"expected_success":success,"capture":result}));
        capture::save(&self.work.join("checks.json"), &json!(self.receipts))?;
        ensure(
            passed,
            &format!("{label}: unexpected command outcome; inspect retained captures"),
        )
    }
    fn install(&mut self, label: &str, success: bool, path: Option<&Path>) -> Result<()> {
        let mut command = if cfg!(windows) {
            let mut command = Command::new(powershell());
            command.args([
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
            ]);
            command
        } else {
            Command::new("sh")
        };
        self.profile.apply(&mut command);
        command
            .arg(&self.installer)
            .env("RIPSCOPE_VERSION", &self.version)
            .env("RIPSCOPE_INSTALL_DIR", self.work.join("installation"))
            .env("RIPSCOPE_RELEASES", &self.mirror.url);
        if let Some(path) = path {
            command.env("PATH", path);
        }
        self.command(label, command, success)
    }
    /// The checkout's installer SCRIPT run as the editors' install command, with
    /// `--force` when FORCE, and PATH when given.
    fn plugin(
        &mut self,
        label: &str,
        script: &Path,
        force: bool,
        success: bool,
        path: Option<&Path>,
    ) -> Result<()> {
        let mut command = if cfg!(windows) {
            let mut command = Command::new(powershell());
            command.args([
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
            ]);
            command.arg(script).arg("-Plugin");
            if force {
                command.arg("-Force");
            }
            command
        } else {
            let mut command = Command::new("sh");
            command.arg(script).arg("--plugin");
            if force {
                command.arg("--force");
            }
            command
        };
        self.profile.apply(&mut command);
        command
            .env_remove("RIPSCOPE_VERSION")
            .env("RIPSCOPE_INSTALL_DIR", self.work.join("installation"))
            .env("RIPSCOPE_RELEASES", &self.mirror.url);
        if let Some(path) = path {
            command.env("PATH", path);
        }
        self.command(label, command, success)
    }
    /// The editor integrations the installer places in the profile.
    fn editor_directories(&self) -> Vec<PathBuf> {
        self.profile
            .editor_directories()
            .into_iter()
            .map(|(_, path)| path)
            .collect()
    }
    fn valid_editors(&self) -> Result<()> {
        let [vim, neovim, emacs] = <[PathBuf; 3]>::try_from(self.editor_directories())
            .map_err(|_| "three editor directories")?;
        for (directory, files) in [
            (
                vim.join("start/ripscope"),
                &["plugin/ripscope.vim", "lua/ripscope.lua"][..],
            ),
            (
                neovim.join("start/ripscope"),
                &["plugin/ripscope.lua", "lua/ripscope/session.lua"][..],
            ),
            (emacs, &["ripscope.el"][..]),
        ] {
            for file in files {
                ensure(
                    directory.join(file).is_file(),
                    &format!(
                        "missing placed editor file {}",
                        directory.join(file).display()
                    ),
                )?;
            }
        }
        Ok(())
    }
    fn notices(&self) -> Result<Vec<PathBuf>> {
        let mut result = Vec::new();
        for entry in fs::read_dir(self.work.join("installation"))? {
            let entry = entry?;
            if entry
                .file_name()
                .to_string_lossy()
                .starts_with(".ripscope-notices-")
            {
                result.push(entry.path());
            }
        }
        Ok(result)
    }
    fn valid_notices(&self) -> Result<()> {
        let notices = self.notices()?;
        ensure(!notices.is_empty(), "no retained notices")?;
        for directory in notices {
            for file in [
                "LICENSE-MIT",
                "LICENSE-APACHE",
                "BUILD.json",
                "DEPENDENCIES.json",
                "ORIGINAL-ARCHIVE-MANIFEST.sha256",
                "INSTALLATION.txt",
                "licenses/SQLITE-NOTICE.txt",
                "licenses/rust/COPYRIGHT-library.html",
                "contrib/emacs/ripscope.el",
                "contrib/vim/plugin/ripscope.vim",
                "contrib/vim/lua/ripscope.lua",
            ] {
                ensure(
                    directory.join(file).is_file(),
                    &format!("missing retained {file}"),
                )?;
            }
        }
        Ok(())
    }
}
#[cfg(unix)]
fn regular_files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            files.extend(regular_files(&entry.path())?);
        } else {
            ensure(entry.file_type()?.is_file(), "nonregular fixture input")?;
            files.push(entry.path());
        }
    }
    Ok(files)
}
#[cfg(unix)]
fn remove_notice(package: &Path, kind: &str) -> Result<()> {
    let rust = package.join("licenses/rust");
    let dependencies = package.join("licenses/dependencies");
    match kind {
        "missing-copyright" => fs::remove_file(rust.join("COPYRIGHT-library.html"))?,
        "empty-copyright" => fs::write(rust.join("COPYRIGHT-library.html"), [])?,
        "missing-rust-tree" | "empty-rust-tree" => {
            fs::remove_dir_all(&rust)?;
            if kind == "empty-rust-tree" {
                fs::create_dir(&rust)?;
            }
        }
        "missing-dependency-tree" | "empty-dependency-tree" => {
            fs::remove_dir_all(&dependencies)?;
            if kind == "empty-dependency-tree" {
                fs::create_dir(&dependencies)?;
            }
        }
        "empty-rust-payload" => {
            for file in regular_files(&rust)? {
                if file
                    .file_name()
                    .is_some_and(|name| name != "COPYRIGHT-library.html")
                {
                    fs::write(file, [])?;
                }
            }
        }
        "empty-dependency-payload" => {
            for file in regular_files(&dependencies)? {
                fs::write(file, [])?;
            }
        }
        _ => return Err("unknown notice fixture".into()),
    }
    // These fixtures have valid inventory and hashes: only notice policy can reject them.
    let mut manifest = BTreeMap::new();
    for file in regular_files(package)? {
        let relative = file.strip_prefix(package)?.to_string_lossy();
        if relative != "MANIFEST.sha256" {
            manifest.insert(relative.into_owned(), capture::digest(&file)?);
        }
    }
    fs::write(
        package.join("MANIFEST.sha256"),
        manifest
            .into_iter()
            .map(|(file, digest)| format!("{digest}  {file}\n"))
            .collect::<String>(),
    )?;
    Ok(())
}
#[cfg(unix)]
fn tar_entry(name: &str, kind: u8) -> Vec<u8> {
    let mut header = [0_u8; 512];
    header[..name.len()].copy_from_slice(name.as_bytes());
    for (offset, text) in [
        (100, "0000644\0"),
        (108, "0000000\0"),
        (116, "0000000\0"),
        (124, "00000000000\0"),
        (136, "00000000000\0"),
    ] {
        header[offset..offset + text.len()].copy_from_slice(text.as_bytes());
    }
    header[148..156].fill(b' ');
    header[156] = kind;
    if kind == b'1' || kind == b'2' {
        header[157..164].copy_from_slice(b"outside");
    }
    header[257..263].copy_from_slice(b"ustar\0");
    header[263..265].copy_from_slice(b"00");
    let sum: u32 = header.iter().map(|&byte| u32::from(byte)).sum();
    header[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
    let mut archive = header.to_vec();
    archive.resize(1536, 0);
    archive
}
/// Release orderings the editors' install command decides by: whether the first version,
/// installed, is older than the second, wanted.
const ORDERINGS: &[(&str, &str, bool)] = &[
    ("0.0.1", "0.1.0", true),
    ("0.1.0", "0.1.0", false),
    ("0.2.0", "0.1.0", false),
    ("0.9.0", "0.10.0", true),
    ("0.10.0", "0.9.0", false),
    ("1", "1.0.1", true),
    ("0.1.0-rc1", "0.1.0", true),
    ("0.1.0", "0.1.0-rc1", false),
    ("0.1.0-rc1", "0.1.0-rc2", false),
    ("", "0.1.0", true),
];
/// The editors' install command: the installer of a checkout with `--plugin`, which
/// installs the checkout's version unless the ripscope the editors find is that version
/// or newer, or not one the installer put there.
fn plugin_checks(
    checks: &mut Checks,
    executable: &Path,
    expected_hash: &str,
    archive_name: &str,
    archive: &[u8],
) -> Result<()> {
    let checkout = checks.work.join("checkout");
    fs::create_dir_all(&checkout)?;
    let script = checkout.join(if cfg!(windows) {
        "install.ps1"
    } else {
        "install.sh"
    });
    fs::copy(&checks.installer, &script)?;
    let manifest = |version: &str| {
        fs::write(
            checkout.join("Cargo.toml"),
            format!("[package]\nname = \"ripscope\"\nversion = \"{version}\"\n"),
        )
    };
    let unchanged = |label: &str| -> Result<()> {
        ensure(
            capture::digest(executable)? == expected_hash,
            &format!("{label}: the installed executable changed"),
        )
    };
    // Every download fails, so what the installer keeps it keeps without one.
    checks
        .mirror
        .archive(&checks.version, archive_name, archive, false);
    manifest(&checks.version)?;
    checks.plugin("plugin-current", &script, false, true, None)?;
    unchanged("plugin-current")?;
    // A newer checkout: the installer's own executable is upgraded, so it downloads.
    manifest("9.9.9")?;
    checks.plugin("plugin-older", &script, false, false, None)?;
    unchanged("plugin-older")?;
    // Another ripscope first on PATH, such as a system package's, is left alone.
    let other = checks.work.join("other-bin");
    fs::create_dir_all(&other)?;
    fs::copy(
        executable,
        other.join(executable.file_name().ok_or("no name")?),
    )?;
    let mut search = vec![other];
    search.extend(std::env::split_paths(
        &checks
            .profile
            .env
            .iter()
            .find(|(name, _)| name == "PATH")
            .map(|(_, value)| value.clone())
            .unwrap_or_default(),
    ));
    let search = std::env::join_paths(search)?;
    checks.plugin(
        "plugin-other",
        &script,
        false,
        true,
        Some(Path::new(&search)),
    )?;
    unchanged("plugin-other")?;
    // Forced, and with the release served, it installs the checkout's version.
    checks
        .mirror
        .archive(&checks.version, archive_name, archive, true);
    manifest(&checks.version)?;
    checks.plugin("plugin-force", &script, true, true, None)?;
    unchanged("plugin-force")?;
    checks
        .mirror
        .archive(&checks.version, archive_name, archive, false);
    plugin_orderings(checks, &script, executable, &manifest)?;
    checks
        .mirror
        .archive(&checks.version, archive_name, archive, true);
    Ok(())
}
#[cfg(unix)]
fn plugin_orderings(
    checks: &mut Checks,
    script: &Path,
    executable: &Path,
    manifest: &dyn Fn(&str) -> io::Result<()>,
) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let kept = checks.work.join("kept-ripscope");
    fs::rename(executable, &kept)?;
    for (index, (have, wanted, older)) in ORDERINGS.iter().enumerate() {
        // An installed executable that reports HAVE, or nothing, while no download works.
        let said = if have.is_empty() {
            "exit 1".to_owned()
        } else {
            format!("echo 'ripscope {have}'")
        };
        fs::write(executable, format!("#!/bin/sh\n{said}\n"))?;
        fs::set_permissions(executable, fs::Permissions::from_mode(0o755))?;
        manifest(wanted)?;
        checks.plugin(
            &format!("plugin-order-{index}"),
            script,
            false,
            !older,
            None,
        )?;
    }
    fs::rename(kept, executable)?;
    Ok(())
}
#[cfg(windows)]
fn plugin_orderings(
    checks: &mut Checks,
    _script: &Path,
    _executable: &Path,
    _manifest: &dyn Fn(&str) -> io::Result<()>,
) -> Result<()> {
    let table = ORDERINGS
        .iter()
        .map(|(have, wanted, older)| format!("@('{have}', '{wanted}', ${older})"))
        .collect::<Vec<_>>()
        .join(", ");
    let path = checks.work.join("plugin-orderings.ps1");
    fs::write(
        &path,
        format!(
            "param($Installer)\n$ErrorActionPreference = 'Stop'\n. $Installer\nforeach ($row in @({table})) {{\n    if ((Test-RipscopeOlder $row[0] $row[1]) -ne $row[2]) {{ throw \"Ordering of '$($row[0])' before '$($row[1])' is not $($row[2])\" }}\n}}\n"
        ),
    )?;
    let mut command = Command::new(powershell());
    command
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(&path)
        .arg("-Installer")
        .arg(&checks.installer);
    checks.profile.apply(&mut command);
    checks.command("plugin-orderings", command, true)
}
#[cfg(unix)]
fn linux_checks(
    checks: &mut Checks,
    archive_name: &str,
    archive: &[u8],
    executable: &Path,
    expected_hash: &str,
) -> Result<()> {
    use std::os::unix::fs::{PermissionsExt, symlink};
    for (label, name, kind) in [
        ("traversal", "../escaped", b'0'),
        (
            "symlink-entry",
            "ripscope-0.1.0-x86_64-linux-gnu/bin/ripscope",
            b'2',
        ),
        (
            "hardlink-entry",
            "ripscope-0.1.0-x86_64-linux-gnu/bin/ripscope",
            b'1',
        ),
    ] {
        let input = checks.work.join(format!("{label}.tar"));
        fs::write(&input, tar_entry(name, kind))?;
        let output = Command::new("gzip").args(["-c"]).arg(&input).output()?;
        ensure(output.status.success(), "fixture gzip failed")?;
        checks
            .mirror
            .archive(&checks.version, archive_name, &output.stdout, true);
        checks.install(label, false, None)?;
        ensure(
            capture::digest(executable)? == expected_hash,
            "refusal changed old executable",
        )?;
    }
    ensure(
        !checks.work.join("escaped").exists(),
        "archive escaped extraction",
    )?;
    let original = checks.work.join("original.tar.gz");
    fs::write(&original, archive)?;
    let package_name = archive_name
        .strip_suffix(".tar.gz")
        .ok_or("expected tar.gz archive")?;
    for kind in NOTICE_FIXTURES {
        let fixture = checks.work.join(kind);
        fs::create_dir(&fixture)?;
        let mut extract = Command::new("tar");
        extract.arg("-xzf").arg(&original).arg("-C").arg(&fixture);
        checks.command(&format!("prepare-{kind}"), extract, true)?;
        let package = fixture.join(package_name);
        remove_notice(&package, kind)?;
        let mut verify = Command::new("sha256sum");
        verify.args(["-c", "MANIFEST.sha256"]).current_dir(&package);
        checks.command(&format!("verify-{kind}-manifest"), verify, true)?;
        let broken = checks.work.join(format!("{kind}.tar.gz"));
        let mut pack = Command::new("tar");
        pack.arg("-czf")
            .arg(&broken)
            .arg("-C")
            .arg(&fixture)
            .arg(package_name);
        checks.command(&format!("pack-{kind}"), pack, true)?;
        checks
            .mirror
            .archive(&checks.version, archive_name, &fs::read(broken)?, true);
        let before = checks.notices()?.len();
        checks.install(kind, false, None)?;
        ensure(
            capture::digest(executable)? == expected_hash && checks.notices()?.len() == before,
            "notice-policy refusal changed executable or retained uncommitted notices",
        )?;
    }
    checks
        .mirror
        .archive(&checks.version, archive_name, archive, true);
    fs::remove_file(executable)?;
    fs::create_dir(executable)?;
    checks.install("directory-destination", false, None)?;
    ensure(executable.is_dir(), "directory destination changed")?;
    fs::remove_dir(executable)?;
    let unrelated = checks.work.join("unrelated-link-target");
    fs::write(&unrelated, "keep")?;
    symlink(&unrelated, executable)?;
    checks.install("symlink-destination", false, None)?;
    ensure(
        fs::read(&unrelated)? == b"keep",
        "symbolic-link target changed",
    )?;
    fs::remove_file(executable)?;
    checks.install("restore-install", true, None)?;
    let copied_tools = checks.work.join("copied-link-tools");
    fs::create_dir(&copied_tools)?;
    let real_cp = Command::new("sh").args(["-c", "command -v cp"]).output()?;
    let real_cp = String::from_utf8(real_cp.stdout)?.trim().to_owned();
    let cp = copied_tools.join("cp");
    fs::write(
        &cp,
        format!(
            "#!/bin/sh\n'{real_cp}' \"$@\"\nln -s /nonexistent-ripscope-fixture \"$3/copied-link\"\n"
        ),
    )?;
    fs::set_permissions(&cp, fs::Permissions::from_mode(0o755))?;
    let mut search = vec![copied_tools];
    search.extend(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    ));
    let search = std::env::join_paths(search)?;
    let before = checks.notices()?.len();
    checks.install("copied-link-refusal", false, Some(Path::new(&search)))?;
    ensure(
        capture::digest(executable)? == expected_hash && checks.notices()?.len() == before,
        "copied-link refusal changed executable or retained uncommitted notices",
    )?;
    let before = checks.notices()?.len();
    let tools = checks.work.join("interruption-tools");
    fs::create_dir(&tools)?;
    let real_mv = Command::new("sh").args(["-c", "command -v mv"]).output()?;
    let real_mv = String::from_utf8(real_mv.stdout)?.trim().to_owned();
    let shim = tools.join("mv");
    fs::write(
        &shim,
        format!(
            "#!/bin/sh\nif [ \"$1\" = -fT ]; then\n '{real_mv}' \"$@\"\n kill -INT \"$PPID\"\n exit 0\nfi\nexec '{real_mv}' \"$@\"\n"
        ),
    )?;
    fs::set_permissions(&shim, fs::Permissions::from_mode(0o755))?;
    let mut search = vec![tools];
    search.extend(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    ));
    let search = std::env::join_paths(search)?;
    checks.install("interrupt-after-rename", false, Some(Path::new(&search)))?;
    ensure(
        capture::digest(executable)? == expected_hash,
        "committed executable missing after interrupt",
    )?;
    ensure(
        checks.notices()?.len() == before + 1,
        "interrupt deleted committed notices",
    )?;
    checks.valid_notices()?;
    Ok(())
}
#[cfg(windows)]
fn windows_checks(
    checks: &mut Checks,
    executable: &Path,
    archive_path: &Path,
    archive_name: &str,
    archive: &[u8],
) -> Result<()> {
    let expected = capture::digest(executable)?;
    let fixture_script = checks.work.join("zip-fixture.ps1");
    fs::write(&fixture_script, ZIP_FIXTURE)?;
    for kind in ["traversal", "linked-entry"]
        .into_iter()
        .chain(NOTICE_FIXTURES.iter().copied())
    {
        let fixture = checks.work.join(format!("{kind}.zip"));
        let mut command = Command::new(powershell());
        command
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
            ])
            .arg(&fixture_script)
            .arg("-Destination")
            .arg(&fixture)
            .arg("-Original")
            .arg(archive_path)
            .arg("-Name")
            .arg(archive_name.trim_end_matches(".zip"))
            .arg("-Kind")
            .arg(kind);
        checks.command(&format!("prepare-{kind}"), command, true)?;
        checks
            .mirror
            .archive(&checks.version, archive_name, &fs::read(fixture)?, true);
        let before = checks.notices()?.len();
        checks.install(kind, false, None)?;
        ensure(
            capture::digest(executable)? == expected && checks.notices()?.len() == before,
            "ZIP refusal changed existing executable or retained uncommitted notices",
        )?;
    }
    checks
        .mirror
        .archive(&checks.version, archive_name, archive, true);
    // Rust requests Developer Mode's unprivileged symlink capability on Windows.
    // Windows PowerShell 5.1's New-Item still requires elevation for this fixture.
    fs::remove_file(executable)?;
    let target = checks.work.join("unrelated-link-target");
    fs::write(&target, "keep")?;
    std::os::windows::fs::symlink_file(&target, executable)?;
    checks.install("symlink-destination", false, None)?;
    ensure(
        fs::read(&target)? == b"keep",
        "symbolic-link target changed",
    )?;
    ensure(
        fs::symlink_metadata(executable)?.file_type().is_symlink(),
        "destination symlink changed",
    )?;
    fs::remove_file(executable)?;
    checks.install("restore-install", true, None)?;
    let path = checks.work.join("native-filesystem.ps1");
    fs::write(&path, WINDOWS_CHECKS)?;
    let mut command = Command::new(powershell());
    command
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(path)
        .arg("-Installer")
        .arg(&checks.installer)
        .arg("-Work")
        .arg(&checks.work)
        .arg("-Executable")
        .arg(executable)
        .arg("-Mirror")
        .arg(&checks.mirror.url)
        .arg("-Version")
        .arg(&checks.version);
    // Its installs place editor integrations too: in the check's own profile.
    checks.profile.apply(&mut command);
    checks.command("native-filesystem", command, true)
}
#[cfg(windows)]
const ZIP_FIXTURE: &str = r#"
param($Destination, $Original, $Name, $Kind)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.IO.Compression.FileSystem
if ($Kind -ne 'traversal' -and $Kind -ne 'linked-entry') {
    $fixture = "$Destination.unpacked"
    [IO.Compression.ZipFile]::ExtractToDirectory($Original, $fixture)
    $package = Join-Path $fixture $Name
    $rust = Join-Path $package 'licenses/rust'
    $dependencies = Join-Path $package 'licenses/dependencies'
    switch ($Kind) {
        'missing-copyright' { [IO.File]::Delete((Join-Path $rust 'COPYRIGHT-library.html')) }
        'empty-copyright' { [IO.File]::WriteAllBytes((Join-Path $rust 'COPYRIGHT-library.html'), [byte[]] @()) }
        'missing-rust-tree' { Remove-Item -LiteralPath $rust -Recurse -Force }
        'empty-rust-tree' { Remove-Item -LiteralPath $rust -Recurse -Force; [IO.Directory]::CreateDirectory($rust) | Out-Null }
        'missing-dependency-tree' { Remove-Item -LiteralPath $dependencies -Recurse -Force }
        'empty-dependency-tree' { Remove-Item -LiteralPath $dependencies -Recurse -Force; [IO.Directory]::CreateDirectory($dependencies) | Out-Null }
        'empty-rust-payload' {
            foreach ($file in Get-ChildItem -LiteralPath $rust -File -Recurse -Force | Where-Object { $_.Name -ne 'COPYRIGHT-library.html' }) {
                [IO.File]::WriteAllBytes($file.FullName, [byte[]] @())
            }
        }
        'empty-dependency-payload' {
            foreach ($file in Get-ChildItem -LiteralPath $dependencies -File -Recurse -Force) { [IO.File]::WriteAllBytes($file.FullName, [byte[]] @()) }
        }
        default { throw "Unknown notice fixture: $Kind" }
    }
    # Rebuild both inventory and hashes so refusal tests isolate mandatory notice policy.
    $manifest = Join-Path $package 'MANIFEST.sha256'
    $records = @(Get-ChildItem -LiteralPath $package -File -Recurse -Force | Where-Object { $_.FullName -ne $manifest } | ForEach-Object {
        $relative = $_.FullName.Substring($package.Length + 1).Replace('\', '/')
        (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant() + '  ' + $relative
    } | Sort-Object)
    [IO.File]::WriteAllText($manifest, ($records -join "`n") + "`n", (New-Object Text.UTF8Encoding $false))
    # Entry by entry with forward slashes, as release archives name them.
    $zip = [IO.Compression.ZipFile]::Open($Destination, 'Create')
    try {
        foreach ($file in Get-ChildItem -LiteralPath $fixture -File -Recurse -Force) {
            $entry = $file.FullName.Substring($fixture.Length + 1).Replace('\', '/')
            [void][IO.Compression.ZipFileExtensions]::CreateEntryFromFile($zip, $file.FullName, $entry)
        }
        # An empty directory, such as an emptied notice tree, is an entry of its own.
        foreach ($directory in Get-ChildItem -LiteralPath $fixture -Directory -Recurse -Force) {
            if (@(Get-ChildItem -LiteralPath $directory.FullName -Force).Count -eq 0) {
                [void]$zip.CreateEntry($directory.FullName.Substring($fixture.Length + 1).Replace('\', '/') + '/')
            }
        }
    } finally { $zip.Dispose() }
    Write-Output "Notice fixture with regenerated valid manifest: $Kind"
    return
}
$output = [IO.Compression.ZipFile]::Open($Destination, 'Create')
try {
    if ($Kind -eq 'traversal') { $output.CreateEntry('../escaped') | Out-Null }
    else {
        $entry = $output.CreateEntry("$Name/bin/ripscope.exe")
        $entry.ExternalAttributes = -1577123840
    }
} finally { $output.Dispose() }
"#;
#[cfg(windows)]
const WINDOWS_CHECKS: &str = r#"
param($Installer, $Work, $Executable, $Mirror, $Version)
$ErrorActionPreference = 'Stop'
. $Installer
function Check($Condition, $Message) { if (-not $Condition) { throw $Message } }
$old = Get-RipscopeHash $Executable
$lock = [IO.File]::Open($Executable, 'Open', 'Read', 'Read')
try {
    $failed = $false
    try { Install-Ripscope -Version $Version -InstallDir ([IO.Path]::GetDirectoryName($Executable)) -Releases $Mirror }
    catch { $failed = $true; Write-Output $_ }
    Check $failed 'Locked executable replacement unexpectedly succeeded'
    Check ((Get-RipscopeHash $Executable) -eq $old) 'Locked executable changed'
} finally { $lock.Dispose() }
$recovery = Join-Path $Work 'recovery'
[IO.Directory]::CreateDirectory($recovery) | Out-Null
$dest = Join-Path $recovery 'destination.exe'
$backup = Join-Path $recovery 'backup.exe'
[IO.File]::WriteAllText($backup, 'old')
$old = Get-RipscopeHash $backup
Check (Restore-RipscopeExecutable $dest $backup $old) 'Absent destination was not restored'
Check ((Get-RipscopeHash $dest) -eq $old) 'Restored bytes differ'
[IO.File]::WriteAllText($backup, 'old')
Check (-not (Restore-RipscopeExecutable $dest $backup $old)) 'Existing destination overwritten'
[IO.File]::Delete($dest)
[IO.File]::WriteAllText($backup, 'wrong')
Check (-not (Restore-RipscopeExecutable $dest $backup $old)) 'Wrong backup restored'
[IO.File]::WriteAllText($dest, 'new')
$new = Get-RipscopeHash $dest
$lock = [IO.File]::Open($backup, 'Open', 'Read', 'Read')
try {
    Check (-not (Complete-RipscopePublication $dest $backup $new)) 'Cleanup fault not reported'
    Check ((Get-RipscopeHash $dest) -eq $new) 'Cleanup fault changed committed executable'
    Check (Test-Path -LiteralPath $backup) 'Cleanup fault discarded backup'
} finally { $lock.Dispose() }
[IO.File]::Delete($Executable)
[IO.Directory]::CreateDirectory($Executable) | Out-Null
try {
    $failed = $false
    try { Install-Ripscope -Version $Version -InstallDir ([IO.Path]::GetDirectoryName($Executable)) -Releases $Mirror }
    catch { $failed = $true }
    Check $failed 'Directory destination accepted'
} finally { [IO.Directory]::Delete($Executable) }
Install-Ripscope -Version $Version -InstallDir ([IO.Path]::GetDirectoryName($Executable)) -Releases $Mirror
# Uninstalling while an editor runs Ripscope says so and removes everything else.
$editors = Get-RipscopeEditorPaths
Check ([IO.Directory]::Exists($editors.Vim)) 'Vim package not placed'
$lock = [IO.File]::Open($Executable, 'Open', 'Read', 'Read')
try {
    $message = $null
    try { Uninstall-Ripscope ([IO.Path]::GetDirectoryName($Executable)) 3>&1 | Write-Output }
    catch { $message = "$_" }
    Check ($message -match 'in use') "Uninstall in use did not say so: $message"
    Check (Test-Path -LiteralPath $Executable) 'Executable in use disappeared'
    foreach ($directory in @($editors.Vim, $editors.Neovim, $editors.Emacs)) {
        Check (-not [IO.Directory]::Exists($directory)) "Uninstall in use kept $directory"
    }
} finally { $lock.Dispose() }
Install-Ripscope -Version $Version -InstallDir ([IO.Path]::GetDirectoryName($Executable)) -Releases $Mirror
Write-Output 'Native locked/directory/recovery/postcommit-cleanup/uninstall-in-use checks passed'
"#;
fn main() {
    if let Err(error) = run() {
        eprintln!("installer check: {error}");
        std::process::exit(1);
    }
}
fn run() -> Result<()> {
    let mut options = BTreeMap::new();
    let mut args = std::env::args_os().skip(1);
    while let Some(key) = args.next() {
        if key == "--help" {
            println!(
                "check-installers --archive FILE --installer FILE --work NEW_DIRECTORY [--version VERSION]"
            );
            return Ok(());
        }
        options.insert(key, args.next().ok_or("missing option value")?);
    }
    let path = |name: &str| -> Result<PathBuf> {
        Ok(PathBuf::from(
            options
                .get(std::ffi::OsStr::new(name))
                .ok_or("missing required option")?,
        ))
    };
    let archive_path = script_path(&path("--archive")?)?;
    let installer = script_path(&path("--installer")?)?;
    let work = path("--work")?;
    ensure(!work.exists(), "work directory must be new")?;
    fs::create_dir_all(&work)?;
    let work = script_path(&work)?;
    let version = options
        .get(std::ffi::OsStr::new("--version"))
        .map_or(env!("CARGO_PKG_VERSION").to_owned(), |value| {
            value.to_string_lossy().into_owned()
        });
    let archive_name = archive_path
        .file_name()
        .ok_or("archive filename missing")?
        .to_string_lossy()
        .into_owned();
    let archive = fs::read(&archive_path)?;
    let mirror = Mirror::start(&version)?;
    mirror.archive(&version, &archive_name, &archive, true);
    let profile = Profile::new(work.join("profile"), None)?;
    let mut checks = Checks {
        work,
        installer,
        mirror,
        version,
        receipts: Vec::new(),
        profile,
    };
    checks.install("clean-install", true, None)?;
    let executable = checks.work.join("installation").join(if cfg!(windows) {
        "ripscope.exe"
    } else {
        "ripscope"
    });
    let expected_hash = capture::digest(&executable)?;
    checks.valid_notices()?;
    checks.valid_editors()?;
    checks
        .mirror
        .archive(&checks.version, &archive_name, &archive, false);
    checks.install("checksum-refusal", false, None)?;
    ensure(
        capture::digest(&executable)? == expected_hash,
        "checksum refusal changed installed bytes",
    )?;
    checks
        .mirror
        .archive(&checks.version, &archive_name, &archive, true);
    checks.install("repeat-install", true, None)?;
    ensure(
        capture::digest(&executable)? == expected_hash,
        "repeat install changed artifact bytes",
    )?;
    #[cfg(unix)]
    linux_checks(
        &mut checks,
        &archive_name,
        &archive,
        &executable,
        &expected_hash,
    )?;
    #[cfg(windows)]
    windows_checks(
        &mut checks,
        &executable,
        &archive_path,
        &archive_name,
        &archive,
    )?;
    plugin_checks(
        &mut checks,
        &executable,
        &expected_hash,
        &archive_name,
        &archive,
    )?;
    checks.valid_notices()?;
    checks.valid_editors()?;
    let index = checks.work.join("installation/project.db");
    let unrelated = checks.work.join("installation/unrelated");
    fs::write(&index, "preserve index")?;
    fs::write(&unrelated, "preserve unrelated file")?;
    let cache = checks.work.join(if cfg!(windows) {
        "profile/localappdata/ripscope/project-0123456789abcdef.db"
    } else {
        "profile/cache/ripscope/project-0123456789abcdef.db"
    });
    fs::create_dir_all(cache.parent().ok_or("no cache parent")?)?;
    fs::write(&cache, "preserve cached index")?;
    // Uninstalling removes what installing added, and nothing else.
    let mut command = if cfg!(windows) {
        let mut command = Command::new(powershell());
        command.args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ]);
        command.arg(&checks.installer).arg("-Uninstall");
        command
    } else {
        let mut command = Command::new("sh");
        command.arg(&checks.installer).arg("--uninstall");
        command
    };
    checks.profile.apply(&mut command);
    command.env("RIPSCOPE_INSTALL_DIR", checks.work.join("installation"));
    checks.command("uninstall", command, true)?;
    ensure(
        !executable.exists()
            && checks.notices()?.is_empty()
            && checks
                .editor_directories()
                .iter()
                .all(|directory| !directory.exists()),
        "uninstall left installed files",
    )?;
    ensure(
        fs::read(index)? == b"preserve index"
            && fs::read(unrelated)? == b"preserve unrelated file"
            && fs::read(cache)? == b"preserve cached index",
        "removal touched unrelated files",
    )?;
    capture::save(
        &checks.work.join("receipt.json"),
        &json!({"complete":true,"archive":archive_path,"archive_sha256":hash(&archive),"installer":checks.installer,"installer_sha256":capture::digest(&checks.installer)?,"binary_sha256":expected_hash,"checks":checks.receipts}),
    )?;
    println!(
        "Installer archive/checksum/replacement/refusal/removal checks passed; evidence: {}",
        checks.work.display()
    );
    Ok(())
}
