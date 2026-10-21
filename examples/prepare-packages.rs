//! Pin package-manager recipes to an explicitly selected Linux release archive.
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
const CHECK: &str = include_str!("../packaging/check-installed.sh");
const TEMPLATES: &[(&str, &str)] = &[
    (
        "homebrew/ripscope.rb",
        include_str!("../packaging/templates/ripscope.rb"),
    ),
    (
        "nix/package.nix",
        include_str!("../packaging/templates/package.nix"),
    ),
    ("nix/check-installed.sh", CHECK),
    (
        "conda/recipe.yaml",
        include_str!("../packaging/templates/recipe.yaml"),
    ),
    (
        "conda/build.sh",
        include_str!("../packaging/templates/build.sh"),
    ),
    ("conda/check-installed.sh", CHECK),
];

fn numeric_version(version: &str) -> bool {
    let parts = version.split('.').collect::<Vec<_>>();
    parts.len() == 3
        && parts.iter().all(|part| {
            !part.is_empty()
                && part.bytes().all(|byte| byte.is_ascii_digit())
                && (part.len() == 1 || !part.starts_with('0'))
        })
}

fn sha256(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            return Ok(base16ct::lower::encode_string(&hash.finalize()));
        }
        hash.update(&buffer[..count]);
    }
}

fn prepare(version: &str, archive: &Path, out: &Path) -> Result<()> {
    if !numeric_version(version) {
        return Err("Use a numeric release version such as 0.1.0".into());
    }
    let filename = format!("ripscope-{version}-x86_64-linux-gnu.tar.gz");
    if archive.file_name().and_then(|name| name.to_str()) != Some(&filename) {
        return Err(format!("Expected archive filename {filename}").into());
    }
    let archive_hash = sha256(archive)?;
    let check_hash = base16ct::lower::encode_string(&Sha256::digest(CHECK.as_bytes()));
    let ruby_check = CHECK
        .lines()
        .map(|line| format!("      {line}"))
        .collect::<Vec<_>>()
        .join("\n");
    // Creating the root, rather than individual files, refuses existing work.
    fs::create_dir(out)?;
    let mut files = BTreeMap::new();
    for (name, template) in TEMPLATES {
        let contents = template
            .replace("@VERSION@", version)
            .replace("@ARCHIVE_SHA256@", &archive_hash)
            .replace("@CHECK_SHA256@", &check_hash)
            .replace("@CHECK_SCRIPT_RUBY@", &ruby_check);
        let path = out.join(name);
        fs::create_dir_all(path.parent().ok_or("Missing output parent")?)?;
        fs::write(path, contents.as_bytes())?;
        files.insert(
            name,
            base16ct::lower::encode_string(&Sha256::digest(contents.as_bytes())),
        );
    }
    let receipt = json!({
        "schema": 1,
        "version": version,
        "archive": filename,
        "archive_sha256": archive_hash,
        "files": files,
        "publication": "prepared locally; no release or channel was changed"
    });
    fs::write(
        out.join("PREPARATION.json"),
        format!("{}\n", serde_json::to_string_pretty(&receipt)?),
    )?;
    println!(
        "Prepared three package-manager recipes in {}",
        out.display()
    );
    Ok(())
}

fn run() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    let (mut version, mut archive, mut out) = (None, None, None);
    while let Some(key) = args.next() {
        let value = args.next().ok_or("Missing option value")?;
        match key.to_str() {
            Some("--version") if version.is_none() => {
                version = Some(value.into_string().map_err(|_| "Version must be UTF-8")?);
            }
            Some("--archive") if archive.is_none() => archive = Some(PathBuf::from(value)),
            Some("--out") if out.is_none() => out = Some(PathBuf::from(value)),
            _ => return Err("Unknown or duplicate option".into()),
        }
    }
    prepare(
        &version.ok_or("--version is required")?,
        &archive.ok_or("--archive is required")?,
        &out.ok_or("--out is required")?,
    )
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        eprintln!(
            "usage: prepare-packages --version VERSION --archive LINUX_TARBALL --out NEW_DIRECTORY"
        );
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn existing_outputs_and_nonrelease_versions_preserve_work() {
        let work = tempfile::tempdir().unwrap();
        let archive = work.path().join("ripscope-0.1.0-x86_64-linux-gnu.tar.gz");
        fs::write(&archive, b"selected archive").unwrap();
        let out = work.path().join("recipes");
        fs::create_dir(&out).unwrap();
        fs::write(out.join("keep"), b"unrelated work").unwrap();
        assert!(prepare("0.1.0", &archive, &out).is_err());
        assert_eq!(fs::read(out.join("keep")).unwrap(), b"unrelated work");
        for version in ["v0.1.0", "0.1", "0.1.0-test", "0.01.0", "0.1.0\n"] {
            assert!(prepare(version, &archive, &work.path().join("unused")).is_err());
        }
        assert!(!work.path().join("unused").exists());
    }

    #[test]
    fn selected_archive_and_check_script_are_pinned_without_local_paths() {
        let work = tempfile::tempdir().unwrap();
        let archive = work.path().join("ripscope-0.1.0-x86_64-linux-gnu.tar.gz");
        fs::write(&archive, b"abc").unwrap();
        let out = work.path().join("recipes");
        prepare("0.1.0", &archive, &out).unwrap();
        let receipt: serde_json::Value =
            serde_json::from_slice(&fs::read(out.join("PREPARATION.json")).unwrap()).unwrap();
        assert_eq!(
            receipt["archive_sha256"],
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert!(!out.join("aur").exists());
        assert!(
            receipt["files"]
                .as_object()
                .unwrap()
                .keys()
                .all(|name| !name.starts_with("aur/"))
        );
        for entry in receipt["files"].as_object().unwrap().keys() {
            let contents = fs::read_to_string(out.join(entry)).unwrap();
            assert!(!contents.contains("@VERSION@"));
            assert!(!contents.contains("@ARCHIVE_SHA256@"));
            assert!(!contents.contains(work.path().to_str().unwrap()));
        }
        // A recipe change needs a fresh preparation; never update a pinned output in place.
        assert!(prepare("0.1.0", &archive, &out).is_err());
    }
}
