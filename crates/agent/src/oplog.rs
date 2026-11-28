//! Incremental backups of MongoDB: the replica set's oplog between two timestamps, for the
//! whole instance. A position is `seconds:ordinal`. An incremental file is the oplog's
//! documents as `mongodump` writes them, which `mongorestore` replays, without the entries
//! about accounts: those stay the target's.

use std::{
    io::{Read, Write},
    process::Stdio,
    time::{SystemTime, UNIX_EPOCH},
};

use dbm_shared::jobs::DatabaseSecret;

use crate::{Result, binlog::ChainBroken, engines};

#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct Timestamp {
    pub seconds: u32,
    pub ordinal: u32,
}

impl std::fmt::Display for Timestamp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.seconds, self.ordinal)
    }
}

impl Timestamp {
    pub fn parse(text: &str) -> Result<Self> {
        let (seconds, ordinal) = text.split_once(':').ok_or("unreadable oplog position")?;
        Ok(Self {
            seconds: seconds.parse()?,
            ordinal: ordinal.parse()?,
        })
    }

    fn json(self) -> String {
        format!(
            r#"{{"$timestamp":{{"t":{},"i":{}}}}}"#,
            self.seconds, self.ordinal
        )
    }
}

const BAD: &str = "unreadable oplog entry";

fn number(bytes: &[u8], at: usize) -> Result<u32> {
    let four = bytes.get(at..at + 4).ok_or(BAD)?;
    Ok(u32::from_le_bytes(four.try_into()?))
}

/// What this module needs from an oplog entry.
struct Entry<'a> {
    at: Timestamp,
    /// `database.collection` the entry is about.
    namespace: &'a [u8],
}

impl Entry<'_> {
    /// Accounts and per-server records are not replayed. `admin.$cmd` is: transactions and
    /// renames across databases are recorded there.
    fn replayed(&self) -> bool {
        let server_own =
            self.namespace.starts_with(b"admin.") || self.namespace.starts_with(b"config.");
        !server_own || self.namespace == b"admin.$cmd"
    }
}

fn entry(document: &[u8]) -> Result<Entry<'_>> {
    let (mut time, mut namespace) = (None, &b""[..]);
    let mut at = 4;
    while let Some(&kind) = document.get(at).filter(|&&k| k != 0) {
        let name = &document[at + 1..];
        let end = name.iter().position(|&b| b == 0).ok_or(BAD)?;
        let name = &name[..end];
        at += end + 2;
        if name == b"ts" && kind == 0x11 {
            time = Some(Timestamp {
                ordinal: number(document, at)?,
                seconds: number(document, at + 4)?,
            });
        }
        if name == b"ns" && kind == 0x02 {
            let length = number(document, at)? as usize;
            namespace = document
                .get(at + 4..(at + 3).saturating_add(length))
                .ok_or(BAD)?;
        }
        // How far the value reaches, for the types an oplog entry holds.
        at += match kind {
            0x01 | 0x09 | 0x11 | 0x12 => 8,
            0x02 => 4 + number(document, at)? as usize,
            0x03 | 0x04 => number(document, at)? as usize,
            0x05 => 5 + number(document, at)? as usize,
            0x07 => 12,
            0x08 => 1,
            0x0A => 0,
            0x10 => 4,
            0x13 => 16,
            _ => return Err(BAD.into()),
        };
    }
    Ok(Entry {
        at: time.ok_or(BAD)?,
        namespace,
    })
}

/// Calls `each` for every document of a stream, holding one at a time.
fn documents(mut input: impl Read, mut each: impl FnMut(&[u8]) -> Result<()>) -> Result<()> {
    let mut document = Vec::new();
    loop {
        let mut size = [0; 4];
        match input.read_exact(&mut size) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(()),
            Err(e) => return Err(e.into()),
        }
        // MongoDB's own limit is 16 MB; entries wrapping a document are a little larger.
        let length = u32::from_le_bytes(size) as usize;
        if !(5..=48 * 1024 * 1024).contains(&length) {
            return Err(BAD.into());
        }
        document.clear();
        document.extend_from_slice(&size);
        document.resize(length, 0);
        input.read_exact(&mut document[4..])?;
        each(&document)?;
    }
}

fn newest(input: impl Read) -> Result<Option<Timestamp>> {
    let mut found: Option<Timestamp> = None;
    documents(input, |document| {
        let at = entry(document)?.at;
        if found.is_none_or(|newest| at > newest) {
            found = Some(at);
        }
        Ok(())
    })?;
    Ok(found)
}

/// Copies the entries to replay; returns how many there are.
fn replayable(input: impl Read, mut out: impl Write) -> Result<u64> {
    let mut count = 0;
    documents(input, |document| {
        if entry(document)?.replayed() {
            out.write_all(document)?;
            count += 1;
        }
        Ok(())
    })?;
    out.flush()?;
    Ok(count)
}

/// A command writing the oplog entries that match `query` to standard output.
fn read(secret: &DatabaseSecret, query: &str) -> Result<engines::Tool> {
    let (mut command, file) = engines::mongo("mongodump", secret)?;
    command.args([
        "--quiet",
        "--db=local",
        "--collection=oplog.rs",
        &format!("--query={query}"),
        "--out=-",
    ]);
    Ok(engines::Tool::holding(command, "mongodump", file))
}

fn newest_matching(secret: &DatabaseSecret, query: &str) -> Result<Option<Timestamp>> {
    let mut tool = read(secret, query)?;
    let mut child = tool
        .command
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| format!("cannot run mongodump: {e}"))?;
    let found = newest(child.stdout.take().ok_or("no output")?);
    if !child.wait()?.success() {
        return Err("mongodump could not read the oplog".into());
    }
    found
}

/// Where the oplog ends now. A replica set writes to it at least every few seconds.
pub fn current(secret: &DatabaseSecret) -> Result<Timestamp> {
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
    // A short window first: the oplog of a busy server is large.
    for minutes in [1, 60] {
        let since = now.saturating_sub(minutes * 60_000);
        let query = format!(r#"{{"wall":{{"$gte":{{"$date":{{"$numberLong":"{since}"}}}}}}}}"#);
        if let Some(found) = newest_matching(secret, &query)? {
            return Ok(found);
        }
    }
    Err("incremental backups need a replica set: no oplog entries were found".into())
}

/// Everything after `since` up to the end of the log now, unless `since` has rolled out of it.
pub fn span(secret: &DatabaseSecret, since: Timestamp) -> Result<Timestamp> {
    let to = current(secret)?;
    let kept = format!(r#"{{"ts":{}}}"#, since.json());
    if newest_matching(secret, &kept)?.is_none() {
        return Err(
            ChainBroken("the oplog no longer reaches back to the last backup".into()).into(),
        );
    }
    Ok(to)
}

/// The incremental dump: entries after `since`, up to and including `to`.
pub fn dump(secret: &DatabaseSecret, since: Timestamp, to: Timestamp) -> Result<engines::Tool> {
    read(
        secret,
        &format!(
            r#"{{"ts":{{"$gt":{},"$lte":{}}}}}"#,
            since.json(),
            to.json()
        ),
    )
}

/// `YYYY-MM-DD HH:MM:SS` in UTC as seconds since 1970.
fn unix_seconds(text: &str) -> Result<i64> {
    let parts: Vec<i64> = text
        .split(['-', ' ', ':'])
        .map(str::parse)
        .collect::<std::result::Result<_, _>>()?;
    let [year, month, day, hour, minute, second] = parts[..] else {
        return Err("unreadable time".into());
    };
    // Days from the civil date, with the year starting in March.
    let year = year - i64::from(month <= 2);
    let of_era = year.rem_euclid(400);
    let of_year = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    let days = year.div_euclid(400) * 146_097 + of_era * 365 + of_era / 4 - of_era / 100 + of_year
        - 719_468;
    Ok(days * 86_400 + hour * 3600 + minute * 60 + second)
}

/// Replays an incremental file, up to `until` when it is not empty.
pub fn apply(secret: &DatabaseSecret, until: &str, input: impl Read) -> Result<()> {
    let folder = tempfile::tempdir()?;
    let path = folder.path().join("oplog.bson");
    let file = std::io::BufWriter::new(std::fs::File::create(&path)?);
    if replayable(input, file)? == 0 {
        return Ok(());
    }
    let empty = folder.path().join("empty");
    std::fs::create_dir(&empty)?;
    let (mut command, _secret_file) = engines::mongo("mongorestore", secret)?;
    command
        .args(["--oplogReplay", "--stopOnError"])
        .arg(format!("--oplogFile={}", path.display()));
    if !until.is_empty() {
        // Entries before the limit are applied: the whole of the last second is wanted.
        command.arg(format!("--oplogLimit={}:0", unix_seconds(until)? + 1));
    }
    let status = command
        .arg(&empty)
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| format!("cannot run mongorestore: {e}"))?;
    if !status.success() {
        return Err(format!("mongorestore failed ({status})").into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `{ ns: namespace, o: {}, ts: Timestamp(seconds, ordinal) }`
    fn document(namespace: &str, seconds: u32, ordinal: u32) -> Vec<u8> {
        let mut body = b"\x02ns\0".to_vec();
        body.extend((namespace.len() as u32 + 1).to_le_bytes());
        body.extend(namespace.bytes());
        body.extend(b"\0\x03o\0\x05\0\0\0\0\x11ts\0");
        body.extend(ordinal.to_le_bytes());
        body.extend(seconds.to_le_bytes());
        body.push(0);
        let mut document = (body.len() as u32 + 4).to_le_bytes().to_vec();
        document.extend(body);
        document
    }

    #[test]
    fn the_newest_entry_wins_and_junk_is_refused() {
        let at = |s, i| document("shop.items", s, i);
        let stream = [at(10, 2), at(11, 1), at(10, 9)].concat();
        let found = newest(stream.as_slice()).unwrap().unwrap();
        assert_eq!(found.to_string(), "11:1");
        assert_eq!(Timestamp::parse("11:1").unwrap(), found);
        assert_eq!(newest(&b""[..]).unwrap(), None);
        assert!(newest(&b"\xff\xff\xff\xffxx"[..]).is_err());
        assert!(newest(&b"\x08\0\0\0\x66a\0\0"[..]).is_err());
        assert!(Timestamp::parse("11").is_err());
    }

    #[test]
    fn accounts_stay_the_targets() {
        let kept = ["shop.items", "shop.$cmd", "admin.$cmd", "", "configx.c"];
        let dropped = [
            "admin.system.users",
            "admin.system.roles",
            "config.system.sessions",
        ];
        let stream: Vec<u8> = kept
            .iter()
            .chain(&dropped)
            .flat_map(|ns| document(ns, 1, 1))
            .collect();
        let mut out = Vec::new();
        assert_eq!(replayable(stream.as_slice(), &mut out).unwrap(), 5);
        let expected: Vec<u8> = kept.iter().flat_map(|ns| document(ns, 1, 1)).collect();
        assert_eq!(out, expected);
    }

    #[test]
    fn times_become_seconds_since_1970() {
        for (text, seconds) in [
            ("1970-01-01 00:00:00", 0),
            ("2000-03-01 00:00:00", 951_868_800),
            ("2024-02-29 12:34:56", 1_709_210_096),
            ("2026-11-28 05:06:25", 1_795_842_385),
        ] {
            assert_eq!(unix_seconds(text).unwrap(), seconds, "{text}");
        }
        assert!(unix_seconds("soon").is_err());
    }
}
