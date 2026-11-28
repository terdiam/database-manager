//! Where backup files are kept: S3 and FTP through OpenDAL, SFTP through OpenSSH, and
//! SMB through `smbclient`; FTPS uploads have their own module. Nothing is visible under
//! its final name before it is whole, except on S3 multipart (invisible until completed)
//! and plain FTP (removed when it fails).

use std::{
    io::{self, Write},
    process::Stdio,
    sync::Arc,
};

use dbm_shared::jobs::{SshSecret, StorageSecret};
use futures_util::TryStreamExt;
use opendal::{Operator, services::Ftp, services::S3};
use russh_sftp::client::SftpSession;
use sha2::{Digest, Sha256};
use tempfile::NamedTempFile;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::{Child, ChildStdin, Command},
    runtime::Handle,
};

use crate::{Result, ftps, progress::Progress, ssh};

/// Multipart size. S3 allows 10,000 parts, so files up to about 160 GB fit.
const PART: usize = 16 * 1024 * 1024;

pub enum Store {
    /// S3 and plain FTP.
    Object(Operator),
    Ftps {
        operator: Operator,
        login: Arc<ftps::Login>,
        folder: String,
    },
    Sftp {
        session: Arc<SftpSession>,
        folder: String,
        _child: Child,
        _key: Option<NamedTempFile>,
    },
    Smb(Arc<Smb>),
}

/// Object keys come from the server: they must stay plain relative paths, because they
/// end up in `smbclient` commands and file paths.
fn safe(key: &str) -> Result<&str> {
    let ok = !key.is_empty()
        && !key.starts_with(['/', '-'])
        && key
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
        && key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-/".contains(&b));
    if ok {
        Ok(key)
    } else {
        Err("unsafe file name".into())
    }
}

/// The configured folder, without a trailing slash.
fn folder(secret: &StorageSecret) -> Result<String> {
    let folder = secret.prefix.trim().trim_end_matches('/');
    if folder.split('/').any(|part| part == "..")
        || folder.contains(['"', ';', '\\', '\n', '\r'])
        || folder.starts_with('-')
    {
        return Err("the folder name is not allowed".into());
    }
    Ok(folder.to_owned())
}

fn joined(folder: &str, key: &str) -> String {
    if folder.is_empty() {
        key.to_owned()
    } else {
        format!("{folder}/{key}")
    }
}

/// `a/b/c` for `a/b/c/file`: the folders to create, outermost first.
fn parents(path: &str) -> Vec<String> {
    let mut made = Vec::new();
    let mut at = 0;
    while let Some(next) = path[at..].find('/') {
        at += next + 1;
        if at > 1 {
            made.push(path[..at - 1].to_owned());
        }
    }
    made
}

fn s3(secret: &StorageSecret, endpoint: &str) -> Result<Operator> {
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

/// `id` names the record, to pin an SFTP host key.
pub async fn open(secret: &StorageSecret, id: &str) -> Result<Store> {
    Ok(match secret.kind.as_str() {
        "" | "s3" => Store::Object(s3(secret, &secret.endpoint)?),
        "ftp" => {
            let scheme = if secret.tls { "ftps" } else { "ftp" };
            let builder = Ftp::default()
                .endpoint(&format!("{scheme}://{}:{}", secret.host, secret.port))
                .root(&format!("/{}", folder(secret)?.trim_start_matches('/')))
                .user(&secret.username)
                .password(&secret.password);
            let operator = Operator::new(builder)?;
            if !secret.tls {
                return Ok(Store::Object(operator));
            }
            Store::Ftps {
                operator,
                login: Arc::new(ftps::Login {
                    host: secret.host.clone(),
                    port: secret.port,
                    username: secret.username.clone(),
                    password: secret.password.clone(),
                }),
                folder: folder(secret)?.trim_start_matches('/').to_owned(),
            }
        }
        "sftp" => {
            let login = SshSecret {
                host: secret.host.clone(),
                port: secret.port,
                username: secret.username.clone(),
                password: secret.password.clone(),
                private_key: secret.private_key.clone(),
                passphrase: secret.passphrase.clone(),
                fingerprint: secret.fingerprint.clone(),
            };
            let folder = folder(secret)?;
            let (session, child, key) = ssh::sftp(id, &login).await?;
            Store::Sftp {
                session: Arc::new(session),
                folder,
                _child: child,
                _key: key,
            }
        }
        "smb" => Store::Smb(Arc::new(Smb::new(secret)?)),
        _ => return Err("unknown storage kind".into()),
    })
}

impl Store {
    /// Starts a new file. Dropping the sink without `commit` leaves nothing behind
    /// except a `.part` file, which `abort` removes.
    pub async fn sink(&self, key: &str) -> Result<Sink> {
        let key = safe(key)?;
        Ok(match self {
            Store::Object(operator) => Sink::Object {
                writer: operator.writer_with(key).chunk(PART).await?,
                operator: operator.clone(),
                key: key.to_owned(),
            },
            Store::Ftps { login, folder, .. } => {
                let path = format!("/{}", joined(folder, key));
                let upload = ftps::Upload::start(login.clone(), &parents(&path), path).await?;
                Sink::Ftps(Box::new(upload))
            }
            Store::Sftp {
                session, folder, ..
            } => {
                let path = joined(folder, key);
                for dir in parents(&path) {
                    // Fails when it exists; a real problem shows when the file is created.
                    let _ = session.create_dir(dir).await;
                }
                let part = format!("{path}.part");
                let file = session
                    .create(part.clone())
                    .await
                    .map_err(|e| format!("SFTP cannot create the file: {e}"))?;
                Sink::Sftp {
                    session: session.clone(),
                    file: Box::new(file),
                    part,
                    path,
                }
            }
            Store::Smb(smb) => {
                let path = smb.path(key);
                for dir in parents(&path) {
                    let _ = smb.run(&format!("mkdir \"{dir}\"")).await;
                }
                let mut child = smb
                    .command(&format!("put - \"{path}.part\""))
                    .stdin(Stdio::piped())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .map_err(|e| format!("cannot run smbclient: {e}"))?;
                let input = child.stdin.take();
                Sink::Smb {
                    smb: smb.clone(),
                    child,
                    input,
                    path,
                }
            }
        })
    }

    /// Downloads to `file` and returns the SHA-256 of what arrived.
    pub async fn download(
        &self,
        key: &str,
        file: &mut tokio::fs::File,
        progress: &Progress,
    ) -> Result<String> {
        let key = safe(key)?;
        let mut hasher = Sha256::new();
        let mut child = None;
        let mut source: Box<dyn tokio::io::AsyncRead + Unpin + Send> = match self {
            Store::Object(operator) | Store::Ftps { operator, .. } => {
                let mut stream = operator
                    .reader_with(key)
                    .chunk(PART)
                    .await?
                    .into_bytes_stream(..)
                    .await?;
                while let Some(part) = stream.try_next().await? {
                    progress.check()?;
                    hasher.update(&part);
                    file.write_all(&part).await?;
                    progress.add(part.len() as u64);
                }
                Box::new(tokio::io::empty())
            }
            Store::Sftp {
                session, folder, ..
            } => Box::new(
                session
                    .open(joined(folder, key))
                    .await
                    .map_err(|e| format!("SFTP cannot open the file: {e}"))?,
            ),
            Store::Smb(smb) => {
                let mut running = smb
                    .command(&format!("get \"{}\" -", smb.path(key)))
                    .stdin(Stdio::null())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::null())
                    .spawn()
                    .map_err(|e| format!("cannot run smbclient: {e}"))?;
                let output = running.stdout.take().ok_or("smbclient has no output")?;
                child = Some(running);
                Box::new(output)
            }
        };
        let mut buffer = vec![0; 128 * 1024];
        loop {
            progress.check()?;
            let n = source.read(&mut buffer).await?;
            if n == 0 {
                break;
            }
            hasher.update(&buffer[..n]);
            file.write_all(&buffer[..n]).await?;
            progress.add(n as u64);
        }
        if let Some(mut running) = child
            && !running.wait().await?.success()
        {
            return Err("SMB: the file could not be read".into());
        }
        file.flush().await?;
        Ok(hex(hasher))
    }

    pub async fn delete(&self, key: &str) -> Result<()> {
        let key = safe(key)?;
        match self {
            Store::Object(operator) | Store::Ftps { operator, .. } => operator.delete(key).await?,
            Store::Sftp {
                session, folder, ..
            } => {
                let path = joined(folder, key);
                // Already gone counts as deleted.
                if let Err(error) = session.remove_file(path.clone()).await
                    && session.try_exists(path).await.unwrap_or(true)
                {
                    return Err(format!("SFTP cannot delete the file: {error}").into());
                }
            }
            Store::Smb(smb) => {
                let path = smb.path(key);
                if let Err(error) = smb.run(&format!("del \"{path}\"")).await
                    && !error.to_string().contains("NO_SUCH_FILE")
                    && !error.to_string().contains("NOT_FOUND")
                {
                    return Err(error);
                }
            }
        }
        Ok(())
    }

    /// Write, read back and delete a small file.
    pub async fn test(&self) -> Result<()> {
        let key = format!("dbm-test-{}", uuid::Uuid::new_v4().simple());
        let mut sink = self.sink(&key).await?;
        sink.write(b"ok".to_vec()).await?;
        sink.commit().await?;
        let temporary = NamedTempFile::new()?;
        let mut file = tokio::fs::File::from_std(temporary.reopen()?);
        let read = self.download(&key, &mut file, &Progress::default()).await;
        self.delete(&key).await?;
        if read? != hex(Sha256::new_with_prefix(b"ok")) {
            return Err("the file read back differs from what was written".into());
        }
        Ok(())
    }
}

fn hex(hasher: Sha256) -> String {
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// A link a browser can use for five minutes (S3 only). The file stays age-encrypted.
pub async fn link(secret: &StorageSecret, key: &str) -> Result<String> {
    if !matches!(secret.kind.as_str(), "" | "s3") {
        return Err("only S3 destinations can make a download link".into());
    }
    // The signature covers the host, so it must be made for the address the browser uses.
    let endpoint = if secret.public_endpoint.is_empty() {
        &secret.endpoint
    } else {
        &secret.public_endpoint
    };
    let request = s3(secret, endpoint)?
        .presign_read(safe(key)?, std::time::Duration::from_secs(300))
        .await?;
    Ok(request.uri().to_string())
}

pub struct Smb {
    share: String,
    port: u16,
    username: String,
    password: String,
    domain: String,
    folder: String,
}

impl Smb {
    fn new(secret: &StorageSecret) -> Result<Self> {
        let plain = |v: &str| {
            !v.is_empty() && !v.starts_with('-') && !v.contains(['/', '\\', '%', ' ', '\n'])
        };
        if !plain(&secret.host) || !plain(&secret.bucket) || secret.username.contains('%') {
            return Err("the SMB host, share and user are required".into());
        }
        Ok(Self {
            share: format!("//{}/{}", secret.host, secret.bucket),
            port: secret.port,
            username: secret.username.clone(),
            password: secret.password.clone(),
            domain: secret.domain.clone(),
            folder: folder(secret)?.trim_start_matches('/').to_owned(),
        })
    }

    fn path(&self, key: &str) -> String {
        joined(&self.folder, key)
    }

    fn command(&self, action: &str) -> Command {
        let mut command = Command::new("smbclient");
        command
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("USER", &self.username)
            .env("PASSWD", &self.password)
            .arg(&self.share)
            .args(["-s", "/dev/null", "-p", &self.port.to_string()])
            .args(["--option=client min protocol=SMB2"])
            .arg(format!("--user={}", self.username))
            .kill_on_drop(true);
        if !self.domain.is_empty() {
            command.arg(format!("--workgroup={}", self.domain));
        }
        command.args(["-c", action]);
        command
    }

    async fn run(&self, action: &str) -> Result<()> {
        let output = self
            .command(action)
            .stdin(Stdio::null())
            .output()
            .await
            .map_err(|e| format!("cannot run smbclient: {e}"))?;
        if !output.status.success() {
            let mut said = String::from_utf8_lossy(&output.stdout).into_owned();
            said.push_str(&String::from_utf8_lossy(&output.stderr));
            let last = said.trim().lines().last().unwrap_or("smbclient failed");
            return Err(format!("SMB: {last}").into());
        }
        Ok(())
    }
}

/// A file being written. It owns what it needs, so the pipeline can move it to a
/// blocking thread; the store must stay open until it is committed or aborted.
pub enum Sink {
    Object {
        writer: opendal::Writer,
        operator: Operator,
        key: String,
    },
    Ftps(Box<ftps::Upload>),
    Sftp {
        session: Arc<SftpSession>,
        file: Box<russh_sftp::client::fs::File>,
        part: String,
        path: String,
    },
    Smb {
        smb: Arc<Smb>,
        child: Child,
        input: Option<ChildStdin>,
        path: String,
    },
}

impl Sink {
    pub async fn write(&mut self, data: Vec<u8>) -> Result<()> {
        match self {
            Sink::Object { writer, .. } => writer.write(data).await?,
            Sink::Ftps(upload) => upload.write(&data).await?,
            Sink::Sftp { file, .. } => file.write_all(&data).await?,
            Sink::Smb { input, .. } => {
                let input = input.as_mut().ok_or("the upload is closed")?;
                input
                    .write_all(&data)
                    .await
                    .map_err(|e| format!("SMB upload stopped: {e}"))?;
            }
        }
        Ok(())
    }

    /// Makes the file visible under its name.
    pub async fn commit(self) -> Result<()> {
        match self {
            Sink::Object { mut writer, .. } => {
                writer.close().await?;
            }
            Sink::Ftps(upload) => upload.commit().await?,
            Sink::Sftp {
                session,
                mut file,
                part,
                path,
            } => {
                file.shutdown().await?;
                session
                    .rename(part, path)
                    .await
                    .map_err(|e| format!("SFTP cannot rename the file: {e}"))?;
            }
            Sink::Smb {
                smb,
                mut child,
                input,
                path,
            } => {
                drop(input);
                if !child.wait().await?.success() {
                    return Err("SMB: the upload failed".into());
                }
                smb.run(&format!("rename \"{path}.part\" \"{path}\""))
                    .await?;
            }
        }
        Ok(())
    }

    pub async fn abort(self) -> Result<()> {
        match self {
            Sink::Object {
                mut writer,
                operator,
                key,
            } => {
                // S3 forgets the parts; FTP has no abort, so the partial file is removed.
                if writer.abort().await.is_err() {
                    operator.delete(&key).await?;
                }
            }
            Sink::Ftps(upload) => upload.abort().await?,
            Sink::Sftp {
                session,
                mut file,
                part,
                ..
            } => {
                let _ = file.shutdown().await;
                let _ = session.remove_file(part).await;
            }
            Sink::Smb {
                smb,
                mut child,
                input,
                path,
            } => {
                drop(input);
                let _ = child.kill().await;
                let _ = smb.run(&format!("del \"{path}.part\"")).await;
            }
        }
        Ok(())
    }
}

/// The sink of the backup pipeline: hashes and counts what is really stored.
pub struct Upload<'a> {
    sink: &'a mut Sink,
    handle: Handle,
    buffer: Vec<u8>,
    hasher: Sha256,
    progress: Progress,
}

impl<'a> Upload<'a> {
    pub fn new(sink: &'a mut Sink, handle: Handle, progress: Progress) -> Self {
        Self {
            sink,
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
            .block_on(self.sink.write(part))
            .map_err(|e| io::Error::other(e.to_string()))?;
        self.progress.add(length);
        Ok(())
    }

    /// Sends the rest and returns the SHA-256; the caller commits the sink.
    pub fn finish(mut self) -> Result<String> {
        self.send()?;
        Ok(hex(self.hasher))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_stay_plain_relative_paths() {
        assert!(safe("prod/shop/2026-x.dump.zst.age").is_ok());
        for bad in [
            "",
            "/etc/passwd",
            "a/../b",
            "a//b",
            "a\"; del x",
            "a b",
            "-x",
        ] {
            assert!(safe(bad).is_err(), "{bad}");
        }
        assert_eq!(parents("a/b/c"), ["a", "a/b"]);
        assert_eq!(parents("/srv/x/file"), ["/srv", "/srv/x"]);
        assert!(parents("file").is_empty());
    }

    /// Building the client is not enough: a request must find an HTTP transport.
    #[tokio::test]
    async fn s3_requests_have_a_transport() {
        opendal::install_default();
        let secret = StorageSecret {
            endpoint: "http://127.0.0.1:1".into(),
            bucket: "b".into(),
            access_key: "a".into(),
            secret_key: "s".into(),
            ..Default::default()
        };
        let Store::Object(operator) = open(&secret, "x").await.unwrap() else {
            panic!("an S3 secret opens an object store");
        };
        let error = operator.stat("x").await.unwrap_err();
        assert!(
            !error.to_string().contains("transport is not installed"),
            "{error}"
        );
    }
}
