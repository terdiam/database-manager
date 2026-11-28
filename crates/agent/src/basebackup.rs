//! Physical backups of a whole PostgreSQL cluster with `pg_basebackup`, and their
//! incrementals. A backup is the tar stream it writes; the manifest inside it is kept by
//! the agent, because the next increment is taken against it. A restore unpacks the chain
//! and `pg_combinebackup` of the same major version turns it into one data directory.

use std::{
    fs::File,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use dbm_shared::jobs::{DatabaseSecret, valid_database_name};
use sha2::{Digest, Sha256};

use crate::{Result, engines};

const MANIFEST: &str = "backup_manifest";

/// Where the manifest of a stored backup is kept: beside the agent's state file.
pub fn manifest_path(object_key: &str) -> Result<PathBuf> {
    let state = std::env::var("DBM_STATE_FILE").unwrap_or_else(|_| "agent-state".into());
    let folder = Path::new(&state)
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
        .join("manifests");
    std::fs::create_dir_all(&folder)?;
    let name: String = Sha256::digest(object_key.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    Ok(folder.join(name))
}

/// `pg_basebackup` to standard output.
fn command(secret: &DatabaseSecret, since: Option<&Path>) -> Result<Command> {
    let mut command = engines::postgres("pg_basebackup", secret, "postgres")?;
    command.env("LC_MESSAGES", "C");
    // The write-ahead log needed to start the copy is part of the stream.
    command.args(["-D", "-", "-Ft", "-X", "fetch", "--checkpoint=fast"]);
    if let Some(manifest) = since {
        let mut option = std::ffi::OsString::from("--incremental=");
        option.push(manifest);
        command.arg(option);
    }
    Ok(command)
}

/// The size field of a tar header: octal text, or binary for members of 8 GB and more.
fn tar_size(field: &[u8]) -> Result<u64> {
    if field[0] & 0x80 != 0 {
        return Ok(field[field.len() - 8..]
            .iter()
            .fold(0, |n, &b| n << 8 | u64::from(b)));
    }
    let text = std::str::from_utf8(field)?.trim_matches(['\0', ' ']);
    Ok(u64::from_str_radix(text, 8)?)
}

/// Passes a tar stream through unchanged and writes its manifest member to `keep`.
fn copy_keeping_manifest(
    mut input: impl Read,
    mut output: impl Write,
    mut keep: impl Write,
) -> Result<()> {
    let mut header = [0; 512];
    let mut chunk = vec![0; 128 * 1024];
    let mut found = false;
    loop {
        match input.read_exact(&mut header) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(e.into()),
        }
        output.write_all(&header)?;
        // Two zero blocks end the archive.
        if header.iter().all(|&b| b == 0) {
            std::io::copy(&mut input, &mut output)?;
            break;
        }
        let name = header[..100].split(|&b| b == 0).next().unwrap_or_default();
        let wanted = name == MANIFEST.as_bytes();
        let size = tar_size(&header[124..136])?;
        // Data is padded to whole blocks.
        let mut left = size.checked_next_multiple_of(512).ok_or("bad tar member")?;
        let mut data = size;
        while left > 0 {
            let n = chunk.len().min(usize::try_from(left).unwrap_or(usize::MAX));
            input.read_exact(&mut chunk[..n])?;
            output.write_all(&chunk[..n])?;
            if wanted {
                let part = n.min(usize::try_from(data).unwrap_or(usize::MAX));
                keep.write_all(&chunk[..part])?;
                data -= part as u64;
            }
            left -= n as u64;
        }
        found |= wanted;
    }
    output.flush()?;
    keep.flush()?;
    if !found {
        return Err("the backup carries no manifest (more than one tablespace?)".into());
    }
    Ok(())
}

/// Runs inside the helper: streams the backup out and leaves its manifest in `keep`.
pub fn dump(
    secret: &DatabaseSecret,
    keep: &Path,
    since: Option<&Path>,
    output: impl Write,
) -> Result<()> {
    let mut child = command(secret, since)?
        .spawn()
        .map_err(|e| format!("cannot run pg_basebackup: {e}"))?;
    let stream = child.stdout.take().ok_or("no output")?;
    // What it says is part of the error: it tells whether the chain can be continued.
    // Read on a thread, so a full pipe cannot stall the backup.
    let mut messages = child.stderr.take().ok_or("no messages")?;
    let said = std::thread::spawn(move || {
        let mut text = Vec::new();
        let mut chunk = [0; 4096];
        while let Ok(n @ 1..) = messages.read(&mut chunk) {
            text.extend_from_slice(&chunk[..n]);
            if text.len() > 8192 {
                text.drain(..text.len() - 4096);
            }
        }
        String::from_utf8_lossy(&text).trim().to_owned()
    });
    let kept = std::io::BufWriter::new(File::create(keep)?);
    let copied = copy_keeping_manifest(stream, output, kept);
    let status = child.wait()?;
    let said = said.join().unwrap_or_default();
    if !status.success() {
        let one_line = said.lines().collect::<Vec<_>>().join(" ");
        return Err(format!("pg_basebackup failed ({status}): {one_line}").into());
    }
    if !said.is_empty() {
        eprintln!("{said}");
    }
    copied
}

/// Messages of a server that can no longer continue from the parent backup.
pub fn cannot_continue(message: &str) -> bool {
    [
        "system identifier",
        "WAL summaries are required",
        "timeline",
        "manifest requires WAL",
    ]
    .iter()
    .any(|part| message.contains(part))
}

/// Unpacks one backup of a chain into `folder`.
pub fn unpack(folder: &Path) -> Result<engines::Tool> {
    std::fs::create_dir_all(folder)?;
    let mut command = engines::quiet("tar");
    command
        .args(["--extract", "--no-same-owner", "--file=-"])
        .arg("--directory")
        .arg(folder)
        .stdin(Stdio::piped())
        .stdout(Stdio::null());
    Ok(engines::Tool::holding(command, "tar", None))
}

/// The folder restores are written to. Without it they are off, so a server cannot make
/// the agent write anywhere else.
pub fn root() -> Result<PathBuf> {
    let root = std::env::var("DBM_PGDATA_ROOT")
        .ok()
        .filter(|v| !v.is_empty())
        .ok_or("physical restores are off on this agent: set DBM_PGDATA_ROOT")?;
    Ok(std::fs::canonicalize(root)?)
}

/// `<root>/<name>`, which must not exist yet or be an empty folder.
pub fn target(root: &Path, name: &str) -> Result<PathBuf> {
    // The name carries no separators or dots, so it cannot leave the root.
    if !valid_database_name(name) {
        return Err("invalid folder name".into());
    }
    let path = root.join(name);
    match path.symlink_metadata() {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(path),
        Ok(found) if found.is_dir() && std::fs::read_dir(&path)?.next().is_none() => Ok(path),
        _ => Err(format!("{name} already exists and is not an empty folder").into()),
    }
}

/// Turns the unpacked chain in `parts` (full first) into the data directory `target`.
pub fn combine(parts: &[PathBuf], work: &Path, target: &Path) -> Result<()> {
    let full = parts.first().ok_or("nothing to restore")?;
    let ready = if parts.len() == 1 {
        full.clone()
    } else {
        let version = std::fs::read_to_string(full.join("PG_VERSION"))?;
        let version: u32 = version.trim().parse()?;
        let program = format!("/usr/lib/postgresql/{version}/bin/pg_combinebackup");
        let out = work.join("combined");
        let mut command = engines::quiet(&program);
        command.args(parts).arg("--output").arg(&out);
        let done = command
            .output()
            .map_err(|e| format!("cannot combine PostgreSQL {version} backups: {e}"))?;
        if !done.status.success() {
            let said = String::from_utf8_lossy(&done.stderr);
            return Err(format!("pg_combinebackup failed: {}", said.trim()).into());
        }
        out
    };
    // PostgreSQL refuses to start on a data directory others can read.
    std::fs::set_permissions(&ready, std::os::unix::fs::PermissionsExt::from_mode(0o700))?;
    // An empty target is replaced in one step.
    std::fs::rename(&ready, target)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member(name: &str, data: &[u8]) -> Vec<u8> {
        let mut block = vec![0; 512];
        block[..name.len()].copy_from_slice(name.as_bytes());
        block[124..135].copy_from_slice(format!("{:011o}", data.len()).as_bytes());
        block.extend_from_slice(data);
        block.resize(512 + data.len().next_multiple_of(512), 0);
        block
    }

    #[test]
    fn the_manifest_is_kept_and_the_stream_is_unchanged() {
        let big = vec![7; 300_000];
        let mut tar = member("base/1", &big);
        tar.extend(member(MANIFEST, b"{\"manifest\": 1}"));
        tar.extend(vec![0; 1024]);
        let (mut out, mut kept) = (Vec::new(), Vec::new());
        copy_keeping_manifest(tar.as_slice(), &mut out, &mut kept).unwrap();
        assert_eq!(out, tar);
        assert_eq!(kept, b"{\"manifest\": 1}");
        let without = [member("x", b"y"), vec![0; 1024]].concat();
        assert!(copy_keeping_manifest(without.as_slice(), Vec::new(), Vec::new()).is_err());
        assert_eq!(
            tar_size(&[0x80, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0]).unwrap(),
            2 << 32
        );
    }

    #[test]
    fn restores_only_go_into_new_or_empty_folders() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("empty")).unwrap();
        std::fs::create_dir(root.path().join("used")).unwrap();
        std::fs::write(root.path().join("used/PG_VERSION"), "19").unwrap();
        std::os::unix::fs::symlink("/tmp", root.path().join("link")).unwrap();
        assert!(target(root.path(), "new").is_ok());
        assert!(target(root.path(), "empty").is_ok());
        for bad in ["used", "link", "..", "a/b", ""] {
            assert!(target(root.path(), bad).is_err(), "{bad}");
        }
    }
}
