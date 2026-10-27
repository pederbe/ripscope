//! The signed apt and dnf repository at `packages.ripscope.com`, made from published
//! release packages. The `.deb` is published unchanged and the `.rpm` gains only a
//! header signature, so the inspected executable is never rebuilt.
//!
//! apt clients fetch indexes by the hash their signed `InRelease` names, and dnf
//! clients read a mirrorlist naming one complete, permanent metadata snapshot. Every
//! earlier package, index and snapshot stays published, so a client that read the
//! previous generation, or a publication that stopped part way, still finds consistent
//! files. The files in [`CURRENT`] are uploaded last.

use crate::common::{Result, relative_name, set_modified, sha, under, unique_id, walk};
use sha2::{Digest, Sha256, Sha512};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Where the repository is served.
pub const BASE_URL: &str = "https://packages.ripscope.com";

const POOL: &str = "apt/pool/main/r/ripscope";
const DISTS: &str = "apt/dists/stable";
const INDEXES: &str = "apt/dists/stable/main/binary-amd64";

/// Repository files that change with each publication, in upload order. Everything
/// else is immutable once written and is uploaded before these.
pub const CURRENT: &[&str] = &[
    "ripscope.asc",
    "ripscope.gpg",
    "ripscope.repo",
    "apt/dists/stable/main/binary-amd64/Packages",
    "apt/dists/stable/main/binary-amd64/Packages.gz",
    "apt/dists/stable/Release",
    "apt/dists/stable/Release.gpg",
    "apt/dists/stable/InRelease",
    "rpm/mirrorlist",
];

/// A release package in the repository, by format and version.
#[derive(Debug, PartialEq, Eq)]
pub enum Package {
    Deb(String),
    Rpm(String),
}

fn version_parts(version: &str) -> Option<(u64, u64, u64)> {
    let mut parts = version.split('.').map(|part| {
        (!part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
            .then(|| part.parse().ok())
            .flatten()
    });
    let parsed = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(parsed)
}

/// A release package's file name, or `None` for any other name.
pub fn package(name: &str) -> Option<Package> {
    if let Some(version) = name
        .strip_prefix("ripscope_")
        .and_then(|rest| rest.strip_suffix("_amd64.deb"))
    {
        return version_parts(version).map(|_| Package::Deb(version.into()));
    }
    let version = name
        .strip_prefix("ripscope-")?
        .strip_suffix("-1.x86_64.rpm")?;
    version_parts(version).map(|_| Package::Rpm(version.into()))
}

fn lower_hex(text: &str) -> bool {
    !text.is_empty()
        && text
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

/// Whether `relative` is a path the repository may hold.
pub fn expected_file(relative: &str) -> bool {
    if let Some(name) = relative
        .strip_prefix(POOL)
        .and_then(|rest| rest.strip_prefix('/'))
    {
        return matches!(package(name), Some(Package::Deb(_)));
    }
    if let Some(rest) = relative.strip_prefix("rpm/snapshots/") {
        return rest.split_once('/').is_some_and(|(generation, file)| {
            generation.len() == 64
                && lower_hex(generation)
                && file
                    .strip_prefix("repodata/")
                    .is_some_and(|name| !name.is_empty() && !name.contains('/'))
        });
    }
    if let Some(name) = relative.strip_prefix("rpm/") {
        return name == "mirrorlist" || matches!(package(name), Some(Package::Rpm(_)));
    }
    relative.starts_with("apt/dists/stable/")
        || matches!(relative, "ripscope.asc" | "ripscope.gpg" | "ripscope.repo")
}

/// The dnf repository definition users install.
pub fn dnf_definition() -> String {
    format!(
        "[ripscope]\nname=Ripscope\nmirrorlist={BASE_URL}/rpm/mirrorlist\nenabled=1\n\
         gpgcheck=1\nrepo_gpgcheck=1\ngpgkey={BASE_URL}/ripscope.asc\n"
    )
}

/// The dnf mirrorlist naming the metadata snapshot `generation`.
pub fn mirrorlist(generation: &str) -> String {
    format!("{BASE_URL}/rpm/snapshots/{generation}/\n")
}

/// The version an upgrade check starts from: when `current` is the newest of
/// `versions`, the one before it, and otherwise none.
pub fn upgrade_from(versions: &[String], current: &str) -> Option<String> {
    let current_parts = version_parts(current)?;
    let mut sorted: Vec<_> = versions
        .iter()
        .filter_map(|version| Some((version_parts(version)?, version)))
        .collect();
    sorted.sort();
    if sorted.last()?.0 != current_parts {
        return None;
    }
    sorted
        .iter()
        .rev()
        .find(|(parts, _)| *parts < current_parts)
        .map(|(_, version)| (*version).clone())
}

/// Rewrites an archive's `MANIFEST.sha256` to the paths the packages install.
pub fn installed_manifest(manifest: &str) -> Result<String> {
    let mut installed = String::new();
    let mut executable = false;
    for line in manifest.lines() {
        let (hash, path) = line
            .split_once("  ")
            .filter(|(hash, _)| lower_hex(hash))
            .ok_or_else(|| format!("unexpected manifest line {line:?}"))?;
        if path.split('/').any(|part| matches!(part, "" | "." | "..")) {
            return Err(format!("unexpected manifest path {path:?}").into());
        }
        let target = if path == "bin/ripscope" {
            executable = true;
            "/usr/bin/ripscope".to_owned()
        } else if path.starts_with("bin/") {
            return Err(format!("the packages do not install {path}").into());
        } else {
            format!("/usr/share/ripscope/{path}")
        };
        installed.push_str(&format!("{hash}  {target}\n"));
    }
    if !executable {
        return Err("the manifest has no bin/ripscope".into());
    }
    Ok(installed)
}

/// Runs a command and gives its standard output, or an error with its standard error.
fn output(command: &mut Command) -> Result<String> {
    let done = command
        .output()
        .map_err(|error| format!("{:?}: {error}", command.get_program()))?;
    if !done.status.success() {
        return Err(format!(
            "{command:?} failed with {}: {}",
            done.status,
            String::from_utf8_lossy(&done.stderr).trim()
        )
        .into());
    }
    Ok(String::from_utf8(done.stdout)?)
}

fn run_command(command: &mut Command) -> Result<()> {
    let status = command
        .status()
        .map_err(|error| format!("{:?}: {error}", command.get_program()))?;
    if !status.success() {
        return Err(format!("{command:?} failed with {status}").into());
    }
    Ok(())
}

/// A directory removed when dropped.
struct Work(PathBuf);

impl Work {
    fn new() -> Result<Self> {
        let path = std::env::temp_dir().join(format!("ripscope-repository-{}", unique_id()));
        fs::create_dir(&path)?;
        Ok(Self(path))
    }
}

impl Drop for Work {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// The one primary key's fingerprint in an armored public key file.
fn fingerprint(public_key: &Path) -> Result<String> {
    let listing = output(
        Command::new("gpg")
            .args(["--batch", "--show-keys", "--with-colons"])
            .arg(public_key),
    )?;
    let mut primary = false;
    let mut found = Vec::new();
    for line in listing.lines() {
        let fields: Vec<&str> = line.split(':').collect();
        match fields[0] {
            "pub" => primary = true,
            "fpr" if primary => {
                found.push(fields.get(9).copied().unwrap_or_default().to_owned());
                primary = false;
            }
            "sub" => primary = false,
            _ => {}
        }
    }
    match found.as_slice() {
        [one] if !one.is_empty() => Ok(one.clone()),
        _ => Err(format!("{} must hold exactly one primary key", public_key.display()).into()),
    }
}

/// Signs with the release key, unattended. `PACKAGE_SIGNING_PASSPHRASE` holds its
/// passphrase when it has one.
struct Signer {
    fingerprint: String,
    passphrase: Option<PathBuf>,
}

impl Signer {
    fn new(public_key: &Path, work: &Path) -> Result<Self> {
        let fingerprint = fingerprint(public_key)?;
        output(Command::new("gpg").args(["--batch", "--list-secret-keys", &fingerprint]))
            .map_err(|_| format!("the secret key for {fingerprint} is not in the keyring"))?;
        let passphrase = match std::env::var("PACKAGE_SIGNING_PASSPHRASE") {
            Ok(text) if !text.is_empty() => {
                let path = work.join("passphrase");
                let mut options = fs::OpenOptions::new();
                #[cfg(unix)]
                std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
                options
                    .write(true)
                    .create_new(true)
                    .open(&path)?
                    .write_all(text.as_bytes())?;
                Some(path)
            }
            _ => None,
        };
        Ok(Self {
            fingerprint,
            passphrase,
        })
    }

    fn unattended(&self) -> Vec<String> {
        let mut args = vec!["--batch".to_owned()];
        if let Some(path) = &self.passphrase {
            args.extend([
                "--pinentry-mode".to_owned(),
                "loopback".to_owned(),
                "--passphrase-file".to_owned(),
                path.display().to_string(),
            ]);
        }
        args
    }

    fn sign(&self, args: &[&str], file: &Path) -> Result<()> {
        output(
            Command::new("gpg")
                .args(self.unattended())
                .args(["--yes", "--local-user", &self.fingerprint])
                .args(["--digest-algo", "SHA512"])
                .args(args)
                .arg(file),
        )
        .map(drop)
    }

    /// Adds a header signature that RPM 4 clients can read. RPM 6 makes one only
    /// when asked, with `--rpmv4`, which RPM 4 does not know.
    fn sign_rpm(&self, file: &Path, rpm: &Rpm) -> Result<()> {
        output(
            Command::new("rpmsign")
                .arg("--addsign")
                .args(rpm.version_6.then_some("--rpmv4"))
                .arg("--define")
                .arg(format!("_gpg_name {}", self.fingerprint))
                .arg("--define")
                .arg(format!(
                    "_gpg_sign_cmd_extra_args {}",
                    self.unattended().join(" ")
                ))
                .arg(file),
        )
        .map(drop)
    }
}

/// RPM queries against a database that trusts only the release key.
struct Rpm {
    database: PathBuf,
    payload_tag: &'static str,
    version_6: bool,
}

impl Rpm {
    fn new(public_key: &Path, work: &Path) -> Result<Self> {
        let database = work.join("rpmdb");
        fs::create_dir(&database)?;
        output(
            Command::new("rpmkeys")
                .arg("--dbpath")
                .arg(&database)
                .arg("--import")
                .arg(public_key),
        )?;
        // RPM 6 renamed PAYLOADDIGEST to PAYLOADSHA256.
        let tags = output(Command::new("rpm").arg("--querytags"))?;
        let payload_tag = ["PAYLOADSHA256", "PAYLOADDIGEST"]
            .into_iter()
            .find(|tag| tags.lines().any(|line| line == *tag))
            .ok_or("rpm has no SHA-256 payload digest tag")?;
        let version = output(Command::new("rpm").arg("--version"))?;
        let major = version
            .trim()
            .rsplit(' ')
            .next()
            .and_then(|number| number.split('.').next()?.parse::<u32>().ok())
            .ok_or_else(|| format!("unexpected rpm version {version:?}"))?;
        Ok(Self {
            database,
            payload_tag,
            version_6: major >= 6,
        })
    }

    fn query(&self, format: &str, file: &Path) -> Result<String> {
        output(
            Command::new("rpm")
                .arg("--dbpath")
                .arg(&self.database)
                .args(["-qp", "--nosignature", "--qf", format])
                .arg(file),
        )
    }

    /// The header and payload digests, which signing must not change.
    fn identity(&self, file: &Path) -> Result<String> {
        let identity = self.query(
            &format!("%{{SHA256HEADER}} %{{{}}}", self.payload_tag),
            file,
        )?;
        match identity.split_once(' ') {
            Some((header, payload))
                if header.len() == 64
                    && payload.len() == 64
                    && lower_hex(header)
                    && lower_hex(payload) =>
            {
                Ok(identity)
            }
            _ => Err(format!(
                "{} has no SHA-256 header and payload digests",
                file.display()
            )
            .into()),
        }
    }

    /// Whether the release key signed `file`. An unsigned or tampered package makes
    /// `rpmkeys` fail, so that is a no too.
    fn signed(&self, file: &Path) -> bool {
        output(
            Command::new("rpmkeys")
                .arg("--dbpath")
                .arg(&self.database)
                .arg("--checksig")
                .arg(file),
        )
        .is_ok_and(|checked| checked.trim_end().ends_with(": digests signatures OK"))
    }
}

/// Copies `from` to `to`, or checks that the file already there is identical.
fn keep_or_copy(from: &Path, to: &Path) -> Result<()> {
    if to.exists() {
        if fs::read(from)? != fs::read(to)? {
            return Err(format!("{} differs from the published file", to.display()).into());
        }
    } else {
        fs::copy(from, to)?;
    }
    Ok(())
}

fn add_packages(
    repository: &Path,
    packages: &[String],
    signer: &Signer,
    rpm: &Rpm,
    work: &Path,
) -> Result<()> {
    let pool = repository.join(POOL);
    fs::create_dir_all(&pool)?;
    fs::create_dir_all(repository.join("rpm"))?;
    for package_path in packages {
        let package_path = Path::new(package_path);
        let name = package_path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| format!("{} has no file name", package_path.display()))?;
        match package(name) {
            Some(Package::Deb(_)) => keep_or_copy(package_path, &pool.join(name))?,
            Some(Package::Rpm(_)) => {
                let signed = work.join(name);
                fs::copy(package_path, &signed)?;
                let before = rpm.identity(&signed)?;
                signer.sign_rpm(&signed, rpm)?;
                if rpm.identity(&signed)? != before {
                    return Err(format!("signing changed the contents of {name}").into());
                }
                if !rpm.signed(&signed) {
                    return Err(format!("{name} signature check failed").into());
                }
                let published = repository.join("rpm").join(name);
                if published.exists() {
                    if rpm.identity(&published)? != before {
                        return Err(format!("{name} differs from the published package").into());
                    }
                } else {
                    fs::copy(&signed, &published)?;
                }
            }
            None => return Err(format!("{name} is not a Ripscope .deb or .rpm").into()),
        }
    }
    Ok(())
}

/// The indexers list every package they find, so the repository may hold only
/// Ripscope packages whose metadata matches their names, and every RPM must carry the
/// release key's signature.
fn check_contents(repository: &Path, rpm: &Rpm) -> Result<()> {
    for path in walk(repository)? {
        if !path.is_file() {
            continue;
        }
        let relative = relative_name(&path, repository)?;
        if !expected_file(&relative) {
            return Err(format!("unexpected file in the repository: {relative}").into());
        }
        let name = relative.rsplit('/').next().unwrap_or_default();
        match package(name) {
            Some(Package::Deb(version)) => {
                let fields = output(Command::new("dpkg-deb").arg("-f").arg(&path).args([
                    "Package",
                    "Version",
                    "Architecture",
                ]))?;
                if fields != format!("Package: ripscope\nVersion: {version}\nArchitecture: amd64\n")
                {
                    return Err(format!("{relative} does not match its control fields").into());
                }
            }
            Some(Package::Rpm(version)) => {
                if rpm.query("%{NAME} %{VERSION} %{ARCH}", &path)?
                    != format!("ripscope {version} x86_64")
                {
                    return Err(format!("{relative} does not match its header").into());
                }
                if !rpm.signed(&path) {
                    return Err(format!("{relative} is not signed by the release key").into());
                }
            }
            None => {}
        }
    }
    Ok(())
}

fn apt_metadata(repository: &Path, signer: &Signer) -> Result<()> {
    let indexes = repository.join(INDEXES);
    fs::create_dir_all(&indexes)?;
    let packages = output(
        Command::new("apt-ftparchive")
            .args(["packages", "pool"])
            .current_dir(repository.join("apt")),
    )?;
    fs::write(indexes.join("Packages"), &packages)?;
    let mut gzip = flate2::GzBuilder::new().write(
        fs::File::create(indexes.join("Packages.gz"))?,
        flate2::Compression::best(),
    );
    gzip.write_all(packages.as_bytes())?;
    gzip.finish()?;
    // Clients holding an earlier InRelease still find its exact indexes.
    for (algorithm, digest) in [
        ("SHA256", hash::<Sha256> as fn(&[u8]) -> String),
        ("SHA512", hash::<Sha512>),
    ] {
        let directory = indexes.join("by-hash").join(algorithm);
        fs::create_dir_all(&directory)?;
        for index in ["Packages", "Packages.gz"] {
            let file = indexes.join(index);
            keep_or_copy(&file, &directory.join(digest(&fs::read(&file)?)))?;
        }
    }
    let dists = repository.join(DISTS);
    for name in ["Release", "InRelease", "Release.gpg"] {
        let _ = fs::remove_file(dists.join(name));
    }
    let mut release = Command::new("apt-ftparchive");
    for (field, value) in [
        ("Origin", "Ripscope"),
        ("Label", "Ripscope"),
        ("Suite", "stable"),
        ("Codename", "stable"),
        ("Architectures", "amd64"),
        ("Components", "main"),
        ("Acquire-By-Hash", "yes"),
        ("Description", "Ripscope release packages"),
    ] {
        release
            .arg("-o")
            .arg(format!("APT::FTPArchive::Release::{field}={value}"));
    }
    let text = output(release.arg("release").arg(&dists))?;
    fs::write(dists.join("Release"), text)?;
    signer.sign(
        &[
            "--clearsign",
            "--output",
            &dists.join("InRelease").display().to_string(),
        ],
        &dists.join("Release"),
    )?;
    signer.sign(
        &[
            "--armor",
            "--detach-sign",
            "--output",
            &dists.join("Release.gpg").display().to_string(),
        ],
        &dists.join("Release"),
    )
}

fn hash<D: Digest>(bytes: &[u8]) -> String {
    base16ct::lower::encode_string(&D::digest(bytes))
}

/// Writes a new dnf metadata snapshot, or completes an identical one, and gives its
/// directory.
fn dnf_metadata(repository: &Path, signer: &Signer, rpm: &Rpm, work: &Path) -> Result<PathBuf> {
    let generated = work.join("rpm");
    fs::create_dir(&generated)?;
    // The same packages give the same snapshot, so a retry completes it instead of
    // adding another. The metadata records each package file's modification time,
    // which a copy or download resets, so each is set to the package's build time,
    // and the snapshot's own timestamps to the newest of those.
    let mut revision = 0;
    for entry in fs::read_dir(repository.join("rpm"))? {
        let path = entry?.path();
        if path.extension().is_some_and(|extension| extension == "rpm") {
            let built = rpm.query("%{BUILDTIME}", &path)?.parse::<u64>()?;
            set_modified(
                &path,
                std::time::UNIX_EPOCH + std::time::Duration::from_secs(built),
            )?;
            revision = revision.max(built);
        }
    }
    // Package locations lead from the snapshot back to the shared rpm pool.
    output(
        Command::new("createrepo_c")
            .args(["--quiet", "--set-timestamp-to-revision", "--revision"])
            .arg(revision.to_string())
            .args(["--location-prefix", "../../", "--outputdir"])
            .arg(&generated)
            .arg(repository.join("rpm")),
    )?;
    let repodata = generated.join("repodata");
    signer.sign(
        &[
            "--armor",
            "--detach-sign",
            "--output",
            &repodata.join("repomd.xml.asc").display().to_string(),
        ],
        &repodata.join("repomd.xml"),
    )?;
    let generation = sha(&repodata.join("repomd.xml"))?;
    let snapshot = repository.join("rpm/snapshots").join(&generation);
    // An interrupted upload can leave part of this snapshot published. Fill in what
    // is missing, and keep an existing signature of the identical metadata.
    fs::create_dir_all(snapshot.join("repodata"))?;
    for entry in fs::read_dir(&repodata)? {
        let file = entry?.path();
        let target = snapshot
            .join("repodata")
            .join(file.file_name().unwrap_or_default());
        if target.exists() && file.ends_with("repomd.xml.asc") {
            continue;
        }
        keep_or_copy(&file, &target)?;
    }
    fs::write(repository.join("rpm/mirrorlist"), mirrorlist(&generation))?;
    Ok(snapshot)
}

/// `package-repository REPOSITORY PUBLIC_KEY PACKAGE...`: adds release packages to a
/// local copy of the published repository (empty the first time) and signs it again.
pub fn run(args: &[String]) -> Result<()> {
    let [repository, public_key, packages @ ..] = args else {
        return Err(USAGE.into());
    };
    if packages.is_empty() {
        return Err(USAGE.into());
    }
    let (repository, public_key) = (Path::new(repository), Path::new(public_key));
    fs::create_dir_all(repository)?;
    let work = Work::new()?;
    let signer = Signer::new(public_key, &work.0)?;
    let rpm = Rpm::new(public_key, &work.0)?;
    add_packages(repository, packages, &signer, &rpm, &work.0)?;
    check_contents(repository, &rpm)?;
    apt_metadata(repository, &signer)?;
    let snapshot = dnf_metadata(repository, &signer, &rpm, &work.0)?;
    fs::copy(public_key, repository.join("ripscope.asc"))?;
    let keyring = repository.join("ripscope.gpg");
    output(
        Command::new("gpg")
            .args(["--batch", "--yes", "--dearmor", "--output"])
            .arg(&keyring)
            .arg(public_key),
    )?;
    fs::write(repository.join("ripscope.repo"), dnf_definition())?;

    // Check the signatures as a client would, with only the published key.
    let dists = repository.join(DISTS);
    let repomd = snapshot.join("repodata/repomd.xml");
    for files in [
        vec![dists.join("InRelease")],
        vec![dists.join("Release.gpg"), dists.join("Release")],
        vec![repomd.with_extension("xml.asc"), repomd],
    ] {
        output(
            Command::new("gpgv")
                .arg("--keyring")
                .arg(&keyring)
                .args(&files),
        )?;
    }
    println!(
        "{}: every package signed into the repository by {}",
        repository.display(),
        signer.fingerprint
    );
    Ok(())
}

const USAGE: &str = "usage: ripscope-release package-repository REPOSITORY PUBLIC_KEY PACKAGE...";

/// `package-repository-upload REPOSITORY BUCKET`: uploads a prepared repository with
/// the AWS CLI, whose environment names the storage and its credentials.
pub fn upload(args: &[String]) -> Result<()> {
    let [repository, bucket] = args else {
        return Err("usage: ripscope-release package-repository-upload REPOSITORY BUCKET".into());
    };
    let destination = format!("s3://{bucket}");
    // Earlier objects stay for clients holding an older index or mirrorlist, so
    // nothing is deleted.
    let mut sync = Command::new("aws");
    sync.args(["s3", "sync", "--only-show-errors", repository, &destination]);
    for path in CURRENT {
        sync.args(["--exclude", path]);
    }
    run_command(sync.args(["--cache-control", "public, max-age=31536000, immutable"]))?;
    for path in CURRENT {
        run_command(
            Command::new("aws")
                .args(["s3", "cp", "--only-show-errors"])
                .arg(Path::new(repository).join(path))
                .arg(format!("{destination}/{path}"))
                .args(["--cache-control", "no-cache"]),
        )?;
    }
    Ok(())
}

const CHECKS: &str = r#"
check_version() { test "$(ripscope --version)" = "ripscope $1"; }
check_installed() {
    check_version "$VERSION"
    sha256sum -c --quiet /check/installed.sha256
    mkdir -p /work/project
    cd /work/project
    printf 'int value;\n' > source.c
    ripscope -bk source.c
    test "$(ripscope -dL -1 value)" = "source.c value 1 int value;"
    printf 'set nocompatible\n' > /root/.vimrc
    cd /
}
check_removed() {
    test ! -e /usr/bin/ripscope
    test ! -e /usr/share/ripscope
    test -f /work/project/source.c
    test -f /work/project/ripscope.db
    test -f /root/.vimrc
}
"#;

const APT: &str = r#"
export DEBIAN_FRONTEND=noninteractive
rm -rf /etc/apt/sources.list /etc/apt/sources.list.d/*
# A client holding another key refuses the repository.
echo "deb [signed-by=/check/wrong.gpg] file:/repo/apt stable main" > /etc/apt/sources.list.d/ripscope.list
apt-get update || true
if apt-get install -y ripscope; then echo "installed with the wrong key" >&2; exit 1; fi
rm -rf /var/lib/apt/lists/*
install -m 644 /repo/ripscope.gpg /usr/share/keyrings/ripscope.gpg
echo "deb [signed-by=/usr/share/keyrings/ripscope.gpg] file:/repo/apt stable main" > /etc/apt/sources.list.d/ripscope.list
apt-get update
if [ -n "$PREVIOUS" ]; then
    apt-get install -y "ripscope=$PREVIOUS"
    check_version "$PREVIOUS"
    apt-get upgrade -y
else
    apt-get install -y "ripscope=$VERSION"
fi
check_installed
apt-get remove -y ripscope
check_removed
"#;

const DNF: &str = r#"
repository() { dnf -y --disablerepo='*' --enablerepo=ripscope "$@"; }
# A client holding another key refuses the repository.
cp /check/wrong.repo /etc/yum.repos.d/ripscope.repo
if repository install ripscope; then echo "installed with the wrong key" >&2; exit 1; fi
dnf clean all
cp /check/ripscope.repo /etc/yum.repos.d/ripscope.repo
if [ -n "$PREVIOUS" ]; then
    repository install "ripscope-$PREVIOUS"
    check_version "$PREVIOUS"
    repository upgrade
else
    repository install "ripscope-$VERSION"
fi
check_installed
repository remove ripscope
check_removed
"#;

/// The clients checked: Debian and Ubuntu through apt, and the oldest supported
/// RHEL-family release through dnf.
const CLIENTS: &[(&str, &str)] = &[
    ("docker.io/library/debian:12", APT),
    ("docker.io/library/ubuntu:22.04", APT),
    ("docker.io/library/ubuntu:24.04", APT),
    ("docker.io/library/almalinux:8", DNF),
];

/// `package-repository-check REPOSITORY ARCHIVE WORK`: installs the version of the
/// extracted release archive ARCHIVE from a generated repository in offline
/// containers, with signature checks on, upgrading from the previous version when it
/// is the newest, then removes it. Add `--published` to check the live HTTPS
/// repository instead. WORK must not exist.
pub fn check(args: &[String]) -> Result<()> {
    let published = args.last().is_some_and(|arg| arg == "--published");
    let args = if published {
        &args[..args.len() - 1]
    } else {
        args
    };
    let [repository, archive, work] = args else {
        return Err(
            "usage: ripscope-release package-repository-check REPOSITORY ARCHIVE WORK [--published]".into(),
        );
    };
    let (repository, archive, work) = (
        fs::canonicalize(repository)?,
        Path::new(archive),
        Path::new(work),
    );
    let version = archive
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_prefix("ripscope-"))
        .and_then(|name| name.strip_suffix("-x86_64-linux-gnu"))
        .filter(|version| version_parts(version).is_some())
        .ok_or("ARCHIVE must be an extracted ripscope-X.Y.Z-x86_64-linux-gnu directory")?;

    // The archive's own files, checked against its manifest, are what the packages
    // must install.
    let manifest = fs::read_to_string(archive.join("MANIFEST.sha256"))?;
    let installed = installed_manifest(&manifest)?;
    for line in manifest.lines() {
        let (hash, path) = line.split_once("  ").unwrap_or_default();
        if sha(&under(archive, path))? != hash {
            return Err(format!("{path} does not match the archive's manifest").into());
        }
    }
    fs::create_dir(work)?;
    let work = fs::canonicalize(work)?;
    fs::write(work.join("installed.sha256"), installed)?;

    // Offline clients read a mounted repository; published checks use its HTTPS URL.
    let local = |path: &str| -> Result<String> {
        let text = fs::read_to_string(repository.join(path))?;
        if !text.contains(BASE_URL) {
            return Err(format!("{path} does not name {BASE_URL}").into());
        }
        Ok(text
            .replace(
                &format!("{BASE_URL}/rpm/mirrorlist"),
                "file:///check/mirrorlist",
            )
            .replace(BASE_URL, "file:///repo"))
    };
    let definition = if published {
        fs::read_to_string(repository.join("ripscope.repo"))?
    } else {
        local("ripscope.repo")?
    };
    fs::write(work.join("mirrorlist"), local("rpm/mirrorlist")?)?;
    fs::write(work.join("ripscope.repo"), &definition)?;
    fs::write(
        work.join("wrong.repo"),
        definition.replace(
            &if published {
                format!("{BASE_URL}/ripscope.asc")
            } else {
                "file:///repo/ripscope.asc".into()
            },
            "file:///check/wrong.asc",
        ),
    )?;
    let home = work.join("wrong-key");
    fs::create_dir(&home)?;
    let gpg = || {
        let mut command = Command::new("gpg");
        command.arg("--homedir").arg(&home).arg("--batch");
        command
    };
    output(gpg().args([
        "--passphrase",
        "",
        "--quick-gen-key",
        "Wrong key",
        "rsa3072",
        "sign",
        "never",
    ]))?;
    fs::write(
        work.join("wrong.asc"),
        output(gpg().args(["--armor", "--export"]))?,
    )?;
    output(
        gpg()
            .args(["--dearmor", "--output"])
            .arg(work.join("wrong.gpg"))
            .arg(work.join("wrong.asc")),
    )?;
    fs::remove_dir_all(&home)?;

    let mut versions = Vec::new();
    for entry in fs::read_dir(repository.join(POOL))? {
        if let Some(Package::Deb(found)) = package(&entry?.file_name().to_string_lossy()) {
            versions.push(found);
        }
    }
    let previous = upgrade_from(&versions, version).unwrap_or_default();
    let engine = std::env::var("CONTAINER_ENGINE").unwrap_or_else(|_| "podman".into());
    for (image, steps) in CLIENTS {
        println!("Checking {image}, from {previous:?} to {version}");
        let mut command = Command::new(&engine);
        command.args([
            "run",
            "--rm",
            "--network",
            if published { "bridge" } else { "none" },
        ]);
        if published {
            // Minimal client images may lack CA roots. Use the hosted runner's
            // normal trust store while keeping HTTPS verification enabled.
            command.args(["-v", "/etc/ssl/certs:/etc/ssl/certs:ro"]);
        }
        let steps = if published {
            steps.replace("file:/repo/apt", &format!("{BASE_URL}/apt"))
        } else {
            (*steps).into()
        };
        run_command(
            command
                .args(["-v"])
                .arg(format!("{}:/repo:ro", repository.display()))
                .arg("-v")
                .arg(format!("{}:/check:ro", work.display()))
                .args(["-e", &format!("VERSION={version}")])
                .args(["-e", &format!("PREVIOUS={previous}")])
                .args([*image, "sh", "-euc"])
                .arg(format!("{CHECKS}{steps}")),
        )
        .map_err(|error| format!("{image}: {error}"))?;
    }
    println!("Signed installation, upgrade, search and removal passed for {version}.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_names_carry_their_format_and_version() {
        assert_eq!(
            package("ripscope_0.2.4_amd64.deb"),
            Some(Package::Deb("0.2.4".into()))
        );
        assert_eq!(
            package("ripscope-0.2.4-1.x86_64.rpm"),
            Some(Package::Rpm("0.2.4".into()))
        );
        for other in [
            "ripscope_0.2_amd64.deb",
            "ripscope_0.2.4-rc1_amd64.deb",
            "ripscope_0.2.4_arm64.deb",
            "ripscope-0.2.4-2.x86_64.rpm",
            "ripscope-0.2.4-1.src.rpm",
            "other_0.2.4_amd64.deb",
            "ripscope_0.2.4_amd64.deb.sha256",
        ] {
            assert_eq!(package(other), None, "{other}");
        }
    }

    #[test]
    fn the_repository_holds_only_packages_metadata_and_keys() {
        let snapshot = format!("rpm/snapshots/{}/repodata/repomd.xml", "a".repeat(64));
        for path in [
            "apt/pool/main/r/ripscope/ripscope_0.2.4_amd64.deb",
            "rpm/ripscope-0.2.4-1.x86_64.rpm",
            "apt/dists/stable/InRelease",
            "apt/dists/stable/main/binary-amd64/by-hash/SHA256/abc",
            snapshot.as_str(),
            "rpm/mirrorlist",
            "ripscope.asc",
            "ripscope.gpg",
            "ripscope.repo",
        ] {
            assert!(expected_file(path), "{path}");
        }
        for path in [
            "apt/pool/main/r/ripscope/other_0.2.4_amd64.deb",
            "apt/pool/main/r/ripscope/ripscope-0.2.4-1.x86_64.rpm",
            "rpm/ripscope_0.2.4_amd64.deb",
            "rpm/snapshots/short/repodata/repomd.xml",
            "rpm/repodata/repomd.xml",
            "index.html",
            "ripscope.sh",
        ] {
            assert!(!expected_file(path), "{path}");
        }
    }

    #[test]
    fn current_files_are_metadata_and_the_signed_indexes_come_last() {
        for path in CURRENT {
            assert!(expected_file(path), "{path}");
            assert!(package(path.rsplit('/').next().unwrap()).is_none());
        }
        let at = |name: &str| CURRENT.iter().position(|path| *path == name).unwrap();
        let in_release = at("apt/dists/stable/InRelease");
        for index in [
            "apt/dists/stable/main/binary-amd64/Packages",
            "apt/dists/stable/main/binary-amd64/Packages.gz",
            "apt/dists/stable/Release",
            "apt/dists/stable/Release.gpg",
            "ripscope.gpg",
        ] {
            assert!(at(index) < in_release, "{index}");
        }
        assert!(at("ripscope.asc") < at("rpm/mirrorlist"));
        assert!(at("ripscope.repo") < at("rpm/mirrorlist"));
    }

    #[test]
    fn dnf_checks_package_and_metadata_signatures() {
        let definition = dnf_definition();
        for line in [
            "[ripscope]",
            "mirrorlist=https://packages.ripscope.com/rpm/mirrorlist",
            "gpgcheck=1",
            "repo_gpgcheck=1",
            "gpgkey=https://packages.ripscope.com/ripscope.asc",
        ] {
            assert!(definition.lines().any(|seen| seen == line), "{line}");
        }
        assert_eq!(
            mirrorlist("ab12"),
            "https://packages.ripscope.com/rpm/snapshots/ab12/\n"
        );
    }

    #[test]
    fn upgrades_start_from_the_previous_version_only_for_the_newest() {
        let versions: Vec<String> = ["0.2.0", "0.10.0", "0.2.4", "0.1.1"]
            .map(String::from)
            .into();
        assert_eq!(upgrade_from(&versions, "0.10.0").as_deref(), Some("0.2.4"));
        assert_eq!(upgrade_from(&versions, "0.2.4"), None);
        assert_eq!(upgrade_from(&["0.2.4".into()], "0.2.4"), None);
    }

    #[test]
    fn the_installed_manifest_follows_the_package_layout() {
        let manifest = "aa  bin/ripscope\nbb  plugin/ripscope.vim\ncc  contrib/emacs/ripscope.el\n";
        assert_eq!(
            installed_manifest(manifest).unwrap(),
            "aa  /usr/bin/ripscope\nbb  /usr/share/ripscope/plugin/ripscope.vim\n\
             cc  /usr/share/ripscope/contrib/emacs/ripscope.el\n"
        );
        assert!(installed_manifest("bb  plugin/ripscope.vim\n").is_err());
        assert!(installed_manifest("aa  bin/ripscope\nbb  ../escape\n").is_err());
        assert!(installed_manifest("aa bin/ripscope\n").is_err());
    }
}
