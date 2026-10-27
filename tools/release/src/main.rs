//! Build, inspect and smoke the local release archives. Apart from the package
//! repository commands below, nothing here uploads, signs or publishes. The guarded
//! release workflow does that separately.
//!
//! - `linux`: build the glibc 2.28 x86-64 archive in pinned offline containers, test it
//!   and smoke the extracted archive.
//! - `windows`: build the static-CRT x86-64 MSVC archive natively, test it, inspect its
//!   imports and run `windows-smoke` on it.
//! - `windows-smoke ARCHIVE WORK`: exercise the exact ZIP bytes with only Windows system
//!   directories on PATH.
//! - `check-tag TAG`, `check-environment FILE` and `profile PLATFORM FIELD`: the release
//!   workflow's guard on its tag and protected environment, and its pinned profile values.
//! - `winget VERSION ARCHIVE DATE OUT`: write the WinGet manifests for a checked release ZIP.
//! - `winget-check PREVIOUS CURRENT INSTALLER WORK`: install, upgrade, use and remove two
//!   versions' manifests with the WinGet client.
//! - `package-repository REPOSITORY PUBLIC_KEY PACKAGE...`: add published packages to a
//!   local copy of the apt and dnf repository and sign it.
//! - `package-repository-check REPOSITORY ARCHIVE WORK`: install, upgrade, use and remove
//!   the archive's version from that repository in offline containers.
//! - `package-repository-upload REPOSITORY BUCKET`: upload it, current indexes last.

mod archive;
mod common;
mod linux;
mod repository;
mod smoke;
mod weekly;
mod windows;
mod winget;
mod workflow;

use std::process::ExitCode;

const USAGE: &str = "usage: ripscope-release (linux | windows | windows-smoke | check-tag | check-environment | check-automated-environment | prepare-weekly | release-notes | winget | winget-check | package-repository | package-repository-check | package-repository-upload | profile) [OPTIONS]\n\
Run a command with --help for its options. See docs/linux-release.md and docs/windows-release.md.";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("linux") => linux::run(&args[1..]),
        Some("windows") => windows::run(&args[1..]),
        Some("windows-smoke") => smoke::run(&args[1..]),
        Some("check-tag") => workflow::check_tag(&args[1..]),
        Some("check-automated-environment") => workflow::check_automated_environment(&args[1..]),
        Some("check-environment") => workflow::check_environment(&args[1..]),
        Some("prepare-weekly") => weekly::prepare(&args[1..]),
        Some("release-notes") => weekly::notes(&args[1..]),
        Some("winget") => winget::run(&args[1..]),
        Some("winget-check") => winget::check(&args[1..]),
        Some("package-repository") => repository::run(&args[1..]),
        Some("package-repository-check") => repository::check(&args[1..]),
        Some("package-repository-upload") => repository::upload(&args[1..]),
        Some("profile") => workflow::profile(&args[1..]),
        // A native user scope runs this to show it sits under the user's manager.
        Some("native-cgroup") => linux::native_cgroup(),
        Some("--help" | "-h") => {
            println!("{USAGE}");
            Ok(())
        }
        _ => Err(USAGE.into()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("ripscope-release: {error}");
            ExitCode::FAILURE
        }
    }
}
