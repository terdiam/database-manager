//! dump → compress → encrypt → upload, and back. Only the restore uses a temporary
//! file, so the whole backup is verified before anything touches a database.

use std::{
    io::{self, BufReader, Read, Write},
    process::{Child, Command},
    str::FromStr,
    thread::JoinHandle,
};

use age::x25519;
use tokio::runtime::Handle;

use crate::{Result, progress::Progress, storage::Upload};

/// Kills the child if the pipeline stops early, and collects what it says on stderr.
struct Tool {
    child: Child,
    stderr: Option<JoinHandle<String>>,
    name: &'static str,
}

impl Tool {
    fn start(mut command: Command, name: &'static str) -> Result<Self> {
        let mut child = command
            .spawn()
            .map_err(|e| format!("cannot run {name}: {e}"))?;
        // Drained on its own thread: a full pipe would otherwise stall the tool.
        let stderr = child.stderr.take().map(|mut pipe| {
            std::thread::spawn(move || {
                let mut tail = Vec::new();
                let mut chunk = [0; 4096];
                while let Ok(n @ 1..) = pipe.read(&mut chunk) {
                    tail.extend_from_slice(&chunk[..n]);
                    if tail.len() > 8192 {
                        tail.drain(..tail.len() - 4096);
                    }
                }
                String::from_utf8_lossy(&tail).trim().to_owned()
            })
        });
        Ok(Self {
            child,
            stderr,
            name,
        })
    }

    fn wait(&mut self, progress: &Progress) -> Result<()> {
        let status = self.child.wait()?;
        let said = self
            .stderr
            .take()
            .and_then(|t| t.join().ok())
            .unwrap_or_default();
        for line in said
            .lines()
            .rev()
            .take(20)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
        {
            progress.log(if status.success() { "warn" } else { "error" }, line);
        }
        if !status.success() {
            let last = said.lines().last().unwrap_or("no message");
            return Err(format!("{} failed ({status}): {last}", self.name).into());
        }
        Ok(())
    }
}

impl Drop for Tool {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn copy(from: &mut impl Read, to: &mut impl Write, progress: &Progress) -> Result<()> {
    let mut buffer = vec![0; 128 * 1024];
    loop {
        progress.check()?;
        match from.read(&mut buffer) {
            Ok(0) => return Ok(()),
            Ok(n) => to.write_all(&buffer[..n])?,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e.into()),
        }
    }
}

fn compress<W: Write>(
    kind: &str,
    level: i32,
    from: &mut impl Read,
    to: W,
    progress: &Progress,
) -> Result<W> {
    let level_u32 = level.clamp(1, 9) as u32;
    Ok(match kind {
        "zstd" => {
            let mut encoder = zstd::Encoder::new(to, level)?;
            copy(from, &mut encoder, progress)?;
            encoder.finish()?
        }
        "gzip" => {
            let mut encoder =
                flate2::write::GzEncoder::new(to, flate2::Compression::new(level_u32));
            copy(from, &mut encoder, progress)?;
            encoder.finish()?
        }
        "lz4" => {
            let mut encoder = lz4_flex::frame::FrameEncoder::new(to);
            copy(from, &mut encoder, progress)?;
            encoder.finish()?
        }
        "xz" => {
            let mut encoder = liblzma::write::XzEncoder::new(to, level_u32);
            copy(from, &mut encoder, progress)?;
            encoder.finish()?
        }
        "bzip2" => {
            let mut encoder = bzip2::write::BzEncoder::new(to, bzip2::Compression::new(level_u32));
            copy(from, &mut encoder, progress)?;
            encoder.finish()?
        }
        "none" => {
            let mut to = to;
            copy(from, &mut to, progress)?;
            to
        }
        other => return Err(format!("unknown compression {other}").into()),
    })
}

fn decompress<'a>(kind: &str, from: impl Read + 'a) -> Result<Box<dyn Read + 'a>> {
    Ok(match kind {
        "zstd" => Box::new(zstd::Decoder::new(from)?),
        "gzip" => Box::new(flate2::read::GzDecoder::new(from)),
        "lz4" => Box::new(lz4_flex::frame::FrameDecoder::new(from)),
        "xz" => Box::new(liblzma::read::XzDecoder::new(from)),
        "bzip2" => Box::new(bzip2::read::BzDecoder::new(from)),
        "none" => Box::new(from),
        other => return Err(format!("unknown compression {other}").into()),
    })
}

/// Blocking. Returns the stored file's SHA-256; on any failure nothing is committed and
/// the caller aborts the upload.
pub fn backup(
    dump: Command,
    compression: &str,
    level: i32,
    recipients: &[String],
    writer: &mut opendal::Writer,
    handle: Handle,
    progress: &Progress,
) -> Result<String> {
    let recipients = recipients
        .iter()
        .map(|r| x25519::Recipient::from_str(r))
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let encryptor =
        age::Encryptor::with_recipients(recipients.iter().map(|r| r as &dyn age::Recipient))?;
    let upload = Upload::new(writer, handle, progress.clone());

    let mut tool = Tool::start(dump, "pg_dump")?;
    let mut stdout = tool.child.stdout.take().ok_or("pg_dump has no output")?;
    let encrypted = encryptor.wrap_output(upload)?;
    let encrypted = compress(compression, level, &mut stdout, encrypted, progress)?;
    // End of output is not success: a dump that died halfway must not be committed.
    tool.wait(progress)?;
    encrypted.finish()?.finish()
}

/// Blocking. `file` has already been checked against the recorded SHA-256.
pub fn restore(
    restore: Command,
    compression: &str,
    identity: &str,
    file: std::fs::File,
    progress: &Progress,
) -> Result<()> {
    let identity = x25519::Identity::from_str(identity.trim())?;
    let decrypted = age::Decryptor::new_buffered(BufReader::new(file))?
        .decrypt(std::iter::once(&identity as &dyn age::Identity))?;
    let mut input = decompress(compression, decrypted)?;

    let mut tool = Tool::start(restore, "pg_restore")?;
    let mut stdin = tool.child.stdin.take().ok_or("pg_restore has no input")?;
    let fed = copy(&mut input, &mut stdin, progress);
    drop(stdin);
    // pg_restore's own message explains a broken pipe better than the pipe does.
    if progress.check().is_ok() {
        tool.wait(progress)?;
    }
    fed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_compression_round_trips() {
        let original: Vec<u8> = (0..200_000u32)
            .flat_map(|n| (n % 251).to_le_bytes())
            .collect();
        for kind in dbm_shared::jobs::COMPRESSIONS {
            let packed = compress(
                kind,
                3,
                &mut original.as_slice(),
                Vec::new(),
                &Progress::default(),
            )
            .unwrap();
            let mut back = Vec::new();
            decompress(kind, packed.as_slice())
                .unwrap()
                .read_to_end(&mut back)
                .unwrap();
            assert_eq!(back, original, "{kind}");
        }
    }

    /// Building the client is not enough: a request must find an HTTP transport.
    #[tokio::test]
    async fn s3_requests_have_a_transport() {
        opendal::install_default();
        let secret = dbm_shared::jobs::S3Secret {
            endpoint: "http://127.0.0.1:1".into(),
            region: String::new(),
            bucket: "b".into(),
            prefix: String::new(),
            access_key: "a".into(),
            secret_key: "s".into(),
            virtual_host: false,
            public_endpoint: String::new(),
        };
        let error = crate::storage::open(&secret)
            .unwrap()
            .stat("x")
            .await
            .unwrap_err();
        assert!(
            !error.to_string().contains("transport is not installed"),
            "{error}"
        );
    }

    #[test]
    fn cancelling_stops_the_copy() {
        let progress = Progress::default();
        progress.cancel();
        assert!(copy(&mut &b"data"[..], &mut Vec::new(), &progress).is_err());
    }
}
