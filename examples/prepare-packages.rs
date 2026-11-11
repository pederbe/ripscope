//! Generate package-manager recipes from the latest validated Linux release archive.
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
        "aur/PKGBUILD",
        include_str!("../packaging/templates/PKGBUILD"),
    ),
    ("aur/check-installed.sh", CHECK),
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

fn prepare(archive: &Path, out: &Path) -> Result<()> {
    prepare_recipes(archive, out, requires_gcc(archive)?)
}

// Read metadata without extracting the archive. Published pre-Zig archives need
// GCC's unwinder, so recipes must follow the selected archive rather than main.
fn requires_gcc(archive: &Path) -> Result<bool> {
    let root = archive
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_suffix(".tar.gz"))
        .ok_or("Expected a Linux release archive filename")?;
    let metadata_path = PathBuf::from(root).join("BUILD.json");
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(File::open(archive)?));
    for entry in tar.entries()? {
        let mut entry = entry?;
        if entry.path()? == metadata_path {
            let metadata: serde_json::Value = serde_json::from_reader(&mut entry)?;
            let needed = metadata["elf"]["needed"]
                .as_array()
                .filter(|needed| {
                    !needed.is_empty() && needed.iter().all(serde_json::Value::is_string)
                })
                .ok_or("Archive BUILD.json has no ELF dependency list")?;
            return Ok(needed.iter().any(|name| name == "libgcc_s.so.1"));
        }
    }
    Err("Archive has no BUILD.json".into())
}

fn select_gcc_blocks(template: &str, requires_gcc: bool) -> String {
    let mut selected = String::new();
    let mut gcc_block = false;
    for line in template.lines() {
        match line.trim() {
            "@GCC_RUNTIME@" => gcc_block = true,
            "@END_GCC_RUNTIME@" => gcc_block = false,
            _ if !gcc_block || requires_gcc => {
                selected.push_str(line);
                selected.push('\n');
            }
            _ => {}
        }
    }
    selected
}

fn prepare_recipes(archive: &Path, out: &Path, requires_gcc: bool) -> Result<()> {
    let filename = archive
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("Expected a Linux release archive filename")?;
    let version = filename
        .strip_prefix("ripscope-")
        .and_then(|name| name.strip_suffix("-x86_64-linux-gnu.tar.gz"))
        .ok_or("Expected a Linux release archive filename")?;
    if !numeric_version(version) {
        return Err("Expected a stable Linux release archive filename".into());
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
        let contents = select_gcc_blocks(template, requires_gcc)
            .replace("@VERSION@", version)
            .replace("@ARCHIVE_SHA256@", &archive_hash)
            .replace("@CHECK_SHA256@", &check_hash)
            .replace("@CHECK_SCRIPT_RUBY@", &ruby_check)
            .replace("@NIX_RUNTIME@", if requires_gcc { "gcc" } else { "glibc" })
            .replace(
                "@NIX_RUNTIME_LIBRARY@",
                if requires_gcc { "gcc.cc.lib" } else { "glibc" },
            )
            .replace("@ARCH_GCC@", if requires_gcc { " 'gcc-libs'" } else { "" });
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
        "requires_gcc_runtime": requires_gcc,
        "files": files,
        "publication": "prepared locally; no release or channel was changed"
    });
    fs::write(
        out.join("PREPARATION.json"),
        format!("{}\n", serde_json::to_string_pretty(&receipt)?),
    )?;
    println!("Prepared four package-manager recipes in {}", out.display());
    Ok(())
}

fn run() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    let (mut archive, mut out) = (None, None);
    while let Some(key) = args.next() {
        let value = args.next().ok_or("Missing option value")?;
        match key.to_str() {
            Some("--archive") if archive.is_none() => archive = Some(PathBuf::from(value)),
            Some("--out") if out.is_none() => out = Some(PathBuf::from(value)),
            _ => return Err("Unknown or duplicate option".into()),
        }
    }
    prepare(
        &archive.ok_or("--archive is required")?,
        &out.ok_or("--out is required")?,
    )
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        eprintln!("usage: prepare-packages --archive LINUX_TARBALL --out NEW_DIRECTORY");
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
        assert!(prepare(&archive, &out).is_err());
        assert_eq!(fs::read(out.join("keep")).unwrap(), b"unrelated work");
        for version in ["v0.1.0", "0.1", "0.1.0-test", "0.01.0", "0.1.0\n"] {
            let archive = work
                .path()
                .join(format!("ripscope-{version}-x86_64-linux-gnu.tar.gz"));
            assert!(prepare(&archive, &work.path().join("unused")).is_err());
        }
        assert!(!work.path().join("unused").exists());
    }

    #[test]
    fn selected_archive_and_check_script_are_pinned_without_local_paths() {
        let work = tempfile::tempdir().unwrap();
        let archive = work.path().join("ripscope-0.1.0-x86_64-linux-gnu.tar.gz");
        fs::write(&archive, b"abc").unwrap();
        let out = work.path().join("recipes");
        prepare_recipes(&archive, &out, false).unwrap();
        let receipt: serde_json::Value =
            serde_json::from_slice(&fs::read(out.join("PREPARATION.json")).unwrap()).unwrap();
        assert_eq!(
            receipt["archive_sha256"],
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let arch = fs::read_to_string(out.join("aur/PKGBUILD")).unwrap();
        assert!(arch.lines().any(|line| line == "pkgname=ripscope-bin"));
        assert!(arch.lines().any(|line| line == "pkgver=0.1.0"));
        assert!(arch.contains(
            "sha256sums=('ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad'"
        ));
        assert_eq!(
            fs::read_to_string(out.join("aur/check-installed.sh")).unwrap(),
            CHECK
        );
        assert!(arch.contains(&format!(
            "'{}')",
            receipt["files"]["aur/check-installed.sh"].as_str().unwrap()
        )));
        for entry in receipt["files"].as_object().unwrap().keys() {
            let contents = fs::read_to_string(out.join(entry)).unwrap();
            assert!(!contents.contains("@VERSION@"));
            assert!(!contents.contains("@ARCHIVE_SHA256@"));
            assert!(!contents.contains("@CHECK_SHA256@"));
            assert!(!contents.contains(work.path().to_str().unwrap()));
        }
        // A recipe change needs a fresh preparation; never update a pinned output in place.
        assert!(prepare(&archive, &out).is_err());
    }

    #[test]
    fn runtime_dependencies_follow_the_selected_archive() {
        let work = tempfile::tempdir().unwrap();
        let archive = work.path().join("ripscope-0.2.7-x86_64-linux-gnu.tar.gz");
        for gcc in [false, true] {
            let needed = if gcc {
                vec!["libc.so.6", "libgcc_s.so.1"]
            } else {
                vec!["libc.so.6"]
            };
            let metadata = serde_json::to_vec(&json!({"elf": {"needed": needed}})).unwrap();
            let encoder = flate2::write::GzEncoder::new(
                File::create(&archive).unwrap(),
                flate2::Compression::fast(),
            );
            let mut tar = tar::Builder::new(encoder);
            let mut header = tar::Header::new_gnu();
            header.set_size(metadata.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            tar.append_data(
                &mut header,
                "ripscope-0.2.7-x86_64-linux-gnu/BUILD.json",
                metadata.as_slice(),
            )
            .unwrap();
            tar.into_inner().unwrap().finish().unwrap();
            assert_eq!(requires_gcc(&archive).unwrap(), gcc);
            let out = work.path().join(format!("recipes-{gcc}"));
            prepare(&archive, &out).unwrap();
            let brew = fs::read_to_string(out.join("homebrew/ripscope.rb")).unwrap();
            assert_eq!(brew.contains("patchelf"), gcc);
            assert_eq!(brew.contains("HOMEBREW-RELOCATION.txt"), gcc);
            let conda = fs::read_to_string(out.join("conda/build.sh")).unwrap();
            assert_eq!(conda.contains("patchelf"), gcc);
            let recipe = fs::read_to_string(out.join("conda/recipe.yaml")).unwrap();
            assert_eq!(recipe.contains("- libgcc"), gcc);
            let arch = fs::read_to_string(out.join("aur/PKGBUILD")).unwrap();
            assert_eq!(arch.contains("'gcc-libs'"), gcc);
            let nix = fs::read_to_string(out.join("nix/package.nix")).unwrap();
            assert!(nix.contains(if gcc {
                "buildInputs = [ gcc.cc.lib ];"
            } else {
                "buildInputs = [ glibc ];"
            }));
            for name in TEMPLATES.iter().map(|(name, _)| name) {
                let recipe = fs::read_to_string(out.join(name)).unwrap();
                assert!(
                    !recipe.contains("@GCC")
                        && !recipe.contains("@END_GCC")
                        && !recipe.contains("@NIX")
                        && !recipe.contains("@ARCH_GCC")
                );
            }
        }
    }
}
