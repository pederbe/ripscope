//! WinGet manifests for a published Windows release ZIP, in the layout of the community
//! repository `microsoft/winget-pkgs`. WinGet installs the ZIP's executable as a portable
//! package with the `ripscope` command. Nothing here submits the manifests.

use crate::archive;
use crate::common::{Result, capture, capture_command, sha};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

const IDENTIFIER: &str = "PederBergan.Ripscope";
const SCHEMA: &str = "1.28.0";
const RELEASES: &str = "https://github.com/pederbe/ripscope/releases";

fn stable_version(version: &str) -> bool {
    let parts: Vec<&str> = version.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
}

fn release_date(date: &str) -> bool {
    let bytes = date.as_bytes();
    bytes.len() == 10
        && bytes.iter().enumerate().all(|(at, byte)| match at {
            4 | 7 => *byte == b'-',
            _ => byte.is_ascii_digit(),
        })
}

fn header(kind: &str) -> String {
    format!(
        "# yaml-language-server: $schema=https://aka.ms/winget-manifest.{kind}.{SCHEMA}.schema.json\n\n"
    )
}

/// The version, installer and default-locale manifests, by file name, for the x86-64
/// ZIP of `version` whose SHA-256 is `sha256`, released on `date` (YYYY-MM-DD).
pub fn manifests(version: &str, sha256: &str, date: &str) -> Result<Vec<(String, String)>> {
    if !stable_version(version) {
        return Err(format!("expected a stable x.y.z version, not {version}").into());
    }
    if sha256.len() != 64 || !sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!("expected a SHA-256 in hexadecimal, not {sha256}").into());
    }
    if !release_date(date) {
        return Err(format!("expected a YYYY-MM-DD release date, not {date}").into());
    }
    let package = format!("ripscope-{version}-x86_64-windows-msvc");
    let identity = format!("PackageIdentifier: {IDENTIFIER}\nPackageVersion: {version}\n");
    let version_manifest = format!(
        "{}{identity}DefaultLocale: en-US\nManifestType: version\nManifestVersion: {SCHEMA}\n",
        header("version")
    );
    let installer = format!(
        "{}{identity}InstallerType: zip\nNestedInstallerType: portable\nNestedInstallerFiles:\n\
         - RelativeFilePath: {package}/bin/ripscope.exe\n  PortableCommandAlias: ripscope\n\
         ReleaseDate: {date}\nInstallers:\n- Architecture: x64\n\
         \x20 InstallerUrl: {RELEASES}/download/v{version}/{package}.zip\n\
         \x20 InstallerSha256: {}\nManifestType: installer\nManifestVersion: {SCHEMA}\n",
        header("installer"),
        sha256.to_ascii_uppercase()
    );
    let locale = format!(
        "{}{identity}PackageLocale: en-US\n\
         Publisher: Peder Bergan\n\
         PublisherUrl: https://github.com/pederbe\n\
         PublisherSupportUrl: https://github.com/pederbe/ripscope/issues\n\
         Author: Peder Bergan\n\
         PackageName: Ripscope\n\
         PackageUrl: https://ripscope.com\n\
         License: MIT OR Apache-2.0\n\
         LicenseUrl: https://github.com/pederbe/ripscope/blob/v{version}/README.md#license\n\
         ShortDescription: Cscope-compatible code navigation for C with content-aware, atomic index updates\n\
         Moniker: ripscope\n\
         Tags:\n- c\n- code-navigation\n- cross-reference\n- cscope\n- emacs\n- neovim\n- vim\n\
         ReleaseNotesUrl: {RELEASES}/tag/v{version}\n\
         Documentations:\n- DocumentLabel: Documentation\n  DocumentUrl: https://ripscope.com/docs\n\
         ManifestType: defaultLocale\nManifestVersion: {SCHEMA}\n",
        header("defaultLocale")
    );
    Ok(vec![
        (format!("{IDENTIFIER}.yaml"), version_manifest),
        (format!("{IDENTIFIER}.installer.yaml"), installer),
        (format!("{IDENTIFIER}.locale.en-US.yaml"), locale),
    ])
}

/// VERSION ARCHIVE DATE OUT: checks the release ZIP `ARCHIVE` against its `.sha256`
/// sidecar and for its executable, then writes the manifests below the new directory
/// `OUT` at their path in the community repository.
pub fn run(args: &[String]) -> Result<()> {
    let [version, archive_path, date, out] = args else {
        return Err("usage: ripscope-release winget VERSION ARCHIVE YYYY-MM-DD OUT".into());
    };
    let archive_path = Path::new(archive_path);
    let package = format!("ripscope-{version}-x86_64-windows-msvc");
    let name = format!("{package}.zip");
    if archive_path.file_name() != Some(name.as_ref()) {
        return Err(format!("expected the archive {name} of version {version}").into());
    }
    let hash = sha(archive_path)?;
    let sidecar = fs::read_to_string(archive_path.with_file_name(format!("{name}.sha256")))?;
    if sidecar.trim_end() != format!("{hash}  {name}") {
        return Err(format!("{name} does not match its sidecar, which says {sidecar:?}").into());
    }
    let executable = format!("{package}/bin/ripscope.exe");
    if !archive::zip_entries(&fs::read(archive_path)?)?
        .iter()
        .any(|entry| entry.name == executable)
    {
        return Err(format!("{name} has no {executable}").into());
    }
    let files = manifests(version, &hash, date)?;
    let directory = Path::new(out)
        .join("manifests/p/PederBergan/Ripscope")
        .join(version);
    fs::create_dir(out)?;
    fs::create_dir_all(&directory)?;
    for (file, text) in files {
        // LF on every platform, as the community repository keeps them.
        fs::write(directory.join(file), text)?;
    }
    println!("{}", directory.display());
    Ok(())
}

/// The `Path` value in `reg query` output, with each `%NAME%` that `lookup` knows
/// expanded, as Windows expands a `REG_EXPAND_SZ`.
fn registry_path(output: &str, lookup: impl Fn(&str) -> Option<String>) -> Option<String> {
    let value = output.lines().find_map(|line| {
        let mut rest = line.trim_start();
        rest = rest
            .get(..4)
            .filter(|name| name.eq_ignore_ascii_case("path"))
            .map(|_| &rest[4..])?;
        let rest = rest.trim_start().strip_prefix("REG_")?;
        let (_, value) = rest.split_once(char::is_whitespace)?;
        Some(value.trim())
    })?;
    let mut expanded = String::new();
    let mut rest = value;
    while let Some(start) = rest.find('%') {
        expanded.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after
            .find('%')
            .and_then(|end| Some((end, lookup(&after[..end])?)))
        {
            Some((end, found)) => {
                expanded.push_str(&found);
                rest = &after[end + 1..];
            }
            None => {
                expanded.push('%');
                rest = after;
            }
        }
    }
    expanded.push_str(rest);
    Some(expanded)
}

/// The first `name` file in the directories of a `;`-separated PATH.
fn on_path(path: &str, name: &str) -> Option<PathBuf> {
    path.split(';')
        .filter(|directory| !directory.is_empty())
        .map(|directory| Path::new(directory).join(name))
        .find(|candidate| candidate.is_file())
}

/// PATH as a newly started program sees it after an installation changed the registry:
/// the machine's directories, then the user's.
fn fresh_path() -> Result<String> {
    let mut directories = Vec::new();
    for key in [
        r"HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Environment",
        r"HKCU\Environment",
    ] {
        let command = ["reg", "query", key, "/v", "Path"].map(str::to_owned);
        if let Some((0, out, _)) = capture(&command, Duration::from_secs(60))? {
            directories.extend(registry_path(&out, |name| std::env::var(name).ok()));
        }
    }
    Ok(directories.join(";"))
}

fn installed() -> Result<Option<PathBuf>> {
    Ok(on_path(&fresh_path()?, "ripscope.exe"))
}

/// Runs a program to completion within ten minutes, echoing it and its output, and
/// returns its standard output once it succeeds.
fn checked(program: &Path, args: &[&str], cwd: &Path) -> Result<String> {
    println!("> {} {}", program.display(), args.join(" "));
    let mut command = Command::new(program);
    command.args(args).current_dir(cwd);
    let (exit, out, err) = capture_command(&mut command, None, Duration::from_secs(600))?
        .ok_or_else(|| format!("{} timed out", program.display()))?;
    let (out, err) = (String::from_utf8_lossy(&out), String::from_utf8_lossy(&err));
    print!("{out}{err}");
    if exit != 0 {
        return Err(format!(
            "{} {} exited with {exit}",
            program.display(),
            args.join(" ")
        )
        .into());
    }
    Ok(out.into_owned())
}

fn expect_version(executable: &Path, version: &str, cwd: &Path) -> Result<()> {
    let reported = checked(executable, &["--version"], cwd)?;
    if reported.lines().next() != Some(&format!("ripscope {version}")) {
        return Err(format!("expected ripscope {version}, not {reported:?}").into());
    }
    Ok(())
}

/// PREVIOUS PREVIOUS_VERSION CURRENT CURRENT_VERSION INSTALLER WORK: with the WinGet
/// client, validates the two manifest directories, installs PREVIOUS and upgrades to
/// CURRENT, finding `ripscope` on a freshly read PATH each time. Then indexes and
/// searches a project in the new directory WORK, checks that the editors' install
/// command (`INSTALLER -Plugin`) keeps the WinGet executable, and removes the package.
pub fn check(args: &[String]) -> Result<()> {
    let [
        previous,
        previous_version,
        current,
        current_version,
        installer,
        work,
    ] = args
    else {
        return Err("usage: ripscope-release winget-check PREVIOUS PREVIOUS_VERSION CURRENT CURRENT_VERSION INSTALLER WORK".into());
    };
    let work = Path::new(work);
    fs::create_dir(work)?;
    let winget = |arguments: &[&str]| checked(Path::new("winget"), arguments, work);
    let agree = [
        "--accept-package-agreements",
        "--accept-source-agreements",
        "--disable-interactivity",
    ];
    winget(&["--version"])?;
    winget(&["settings", "--enable", "LocalManifestFiles"])?;
    for manifest in [previous, current] {
        winget(&["validate", "--manifest", manifest])?;
    }
    let home = PathBuf::from(std::env::var("USERPROFILE")?);
    let separate = home.join(r".local\bin\ripscope.exe");
    if installed()?.is_some() || separate.exists() {
        return Err("a ripscope.exe is present before the WinGet installation".into());
    }

    winget(&[&["install", "--manifest", previous.as_str()][..], &agree].concat())?;
    let executable = installed()?.ok_or("ripscope.exe is not on PATH after installation")?;
    expect_version(&executable, previous_version, work)?;
    winget(&[&["upgrade", "--manifest", current.as_str()][..], &agree].concat())?;
    let executable = installed()?.ok_or("ripscope.exe is not on PATH after the upgrade")?;
    expect_version(&executable, current_version, work)?;

    let project = work.join("project");
    fs::create_dir(&project)?;
    fs::write(project.join("source.c"), "int value;\n")?;
    checked(&executable, &["-bk", "source.c"], &project)?;
    let found = checked(&executable, &["-dL", "-1", "value"], &project)?;
    if found.lines().collect::<Vec<_>>() != ["source.c value 1 int value;"] {
        return Err(format!("unexpected search result {found:?}").into());
    }

    let powershell = Path::new("powershell.exe");
    let plugin = [
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
        "-File",
        installer.as_str(),
        "-Plugin",
    ];
    let mut command = Command::new(powershell);
    command.args(plugin).env("Path", fresh_path()?);
    println!("> {} {}", powershell.display(), plugin.join(" "));
    let (exit, out, err) = capture_command(&mut command, None, Duration::from_secs(600))?
        .ok_or("the editors' install command timed out")?;
    let said = String::from_utf8_lossy(&out);
    print!("{said}{}", String::from_utf8_lossy(&err));
    let kept = said.lines().any(|line| {
        line.strip_prefix(&format!("Ripscope {current_version} is installed: "))
            .is_some_and(|path| path.eq_ignore_ascii_case(&executable.to_string_lossy()))
    });
    if exit != 0 || !kept || separate.exists() {
        return Err("the editors' install command did not keep the WinGet ripscope.exe".into());
    }

    winget(&[
        "uninstall",
        "--manifest",
        current,
        "--disable-interactivity",
    ])?;
    if let Some(left) = installed()? {
        return Err(format!("{} remains on PATH after removal", left.display()).into());
    }
    if !project.join("ripscope.db").is_file() {
        return Err("removing the package removed the project's index".into());
    }
    println!("WinGet installation, upgrade, search, editor setup and removal passed.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const HASH: &str = "dd9c28a413ed2eef10c5780c1250230a8ca4ec22938539e4b3b8cbf2fd4e8c9a";

    #[test]
    fn manifests_name_the_release_archive_its_hash_and_its_command() {
        let files = manifests("0.2.4", HASH, "2026-10-26").unwrap();
        let names: Vec<&str> = files.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(
            names,
            [
                "PederBergan.Ripscope.yaml",
                "PederBergan.Ripscope.installer.yaml",
                "PederBergan.Ripscope.locale.en-US.yaml",
            ]
        );
        assert_eq!(
            files[0].1,
            "# yaml-language-server: $schema=https://aka.ms/winget-manifest.version.1.28.0.schema.json\n\
             \n\
             PackageIdentifier: PederBergan.Ripscope\n\
             PackageVersion: 0.2.4\n\
             DefaultLocale: en-US\n\
             ManifestType: version\n\
             ManifestVersion: 1.28.0\n"
        );
        assert_eq!(
            files[1].1,
            "# yaml-language-server: $schema=https://aka.ms/winget-manifest.installer.1.28.0.schema.json\n\
             \n\
             PackageIdentifier: PederBergan.Ripscope\n\
             PackageVersion: 0.2.4\n\
             InstallerType: zip\n\
             NestedInstallerType: portable\n\
             NestedInstallerFiles:\n\
             - RelativeFilePath: ripscope-0.2.4-x86_64-windows-msvc/bin/ripscope.exe\n\
             \x20 PortableCommandAlias: ripscope\n\
             ReleaseDate: 2026-10-26\n\
             Installers:\n\
             - Architecture: x64\n\
             \x20 InstallerUrl: https://github.com/pederbe/ripscope/releases/download/v0.2.4/ripscope-0.2.4-x86_64-windows-msvc.zip\n\
             \x20 InstallerSha256: DD9C28A413ED2EEF10C5780C1250230A8CA4EC22938539E4B3B8CBF2FD4E8C9A\n\
             ManifestType: installer\n\
             ManifestVersion: 1.28.0\n"
        );
        let locale = &files[2].1;
        for line in [
            "PackageIdentifier: PederBergan.Ripscope\n",
            "PackageVersion: 0.2.4\n",
            "PackageLocale: en-US\n",
            "License: MIT OR Apache-2.0\n",
            "Moniker: ripscope\n",
            "ReleaseNotesUrl: https://github.com/pederbe/ripscope/releases/tag/v0.2.4\n",
            "ManifestType: defaultLocale\nManifestVersion: 1.28.0\n",
        ] {
            assert!(locale.contains(line), "{line:?} missing from\n{locale}");
        }
    }

    #[test]
    fn manifests_refuse_malformed_metadata() {
        assert!(manifests("0.2", HASH, "2026-10-26").is_err());
        assert!(manifests("0.2.4-rc1", HASH, "2026-10-26").is_err());
        assert!(manifests("0.2.4", &HASH[1..], "2026-10-26").is_err());
        assert!(manifests("0.2.4", &HASH.replace('d', "g"), "2026-10-26").is_err());
        assert!(manifests("0.2.4", HASH, "26-10-2026").is_err());
    }

    /// A release ZIP for `version` holding `members` below its package directory, with
    /// its checksum sidecar unless `sidecar` replaces it.
    fn release(dir: &Path, version: &str, members: &[&str], sidecar: Option<&str>) -> String {
        let name = format!("ripscope-{version}-x86_64-windows-msvc");
        let package = dir.join("build").join(&name);
        fs::create_dir_all(&package).unwrap();
        for member in members {
            let path = package.join(member);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, member.as_bytes()).unwrap();
        }
        let archive = dir.join(format!("{name}.zip"));
        archive::zip(&package, &archive, 1_700_000_000).unwrap();
        let hash = crate::common::sha(&archive).unwrap();
        let line = sidecar.map_or_else(|| format!("{hash}  {name}.zip\n"), str::to_owned);
        fs::write(dir.join(format!("{name}.zip.sha256")), line).unwrap();
        hash
    }

    fn args(dir: &Path, version: &str, out: &Path) -> Vec<String> {
        let archive = dir.join(format!("ripscope-{version}-x86_64-windows-msvc.zip"));
        [
            version,
            &archive.to_string_lossy(),
            "2026-10-26",
            &out.to_string_lossy(),
        ]
        .map(str::to_owned)
        .to_vec()
    }

    #[test]
    fn run_writes_the_community_repository_layout_for_a_checked_archive() {
        let dir = tempfile::tempdir().unwrap();
        let hash = release(
            dir.path(),
            "1.2.3",
            &["bin/ripscope.exe", "BUILD.json"],
            None,
        );
        let out = dir.path().join("winget");
        run(&args(dir.path(), "1.2.3", &out)).unwrap();
        let version = out.join("manifests/p/PederBergan/Ripscope/1.2.3");
        let installer =
            fs::read_to_string(version.join("PederBergan.Ripscope.installer.yaml")).unwrap();
        assert!(installer.contains(&format!("InstallerSha256: {}\n", hash.to_uppercase())));
        assert!(version.join("PederBergan.Ripscope.yaml").is_file());
        assert!(
            version
                .join("PederBergan.Ripscope.locale.en-US.yaml")
                .is_file()
        );
        // An existing output is never overwritten.
        assert!(run(&args(dir.path(), "1.2.3", &out)).is_err());
    }

    #[test]
    fn registry_path_reads_and_expands_the_path_value() {
        let output = "\r\nHKEY_CURRENT_USER\\Environment\r\n    Path    REG_EXPAND_SZ    \
                      %LOCALAPPDATA%\\Microsoft\\WinGet\\Links;C:\\Program Files\\Tool;%UNSET%\r\n\r\n";
        let lookup = |name: &str| (name == "LOCALAPPDATA").then(|| "C:\\Local".to_owned());
        assert_eq!(
            registry_path(output, lookup).as_deref(),
            Some("C:\\Local\\Microsoft\\WinGet\\Links;C:\\Program Files\\Tool;%UNSET%")
        );
        let other = "HKEY_CURRENT_USER\\Environment\n    TEMP    REG_SZ    C:\\Temp\n";
        assert_eq!(registry_path(other, lookup), None);
    }

    #[test]
    fn on_path_finds_the_first_directory_holding_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let [empty, first, second] = ["empty", "first", "second"].map(|name| dir.path().join(name));
        for directory in [&empty, &first, &second] {
            fs::create_dir(directory).unwrap();
        }
        fs::write(first.join("ripscope.exe"), "").unwrap();
        fs::write(second.join("ripscope.exe"), "").unwrap();
        let path = [&empty, &first, &second]
            .map(|p| p.display().to_string())
            .join(";");
        assert_eq!(
            on_path(&format!(";{path}"), "ripscope.exe"),
            Some(first.join("ripscope.exe"))
        );
        assert_eq!(on_path(&empty.display().to_string(), "ripscope.exe"), None);
    }

    #[test]
    fn run_refuses_an_archive_its_sidecar_or_executable_does_not_match() {
        let dir = tempfile::tempdir().unwrap();
        let wrong = format!(
            "{}  ripscope-1.2.3-x86_64-windows-msvc.zip\n",
            "0".repeat(64)
        );
        release(dir.path(), "1.2.3", &["bin/ripscope.exe"], Some(&wrong));
        let error = run(&args(dir.path(), "1.2.3", &dir.path().join("a"))).unwrap_err();
        assert!(error.to_string().contains("sidecar"), "{error}");

        let dir = tempfile::tempdir().unwrap();
        release(dir.path(), "1.2.3", &["BUILD.json"], None);
        let error = run(&args(dir.path(), "1.2.3", &dir.path().join("a"))).unwrap_err();
        assert!(error.to_string().contains("ripscope.exe"), "{error}");

        // The version must be the archive's.
        let dir = tempfile::tempdir().unwrap();
        release(dir.path(), "1.2.3", &["bin/ripscope.exe"], None);
        let mut mismatched = args(dir.path(), "1.2.3", &dir.path().join("a"));
        mismatched[0] = "1.2.4".to_owned();
        assert!(run(&mismatched).is_err());
        assert!(!dir.path().join("a").exists());
    }
}
