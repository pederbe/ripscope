//! Release archives with fixed ownership, times and permissions, so the same inputs give
//! the same bytes: a POSIX tar in gzip for Linux and a deflated ZIP for Windows. The ZIP
//! reader checks every entry before the Windows smoke extracts it.

use crate::common::{Result, archive_order, relative_name, walk};
use flate2::GzBuilder;
use flate2::write::DeflateEncoder;
use flate2::{Compression, Crc};
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};

const BLOCK: usize = 512;
const RECORD: usize = 20 * BLOCK;

/// Writes `package`, a directory, as a gzip-compressed tar named by its parent-relative
/// path. Entries are root-owned with time `epoch`; directories and `bin/ripscope` are
/// executable and everything else is read-only for others.
pub fn tar_gz(package: &Path, destination: &Path, epoch: u64) -> Result<()> {
    let raw = File::create_new(destination)?;
    let mut zipped = GzBuilder::new()
        .mtime(u32::try_from(epoch)?)
        .operating_system(255)
        .write(raw, Compression::best());
    let base = package.parent().ok_or("package has no parent")?;
    let mut entries = vec![package.to_owned()];
    entries.extend(archive_order(walk(package)?, package));
    let mut written = 0;
    for path in entries {
        let name = relative_name(&path, base)?;
        let meta = fs::metadata(&path)?;
        let executable = meta.is_dir() || path == package.join("bin/ripscope");
        let mode = if executable { 0o755 } else { 0o644 };
        let (name, kind, size) = if meta.is_dir() {
            (format!("{name}/"), b'5', 0)
        } else {
            (name, b'0', meta.len())
        };
        let header = tar_header(&name, mode, size, epoch, kind);
        zipped.write_all(&header)?;
        written += header.len();
        if kind == b'0' {
            let mut file = File::open(&path)?;
            let copied = io::copy(&mut file, &mut zipped)?;
            if copied != size {
                return Err(format!("{} changed while it was archived", path.display()).into());
            }
            let padding = (BLOCK - (size as usize % BLOCK)) % BLOCK;
            zipped.write_all(&vec![0; padding])?;
            written += size as usize + padding;
        }
    }
    zipped.write_all(&[0; 2 * BLOCK])?;
    written += 2 * BLOCK;
    let remainder = written % RECORD;
    if remainder > 0 {
        zipped.write_all(&vec![0; RECORD - remainder])?;
    }
    zipped.finish()?.sync_all()?;
    Ok(())
}

/// One member's header, preceded by a POSIX extended header when the name does not fit
/// the classic 100-byte field or is not ASCII.
fn tar_header(name: &str, mode: u32, size: u64, mtime: u64, kind: u8) -> Vec<u8> {
    let mut out = Vec::new();
    if name.len() > 100 || !name.is_ascii() {
        let record = pax_record("path", name);
        out.extend(ustar_block(
            "././@PaxHeader",
            0,
            record.len() as u64,
            0,
            b'x',
        ));
        out.extend_from_slice(&record);
        out.resize(out.len().div_ceil(BLOCK) * BLOCK, 0);
    }
    out.extend(ustar_block(name, mode, size, mtime, kind));
    out
}

/// `"<length> <keyword>=<value>\n"`, whose length counts itself.
fn pax_record(keyword: &str, value: &str) -> Vec<u8> {
    let body = keyword.len() + value.len() + 3;
    let mut length = 0;
    loop {
        let total = body + length.to_string().len();
        if total == length {
            break;
        }
        length = total;
    }
    format!("{length} {keyword}={value}\n").into_bytes()
}

fn ustar_block(name: &str, mode: u32, size: u64, mtime: u64, kind: u8) -> [u8; BLOCK] {
    let mut block = [0_u8; BLOCK];
    let field = |block: &mut [u8; BLOCK], at: usize, width: usize, bytes: &[u8]| {
        let count = bytes.len().min(width);
        block[at..at + count].copy_from_slice(&bytes[..count]);
    };
    let octal =
        |value: u64, width: usize| format!("{value:0>digits$o}\0", digits = width - 1).into_bytes();
    let ascii: Vec<u8> = name
        .chars()
        .map(|ch| if ch.is_ascii() { ch as u8 } else { b'?' })
        .collect();
    field(&mut block, 0, 100, &ascii);
    field(&mut block, 100, 8, &octal(u64::from(mode & 0o7777), 8));
    field(&mut block, 108, 8, &octal(0, 8));
    field(&mut block, 116, 8, &octal(0, 8));
    field(&mut block, 124, 12, &octal(size, 12));
    field(&mut block, 136, 12, &octal(mtime, 12));
    field(&mut block, 148, 8, b"        ");
    block[156] = kind;
    field(&mut block, 257, 8, b"ustar\x0000");
    let checksum: u32 = block.iter().map(|byte| u32::from(*byte)).sum();
    field(&mut block, 148, 7, format!("{checksum:06o}\0").as_bytes());
    block
}

/// The MS-DOS date and time of a Unix time, clamped to 1980 as ZIP requires.
fn dos_time(epoch: u64) -> (u16, u16) {
    let seconds = epoch.max(315_532_800);
    let days = (seconds / 86_400) as i64;
    let rest = seconds % 86_400;
    // Days since 1970-01-01 to a civil date (proleptic Gregorian).
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    let date = ((year - 1980) << 9) | (month << 5) | day;
    let time = ((rest / 3600) << 11) | (((rest % 3600) / 60) << 5) | ((rest % 60) / 2);
    (date as u16, time as u16)
}

/// Writes every file under `package` to a new deflated ZIP, named by its parent-relative
/// path with forward slashes, as regular files dated `epoch` with mode 0644.
pub fn zip(package: &Path, destination: &Path, epoch: u64) -> Result<()> {
    let base = package.parent().ok_or("package has no parent")?;
    let (date, time) = dos_time(epoch);
    let mut output = io::BufWriter::new(File::create_new(destination)?);
    let mut central = Vec::new();
    let mut offset: u64 = 0;
    let mut count: u16 = 0;
    for path in archive_order(walk(package)?, package) {
        if !fs::metadata(&path)?.is_file() {
            continue;
        }
        let name = relative_name(&path, base)?;
        let data = fs::read(&path)?;
        let mut crc = Crc::new();
        crc.update(&data);
        let mut encoder = DeflateEncoder::new(Vec::new(), Compression::new(6));
        encoder.write_all(&data)?;
        let compressed = encoder.finish()?;
        let flags: u16 = if name.is_ascii() { 0 } else { 0x0800 };
        let sizes = [
            u32::try_from(compressed.len()).map_err(|_| "archive member too large")?,
            u32::try_from(data.len()).map_err(|_| "archive member too large")?,
        ];
        let mut local = Vec::new();
        local.extend_from_slice(b"PK\x03\x04");
        local.extend_from_slice(&[20, 0]);
        local.extend_from_slice(&flags.to_le_bytes());
        local.extend_from_slice(&8_u16.to_le_bytes());
        local.extend_from_slice(&time.to_le_bytes());
        local.extend_from_slice(&date.to_le_bytes());
        local.extend_from_slice(&crc.sum().to_le_bytes());
        local.extend_from_slice(&sizes[0].to_le_bytes());
        local.extend_from_slice(&sizes[1].to_le_bytes());
        local.extend_from_slice(&u16::try_from(name.len())?.to_le_bytes());
        local.extend_from_slice(&0_u16.to_le_bytes());
        local.extend_from_slice(name.as_bytes());
        output.write_all(&local)?;
        output.write_all(&compressed)?;
        central.extend_from_slice(b"PK\x01\x02");
        // Made by version 2.0 on Unix, so the mode in the external attributes applies.
        central.extend_from_slice(&[20, 3, 20, 0]);
        central.extend_from_slice(&flags.to_le_bytes());
        central.extend_from_slice(&8_u16.to_le_bytes());
        central.extend_from_slice(&time.to_le_bytes());
        central.extend_from_slice(&date.to_le_bytes());
        central.extend_from_slice(&crc.sum().to_le_bytes());
        central.extend_from_slice(&sizes[0].to_le_bytes());
        central.extend_from_slice(&sizes[1].to_le_bytes());
        central.extend_from_slice(&u16::try_from(name.len())?.to_le_bytes());
        central.extend_from_slice(&[0; 8]);
        central.extend_from_slice(&(0o100644_u32 << 16).to_le_bytes());
        central.extend_from_slice(
            &u32::try_from(offset)
                .map_err(|_| "archive too large")?
                .to_le_bytes(),
        );
        central.extend_from_slice(name.as_bytes());
        offset += (local.len() + compressed.len()) as u64;
        count = count.checked_add(1).ok_or("too many archive members")?;
    }
    output.write_all(&central)?;
    output.write_all(b"PK\x05\x06")?;
    output.write_all(&[0; 4])?;
    output.write_all(&count.to_le_bytes())?;
    output.write_all(&count.to_le_bytes())?;
    output.write_all(&u32::try_from(central.len())?.to_le_bytes())?;
    output.write_all(
        &u32::try_from(offset)
            .map_err(|_| "archive too large")?
            .to_le_bytes(),
    )?;
    output.write_all(&[0; 2])?;
    output
        .into_inner()
        .map_err(|error| error.into_error())?
        .sync_all()?;
    Ok(())
}

/// One ZIP member, as its central directory describes it.
pub struct ZipEntry {
    pub name: String,
    pub external: u32,
    method: u16,
    crc: u32,
    compressed: u64,
    size: u64,
    offset: u64,
}

fn u16_at(data: &[u8], at: usize) -> Result<u16> {
    Ok(u16::from_le_bytes(
        data.get(at..at + 2).ok_or("truncated ZIP")?.try_into()?,
    ))
}

fn u32_at(data: &[u8], at: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(
        data.get(at..at + 4).ok_or("truncated ZIP")?.try_into()?,
    ))
}

/// The members a ZIP's central directory lists.
pub fn zip_entries(data: &[u8]) -> Result<Vec<ZipEntry>> {
    let search = data.len().saturating_sub(22 + 65_535);
    let end = (search..data.len().saturating_sub(21))
        .rev()
        .find(|at| data[*at..].starts_with(b"PK\x05\x06"))
        .ok_or("not a ZIP archive")?;
    let total = usize::from(u16_at(data, end + 10)?);
    let mut at = u32_at(data, end + 16)? as usize;
    let mut entries = Vec::with_capacity(total);
    for _ in 0..total {
        if data.get(at..at + 4) != Some(b"PK\x01\x02") {
            return Err("damaged ZIP central directory".into());
        }
        let flags = u16_at(data, at + 8)?;
        let name_length = usize::from(u16_at(data, at + 28)?);
        let extra_length = usize::from(u16_at(data, at + 30)?);
        let comment_length = usize::from(u16_at(data, at + 32)?);
        let raw_name = data
            .get(at + 46..at + 46 + name_length)
            .ok_or("truncated ZIP")?;
        if flags & 1 != 0 {
            return Err("encrypted ZIP members are not supported".into());
        }
        if flags & 0x0800 == 0 && !raw_name.is_ascii() {
            return Err("ZIP member names must be ASCII or marked UTF-8".into());
        }
        let entry = ZipEntry {
            name: String::from_utf8(raw_name.to_vec())?,
            external: u32_at(data, at + 38)?,
            method: u16_at(data, at + 10)?,
            crc: u32_at(data, at + 16)?,
            compressed: u64::from(u32_at(data, at + 20)?),
            size: u64::from(u32_at(data, at + 24)?),
            offset: u64::from(u32_at(data, at + 42)?),
        };
        if [entry.compressed, entry.size, entry.offset].contains(&0xffff_ffff) {
            return Err("ZIP64 members are not supported".into());
        }
        entries.push(entry);
        at += 46 + name_length + extra_length + comment_length;
    }
    Ok(entries)
}

/// The member's path under `root`, unless its name leaves `root`.
pub fn entry_path(root: &Path, name: &str) -> Option<PathBuf> {
    let mut parts: Vec<&str> = Vec::new();
    for part in name.split(['/', '\\']) {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            part => {
                if Path::new(part)
                    .components()
                    .any(|c| !matches!(c, Component::Normal(_)))
                    || part.contains(':')
                {
                    return None;
                }
                parts.push(part);
            }
        }
    }
    if name.starts_with(['/', '\\']) || parts.is_empty() {
        return None;
    }
    Some(
        parts
            .iter()
            .fold(root.to_owned(), |path, part| path.join(part)),
    )
}

/// A member's checked contents.
pub fn zip_read(data: &[u8], entry: &ZipEntry) -> Result<Vec<u8>> {
    let at = entry.offset as usize;
    if data.get(at..at + 4) != Some(b"PK\x03\x04") {
        return Err(format!("damaged ZIP member {}", entry.name).into());
    }
    let start = at + 30 + usize::from(u16_at(data, at + 26)?) + usize::from(u16_at(data, at + 28)?);
    let stored = data
        .get(start..start + entry.compressed as usize)
        .ok_or("truncated ZIP member")?;
    let bytes = match entry.method {
        0 => stored.to_vec(),
        8 => {
            let mut bytes = Vec::new();
            flate2::read::DeflateDecoder::new(stored).read_to_end(&mut bytes)?;
            bytes
        }
        method => return Err(format!("unsupported ZIP method {method}").into()),
    };
    let mut crc = Crc::new();
    crc.update(&bytes);
    if bytes.len() as u64 != entry.size || crc.sum() != entry.crc {
        return Err(format!("ZIP member {} fails its checksum", entry.name).into());
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn tar_headers_take_long_names_through_an_extended_header() {
        let short = tar_header("pkg/bin/ripscope", 0o755, 15, 123_456, b'0');
        assert_eq!(short.len(), BLOCK);
        assert_eq!(&short[100..108], b"0000755\0");
        assert_eq!(&short[124..136], b"00000000017\0");
        assert_eq!(&short[257..265], b"ustar\x0000");
        let sum: u32 = short
            .iter()
            .enumerate()
            .map(|(at, byte)| {
                if (148..156).contains(&at) {
                    32
                } else {
                    u32::from(*byte)
                }
            })
            .sum();
        assert_eq!(&short[148..156], format!("{sum:06o}\0 ").as_bytes());
        let name = format!("pkg/{}", "x".repeat(120));
        let long = tar_header(&name, 0o644, 0, 0, b'0');
        assert_eq!(long.len(), 3 * BLOCK);
        assert_eq!(long[156], b'x');
        let record = format!("134 path={name}\n");
        assert_eq!(&long[BLOCK..BLOCK + record.len()], record.as_bytes());
        assert_eq!(pax_record("path", &name).len(), 134);
    }

    #[test]
    fn dos_times_follow_the_civil_calendar() {
        // 2023-11-14 22:13:20 UTC.
        assert_eq!(
            dos_time(1_700_000_000),
            ((43 << 9) | (11 << 5) | 14, (22 << 11) | (13 << 5) | 10)
        );
        assert_eq!(dos_time(0), ((1 << 5) | 1, 0));
    }

    #[test]
    fn zips_read_back_and_refuse_escaping_names() {
        let root = tempfile::tempdir().unwrap();
        let package = root.path().join("candidate");
        fs::create_dir_all(package.join("bin")).unwrap();
        fs::write(package.join("bin/ripscope.exe"), b"candidate").unwrap();
        fs::write(package.join("README.md"), b"read me").unwrap();
        zip(&package, &root.path().join("one.zip"), 1_700_000_000).unwrap();
        let data = fs::read(root.path().join("one.zip")).unwrap();
        let entries = zip_entries(&data).unwrap();
        let names: Vec<&str> = entries.iter().map(|entry| entry.name.as_str()).collect();
        let expected = if cfg!(windows) {
            ["candidate/bin/ripscope.exe", "candidate/README.md"]
        } else {
            ["candidate/README.md", "candidate/bin/ripscope.exe"]
        };
        assert_eq!(names, expected);
        let binary = entries
            .iter()
            .find(|entry| entry.name == "candidate/bin/ripscope.exe")
            .unwrap();
        assert_eq!(binary.external >> 16, 0o100644);
        assert_eq!(zip_read(&data, binary).unwrap(), b"candidate");
        let install = Path::new("install");
        assert_eq!(entry_path(install, "a/./b"), Some(install.join("a/b")));
        assert_eq!(entry_path(install, "a/../b"), Some(install.join("b")));
        assert_eq!(entry_path(install, "../b"), None);
        assert_eq!(entry_path(install, "/b"), None);
        assert_eq!(entry_path(install, "c:/b"), None);
        let mut gz = Vec::new();
        tar_gz(&package, &root.path().join("one.tar.gz"), 5).unwrap();
        flate2::read::GzDecoder::new(File::open(root.path().join("one.tar.gz")).unwrap())
            .read_to_end(&mut gz)
            .unwrap();
        assert_eq!(gz.len() % RECORD, 0);
        assert_eq!(&gz[..10], b"candidate/");
    }
}
