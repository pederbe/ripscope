//! Where an index may live (ADR 0005): SQLite's write-ahead log needs a
//! local file system whose locks and shared memory work, so network, cluster and
//! shared-folder file systems and WSL's view of Windows drives are refused. A folder
//! OneDrive synchronises is local, but OneDrive may lock the index or copy it while it is
//! written, so an index build that makes the index there warns about it.

#[cfg(any(windows, test))]
use std::ffi::OsString;
use std::path::Path;
#[cfg(target_os = "linux")]
use std::path::PathBuf;

/// Why an index in `directory` would be unsafe, if it would.
#[cfg(target_os = "linux")]
pub(super) fn unsafe_place(directory: &Path) -> std::io::Result<Option<&'static str>> {
    let info = rustix::fs::statfs(directory)?;
    // The magic number is a machine word; its low 32 bits identify the file system.
    let kind = info.f_type as u32;
    if kind == magic::FUSE {
        // FUSE serves both disks (ntfs-3g, as `fuseblk`) and services (sshfs, cloud-sync
        // clients); only the mount table tells them apart.
        let canonical = std::fs::canonicalize(directory)?;
        let table = std::fs::read_to_string("/proc/self/mountinfo").unwrap_or_default();
        return Ok((mount_type(&table, &canonical) != Some("fuseblk"))
            .then_some("on a FUSE mount, such as sshfs or a cloud-sync client"));
    }
    Ok(remote_file_system(kind))
}

/// `statfs` magic numbers, as Linux's `include/uapi/linux/magic.h` and the file systems'
/// own sources define them.
#[cfg(target_os = "linux")]
mod magic {
    pub(super) const NFS: u32 = 0x6969;
    pub(super) const SMB: u32 = 0x517b;
    pub(super) const CIFS: u32 = 0xff53_4d42;
    pub(super) const SMB2: u32 = 0xfe53_4d42;
    pub(super) const V9FS: u32 = 0x0102_1997;
    pub(super) const VBOXSF: u32 = 0x786f_4256;
    pub(super) const PRL_FS: u32 = 0x7c7c_6673;
    pub(super) const CEPH: u32 = 0x00c3_6400;
    pub(super) const AFS: u32 = 0x5346_414f;
    pub(super) const AFS_FS: u32 = 0x6b41_4653;
    pub(super) const CODA: u32 = 0x7375_7245;
    pub(super) const LUSTRE: u32 = 0x0bd0_0bd0;
    pub(super) const GFS2: u32 = 0x0116_1970;
    pub(super) const OCFS2: u32 = 0x7461_636f;
    pub(super) const ORANGEFS: u32 = 0x2003_0528;
    pub(super) const BEEGFS: u32 = 0x1983_0326;
    pub(super) const GPFS: u32 = 0x4750_4653;
    pub(super) const FUSE: u32 = 0x6573_5546;
}

/// File systems where the write-ahead log is unsafe: network, cluster and shared-folder
/// file systems, and 9p (how WSL shows Windows drives).
#[cfg(target_os = "linux")]
fn remote_file_system(kind: u32) -> Option<&'static str> {
    use magic::*;
    Some(match kind {
        NFS => "on NFS",
        SMB | CIFS | SMB2 => "on an SMB share",
        V9FS => "on a 9p mount, such as WSL's view of a Windows drive",
        VBOXSF | PRL_FS => "in a virtual machine's shared folder",
        CEPH => "on Ceph",
        AFS | AFS_FS => "on AFS",
        CODA => "on Coda",
        LUSTRE => "on Lustre",
        GFS2 | OCFS2 => "on a cluster file system",
        ORANGEFS => "on OrangeFS",
        BEEGFS => "on BeeGFS",
        GPFS => "on GPFS",
        _ => return None,
    })
}

/// The type of the mount holding `path`, from a `/proc/self/mountinfo` table: the
/// longest mount point containing it, the later one where a mount covers another.
#[cfg(target_os = "linux")]
fn mount_type<'t>(table: &'t str, path: &Path) -> Option<&'t str> {
    table
        .lines()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split(' ').collect();
            let point = PathBuf::from(unescape_mount_field(fields.get(4)?));
            let separator = fields.iter().position(|field| *field == "-")?;
            let kind = *fields.get(separator + 1)?;
            path.starts_with(&point)
                .then(|| (point.as_os_str().len(), kind))
        })
        .max_by_key(|(length, _)| *length)
        .map(|(_, kind)| kind)
}

/// A mount table field with its octal escapes (`\040` for a space) decoded.
#[cfg(target_os = "linux")]
fn unescape_mount_field(field: &str) -> String {
    let bytes = field.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        match bytes.get(at + 1..at + 4) {
            Some(digits)
                if bytes[at] == b'\\' && digits.iter().all(|d| (b'0'..=b'7').contains(d)) =>
            {
                let value = digits
                    .iter()
                    .fold(0u32, |value, digit| value * 8 + u32::from(digit - b'0'));
                decoded.push(u8::try_from(value).unwrap_or(b'?'));
                at += 4;
            }
            _ => {
                decoded.push(bytes[at]);
                at += 1;
            }
        }
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

#[cfg(windows)]
pub(super) fn unsafe_place(directory: &Path) -> std::io::Result<Option<&'static str>> {
    // A mapped network drive resolves to its share.
    let canonical = std::fs::canonicalize(directory)?;
    Ok((!local_drive(&canonical))
        .then_some("not on a local drive (a network share, or WSL's files seen from Windows)"))
}

/// A path on a drive letter rather than a share.
#[cfg(windows)]
fn local_drive(path: &Path) -> bool {
    use std::path::{Component, Prefix};
    matches!(
        path.components().next(),
        Some(Component::Prefix(prefix))
            if matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_))
    )
}

#[cfg(not(any(target_os = "linux", windows)))]
pub(super) fn unsafe_place(_directory: &Path) -> std::io::Result<Option<&'static str>> {
    Ok(None)
}

/// Whether `directory` is in a folder OneDrive synchronises. The cloud-files driver shows
/// such a folder to this process as a plain directory, so OneDrive's folders are known by
/// the variables that name them.
#[cfg(windows)]
pub(super) fn in_onedrive(directory: &Path) -> bool {
    let roots = ["OneDrive", "OneDriveCommercial", "OneDriveConsumer"]
        .into_iter()
        .filter_map(std::env::var_os);
    inside_any(directory, roots)
}

#[cfg(not(windows))]
pub(super) fn in_onedrive(_directory: &Path) -> bool {
    false
}

/// Whether `directory` is one of `roots` or inside one, as their canonical paths say. A
/// root that does not exist holds nothing.
#[cfg(any(windows, test))]
fn inside_any(directory: &Path, roots: impl IntoIterator<Item = OsString>) -> bool {
    let Ok(directory) = std::fs::canonicalize(directory) else {
        return false;
    };
    roots
        .into_iter()
        .filter_map(|root| std::fs::canonicalize(root).ok())
        .any(|root| directory.starts_with(root))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_local_directory_takes_an_index() {
        let directory = tempfile::tempdir().expect("directory");
        assert_eq!(unsafe_place(directory.path()).expect("checked"), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn network_and_cluster_file_systems_are_unsafe_for_an_index() {
        // Values from Linux's magic.h and the file systems' sources: NFS, SMB, SMB2, 9p,
        // VirtualBox shared folders, Ceph, OrangeFS, BeeGFS.
        for kind in [
            0x6969,
            0xff53_4d42,
            0xfe53_4d42,
            0x0102_1997,
            0x786f_4256,
            0x00c3_6400,
            0x2003_0528,
            0x1983_0326,
        ] {
            assert!(remote_file_system(kind).is_some(), "{kind:#x}");
        }
        // ext4, Btrfs, XFS, tmpfs, ZFS, F2FS.
        for kind in [
            0xef53,
            0x9123_683e,
            0x5846_5342,
            0x0102_1994,
            0x2fc1_2fc1,
            0xf2f5_2010,
        ] {
            assert_eq!(remote_file_system(kind), None, "{kind:#x}");
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_fuse_mount_is_local_only_when_a_block_device_backs_it() {
        let table = "\
22 1 8:2 / / rw,relatime shared:1 - ext4 /dev/sda2 rw
40 22 0:36 / /media/data rw,relatime shared:20 - fuseblk /dev/sdb1 rw,user_id=0
41 22 0:37 / /home/me/remote\\040box rw,nosuid shared:21 - fuse.sshfs me@host:/srv rw
42 41 0:38 / /home/me/remote\\040box/cache rw shared:22 - tmpfs tmpfs rw
";
        let kind = |path: &str| mount_type(table, Path::new(path));
        assert_eq!(kind("/media/data/src"), Some("fuseblk"));
        assert_eq!(kind("/home/me/remote box/src"), Some("fuse.sshfs"));
        assert_eq!(kind("/home/me/remote box/cache/x"), Some("tmpfs"));
        assert_eq!(kind("/home/me/remote"), Some("ext4"));
        assert_eq!(kind("/media/database"), Some("ext4"));
    }

    #[cfg(windows)]
    #[test]
    fn a_share_is_unsafe_for_an_index() {
        assert!(!local_drive(Path::new(r"\\?\UNC\server\share\project")));
        assert!(!local_drive(Path::new(r"\\wsl.localhost\Ubuntu\home")));
        assert!(local_drive(Path::new(r"\\?\C:\Users\project")));
        assert!(local_drive(Path::new(r"D:\project")));
    }

    #[test]
    fn a_folder_inside_a_synchronised_root_is_synchronised() {
        let home = tempfile::tempdir().expect("home");
        let root = home.path().join("OneDrive - Example");
        let project = root.join("src");
        let beside = home.path().join("OneDrive");
        for directory in [&project, &beside] {
            std::fs::create_dir_all(directory).expect("directory");
        }
        let roots = || {
            [
                root.clone().into_os_string(),
                home.path().join("none").into(),
            ]
        };
        assert!(inside_any(&project, roots()));
        assert!(inside_any(&root, roots()));
        // Through `..`, which only the canonical path resolves.
        assert!(inside_any(
            &beside.join("../OneDrive - Example/src"),
            roots()
        ));
        assert!(!inside_any(&beside, roots()));
        assert!(!inside_any(home.path(), roots()));
        // A root that does not exist holds nothing, and neither does a missing folder.
        assert!(!inside_any(
            &project,
            [home.path().join("none").into_os_string()]
        ));
        assert!(!inside_any(&root.join("missing"), roots()));
    }
}
