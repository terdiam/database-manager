//! Work the server hands to agents, and what agents report back.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const CLAIM_PATH: &str = "/api/agent/jobs/claim";
pub const REPORT_PATH: &str = "/api/agent/jobs/report";
pub const FINISH_PATH: &str = "/api/agent/jobs/finish";
/// A running job whose agent stays silent this long is marked failed.
pub const LEASE_SECS: i32 = 60;
pub const MAX_LOG_LINES: usize = 200;

pub const COMPRESSIONS: [&str; 6] = ["zstd", "gzip", "lz4", "xz", "bzip2", "none"];

pub fn extension(compression: &str) -> &'static str {
    match compression {
        "zstd" => ".zst",
        "gzip" => ".gz",
        "lz4" => ".lz4",
        "xz" => ".xz",
        "bzip2" => ".bz2",
        _ => "",
    }
}

pub const ENGINES: [&str; 8] = [
    "postgresql",
    "mysql",
    "mariadb",
    "mongodb",
    "sqlite",
    "redis",
    "elasticsearch",
    "opensearch",
];

/// Redis has no named databases: a backup is the whole instance.
pub const REDIS_ALL: &str = "all";
/// Every index; Elasticsearch and OpenSearch reserve the name, so no index can have it.
pub const SEARCH_ALL: &str = "_all";

/// What a dump of `engine` is, before compression and encryption.
pub fn dump_extension(engine: &str) -> &'static str {
    match engine {
        "mysql" | "mariadb" | "sqlite" => ".sql",
        "mongodb" => ".archive",
        "redis" => ".rdb",
        "elasticsearch" | "opensearch" => ".ndjson",
        _ => ".dump",
    }
}

/// A MongoDB backup of the whole instance. Only that one can be followed by incrementals:
/// the oplog it continues with cannot be replayed for one database.
pub const MONGO_ALL: &str = "_all";

/// Backups that incremental ones can follow.
pub fn can_follow(engine: &str, database: &str) -> bool {
    matches!(engine, "mysql" | "mariadb") || (engine == "mongodb" && database == MONGO_ALL)
}

/// What a job may name: a database, for SQLite a file name without a path, for Redis the
/// whole instance, for the search engines an index or all of them.
pub fn valid_target(engine: &str, name: &str) -> bool {
    match engine {
        "sqlite" => {}
        "redis" => return name == REDIS_ALL,
        "elasticsearch" | "opensearch" => return name == SEARCH_ALL || valid_index_name(name),
        _ => return valid_database_name(name),
    }
    (1..=63).contains(&name.len())
        && !name.starts_with(['-', '.'])
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
}

/// An index name goes into URL paths: nothing that could change the path or the query,
/// and no hidden or system indices.
pub fn valid_index_name(name: &str) -> bool {
    (1..=63).contains(&name.len())
        && !name.starts_with(['-', '_', '.'])
        && name.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'-' | b'.')
        })
}

/// Deliberately narrow: names reach `pg_restore -d`, which would read `a=b` or a URL
/// as a connection string.
pub fn valid_database_name(name: &str) -> bool {
    (1..=63).contains(&name.len())
        && !name.starts_with('-')
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// Plaintext of a sealed database secret. The browser builds the same JSON.
/// For SQLite, `host` is a folder below the agent's `DBM_SQLITE_ROOT`.
#[derive(Serialize, Deserialize)]
pub struct DatabaseSecret {
    /// Sealed with the rest, so the server cannot make the agent treat it as another engine.
    #[serde(default = "postgresql")]
    pub engine: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub ssl_mode: String,
}

fn postgresql() -> String {
    "postgresql".into()
}

/// Plaintext of a sealed destination secret. Which fields are used depends on `kind`.
#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
pub struct StorageSecret {
    /// `s3` (also when empty, as before the other kinds), `sftp`, `ftp` or `smb`.
    pub kind: String,
    /// S3 only.
    pub endpoint: String,
    pub region: String,
    pub access_key: String,
    pub secret_key: String,
    pub virtual_host: bool,
    /// Address browsers use to download, when `endpoint` is only reachable by the agent.
    pub public_endpoint: String,
    /// S3 bucket or SMB share.
    pub bucket: String,
    /// Folder inside the storage.
    pub prefix: String,
    /// SFTP, FTP and SMB.
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    /// SFTP with a key.
    pub private_key: String,
    pub passphrase: String,
    /// SFTP host key fingerprint (`SHA256:…`); empty trusts the first one seen.
    pub fingerprint: String,
    /// SMB.
    pub domain: String,
    /// FTP over TLS.
    pub tls: bool,
}

/// Plaintext of a sealed SSH secret, used to reach a database through a bastion.
#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
pub struct SshSecret {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub private_key: String,
    pub passphrase: String,
    pub fingerprint: String,
}

/// A value only this agent can open (HPKE, base64url) and what it is bound to.
#[derive(Serialize, Deserialize)]
pub struct Sealed {
    pub id: String,
    pub version: u32,
    pub data: String,
}

/// One incremental file of a chain.
#[derive(Serialize, Deserialize)]
pub struct Increment {
    pub object_key: String,
    pub sha256: String,
    pub manifest: Option<String>,
    pub compression: String,
    /// age identity, sealed like the full backup's.
    pub identity: Sealed,
    pub from: String,
    pub to: String,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Task {
    Backup {
        database: Sealed,
        destination: Sealed,
        name: String,
        compression: String,
        level: i32,
        /// age recipients the file is encrypted to.
        recipients: Vec<String>,
        object_key: String,
        #[serde(default)]
        ssh: Option<Sealed>,
        /// A full backup that increments will follow: record where it ends.
        #[serde(default)]
        track: bool,
        /// An incremental backup: the position its parent ended at.
        #[serde(default)]
        since: Option<String>,
    },
    Restore {
        database: Sealed,
        destination: Sealed,
        /// age identity, sealed for this job only (kind "backup-key", id = job id).
        identity: Sealed,
        object_key: String,
        sha256: String,
        /// The agent's own MAC over the record; absent for snapshots made before it existed.
        manifest: Option<String>,
        compression: String,
        name: String,
        /// The database the backup was taken from.
        #[serde(default)]
        source: String,
        #[serde(default)]
        ssh: Option<Sealed>,
        /// Where the full backup ends, when increments follow it.
        #[serde(default)]
        position: Option<String>,
        /// Applied after the full backup, oldest first.
        #[serde(default)]
        increments: Vec<Increment>,
        /// Stop replaying here (`YYYY-MM-DD HH:MM:SS`, UTC).
        #[serde(default)]
        until: Option<String>,
    },
    TestDatabase {
        database: Sealed,
        #[serde(default)]
        ssh: Option<Sealed>,
    },
    TestSsh {
        ssh: Sealed,
    },
    TestDestination {
        destination: Sealed,
    },
    DeleteSnapshot {
        destination: Sealed,
        object_key: String,
    },
    /// Make a short-lived download link for a stored file.
    PresignSnapshot {
        destination: Sealed,
        object_key: String,
    },
}

#[derive(Serialize, Deserialize)]
pub struct Job {
    pub id: Uuid,
    /// Proves the report comes from the attempt that currently holds the job.
    pub lease: Uuid,
    pub task: Task,
}

#[derive(Serialize, Deserialize)]
pub struct Claim {
    pub job: Option<Job>,
}

#[derive(Serialize, Deserialize)]
pub struct LogLine {
    pub level: String,
    pub message: String,
}

#[derive(Serialize, Deserialize)]
pub struct Report {
    pub id: Uuid,
    pub lease: Uuid,
    /// Bytes moved so far.
    pub bytes: i64,
    pub logs: Vec<LogLine>,
}

#[derive(Serialize, Deserialize)]
pub struct ReportReply {
    pub cancel: bool,
}

#[derive(Serialize, Deserialize)]
pub struct Finish {
    pub id: Uuid,
    pub lease: Uuid,
    pub bytes: i64,
    pub logs: Vec<LogLine>,
    pub error: Option<String>,
    /// SHA-256 (hex) of the stored file, for successful backups.
    pub sha256: Option<String>,
    /// MAC over the stored file's record, for successful backups.
    #[serde(default)]
    pub manifest: Option<String>,
    /// The download link, for presign jobs.
    #[serde(default)]
    pub result: Option<String>,
    /// Log positions of a backup in a chain.
    #[serde(default)]
    pub position_from: Option<String>,
    #[serde(default)]
    pub position_to: Option<String>,
    /// An incremental backup could not continue its parent: a full one is needed.
    #[serde(default)]
    pub chain_broken: bool,
}

#[cfg(test)]
mod tests {
    use super::{valid_database_name, valid_target};

    #[test]
    fn sqlite_targets_are_file_names_only() {
        assert!(valid_target("sqlite", "app.db"));
        assert!(!valid_target("mysql", "app.db"));
        for bad in ["", ".env", "..", "a/b.db", "../x", "-readonly", "a b"] {
            assert!(!valid_target("sqlite", bad), "{bad}");
        }
    }

    #[test]
    fn redis_and_search_targets_cannot_reshape_a_request() {
        assert!(valid_target("redis", "all") && !valid_target("redis", "0"));
        for good in ["_all", "orders", "logs-2026.11"] {
            assert!(valid_target("elasticsearch", good), "{good}");
        }
        for bad in [
            "",
            "*",
            "a,b",
            "a/b",
            "a?x=1",
            "..",
            ".security",
            "_cat",
            "Orders",
            "-x",
        ] {
            assert!(!valid_target("opensearch", bad), "{bad}");
        }
    }

    #[test]
    fn database_names_cannot_smuggle_options_or_connection_strings() {
        assert!(valid_database_name("shop_2026-test"));
        for bad in [
            "",
            "-x",
            "a b",
            "host=evil",
            "postgresql://evil/db",
            "a;drop",
            "../x",
        ] {
            assert!(!valid_database_name(bad), "{bad}");
        }
    }
}
