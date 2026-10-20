//! Build, inspect and smoke a local static-CRT Windows candidate archive.

use crate::archive;
use crate::common::{
    Ended, Result, collect_notices, copy_file, copy_source, crate_version, delivery_files, inside,
    inventory, json_line, json_text, repository, resolved, run_logged, set_modified, sha,
    unchanged_sources, under, walk, write_text,
};
use regex::Regex;
use serde_json::{Map, Value, json};
use std::collections::BTreeSet;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

const USAGE: &str = "ripscope-release windows --work NEW_DIR --toolchain RUST_TOOLCHAIN --msvc MSVC_DIR --sdk WINDOWS_KITS_10 [--target-dir DIR]";

/// The build's own inputs, recorded beside the product source.
const EXTRAS: [&str; 5] = [
    "docs/getting-started/installation.md",
    "tools/release/Cargo.toml",
    "tools/release/Cargo.lock",
    "tools/release/src",
    "tools/release/windows-release-profile.json",
];

/// Accepts an x86-64 PE32+ executable whose imports, delayed ones included, are all
/// Windows system DLLs, so it needs no dynamic C runtime.
pub fn inspect_pe(headers: &str, imports: &str) -> Result<Value> {
    if !Regex::new(r"(?i)8664 machine \(x64\)")?.is_match(headers)
        || !headers.contains("20B magic # (PE32+)")
    {
        return Err("Expected an x86-64 PE32+ executable".into());
    }
    let libraries: BTreeSet<String> = Regex::new(r"(?im)^[ \t]+([^\r\n]+?\.dll)[ \t]*\r?$")?
        .captures_iter(imports)
        .map(|found| found[1].to_owned())
        .collect();
    let allowed = [
        "kernel32.dll",
        "ntdll.dll",
        "bcrypt.dll",
        "bcryptprimitives.dll",
        "advapi32.dll",
        "userenv.dll",
        "ws2_32.dll",
        "ole32.dll",
        "shell32.dll",
        "user32.dll",
        "gdi32.dll",
        "secur32.dll",
        "rpcrt4.dll",
        "crypt32.dll",
        "api-ms-win-core-synch-l1-2-0.dll",
    ];
    if libraries.is_empty()
        || libraries
            .iter()
            .any(|name| !allowed.contains(&name.to_lowercase().as_str()))
    {
        return Err(format!("Unexpected native or dynamic CRT imports: {libraries:?}").into());
    }
    Ok(json!({
        "machine": "x86-64 PE32+",
        "imports": libraries,
        "cpu": "baseline Rust x86-64 and native x64 defaults; not instruction-level certification",
    }))
}

struct Options {
    work: PathBuf,
    toolchain: PathBuf,
    msvc: PathBuf,
    sdk: PathBuf,
    target_dir: Option<PathBuf>,
}

fn options(args: &[String]) -> Result<Options> {
    let mut values = std::collections::BTreeMap::new();
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        if arg == "--help" || arg == "-h" {
            println!("{USAGE}");
            std::process::exit(0);
        }
        let value = PathBuf::from(args.next().ok_or_else(|| format!("{arg} needs a value"))?);
        match arg.as_str() {
            "--work" | "--toolchain" | "--msvc" | "--sdk" | "--target-dir" => {
                values.insert(arg[2..].to_owned(), value);
            }
            _ => return Err(format!("unknown option {arg}\n{USAGE}").into()),
        }
    }
    let mut required = |name: &str| {
        values
            .remove(name)
            .ok_or_else(|| format!("--{name} is required\n{USAGE}"))
    };
    Ok(Options {
        work: required("work")?,
        toolchain: required("toolchain")?,
        msvc: required("msvc")?,
        sdk: required("sdk")?,
        target_dir: values.remove("target-dir"),
    })
}

fn text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

struct Build {
    work: PathBuf,
    logs: PathBuf,
    receipt: Map<String, Value>,
}

impl Build {
    fn save(&self) -> Result<()> {
        write_text(
            &self.work.join("receipt.json"),
            &(json_text(&Value::Object(self.receipt.clone())) + "\n"),
        )
    }

    fn commands(&mut self) -> &mut Vec<Value> {
        match self.receipt.get_mut("commands") {
            Some(Value::Array(commands)) => commands,
            _ => unreachable!("the receipt always lists its commands"),
        }
    }

    /// Runs a recorded command with its output in `logs/<name>.log`, returning that log.
    fn run(
        &mut self,
        name: &str,
        command: &[String],
        cwd: Option<&Path>,
        environment: Option<&[(OsString, OsString)]>,
        accepted: &[i32],
    ) -> Result<String> {
        self.commands()
            .push(json!({"name": name, "args": command, "accepted_exit_codes": accepted}));
        self.save()?;
        let log = self.logs.join(format!("{name}.log"));
        let Ended::Exit(exit) = run_logged(command, cwd, environment, &log, None, None)? else {
            unreachable!("a command without a deadline or interrupt flag runs to its end");
        };
        if let Some(Value::Object(entry)) = self.commands().last_mut() {
            entry.insert("exit".into(), json!(exit));
        }
        self.save()?;
        if !accepted.contains(&exit) {
            return Err(format!("{name} failed; see {}", log.display()).into());
        }
        let bytes = fs::read(&log)?;
        Ok(String::from_utf8_lossy(&bytes).replace("\r\n", "\n"))
    }
}

pub fn run(args: &[String]) -> Result<()> {
    let options = options(args)?;
    if !cfg!(windows) {
        return Err("Run on native Windows".into());
    }
    let repo = repository()?;
    let profile: Value = serde_json::from_slice(&fs::read(
        repo.join("tools/release/windows-release-profile.json"),
    )?)?;
    let field = |name: &str| -> Result<String> {
        Ok(profile[name]
            .as_str()
            .ok_or_else(|| format!("profile has no {name}"))?
            .to_owned())
    };
    let (work, toolchain, msvc, sdk) = (
        resolved(&options.work)?,
        resolved(&options.toolchain)?,
        resolved(&options.msvc)?,
        resolved(&options.sdk)?,
    );
    // Links and junctions are resolved, so one into the checkout cannot pass for an
    // outside path. The build itself keeps the ordinary drive form.
    if work.exists() || inside(&work, &repo)? {
        return Err("Use a new external work directory".into());
    }
    // Cargo searches ancestors even with a private CARGO_HOME.
    for parent in work.ancestors() {
        if ["config", "config.toml"]
            .iter()
            .any(|name| parent.join(".cargo").join(name).exists())
        {
            return Err(format!("Ancestor Cargo configuration at {}", parent.display()).into());
        }
    }
    if msvc
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        != Some(field("msvc_directory")?)
    {
        return Err("MSVC directory differs from the recorded profile".into());
    }
    fs::create_dir_all(&work)?;
    let logs = work.join("logs");
    fs::create_dir(&logs)?;
    let mut receipt = Map::new();
    receipt.insert("complete".into(), json!(false));
    receipt.insert("profile".into(), profile.clone());
    receipt.insert("commands".into(), json!([]));
    let mut build = Build {
        work: work.clone(),
        logs,
        receipt,
    };
    let git = |args: &[&str]| -> Vec<String> {
        ["git", "-C", &text(&repo)]
            .iter()
            .chain(args)
            .map(|word| (*word).to_owned())
            .collect()
    };
    let commit = build.run(
        "source-commit",
        &git(&["rev-parse", "HEAD"]),
        None,
        None,
        &[0],
    )?;
    build
        .receipt
        .insert("source_commit".into(), json!(commit.trim()));
    let epoch: u64 = build
        .run(
            "source-epoch",
            &git(&["show", "-s", "--format=%ct", "HEAD"]),
            None,
            None,
            &[0],
        )?
        .trim()
        .parse()?;
    let source = work.join("source");
    fs::create_dir(&source)?;
    copy_source(&repo, &source, &EXTRAS)?;
    // Copied checkouts can retain timestamps older than reused Windows test binaries.
    let now = std::time::SystemTime::now();
    for path in walk(&source)? {
        if path.extension().is_some_and(|extension| extension == "rs") && path.is_file() {
            set_modified(&path, now)?;
        }
    }
    let source_inputs = inventory(&source)?;
    build
        .receipt
        .insert("source_inputs".into(), Value::Object(source_inputs.clone()));
    let native_bin = under(&msvc, "bin/Hostx64/x64");
    let sdk_version = field("sdk_version")?;
    let input_roots: Vec<(&str, PathBuf)> = vec![
        ("rust-bin", toolchain.join("bin")),
        ("rust-lib", toolchain.join("lib")),
        (
            "rust-licenses",
            under(&toolchain, "share/doc/rust/licenses"),
        ),
        ("msvc-bin", native_bin.clone()),
        ("msvc-include", msvc.join("include")),
        ("msvc-lib", under(&msvc, "lib/x64")),
        ("sdk-include", sdk.join("Include").join(&sdk_version)),
        (
            "sdk-ucrt",
            sdk.join("Lib").join(&sdk_version).join("ucrt").join("x64"),
        ),
        (
            "sdk-um",
            sdk.join("Lib").join(&sdk_version).join("um").join("x64"),
        ),
        ("sdk-bin", sdk.join("bin").join(&sdk_version).join("x64")),
    ];
    if !input_roots.iter().all(|(_, path)| path.is_dir()) {
        return Err("Required toolchain or SDK input directory missing".into());
    }
    let mut tool_inputs = Map::new();
    for (name, path) in &input_roots {
        tool_inputs.insert((*name).into(), Value::Object(inventory(path)?));
    }
    build
        .receipt
        .insert("tool_inputs".into(), Value::Object(tool_inputs.clone()));
    for name in ["tmp", "home", "cargo-home"] {
        fs::create_dir(work.join(name))?;
    }
    let system = PathBuf::from(std::env::var_os("SystemRoot").ok_or("SystemRoot is not set")?);
    let target = match &options.target_dir {
        Some(directory) => resolved(directory)?,
        None => work.join("target"),
    };
    if inside(&target, &repo)? || inside(&target, &source)? {
        return Err("Cargo cache must remain outside the source checkout".into());
    }
    let sdk_include = sdk.join("Include").join(&sdk_version);
    let sdk_lib = sdk.join("Lib").join(&sdk_version);
    let joined = |paths: &[PathBuf]| {
        paths
            .iter()
            .map(|path| text(path))
            .collect::<Vec<_>>()
            .join(";")
    };
    let rustflags: Vec<String> = profile["rustflags"]
        .as_array()
        .ok_or("profile has no rustflags")?
        .iter()
        .map(|flag| {
            flag.as_str()
                .map(str::to_owned)
                .ok_or("rustflag is not text")
        })
        .collect::<std::result::Result<_, _>>()?;
    let mut encoded = rustflags;
    encoded.extend([
        "-C".to_owned(),
        format!("linker={}", text(&native_bin.join("link.exe"))),
    ]);
    let environment: Vec<(String, String)> = vec![
        ("SystemRoot".into(), text(&system)),
        ("WINDIR".into(), text(&system)),
        ("COMSPEC".into(), text(&under(&system, "System32/cmd.exe"))),
        ("USERPROFILE".into(), text(&work.join("home"))),
        ("TEMP".into(), text(&work.join("tmp"))),
        ("TMP".into(), text(&work.join("tmp"))),
        (
            "PATH".into(),
            joined(&[
                toolchain.join("bin"),
                native_bin.clone(),
                sdk.join("bin").join(&sdk_version).join("x64"),
                system.join("System32"),
            ]),
        ),
        (
            "INCLUDE".into(),
            joined(&[
                msvc.join("include"),
                sdk_include.join("ucrt"),
                sdk_include.join("shared"),
                sdk_include.join("um"),
                sdk_include.join("winrt"),
            ]),
        ),
        (
            "LIB".into(),
            joined(&[
                under(&msvc, "lib/x64"),
                sdk_lib.join("ucrt").join("x64"),
                sdk_lib.join("um").join("x64"),
            ]),
        ),
        ("CC".into(), text(&native_bin.join("cl.exe"))),
        ("CXX".into(), text(&native_bin.join("cl.exe"))),
        ("CFLAGS".into(), field("cflags")?),
        ("CXXFLAGS".into(), field("cflags")?),
        ("CARGO_HOME".into(), text(&work.join("cargo-home"))),
        ("CARGO_TARGET_DIR".into(), text(&target)),
        ("CARGO_BUILD_JOBS".into(), "4".into()),
        ("CC_ENABLE_DEBUG_OUTPUT".into(), "1".into()),
        ("VSLANG".into(), "1033".into()),
        ("CARGO_ENCODED_RUSTFLAGS".into(), encoded.join("\u{1f}")),
        ("SOURCE_DATE_EPOCH".into(), epoch.to_string()),
    ];
    let recorded: Map<String, Value> = environment
        .iter()
        .map(|(k, v)| (k.clone(), json!(v)))
        .collect();
    build
        .receipt
        .insert("build_environment".into(), Value::Object(recorded));
    let environment: Vec<(OsString, OsString)> = environment
        .into_iter()
        .map(|(k, v)| (k.into(), v.into()))
        .collect();
    let env = Some(environment.as_slice());
    let rust = build.run(
        "rust-version",
        &[text(&under(&toolchain, "bin/rustc.exe")), "-Vv".into()],
        None,
        env,
        &[0],
    )?;
    if !rust.contains(&format!("release: {}", field("rust_version")?))
        || !rust.contains(&field("rust_commit")?)
    {
        return Err("Unexpected Rust toolchain".into());
    }
    build.run(
        "msvc-version",
        &[text(&native_bin.join("cl.exe"))],
        None,
        env,
        &[0],
    )?;
    // This pinned MSVC linker returns 1100 after printing its help/version banner.
    build.run(
        "linker-version",
        &[text(&native_bin.join("link.exe")), "/?".into()],
        None,
        env,
        &[0, 1100],
    )?;
    let cargo = text(&under(&toolchain, "bin/cargo.exe"));
    let vendor = work.join("vendor");
    build.run(
        "vendor",
        &[
            cargo.clone(),
            "vendor".into(),
            "--locked".into(),
            "--versioned-dirs".into(),
            text(&vendor),
        ],
        Some(&source),
        env,
        &[0],
    )?;
    let vendor_inputs = inventory(&vendor)?;
    build
        .receipt
        .insert("vendor_inputs".into(), Value::Object(vendor_inputs.clone()));
    fs::create_dir(source.join(".cargo"))?;
    let vendor_text = text(&vendor).replace('\\', "/");
    write_text(
        &source.join(".cargo/config.toml"),
        &format!(
            "[source.crates-io]\nreplace-with = \"vendored-sources\"\n[source.vendored-sources]\ndirectory = {}\n",
            serde_json::to_string(&vendor_text)?
        ),
    )?;
    build.receipt.insert(
        "cargo_config_sha256".into(),
        json!(sha(&source.join(".cargo/config.toml"))?),
    );
    let selection = [
        "--release",
        "--locked",
        "--offline",
        "--target",
        &field("target")?,
    ]
    .map(str::to_owned);
    let mut release = vec![cargo.clone(), "build".to_owned()];
    release.extend(selection.iter().cloned());
    release.extend(["--bins", "--examples", "-vv"].map(str::to_owned));
    build.run("release-build", &release, Some(&source), env, &[0])?;
    let executable = under(&target.join(field("target")?), "release/ripscope.exe");
    let binary_hash = sha(&executable)?;
    let mut tests = vec![cargo.clone(), "test".to_owned()];
    tests.extend(selection.iter().cloned());
    tests.extend(["--", "--test-threads=1"].map(str::to_owned));
    build.run("release-tests", &tests, Some(&source), env, &[0])?;
    if sha(&executable)? != binary_hash {
        return Err("Executable changed during tests".into());
    }
    let dumpbin = text(&native_bin.join("dumpbin.exe"));
    let headers = build.run(
        "pe-headers",
        &[dumpbin.clone(), "/headers".into(), text(&executable)],
        None,
        env,
        &[0],
    )?;
    let imports = build.run(
        "pe-imports",
        &[dumpbin, "/imports".into(), text(&executable)],
        None,
        env,
        &[0],
    )?;
    build
        .receipt
        .insert("pe".into(), inspect_pe(&headers, &imports)?);
    build
        .receipt
        .insert("binary_sha256".into(), json!(binary_hash));
    let version = crate_version(&source.join("Cargo.toml"))?;
    let package = work.join(format!("ripscope-{version}-x86_64-windows-msvc"));
    fs::create_dir_all(package.join("bin"))?;
    copy_file(&executable, &package.join("bin/ripscope.exe"))?;
    delivery_files(&source, &package)?;
    copy_file(
        &source.join("docs/getting-started/installation.md"),
        &package.join("INSTALL.md"),
    )?;
    collect_notices(&vendor, &toolchain, &package)?;
    // The local receipt retains paths needed to repeat commands; the package does not.
    let mut recorded: Map<String, Value> = build
        .receipt
        .iter()
        .filter(|(key, _)| !["commands", "complete", "build_environment"].contains(&key.as_str()))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    recorded.insert("source_date_epoch".into(), json!(epoch));
    write_text(
        &package.join("BUILD.json"),
        &(json_text(&Value::Object(recorded)) + "\n"),
    )?;
    let manifest: String = inventory(&package)?
        .iter()
        .map(|(name, digest)| format!("{}  {name}\n", digest.as_str().unwrap_or_default()))
        .collect();
    write_text(&package.join("MANIFEST.sha256"), &manifest)?;
    let dist = work.join("dist");
    fs::create_dir(&dist)?;
    let archive_name = format!(
        "{}.zip",
        package
            .file_name()
            .ok_or("package without a name")?
            .to_string_lossy()
    );
    let output = dist.join(&archive_name);
    archive::zip(&package, &output, epoch)?;
    let line = format!("{}  {archive_name}\n", sha(&output)?);
    write_text(&dist.join("SHA256SUMS"), &line)?;
    write_text(&dist.join(format!("{archive_name}.sha256")), &line)?;
    let smoke = [
        text(&std::env::current_exe()?),
        "windows-smoke".to_owned(),
        text(&output),
        text(&work.join("smoke")),
    ];
    build.run("archive-smoke", &smoke, None, None, &[0])?;
    unchanged_sources(
        &source_inputs,
        &source,
        &repo,
        "Source input changed during execution",
    )?;
    for (name, path) in &input_roots {
        if Value::Object(inventory(path)?) != tool_inputs[*name] {
            return Err("Build tool inputs changed during execution".into());
        }
    }
    if inventory(&vendor)? != vendor_inputs {
        return Err("Vendored input changed during execution".into());
    }
    let archive = json!({"sha256": sha(&output)?, "bytes": fs::metadata(&output)?.len(), "path": text(&output)});
    build.receipt.insert("archive".into(), archive.clone());
    build.receipt.insert("complete".into(), json!(true));
    build.save()?;
    println!("{}", json_line(&archive));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn import_gate_rejects_dynamic_crt_and_foreign_libraries() {
        let headers = "8664 machine (x64)\n20B magic # (PE32+)";
        inspect_pe(
            headers,
            "    KERNEL32.dll\n    bcrypt.dll\n    api-ms-win-core-synch-l1-2-0.dll\n",
        )
        .unwrap();
        for name in [
            "VCRUNTIME140.dll",
            "MSVCP140.dll",
            "ucrtbase.dll",
            "MSVCRT.dll",
            "api-ms-win-crt-runtime-l1-1-0.dll",
            "sqlite3.dll",
            "tree-sitter.dll",
            "custom runtime.dll",
        ] {
            for prefix in ["", "Section contains the following delay load imports:\n"] {
                let imports = format!("{prefix}    KERNEL32.dll\n    {name}\n");
                assert!(inspect_pe(headers, &imports).is_err(), "{name}");
            }
        }
        assert!(inspect_pe("14C machine (x86)", "    KERNEL32.dll\n").is_err());
        assert!(inspect_pe(headers, "").is_err());
    }

    #[test]
    fn zip_bytes_ignore_input_metadata() {
        let root = tempfile::tempdir().unwrap();
        let package = root.path().join("candidate");
        fs::create_dir_all(package.join("bin")).unwrap();
        let path = package.join("bin/ripscope.exe");
        fs::write(&path, b"candidate").unwrap();
        archive::zip(&package, &root.path().join("one.zip"), 1_700_000_000).unwrap();
        fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_800_000_000))
            .unwrap();
        archive::zip(&package, &root.path().join("two.zip"), 1_700_000_000).unwrap();
        let one = fs::read(root.path().join("one.zip")).unwrap();
        assert_eq!(one, fs::read(root.path().join("two.zip")).unwrap());
        let entries = archive::zip_entries(&one).unwrap();
        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>(),
            ["candidate/bin/ripscope.exe"]
        );
        assert_eq!(archive::zip_read(&one, &entries[0]).unwrap(), b"candidate");
    }
}
