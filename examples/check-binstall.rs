//! Exercise published discovery and candidate metadata against the same release bytes.
//! Run through .github/workflows/cargo-binstall.yml on native hosted runners.

use sha2::{Digest, Sha256};
use std::env;
use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn checked(command: &mut Command) -> Output {
    let output = command.output().expect("start command");
    print!("{}", String::from_utf8_lossy(&output.stdout));
    eprint!("{}", String::from_utf8_lossy(&output.stderr));
    assert!(output.status.success(), "{command:?}: {}", output.status);
    output
}

fn verify(root: &Path, archive_binary: &Path, version: &str) {
    let bin = root.join("bin");
    let installed = bin.join(format!("ripscope{}", env::consts::EXE_SUFFIX));
    let hash = Sha256::digest(fs::read(&installed).unwrap());
    assert_eq!(hash, Sha256::digest(fs::read(archive_binary).unwrap()));
    println!(
        "Installed executable SHA-256: {}",
        base16ct::lower::encode_string(&hash)
    );
    let path = env::join_paths(
        [bin.clone()]
            .into_iter()
            .chain(env::split_paths(&env::var_os("PATH").unwrap())),
    )
    .unwrap();
    // Invoke by name from outside the checkout, exercising PATH discovery.
    let run = |args: &[&str]| {
        checked(
            Command::new("ripscope")
                .args(args)
                .env("PATH", &path)
                .current_dir(root),
        )
    };
    let output = run(&["--version"]);
    assert_eq!(
        String::from_utf8(output.stdout).unwrap().trim(),
        format!("ripscope {version}")
    );
    fs::write(root.join("source.c"), "int value;\n").unwrap();
    run(&["-bk", "source.c"]);
    let output = run(&["-dL", "-1", "value"]);
    assert_eq!(
        String::from_utf8(output.stdout).unwrap().replace('\r', ""),
        "source.c value 1 int value;\n"
    );
    checked(
        Command::new("cargo")
            .args(["uninstall", "--root"])
            .arg(root)
            .arg("ripscope"),
    );
    assert!(!installed.exists());
    assert!(fs::read_dir(bin).unwrap().next().is_none());
    assert!(root.join("ripscope.db").is_file());
}

fn main() {
    let args: Vec<_> = env::args().collect();
    assert_eq!(args.len(), 4, "VERSION ARCHIVE_BINARY WORK_DIR");
    let version = &args[1];
    assert!(version.split('.').count() == 3);
    assert!(version.bytes().all(|b| b.is_ascii_digit() || b == b'.'));
    let archive_binary = Path::new(&args[2]).canonicalize().unwrap();
    fs::create_dir_all(&args[3]).unwrap();
    let work = Path::new(&args[3]).canonicalize().unwrap();
    let target = if cfg!(windows) {
        "x86_64-pc-windows-msvc"
    } else {
        "x86_64-unknown-linux-gnu"
    };
    let install = |root: &Path| {
        let mut command = Command::new("cargo-binstall");
        command
            .arg(format!("ripscope@={version}"))
            .args([
                "--no-confirm",
                "--disable-telemetry",
                "--strategies",
                "crate-meta-data",
            ])
            .args(["--targets", target, "--root"])
            .arg(root);
        command
    };
    checked(Command::new("cargo-binstall").arg("-V"));
    let published = work.join("published");
    let output = install(&published).output().unwrap();
    fs::write(work.join("published.stdout"), &output.stdout).unwrap();
    fs::write(work.join("published.stderr"), &output.stderr).unwrap();
    println!("Published-package discovery: {}", output.status);
    print!("{}", String::from_utf8_lossy(&output.stdout));
    eprint!("{}", String::from_utf8_lossy(&output.stderr));
    if output.status.success() {
        verify(&published, &archive_binary, version);
    } else {
        assert!(
            !published
                .join("bin")
                .join(format!("ripscope{}", env::consts::EXE_SUFFIX))
                .exists()
        );
        println!("Published discovery failed. Candidate metadata is tested separately below.");
    }

    // Use exactly the candidate mappings, with the selected published version.
    // This is preparation evidence, not proof that crates.io ships the metadata.
    let manifest = include_str!("../Cargo.toml");
    let mappings_start = manifest.find("[package.metadata.binstall.").unwrap();
    let mappings = manifest[mappings_start..]
        .split("[features]")
        .next()
        .unwrap();
    let candidate = work.join("manifest");
    fs::create_dir(&candidate).unwrap();
    fs::write(
        candidate.join("Cargo.toml"),
        format!("[package]\nname = \"ripscope\"\nversion = \"{version}\"\nrepository = \"https://github.com/pederbe/ripscope\"\n[[bin]]\nname = \"ripscope\"\npath = \"main.rs\"\n{mappings}"),
    ).unwrap();
    fs::write(candidate.join("main.rs"), "fn main() {}\n").unwrap();
    let mapped = work.join("mapped");
    checked(install(&mapped).arg("--manifest-path").arg(&candidate));
    verify(&mapped, &archive_binary, version);
    println!(
        "Candidate mappings passed installation, hash, PATH, version, search and removal checks."
    );
}
