//! Redis: `redis-cli --rdb` takes the snapshot, trimmed to a standard RDB file. Restoring loads it into a throwaway
//! `redis-server` and copies the keys to the target, which needs no access to its disk.

use std::{
    io::{Read, Write},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

use ::redis::{
    Client, Connection, ConnectionAddr, ConnectionInfo, IntoConnectionInfo, RedisConnectionInfo,
};
use dbm_shared::jobs::DatabaseSecret;

use crate::Result;

/// Keys copied per round trip.
const BATCH: usize = 100;

/// `(use TLS, check the certificate)`.
fn tls(secret: &DatabaseSecret) -> Result<(bool, bool)> {
    match secret.ssl_mode.as_str() {
        "disable" | "allow" | "prefer" => Ok((false, false)),
        "require" => Ok((true, false)),
        "verify-ca" | "verify-full" => Ok((true, true)),
        _ => Err("unknown ssl mode".into()),
    }
}

/// An EOF opcode and a checksum come before the marker a diskless transfer ends with.
const END: usize = 1 + 8;
const MARKER: usize = 40;

/// A server that sends its snapshot without writing it to disk ends it with a random
/// 40-character marker. `redis-cli` can cut that off a file but not off a pipe, and Redis
/// rejects a file that keeps it.
fn without_marker(mut input: impl Read, mut output: impl Write) -> Result<()> {
    let mut held = Vec::new();
    let mut chunk = vec![0; 128 * 1024];
    loop {
        let n = match input.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e.into()),
        };
        held.extend_from_slice(&chunk[..n]);
        if let Some(ready) = held.len().checked_sub(END + MARKER) {
            output.write_all(&held[..ready])?;
            held.drain(..ready);
        }
    }
    if held.len() == END + MARKER
        && held[0] == 0xff
        && held[END..].iter().all(u8::is_ascii_hexdigit)
    {
        held.truncate(END);
    }
    output.write_all(&held)?;
    output.flush()?;
    Ok(())
}

/// Writes the whole instance, every numbered database, to standard output.
pub fn dump(secret: &DatabaseSecret, output: impl Write) -> Result<()> {
    let mut child = Server(
        snapshot(secret)?
            .spawn()
            .map_err(|e| format!("cannot run redis-cli: {e}"))?,
    );
    let stdout = child.0.stdout.take().ok_or("redis-cli has no output")?;
    without_marker(stdout, output)?;
    let status = child.0.wait()?;
    if !status.success() {
        return Err(format!("redis-cli failed ({status})").into());
    }
    Ok(())
}

fn snapshot(secret: &DatabaseSecret) -> Result<Command> {
    // A value starting with '-' would be read as an option.
    if secret.host.starts_with('-') || secret.username.starts_with('-') {
        return Err("invalid host or user name".into());
    }
    let (encrypted, verified) = tls(secret)?;
    let mut command = Command::new("redis-cli");
    command
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        // Its messages go straight to the job log.
        .stderr(Stdio::inherit())
        .args(["-h", &secret.host, "-p", &secret.port.to_string()]);
    if !secret.password.is_empty() {
        command.env("REDISCLI_AUTH", &secret.password);
    }
    if !secret.username.is_empty() {
        command.args(["--user", &secret.username]);
    }
    if encrypted {
        command.arg("--tls");
        if !verified {
            command.arg("--insecure");
        }
    }
    command.args(["--rdb", "-"]);
    Ok(command)
}

fn connect(secret: &DatabaseSecret) -> Result<Connection> {
    let (encrypted, verified) = tls(secret)?;
    let address = if encrypted {
        ConnectionAddr::TcpTls {
            host: secret.host.clone(),
            port: secret.port,
            insecure: !verified,
            tls_params: None,
        }
    } else {
        ConnectionAddr::Tcp(secret.host.clone(), secret.port)
    };
    let mut login = RedisConnectionInfo::default();
    if !secret.username.is_empty() {
        login = login.set_username(&secret.username);
    }
    if !secret.password.is_empty() {
        login = login.set_password(&secret.password);
    }
    let info: ConnectionInfo = address.into_connection_info()?.set_redis_settings(login);
    Ok(Client::open(info)?.get_connection_with_timeout(Duration::from_secs(10))?)
}

fn info_value(connection: &mut Connection, section: &str, key: &str) -> Result<String> {
    let text: String = ::redis::cmd("INFO").arg(section).query(connection)?;
    text.lines()
        .find_map(|line| line.strip_prefix(key)?.strip_prefix(':'))
        .map(|v| v.trim().to_owned())
        .ok_or_else(|| format!("the server did not report {key}").into())
}

pub fn test(secret: &DatabaseSecret) -> Result<String> {
    let version = info_value(&mut connect(secret)?, "server", "redis_version")?;
    Ok(format!("Redis {version}"))
}

/// A child process, stopped with whatever happens.
struct Server(Child);

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Starts `redis-server` on the file in `folder`, reachable only through a socket there.
fn load(folder: &std::path::Path) -> Result<(Server, Connection)> {
    let socket = folder.join("redis.sock");
    let log = folder.join("redis.log");
    let child = Command::new("redis-server")
        // Nothing of the helper's environment, which holds the target's password.
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .args(["--port", "0", "--save", "", "--appendonly", "no"])
        .args(["--dbfilename", "dump.rdb", "--databases", "1024"])
        .arg("--dir")
        .arg(folder)
        .arg("--unixsocket")
        .arg(&socket)
        .arg("--logfile")
        .arg(&log)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("cannot run redis-server: {e}"))?;
    let mut server = Server(child);
    let client = Client::open(format!("redis+unix://{}", socket.display()))?;
    let started = Instant::now();
    loop {
        if let Ok(mut connection) = client.get_connection() {
            // Answers with an error while it is still reading the file.
            if ::redis::cmd("PING")
                .query::<String>(&mut connection)
                .is_ok()
                && info_value(&mut connection, "persistence", "loading")? == "0"
            {
                return Ok((server, connection));
            }
        }
        if let Some(status) = server.0.try_wait()? {
            let said = std::fs::read_to_string(&log).unwrap_or_default();
            let last: Vec<_> = said.lines().rev().take(3).collect();
            return Err(
                format!("redis-server could not load the backup ({status}): {last:?}").into(),
            );
        }
        if started.elapsed() > Duration::from_secs(3600) {
            return Err("redis-server took too long to load the backup".into());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Numbers of the databases that hold keys.
fn databases(connection: &mut Connection) -> Result<Vec<u32>> {
    let text: String = ::redis::cmd("INFO").arg("keyspace").query(connection)?;
    Ok(text
        .lines()
        .filter_map(|line| line.strip_prefix("db")?.split(':').next()?.parse().ok())
        .collect())
}

/// Copies every key with its expiry time. Keys of the same name are replaced; other keys
/// on the target stay.
fn copy(from: &mut Connection, to: &mut Connection) -> Result<u64> {
    let mut copied = 0;
    for database in databases(from)? {
        ::redis::cmd("SELECT").arg(database).exec(from)?;
        ::redis::cmd("SELECT").arg(database).exec(to)?;
        let mut cursor = 0u64;
        loop {
            let (next, keys): (u64, Vec<Vec<u8>>) = ::redis::cmd("SCAN")
                .arg(cursor)
                .arg("COUNT")
                .arg(BATCH)
                .query(from)?;
            if !keys.is_empty() {
                let mut read = ::redis::pipe();
                for key in &keys {
                    read.cmd("DUMP").arg(key).cmd("PEXPIRETIME").arg(key);
                }
                let values: Vec<::redis::Value> = read.query(from)?;
                let mut write = ::redis::pipe();
                for (key, pair) in keys.iter().zip(values.chunks(2)) {
                    // Gone between SCAN and DUMP: it expired.
                    let Some(payload) =
                        ::redis::from_redis_value::<Option<Vec<u8>>>(pair[0].clone())?
                    else {
                        continue;
                    };
                    let expires: i64 = ::redis::from_redis_value(pair[1].clone())?;
                    write
                        .cmd("RESTORE")
                        .arg(key)
                        .arg(expires.max(0))
                        .arg(payload)
                        .arg("REPLACE")
                        .arg("ABSTTL")
                        .ignore();
                    copied += 1;
                }
                write.exec(to).map_err(|e| {
                    format!("the target refused a key (it may be older than the source): {e}")
                })?;
            }
            cursor = next;
            if cursor == 0 {
                break;
            }
        }
    }
    Ok(copied)
}

/// Reads an RDB file and writes its keys to the target.
pub fn restore(secret: &DatabaseSecret, mut input: impl Read) -> Result<()> {
    // Before the long part: a wrong password should fail at once.
    let mut target = connect(secret)?;
    let folder = tempfile::tempdir()?;
    let mut file = std::fs::File::create(folder.path().join("dump.rdb"))?;
    std::io::copy(&mut input, &mut file)?;
    drop(file);
    let (_server, mut source) = load(folder.path())?;
    let copied = copy(&mut source, &mut target)?;
    eprintln!("{copied} keys restored");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trimmed(input: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        without_marker(input, &mut out).unwrap();
        out
    }

    #[test]
    fn only_a_transfer_marker_is_cut_off() {
        let mut file = b"REDIS0012".to_vec();
        file.extend(vec![7; 300_000]);
        file.extend([0xff, 1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(trimmed(&file), file);
        let marked = [
            file.as_slice(),
            &b"0123456789abcdef0123456789abcdef01234567"[..],
        ]
        .concat();
        assert_eq!(trimmed(&marked), file);
        assert_eq!(trimmed(b"short"), b"short");
    }
}
