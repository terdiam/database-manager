//! S3 and S3-compatible storage.

use std::io::{self, Write};

use dbm_shared::jobs::S3Secret;
use opendal::{Operator, services::S3};
use sha2::{Digest, Sha256};
use tokio::{io::AsyncWriteExt, runtime::Handle};

use crate::{Result, progress::Progress};

/// Multipart size. S3 allows 10,000 parts, so files up to about 160 GB fit.
const PART: usize = 16 * 1024 * 1024;

pub fn open(secret: &S3Secret) -> Result<Operator> {
    open_at(secret, &secret.endpoint)
}

fn open_at(secret: &S3Secret, endpoint: &str) -> Result<Operator> {
    let mut builder = S3::default()
        .bucket(&secret.bucket)
        .root(&secret.prefix)
        .region(if secret.region.is_empty() {
            "us-east-1"
        } else {
            &secret.region
        })
        .access_key_id(&secret.access_key)
        .secret_access_key(&secret.secret_key)
        // Only the credentials given here, never ones lying around on the host.
        .disable_config_load()
        .disable_ec2_metadata();
    if !endpoint.is_empty() {
        builder = builder.endpoint(endpoint);
    }
    if secret.virtual_host {
        builder = builder.enable_virtual_host_style();
    }
    Ok(Operator::new(builder)?)
}

/// A link a browser can use for five minutes. The file stays age-encrypted.
pub async fn link(secret: &S3Secret, key: &str) -> Result<String> {
    // The signature covers the host, so it must be made for the address the browser uses.
    let operator = if secret.public_endpoint.is_empty() {
        open(secret)?
    } else {
        open_at(secret, &secret.public_endpoint)?
    };
    let request = operator
        .presign_read(key, std::time::Duration::from_secs(300))
        .await?;
    Ok(request.uri().to_string())
}

/// Write, read back and delete a small object.
pub async fn test(operator: &Operator) -> Result<()> {
    let key = format!("dbm-test-{}", uuid::Uuid::new_v4().simple());
    operator.write(&key, "ok").await?;
    let read = operator.read(&key).await;
    operator.delete(&key).await?;
    if read?.to_vec() != b"ok" {
        return Err("the file read back differs from what was written".into());
    }
    Ok(())
}

/// The sink of the backup pipeline: hashes and counts what is really stored.
/// Nothing becomes visible until `finish`.
pub struct Upload<'a> {
    pub writer: &'a mut opendal::Writer,
    pub handle: Handle,
    pub buffer: Vec<u8>,
    pub hasher: Sha256,
    pub progress: Progress,
}

impl<'a> Upload<'a> {
    pub async fn start(operator: &Operator, key: &str) -> Result<opendal::Writer> {
        Ok(operator.writer_with(key).chunk(PART).await?)
    }

    pub fn new(writer: &'a mut opendal::Writer, handle: Handle, progress: Progress) -> Self {
        Self {
            writer,
            handle,
            buffer: Vec::with_capacity(PART),
            hasher: Sha256::new(),
            progress,
        }
    }

    fn send(&mut self) -> io::Result<()> {
        if self.buffer.is_empty() {
            return Ok(());
        }
        let part = std::mem::replace(&mut self.buffer, Vec::with_capacity(PART));
        self.hasher.update(&part);
        let length = part.len() as u64;
        self.handle
            .block_on(self.writer.write(part))
            .map_err(io::Error::other)?;
        self.progress.add(length);
        Ok(())
    }

    /// Commits the object and returns its SHA-256.
    pub fn finish(mut self) -> Result<String> {
        self.send()?;
        self.handle.block_on(self.writer.close())?;
        Ok(self
            .hasher
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect())
    }
}

impl Write for Upload<'_> {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        let room = PART - self.buffer.len();
        let take = room.min(data.len());
        self.buffer.extend_from_slice(&data[..take]);
        if self.buffer.len() == PART {
            self.send()?;
        }
        Ok(take)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(()) // parts are sent when full, the rest on finish
    }
}

/// Downloads to `file` and returns the SHA-256 of what arrived.
pub async fn download(
    operator: &Operator,
    key: &str,
    file: &mut tokio::fs::File,
    progress: &Progress,
) -> Result<String> {
    use futures_util::TryStreamExt;

    let mut stream = operator
        .reader_with(key)
        .chunk(PART)
        .await?
        .into_bytes_stream(..)
        .await?;
    let mut hasher = Sha256::new();
    while let Some(part) = stream.try_next().await? {
        progress.check()?;
        hasher.update(&part);
        file.write_all(&part).await?;
        progress.add(part.len() as u64);
    }
    file.flush().await?;
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}
