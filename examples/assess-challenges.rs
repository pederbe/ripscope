//! Run separately labelled challenge ranges under the campaign's resource bounds.
#[path = "support/capture.rs"]
#[allow(dead_code)] // This runner uses the guarded variant of the shared capture API.
mod capture;

use serde_json::{Value, json};
use std::fs;
use std::path::Path;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn read(path: &Path) -> Result<Value> {
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}

fn run(root: &Path) -> Result<()> {
    let repo = std::env::current_dir()?.canonicalize()?;
    let identity = read(&root.join("identities.json"))?;
    let binary = identity["binary"]
        .as_str()
        .ok_or("missing product binary")?;
    if capture::digest(Path::new(binary))? != identity["binary_sha256"] {
        return Err("frozen product identity changed".into());
    }
    fs::create_dir_all(root.join("evidence"))?;
    for name in ["memcached", "cjson", "libgit2", "micropython", "winfsp"] {
        if capture::digest(Path::new(binary))? != identity["binary_sha256"] {
            return Err("frozen product identity changed between challenges".into());
        }
        let directory = root.join(format!("challenges/{name}"));
        if directory.join("report.json").exists() {
            return Err("challenge already measured; preserve it before repeating".into());
        }
        if capture::tree_size(root)? > 76 << 30 {
            return Err("campaign scratch reserve exhausted".into());
        }
        let project = read(&root.join(format!("manifests/{name}.json")))?;
        let source = project["source"].as_str().ok_or("missing project source")?;
        let manifest = directory.join("manifest.json");
        let manifest_sha256 = capture::digest(&manifest)?;
        let out = directory.join("raw");
        let launch = capture::run_guarded(
            &repo.join("target/release/examples/assess-navigation"),
            &[
                "run".into(),
                binary.into(),
                source.into(),
                manifest.to_string_lossy().into_owned(),
                out.to_string_lossy().into_owned(),
            ],
            &repo,
            &directory.join("launch"),
            86400,
            true,
            &[
                (out.clone(), 18 << 30),
                (directory.clone(), 20 << 30),
                (root.to_owned(), 100 << 30),
            ],
        )?;
        if launch["cleanup_error"].is_string() {
            return Err("challenge scope cleanup failed; raw evidence retained".into());
        }
        if capture::digest(&manifest)? != manifest_sha256
            || capture::digest(Path::new(binary))? != identity["binary_sha256"]
        {
            return Err(
                "challenge manifest or product identity changed; raw evidence retained".into(),
            );
        }
        let mut report = if out.join("report.json").exists() {
            read(&out.join("report.json"))?
        } else {
            json!({"complete":false,"error":"assessment failed; see launch and raw evidence"})
        };
        if launch["complete"] != true {
            report["complete"] = json!(false);
        }
        report["launch"] = launch;
        if report["manifest_sha256"].is_string()
            && (report["manifest_sha256"] != manifest_sha256
                || report["binary_sha256"] != identity["binary_sha256"])
        {
            return Err("challenge report identity differs from frozen inputs".into());
        }
        report["manifest_sha256"] = json!(manifest_sha256);
        report["runner_sha256"] = json!(capture::digest(&std::env::current_exe()?)?);
        if out.join("index.db").exists() {
            report["disposable_index_sha256"] = json!(capture::digest(&out.join("index.db"))?);
            fs::remove_file(out.join("index.db"))?;
        }
        let archive = root.join(format!("evidence/challenge-{name}.tar.zst"));
        let compressed = capture::run_guarded(
            Path::new("tar"),
            &[
                "--zstd".into(),
                "-cf".into(),
                "-".into(),
                "-C".into(),
                directory.to_string_lossy().into_owned(),
                "raw".into(),
            ],
            &repo,
            &directory.join("compression"),
            900,
            true,
            &[(directory.clone(), 20 << 30), (root.to_owned(), 100 << 30)],
        )?;
        if compressed["complete"] != true {
            return Err("challenge evidence compression failed; raw evidence retained".into());
        }
        fs::rename(
            compressed["stdout_path"]
                .as_str()
                .ok_or("missing archive capture")?,
            &archive,
        )?;
        report["archive"] = json!({"path":archive,"sha256":capture::digest(&archive)?,"bytes":archive.metadata()?.len()});
        report["finalized"] = json!(true);
        capture::save(&directory.join("report.json"), &report)?;
        fs::remove_dir_all(out)?;
        println!("{name}: complete={}", report["complete"]);
    }
    Ok(())
}

fn main() -> Result<()> {
    let root = std::env::args()
        .nth(1)
        .ok_or("usage: assess-challenges CAMPAIGN_ROOT")?;
    run(&Path::new(&root).canonicalize()?)
}
