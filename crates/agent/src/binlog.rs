//! Incremental backups of MySQL and MariaDB: the binary log between two positions.
//! A position is `server|file|offset`; the first part tells a replaced server apart.
//!
//! An incremental file holds the raw log files, each as `name size\n` and its bytes. They
//! are only decoded at restore time, so the target name and the moment to stop at can be
//! chosen then.

use std::{
    fs::File,
    io::{BufRead, Read, Write},
    path::{Path, PathBuf},
    process::Stdio,
};

use dbm_shared::jobs::{DatabaseSecret, valid_target};

use crate::{Result, engines};

/// An increment cannot continue its parent: the log in between is gone, or this is another server.
#[derive(Debug)]
pub struct ChainBroken(pub String);

impl std::fmt::Display for ChainBroken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "the chain is broken ({}): a full backup is needed",
            self.0
        )
    }
}

impl std::error::Error for ChainBroken {}

#[derive(Debug, PartialEq)]
pub struct Position {
    pub server: String,
    pub file: String,
    pub offset: u64,
}

impl std::fmt::Display for Position {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}|{}|{}", self.server, self.file, self.offset)
    }
}

/// A log file name ends up on a command line and in a path.
fn file_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && !name.starts_with(['-', '.'])
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
}

impl Position {
    pub fn parse(text: &str) -> Result<Self> {
        let mut parts = text.rsplitn(3, '|');
        let (Some(offset), Some(file), Some(server)) = (parts.next(), parts.next(), parts.next())
        else {
            return Err("unreadable log position".into());
        };
        if !file_name(file) {
            return Err("unreadable log position".into());
        }
        Ok(Self {
            server: server.into(),
            file: file.into(),
            offset: offset.parse()?,
        })
    }
}

fn maria(secret: &DatabaseSecret) -> bool {
    secret.engine == "mariadb"
}

/// Which server this is. Logs of another one have nothing to do with the chain.
pub fn server(secret: &DatabaseSecret) -> Result<String> {
    let sql = if maria(secret) {
        "SELECT CONCAT(@@hostname, '/', @@server_id)"
    } else {
        "SELECT @@server_uuid"
    };
    engines::mysql_query(secret, sql)
}

/// The bundled mysqldump asks for the position with a statement MySQL has since 8.2.
fn too_old(version: &str) -> bool {
    let mut numbers = version.split('.').map(|n| n.parse::<u32>().unwrap_or(0));
    (numbers.next(), numbers.next()) < (Some(8), Some(2))
}

/// Refuses early when the server does not keep a usable log.
pub fn check(secret: &DatabaseSecret) -> Result<()> {
    if !maria(secret) && too_old(&engines::mysql_query(secret, "SELECT VERSION()")?) {
        return Err("incremental backups need MySQL 8.4 or newer".into());
    }
    let settings = engines::mysql_query(secret, "SELECT @@log_bin, @@binlog_format")?;
    if settings.split_whitespace().collect::<Vec<_>>() != ["1", "ROW"] {
        return Err("incremental backups need the binary log on, in ROW format".into());
    }
    Ok(())
}

/// Where the log ends now.
fn current(secret: &DatabaseSecret) -> Result<Position> {
    let status = if maria(secret) {
        engines::mysql_query(secret, "SHOW MASTER STATUS")?
    } else {
        // The name changed in 8.4.
        engines::mysql_query(secret, "SHOW BINARY LOG STATUS")
            .or_else(|_| engines::mysql_query(secret, "SHOW MASTER STATUS"))?
    };
    let mut columns = status.split_whitespace();
    let (Some(file), Some(offset)) = (columns.next(), columns.next()) else {
        return Err("the server did not report its log position".into());
    };
    Position::parse(&format!("{}|{file}|{offset}", server(secret)?))
}

/// The position a full dump was taken at, from the comment the dump tool writes near its top.
pub fn dump_position(head: &[u8]) -> Option<(String, u64)> {
    let text = String::from_utf8_lossy(head);
    let line = text
        .lines()
        .find(|l| l.starts_with("-- CHANGE ") && l.contains("_LOG_FILE="))?;
    let file = line.split("_LOG_FILE='").nth(1)?.split('\'').next()?;
    let offset = line.split("_LOG_POS=").nth(1)?;
    let offset = offset.split(|c: char| !c.is_ascii_digit()).next()?;
    Some((file.into(), offset.parse().ok()?))
}

/// The log files from where the parent ended up to now.
pub struct Span {
    pub to: Position,
    pub files: Vec<String>,
}

pub fn span(secret: &DatabaseSecret, since: &Position) -> Result<Span> {
    check(secret)?;
    let to = current(secret)?;
    if to.server != since.server {
        return Err(ChainBroken("this is not the server the full backup came from".into()).into());
    }
    let listed = engines::mysql_query(secret, "SHOW BINARY LOGS")?;
    let all: Vec<&str> = listed
        .lines()
        .filter_map(|line| line.split_whitespace().next())
        .collect();
    let first = all.iter().position(|f| *f == since.file);
    let last = all.iter().position(|f| *f == to.file);
    match (first, last) {
        (Some(first), Some(last))
            if first < last || (first == last && since.offset <= to.offset) =>
        {
            let files = all[first..=last].iter().map(|f| (*f).to_owned()).collect();
            Ok(Span { to, files })
        }
        _ => Err(ChainBroken(format!("{} is no longer on the server", since.file)).into()),
    }
}

fn tool(secret: &DatabaseSecret) -> &'static str {
    if maria(secret) {
        "mariadb-binlog"
    } else {
        "mysqlbinlog"
    }
}

fn run(command: &mut std::process::Command, name: &str) -> Result<()> {
    let status = command
        .status()
        .map_err(|e| format!("cannot run {name}: {e}"))?;
    if !status.success() {
        return Err(format!("{name} failed ({status})").into());
    }
    Ok(())
}

/// Fetches the files and writes them as one stream.
pub fn dump(secret: &DatabaseSecret, files: &[String], mut out: impl Write) -> Result<()> {
    if !files.iter().all(|f| file_name(f)) {
        return Err("unexpected log file name".into());
    }
    let folder = tempfile::tempdir()?;
    let mut command = engines::mysql(tool(secret), secret)?;
    let mut prefix = folder.path().as_os_str().to_owned();
    prefix.push("/");
    command
        .args(["--read-from-remote-server", "--raw"])
        .arg(format!(
            "--result-file={}",
            prefix.to_str().ok_or("unreadable folder")?
        ))
        .args(files)
        .stdout(Stdio::null())
        .stderr(Stdio::inherit());
    run(&mut command, tool(secret))?;
    pack(folder.path(), files, &mut out)
}

fn pack(folder: &Path, files: &[String], out: &mut impl Write) -> Result<()> {
    for name in files {
        let mut file = File::open(folder.join(name))?;
        let size = file.metadata()?.len();
        writeln!(out, "{name} {size}")?;
        std::io::copy(&mut file, out)?;
    }
    out.flush()?;
    Ok(())
}

fn unpack(folder: &Path, input: &mut impl BufRead) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    loop {
        let mut header = String::new();
        if input.by_ref().take(200).read_line(&mut header)? == 0 {
            return Ok(paths);
        }
        let (name, size) = header
            .trim_end()
            .split_once(' ')
            .ok_or("this is not an incremental backup")?;
        if !file_name(name) {
            return Err("unexpected log file name".into());
        }
        let size: u64 = size.parse()?;
        let path = folder.join(name);
        let mut file = File::create_new(&path)?;
        if std::io::copy(&mut input.by_ref().take(size), &mut file)? != size {
            return Err("the incremental backup is cut short".into());
        }
        paths.push(path);
    }
}

/// Which part of the log to replay.
pub struct Replay<'a> {
    /// Database the backup was taken from, and the one to write into.
    pub source: &'a str,
    pub target: &'a str,
    pub from: Position,
    pub to: Position,
    /// `YYYY-MM-DD HH:MM:SS` in UTC; empty replays everything.
    pub until: &'a str,
}

fn decoder(secret: &DatabaseSecret, replay: &Replay, files: &[PathBuf]) -> std::process::Command {
    let mut command = engines::quiet(tool(secret));
    command
        .env("TZ", "UTC")
        .arg("--no-defaults")
        .arg(format!("--start-position={}", replay.from.offset))
        .arg(format!("--stop-position={}", replay.to.offset));
    let renamed = replay.source != replay.target;
    if renamed {
        command.arg(format!("--rewrite-db={}->{}", replay.source, replay.target));
    }
    // MySQL filters on the new name, MariaDB on the one in the log.
    if maria(secret) {
        command.arg(format!("--database={}", replay.source));
    } else {
        // The server would skip transactions whose ids it has already run.
        command
            .arg("--skip-gtids")
            .arg(format!("--database={}", replay.target));
    }
    if !replay.until.is_empty() {
        command.arg(format!("--stop-datetime={}", replay.until));
    }
    command.args(files);
    command
}

/// Replays an incremental backup into the target database.
pub fn apply(secret: &DatabaseSecret, replay: &Replay, mut input: impl BufRead) -> Result<()> {
    if !valid_target(&secret.engine, replay.source) || !valid_target(&secret.engine, replay.target)
    {
        return Err("invalid database name".into());
    }
    let folder = tempfile::tempdir()?;
    let files = unpack(folder.path(), &mut input)?;
    if files.is_empty() {
        return Err("the incremental backup is empty".into());
    }
    let mut decode = decoder(secret, replay, &files)
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| format!("cannot run {}: {e}", tool(secret)))?;
    let events = decode.stdout.take().ok_or("no output")?;
    let client = if maria(secret) { "mariadb" } else { "mysql" };
    let mut apply = engines::mysql(client, secret)?;
    apply
        .args(["--binary-mode", &format!("--database={}", replay.target)])
        .stdin(events)
        .stdout(Stdio::null())
        .stderr(Stdio::inherit());
    let applied = run(&mut apply, client);
    // Drop the pipe before waiting, so a stopped client cannot leave the decoder hanging.
    drop(apply);
    let decoded = decode.wait()?;
    applied?;
    if !decoded.success() {
        return Err(format!("{} failed ({decoded})", tool(secret)).into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secret(engine: &str) -> DatabaseSecret {
        DatabaseSecret {
            engine: engine.into(),
            host: "db".into(),
            port: 1,
            username: "u".into(),
            password: "hunter2-secret".into(),
            ssl_mode: "disable".into(),
            tunnel: None,
        }
    }

    #[test]
    fn old_mysql_cannot_be_followed() {
        assert!(too_old("8.0.43"));
        assert!(!too_old("8.4.6"));
        assert!(!too_old("9.1.0-commercial"));
    }

    #[test]
    fn positions_are_read_from_both_kinds_of_dump() {
        for (line, file, offset) in [
            (
                "-- CHANGE REPLICATION SOURCE TO SOURCE_LOG_FILE='binlog.000002', SOURCE_LOG_POS=661;",
                "binlog.000002",
                661,
            ),
            (
                "-- CHANGE MASTER TO MASTER_LOG_FILE='mysqld-bin.000009', MASTER_LOG_POS=7;",
                "mysqld-bin.000009",
                7,
            ),
        ] {
            let head = format!("-- dump\n--\n{line}\nCREATE TABLE t;");
            assert_eq!(dump_position(head.as_bytes()), Some((file.into(), offset)));
        }
        assert_eq!(dump_position(b"CREATE TABLE t;"), None);
        let at = Position::parse("a|b/1|binlog.000002|661").unwrap();
        assert_eq!((at.server.as_str(), at.offset), ("a|b/1", 661));
        assert_eq!(Position::parse(&at.to_string()).unwrap(), at);
        for bad in ["", "x|y", "s|../etc|1", "s|-x|1", "s|f|x"] {
            assert!(Position::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn log_files_survive_the_stream_and_cannot_leave_the_folder() {
        let from = tempfile::tempdir().unwrap();
        std::fs::write(from.path().join("bin.000001"), b"one\nwith lines").unwrap();
        std::fs::write(from.path().join("bin.000002"), b"").unwrap();
        let names = ["bin.000001".to_owned(), "bin.000002".to_owned()];
        let mut stream = Vec::new();
        pack(from.path(), &names, &mut stream).unwrap();
        let to = tempfile::tempdir().unwrap();
        let paths = unpack(to.path(), &mut stream.as_slice()).unwrap();
        assert_eq!(paths.len(), 2);
        assert_eq!(std::fs::read(&paths[0]).unwrap(), b"one\nwith lines");
        for bad in ["../x 0\n", "/etc/passwd 0\n", "a 9\nshort"] {
            let to = tempfile::tempdir().unwrap();
            assert!(unpack(to.path(), &mut bad.as_bytes()).is_err(), "{bad}");
        }
    }

    #[test]
    fn each_server_filters_on_the_name_it_expects() {
        let args = |engine| {
            let at = |offset| Position::parse(&format!("s|bin.000001|{offset}")).unwrap();
            let replay = Replay {
                source: "shop",
                target: "copy",
                from: at(4),
                to: at(9),
                until: "",
            };
            let command = decoder(&secret(engine), &replay, &[]);
            command
                .get_args()
                .map(|a| a.to_string_lossy().into_owned())
                .collect::<Vec<_>>()
        };
        assert!(args("mysql").contains(&"--database=copy".into()));
        assert!(args("mariadb").contains(&"--database=shop".into()));
        assert!(args("mariadb").contains(&"--rewrite-db=shop->copy".into()));
    }
}
