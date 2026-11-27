//! The system's OpenSSH client, for tunnels to databases and for SFTP. Passwords and
//! passphrases reach it through `SSH_ASKPASS` (this binary), never the command line.
//! Host keys are pinned: by the fingerprint given, otherwise by the first one seen.

use std::{
    io::Write,
    net::TcpListener,
    path::{Path, PathBuf},
    process::Stdio,
    time::{Duration, Instant},
};

use dbm_shared::jobs::SshSecret;
use tempfile::NamedTempFile;
use tokio::{
    io::AsyncReadExt,
    net::TcpStream,
    process::{Child, Command},
};

use crate::Result;

const ASKPASS: &str = "DBM_ASKPASS";

/// When ssh runs this binary to ask for a secret, print it and stop.
pub fn answer_askpass() -> bool {
    if std::env::var_os(ASKPASS).is_none() {
        return false;
    }
    let prompt = std::env::args().nth(1).unwrap_or_default().to_lowercase();
    let name = if prompt.contains("passphrase") {
        "DBM_SSH_PASSPHRASE"
    } else {
        "DBM_SSH_PASSWORD"
    };
    println!("{}", std::env::var(name).unwrap_or_default());
    true
}

fn known_hosts() -> PathBuf {
    let state = std::env::var("DBM_STATE_FILE").unwrap_or_else(|_| "agent-state".into());
    Path::new(&state).with_file_name("known_hosts")
}

fn checked(secret: &SshSecret) -> Result<()> {
    let plain =
        |v: &str| !v.is_empty() && !v.starts_with('-') && !v.contains(|c: char| c.is_whitespace());
    if !plain(&secret.host) || !plain(&secret.username) || secret.port == 0 {
        return Err("the SSH host, port and user are required".into());
    }
    Ok(())
}

/// A pinned name per record and address: a changed host key then stops the connection,
/// while editing the address starts afresh.
fn alias(id: &str, secret: &SshSecret) -> String {
    format!("dbm-{id}-{}-{}", secret.host, secret.port)
}

/// Compares the server's host keys with the fingerprint the user gave, and pins the match
/// before ssh sends anything to the server.
async fn pin(id: &str, secret: &SshSecret) -> Result<()> {
    let scanned = Command::new("ssh-keyscan")
        .args([
            "-T",
            "10",
            "-p",
            &secret.port.to_string(),
            "--",
            &secret.host,
        ])
        .output()
        .await?;
    let mut list = NamedTempFile::new()?;
    for line in String::from_utf8_lossy(&scanned.stdout).lines() {
        if line.starts_with('#') {
            continue;
        }
        let key: Vec<&str> = line.split_whitespace().skip(1).collect();
        if key.len() != 2 {
            continue;
        }
        list.as_file_mut().set_len(0)?;
        std::io::Seek::rewind(list.as_file_mut())?;
        writeln!(list, "{} {} {}", alias(id, secret), key[0], key[1])?;
        let printed = Command::new("ssh-keygen")
            .arg("-lf")
            .arg(list.path())
            .output()
            .await?;
        let printed = String::from_utf8_lossy(&printed.stdout);
        if printed.split_whitespace().any(|w| w == secret.fingerprint) {
            let known = known_hosts();
            let old = std::fs::read_to_string(&known).unwrap_or_default();
            let name = alias(id, secret);
            let mut text: String = old
                .lines()
                .filter(|l| l.split_whitespace().next() != Some(&name))
                .map(|l| format!("{l}\n"))
                .collect();
            text.push_str(&format!("{name} {} {}\n", key[0], key[1]));
            std::fs::write(known, text)?;
            return Ok(());
        }
    }
    Err("the SSH host key does not match the fingerprint".into())
}

/// An `ssh` ready for its mode-specific arguments. The key file lives as long as it is kept.
async fn command(id: &str, secret: &SshSecret) -> Result<(Command, Option<NamedTempFile>)> {
    checked(secret)?;
    let strict = if secret.fingerprint.is_empty() {
        "accept-new"
    } else {
        pin(id, secret).await?;
        "yes"
    };
    let mut command = Command::new("ssh");
    command
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("SSH_ASKPASS", std::env::current_exe()?)
        .env("SSH_ASKPASS_REQUIRE", "force")
        .env(ASKPASS, "1")
        .env("DBM_SSH_PASSWORD", &secret.password)
        .env("DBM_SSH_PASSPHRASE", &secret.passphrase)
        .args(["-F", "/dev/null", "-T"])
        .args(["-p", &secret.port.to_string(), "-l", &secret.username])
        .args(["-o", &format!("StrictHostKeyChecking={strict}")])
        .args([
            "-o",
            &format!("UserKnownHostsFile={}", known_hosts().display()),
        ])
        .args(["-o", &format!("HostKeyAlias={}", alias(id, secret))])
        .args(["-o", "GlobalKnownHostsFile=/dev/null"])
        .args(["-o", "ConnectTimeout=10"])
        .args(["-o", "ServerAliveInterval=15"])
        .args(["-o", "NumberOfPasswordPrompts=1"])
        .kill_on_drop(true);
    let key = if secret.private_key.is_empty() {
        command.args([
            "-o",
            "PreferredAuthentications=password,keyboard-interactive",
        ]);
        None
    } else {
        // Created readable by the agent only.
        let mut file = NamedTempFile::new()?;
        writeln!(file, "{}", secret.private_key.replace('\r', "").trim_end())?;
        command
            .arg("-i")
            .arg(file.path())
            .args(["-o", "IdentitiesOnly=yes"])
            .args(["-o", "PreferredAuthentications=publickey"]);
        Some(file)
    };
    Ok((command, key))
}

/// What ssh said, shortened, for an error message.
async fn complaint(child: &mut Child) -> String {
    let mut said = String::new();
    if let Some(mut pipe) = child.stderr.take() {
        let _ = tokio::time::timeout(Duration::from_secs(2), pipe.read_to_string(&mut said)).await;
    }
    let said = said.trim();
    if said.contains("REMOTE HOST IDENTIFICATION HAS CHANGED") {
        return "the SSH host key changed: check the server, then enter its fingerprint".into();
    }
    said.lines().last().unwrap_or("ssh ended").to_owned()
}

/// A local port forwarded through the SSH server. Closed when dropped.
pub struct Tunnel {
    pub port: u16,
    _child: Child,
    _key: Option<NamedTempFile>,
}

impl Tunnel {
    pub async fn open(id: &str, secret: &SshSecret, host: &str, port: u16) -> Result<Self> {
        if host.starts_with('-') || host.contains(|c: char| c.is_whitespace() || c == '/') {
            return Err("invalid database host".into());
        }
        let host = if host.contains(':') {
            format!("[{host}]")
        } else {
            host.to_owned()
        };
        // Free at this moment; ssh fails loudly if something takes it first.
        let local = TcpListener::bind("127.0.0.1:0")?.local_addr()?.port();
        let (mut command, key) = command(id, secret).await?;
        command
            .args(["-N", "-o", "ExitOnForwardFailure=yes"])
            .args(["-L", &format!("127.0.0.1:{local}:{host}:{port}")])
            .arg("--")
            .arg(&secret.host)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        let mut child = command
            .spawn()
            .map_err(|e| format!("cannot run ssh: {e}"))?;
        // The port opens once ssh has authenticated.
        let started = Instant::now();
        loop {
            if child.try_wait()?.is_some() {
                return Err(format!("SSH: {}", complaint(&mut child).await).into());
            }
            if TcpStream::connect(("127.0.0.1", local)).await.is_ok() {
                break;
            }
            if started.elapsed() > Duration::from_secs(20) {
                return Err("SSH: timed out".into());
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        Ok(Self {
            port: local,
            _child: child,
            _key: key,
        })
    }
}

/// The SFTP subsystem of an SSH server, speaking the protocol on the child's pipes.
pub async fn sftp(
    id: &str,
    secret: &SshSecret,
) -> Result<(
    russh_sftp::client::SftpSession,
    Child,
    Option<NamedTempFile>,
)> {
    let (mut command, key) = command(id, secret).await?;
    command
        .arg("-s")
        .arg("--")
        .arg(&secret.host)
        .arg("sftp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|e| format!("cannot run ssh: {e}"))?;
    let input = child.stdin.take().ok_or("ssh has no input")?;
    let output = child.stdout.take().ok_or("ssh has no output")?;
    let stream = tokio::io::join(output, input);
    match tokio::time::timeout(
        Duration::from_secs(30),
        russh_sftp::client::SftpSession::new(stream),
    )
    .await
    {
        Ok(Ok(session)) => Ok((session, child, key)),
        _ => {
            let _ = child.start_kill();
            Err(format!("SFTP: {}", complaint(&mut child).await).into())
        }
    }
}
