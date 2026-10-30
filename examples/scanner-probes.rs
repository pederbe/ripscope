//! Regenerate the scanner probe fixtures from the pinned reference cscope (ADR 0001, ADR 0004).
//!
//! Each file in `tests/fixtures/scanner/probes/` is indexed on its own by the pinned
//! cscope (`-bk`, default compressed database, run on Linux with a clean environment and
//! `LC_ALL=C`). Its database is decoded as cscope displays it (`examples/support/
//! cscope_db.rs`), and the records are written to `tests/fixtures/scanner/expected/
//! <probe>.json`. The analyzer's tests compare ripscope's own Analysis units with these
//! files.
//!
//! Probes are synthetic C written for ripscope, so the expected files are observations of
//! the binary's behaviour, not copies of any project's code.
//!
//! With `--open`, the probes in `tests/fixtures/scanner/open/probes/` are captured the same
//! way into `open/expected/`: readings the baseline does not reproduce yet, pending a
//! decision, whose reference records are kept for when one is made.
//!
//! usage: cargo run --example scanner-probes -- [--open] [probe-name ...]

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[path = "support/cscope_db.rs"]
#[allow(dead_code)]
mod cscope_db;
#[path = "support/json_text.rs"]
#[allow(dead_code)]
mod json_text;
#[path = "support/reference.rs"]
#[allow(dead_code)]
mod reference;

use json_text::{FIXTURE, Json};
use reference::{CSCOPE_SHA256, Reference, quote};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn main() -> ExitCode {
    let mut arguments: Vec<String> = std::env::args().skip(1).collect();
    let mut root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/scanner");
    if arguments.first().map(String::as_str) == Some("--open") {
        arguments.swap_remove(0);
        root.push("open");
    }
    let names: BTreeSet<String> = arguments.into_iter().collect();
    match capture(&root, &names, &Reference::pinned()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("scanner probes: {error}");
            ExitCode::FAILURE
        }
    }
}

fn capture(root: &Path, names: &BTreeSet<String>, reference: &Reference) -> Result<()> {
    let (probes_dir, expected_dir) = (root.join("probes"), root.join("expected"));
    let mut probes: Vec<PathBuf> = fs::read_dir(&probes_dir)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<std::io::Result<_>>()?;
    probes.retain(|probe| {
        probe.is_file()
            && (names.is_empty()
                || names.contains(&*probe.file_stem().unwrap_or_default().to_string_lossy()))
    });
    probes.sort();
    fs::create_dir_all(&expected_dir)?;
    for probe in probes {
        let name = probe
            .file_name()
            .ok_or("probe without a name")?
            .to_string_lossy()
            .into_owned();
        // One database per probe, so each fixture holds exactly one file's records.
        let script = format!(
            "set -eo pipefail\n\
             work=$(mktemp -d)\n\
             trap 'rm -rf \"$work\"' EXIT\n\
             cp {} \"$work/\"{}\n\
             cd \"$work\"\n\
             timeout --foreground 60 env -i LC_ALL=C PATH=/usr/bin:/bin HOME=\"$work\" {} -bk {}\n\
             cat cscope.out\n",
            quote(&reference.path(&probe)),
            quote(&name),
            reference.command(),
            quote(&name),
        );
        let done = reference.run(&["bash", "-c", &script], None, 1900.0)?;
        let units = cscope_db::parse_cscope_database(&done.stdout)?;
        let records = units
            .into_iter()
            .find(|(file, _)| *file == name)
            .map(|(_, records)| records)
            .unwrap_or_default();
        let fixture = Json::object([
            ("probe", Json::from(format!("probes/{name}"))),
            (
                "reference",
                Json::object([
                    ("binary_sha256", Json::from(CSCOPE_SHA256)),
                    ("command", Json::from("cscope -bk <probe>")),
                    (
                        "decoding",
                        Json::from("examples/support/cscope_db.rs (as cscope displays it)"),
                    ),
                ]),
            ),
            (
                "stderr",
                Json::from(String::from_utf8_lossy(&done.stderr).into_owned()),
            ),
            (
                "records",
                Json::Array(records.iter().map(cscope_db::Record::to_json).collect()),
            ),
        ]);
        let stem = probe
            .file_stem()
            .ok_or("probe without a stem")?
            .to_string_lossy()
            .into_owned();
        fs::write(
            expected_dir.join(format!("{stem}.json")),
            fixture.text(FIXTURE) + "\n",
        )?;
        println!("{name}: {} records", records.len());
    }
    Ok(())
}
