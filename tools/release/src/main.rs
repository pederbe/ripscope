//! Build, inspect and smoke the local release archives. Nothing here uploads, signs or
//! publishes; the guarded release workflow does that separately.
//!
//! - `linux`: build the glibc 2.28 x86-64 archive in pinned offline containers, test it
//!   and smoke the extracted archive.
//! - `windows`: build the static-CRT x86-64 MSVC archive natively, test it, inspect its
//!   imports and run `windows-smoke` on it.
//! - `windows-smoke ARCHIVE WORK`: exercise the exact ZIP bytes with only Windows system
//!   directories on PATH.
//! - `check-tag TAG`, `check-environment FILE` and `profile PLATFORM FIELD`: the release
//!   workflow's guard on its tag and protected environment, and its pinned profile values.

mod archive;
mod common;
mod linux;
mod smoke;
mod weekly;
mod windows;
mod workflow;

use std::process::ExitCode;

const USAGE: &str = "usage: ripscope-release (linux | windows | windows-smoke | check-tag | check-environment | check-automated-environment | prepare-weekly | release-notes | profile) [OPTIONS]\n\
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
