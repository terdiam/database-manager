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

pub const ENGINES: [&str; 5] = ["postgresql", "mysql", "mariadb", "mongodb", "sqlite"];

/// What a dump of `engine` is, before compression and encryption.
pub fn dump_extension(engine: &str) -> &'static str {
    match engine {
        "mysql" | "mariadb" | "sqlite" => ".sql",
        "mongodb" => ".archive",
        _ => ".dump",
    }
}

/// A database name, or for SQLite a file name: it may carry an extension but no path.
pub fn valid_target(engine: &str, name: &str) -> bool {
    if engine != "sqlite" {
        return valid_database_name(name);
    }
    (1..=63).contains(&name.len())
        && !name.starts_with(['-', '.'])
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
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

/// Plaintext of a sealed S3 destination secret.
#[derive(Serialize, Deserialize)]
pub struct S3Secret {
    pub endpoint: String,
    pub region: String,
    pub bucket: String,
    pub prefix: String,
    pub access_key: String,
    pub secret_key: String,
    #[serde(default)]
    pub virtual_host: bool,
    /// Address browsers use to download, when `endpoint` is only reachable by the agent.
    #[serde(default)]
    pub public_endpoint: String,
}

/// A value only this agent can open (HPKE, base64url) and what it is bound to.
#[derive(Serialize, Deserialize)]
pub struct Sealed {
    pub id: String,
    pub version: u32,
    pub data: String,
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
    },
    TestDatabase {
        database: Sealed,
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
