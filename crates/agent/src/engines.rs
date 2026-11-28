//! The client tools of each database engine. Passwords travel in the environment or a
//! private file: never on the command line, and tools never see the agent's variables.

use std::{
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use dbm_shared::jobs::{DatabaseSecret, SEARCH_ALL, WHOLE, valid_target};
use tempfile::{NamedTempFile, TempPath};

use crate::{
    Result, helper,
    oplog::{self, Timestamp},
    redis, search,
};

/// A command ready to run, and what has to live or happen around it.
pub struct Tool {
    pub command: Command,
    pub name: &'static str,
    /// Removed when the tool is done with it.
    _secret_file: Option<NamedTempFile>,
    /// SQLite restores into a new file that replaces the target only on success.
    replace: Option<(TempPath, PathBuf)>,
}

impl Tool {
    /// A command whose login is in a file that must outlive it.
    pub fn holding(command: Command, name: &'static str, file: Option<NamedTempFile>) -> Self {
        Self {
            _secret_file: file,
            ..Self::new(command, name)
        }
    }

    fn new(command: Command, name: &'static str) -> Self {
        Self {
            command,
            name,
            _secret_file: None,
            replace: None,
        }
    }

    /// Puts a restored SQLite file in place. Nothing to do for servers.
    pub fn commit(replace: Option<(TempPath, PathBuf)>) -> Result<()> {
        if let Some((new, target)) = replace {
            // Journals of the old file would be applied to the new one.
            for suffix in ["-wal", "-shm", "-journal"] {
                let mut stale = target.clone().into_os_string();
                stale.push(suffix);
                let _ = std::fs::remove_file(stale);
            }
            new.persist(target).map_err(|e| e.error)?;
        }
        Ok(())
    }

    pub fn take_replace(&mut self) -> Option<(TempPath, PathBuf)> {
        self.replace.take()
    }
}

pub fn quiet(program: &str) -> Command {
    let mut command = Command::new(program);
    command
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

/// Runs a short command and returns what it printed.
fn output(mut command: Command, name: &str) -> Result<String> {
    let output = command
        .output()
        .map_err(|e| format!("cannot run {name}: {e}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().into());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn checked<'a>(secret: &DatabaseSecret, name: &'a str) -> Result<&'a str> {
    if valid_target(&secret.engine, name) {
        Ok(name)
    } else {
        Err("invalid database name".into())
    }
}

// ---------------------------------------------------------------- PostgreSQL

const SSL_MODES: [&str; 6] = [
    "disable",
    "allow",
    "prefer",
    "require",
    "verify-ca",
    "verify-full",
];

pub fn postgres(program: &str, secret: &DatabaseSecret, database: &str) -> Result<Command> {
    if !SSL_MODES.contains(&secret.ssl_mode.as_str()) {
        return Err("unknown ssl mode".into());
    }
    let mut command = quiet(program);
    command
        .env("PGHOST", &secret.host)
        .env("PGPORT", secret.port.to_string())
        .env("PGUSER", &secret.username)
        .env("PGPASSWORD", &secret.password)
        .env("PGSSLMODE", &secret.ssl_mode)
        .env("PGDATABASE", database)
        .env("PGCONNECT_TIMEOUT", "10")
        .arg("--no-password");
    Ok(command)
}

fn psql(secret: &DatabaseSecret, sql: &str) -> Result<String> {
    let mut command = postgres("psql", secret, "postgres")?;
    command.args([
        "--no-psqlrc",
        "--tuples-only",
        "--no-align",
        "--command",
        sql,
    ]);
    output(command, "psql")
}

// ---------------------------------------------------------------- MySQL and MariaDB

/// Oracle's tools for MySQL, MariaDB's for MariaDB: their options and dumps differ.
pub fn mysql(program: &str, secret: &DatabaseSecret) -> Result<Command> {
    let maria = secret.engine == "mariadb";
    let ssl: &[&str] = match (secret.ssl_mode.as_str(), maria) {
        ("disable", false) => &["--ssl-mode=DISABLED"],
        ("allow" | "prefer", false) => &["--ssl-mode=PREFERRED"],
        ("require", false) => &["--ssl-mode=REQUIRED"],
        ("verify-ca", false) => &["--ssl-mode=VERIFY_CA"],
        ("verify-full", false) => &["--ssl-mode=VERIFY_IDENTITY"],
        ("disable", true) => &["--skip-ssl"],
        ("allow" | "prefer", true) => &["--skip-ssl-verify-server-cert"],
        ("require", true) => &["--ssl", "--skip-ssl-verify-server-cert"],
        ("verify-ca" | "verify-full", true) => &["--ssl", "--ssl-verify-server-cert"],
        _ => return Err("unknown ssl mode".into()),
    };
    // A value starting with '-' would be read as an option.
    if secret.host.starts_with('-') || secret.username.starts_with('-') {
        return Err("invalid host or user name".into());
    }
    let mut command = quiet(program);
    command
        .env("MYSQL_PWD", &secret.password)
        // Must come first: no option file of the host is read.
        .arg("--no-defaults")
        .arg(format!("--host={}", secret.host))
        .arg(format!("--port={}", secret.port))
        .arg(format!("--user={}", secret.username))
        .arg("--protocol=TCP")
        .args(ssl);
    Ok(command)
}

fn mysql_programs(secret: &DatabaseSecret) -> (&'static str, &'static str) {
    if secret.engine == "mariadb" {
        ("mariadb", "mariadb-dump")
    } else {
        ("mysql", "mysqldump")
    }
}

pub fn mysql_query(secret: &DatabaseSecret, sql: &str) -> Result<String> {
    let (client, _) = mysql_programs(secret);
    let mut command = mysql(client, secret)?;
    command.args([
        "--connect-timeout=10",
        "--batch",
        "--skip-column-names",
        "-e",
        sql,
    ]);
    output(command, client)
}

// ---------------------------------------------------------------- MongoDB

/// The password goes into a file only the agent can read, never onto the command line.
pub fn mongo(program: &str, secret: &DatabaseSecret) -> Result<(Command, Option<NamedTempFile>)> {
    let tls: &[&str] = match secret.ssl_mode.as_str() {
        "disable" | "allow" | "prefer" => &[],
        "require" => &["--ssl", "--tlsInsecure"],
        "verify-ca" | "verify-full" => &["--ssl"],
        _ => return Err("unknown ssl mode".into()),
    };
    let mut command = quiet(program);
    command
        .arg(format!("--host={}", secret.host))
        .arg(format!("--port={}", secret.port))
        .args(tls);
    if secret.username.is_empty() {
        return Ok((command, None));
    }
    let mut file = NamedTempFile::new()?;
    // A JSON string is a valid YAML string.
    writeln!(
        file,
        "password: {}",
        serde_json::to_string(&secret.password)?
    )?;
    command
        .arg(format!("--config={}", file.path().display()))
        .arg(format!("--username={}", secret.username))
        .arg("--authenticationDatabase=admin");
    Ok((command, Some(file)))
}

// ---------------------------------------------------------------- SQLite

/// The folder SQLite files are read from and written to. Without it SQLite is off, so a
/// server cannot ask for any file the agent can read.
fn sqlite_root() -> Result<PathBuf> {
    let root = std::env::var("DBM_SQLITE_ROOT")
        .ok()
        .filter(|v| !v.is_empty())
        .ok_or("SQLite is off on this agent: set DBM_SQLITE_ROOT")?;
    let root = std::fs::canonicalize(root)?;
    let state = std::env::var("DBM_STATE_FILE").unwrap_or_else(|_| "agent-state".into());
    if std::fs::canonicalize(state).is_ok_and(|state| state.starts_with(&root)) {
        return Err("DBM_SQLITE_ROOT must not contain the agent's state file".into());
    }
    Ok(root)
}

/// `<root>/<folder>/<name>`, refused if links or `..` lead outside the root.
fn sqlite_path(root: &Path, folder: &str, name: &str) -> Result<PathBuf> {
    let outside = "that folder or file is outside DBM_SQLITE_ROOT";
    let folder = std::fs::canonicalize(root.join(folder.trim_start_matches('/')))
        .map_err(|e| format!("cannot open the folder: {e}"))?;
    if !folder.starts_with(root) {
        return Err(outside.into());
    }
    let path = folder.join(name);
    match std::fs::canonicalize(&path) {
        Ok(real) if real.starts_with(root) => Ok(real),
        Ok(_) => Err(outside.into()),
        Err(_) if path.symlink_metadata().is_err() => Ok(path),
        Err(_) => Err(outside.into()),
    }
}

fn sqlite(path: &Path) -> Command {
    let mut command = quiet("sqlite3");
    // -safe: nothing in the input can run programs or touch other files.
    command.args(["-batch", "-bail", "-safe"]).arg(path);
    command
}

// ---------------------------------------------------------------- Redis, Elasticsearch, OpenSearch

/// Redis has no dump tool that restores over the network, and the search engines have none
/// at all, so the agent runs itself for those. The login travels in the environment.
fn helper(
    task: &'static str,
    secret: &DatabaseSecret,
    name: &str,
    source: &str,
) -> Result<Command> {
    let mut command = quiet(
        std::env::current_exe()?
            .to_str()
            .ok_or("unreadable program path")?,
    );
    command
        .env(helper::TASK, task)
        .env(helper::SECRET, serde_json::to_string(secret)?)
        .env(helper::NAME, name)
        .env(helper::SOURCE, source);
    // The temporary Redis server and its files go where the agent's own temporaries go.
    if let Some(folder) = std::env::var_os("TMPDIR") {
        command.env("TMPDIR", folder);
    }
    Ok(command)
}

// ---------------------------------------------------------------- what jobs ask for

/// Connects and says what answered.
pub fn test(secret: &DatabaseSecret) -> Result<String> {
    match secret.engine.as_str() {
        "postgresql" => Ok(format!(
            "PostgreSQL {}",
            psql(secret, "SHOW server_version")?
        )),
        "mysql" | "mariadb" => {
            let version = mysql_query(secret, "SELECT VERSION()")?;
            Ok(format!("{} {version}", secret.engine))
        }
        "mongodb" => {
            // Dumping a database that does not exist needs a connection and a login, nothing else.
            let (mut command, _file) = mongo("mongodump", secret)?;
            command.args(["--quiet", "--db=dbm_connection_test", "--archive=/dev/null"]);
            output(command, "mongodump")?;
            Ok("MongoDB".into())
        }
        "sqlite" => {
            let root = sqlite_root()?;
            let folder = sqlite_path(&root, &secret.host, ".")?;
            if !folder.is_dir() {
                return Err("that folder does not exist".into());
            }
            Ok(format!("SQLite folder {}", folder.display()))
        }
        "redis" => redis::test(secret),
        "elasticsearch" | "opensearch" => search::test(secret),
        _ => Err("unknown engine".into()),
    }
}

/// An incremental backup: the log files since the parent, fetched by the agent itself.
pub fn dump_increment(secret: &DatabaseSecret, files: &[String]) -> Result<Tool> {
    let mut command = helper(helper::BINLOG_DUMP, secret, "", "")?;
    command.env(helper::ARGS, serde_json::to_string(files)?);
    Ok(Tool::new(command, "log dump"))
}

/// A physical backup of a whole PostgreSQL cluster. Its manifest is left in `keep`;
/// `since` is the parent's manifest for an incremental one.
pub fn dump_cluster(secret: &DatabaseSecret, keep: &Path, since: Option<&Path>) -> Result<Tool> {
    let mut command = helper(helper::BASE_DUMP, secret, "", "")?;
    command.env(helper::ARGS, serde_json::to_string(&(keep, since))?);
    Ok(Tool::new(command, "base backup"))
}

/// MongoDB's incremental backup: the oplog between two positions.
pub fn dump_oplog(secret: &DatabaseSecret, since: &str, to: &str) -> Result<Tool> {
    oplog::dump(secret, Timestamp::parse(since)?, Timestamp::parse(to)?)
}

/// Replays one incremental backup; it reads the file from standard input.
pub fn restore_increment(
    secret: &DatabaseSecret,
    name: &str,
    source: &str,
    from: &str,
    to: &str,
    until: &str,
) -> Result<Tool> {
    let name = checked(secret, name)?;
    let task = if secret.engine == "mongodb" {
        helper::OPLOG_APPLY
    } else {
        helper::BINLOG_APPLY
    };
    let mut command = helper(task, secret, name, source)?;
    command.env(helper::ARGS, serde_json::to_string(&[from, to, until])?);
    command.stdin(Stdio::piped()).stdout(Stdio::null());
    Ok(Tool::new(command, "log replay"))
}

/// A chain is replayed onto what its full backup creates, so nothing else may be there.
pub fn require_empty(secret: &DatabaseSecret, name: &str) -> Result<()> {
    let name = checked(secret, name)?;
    // The name is restricted to [A-Za-z0-9_-], so quoting it cannot be escaped.
    let tables = mysql_query(
        secret,
        &format!("SELECT COUNT(*) FROM information_schema.tables WHERE table_schema = '{name}'"),
    )?;
    if tables != "0" {
        return Err(
            "the target database is not empty: a chain restores into a new or empty one".into(),
        );
    }
    Ok(())
}

/// A command that writes the dump of `name` to standard output. `track` records the log
/// position in it, for increments to start from.
pub fn dump(secret: &DatabaseSecret, name: &str, track: bool) -> Result<Tool> {
    let name = checked(secret, name)?;
    Ok(match secret.engine.as_str() {
        "postgresql" => {
            let mut command = postgres("pg_dump", secret, name)?;
            // Uncompressed: compression is done by the agent's own stage.
            command.args(["--format=custom", "--compress=0"]);
            Tool::new(command, "pg_dump")
        }
        "mysql" | "mariadb" => {
            let (_, program) = mysql_programs(secret);
            let mut command = mysql(program, secret)?;
            command.args([
                "--single-transaction",
                "--routines",
                "--triggers",
                "--events",
                "--hex-blob",
                "--no-tablespaces",
            ]);
            if secret.engine == "mysql" {
                command.arg("--set-gtid-purged=OFF");
            }
            if track {
                // Written as a comment: restoring the dump does not act on it.
                command.arg(if secret.engine == "mysql" {
                    "--source-data=2"
                } else {
                    "--master-data=2"
                });
            }
            // Without --databases the dump can be restored under another name.
            command.arg(name);
            Tool::new(command, program)
        }
        "mongodb" => {
            let (mut command, file) = mongo("mongodump", secret)?;
            command.arg("--quiet").arg("--archive");
            if name != WHOLE {
                command.arg(format!("--db={name}"));
            }
            Tool::holding(command, "mongodump", file)
        }
        "sqlite" => {
            let path = sqlite_path(&sqlite_root()?, &secret.host, name)?;
            // sqlite3 would create an empty database for a name that is not there.
            if !path.is_file() {
                return Err(format!("{name} does not exist").into());
            }
            let mut command = sqlite(&path);
            command.arg(".dump");
            Tool::new(command, "sqlite3")
        }
        "redis" => Tool::new(
            helper(helper::REDIS_DUMP, secret, name, name)?,
            "redis dump",
        ),
        "elasticsearch" | "opensearch" => Tool::new(
            helper(helper::SEARCH_DUMP, secret, name, name)?,
            "index dump",
        ),
        _ => return Err("unknown engine".into()),
    })
}

/// Creates the target if needed and returns the command that reads the dump from
/// standard input. `source` is the name the backup was taken from. The flag says
/// whether a database was created.
pub fn restore(secret: &DatabaseSecret, name: &str, source: &str) -> Result<(Tool, bool)> {
    let name = checked(secret, name)?;
    let (mut tool, created) = match secret.engine.as_str() {
        "postgresql" => {
            // The name is restricted to [A-Za-z0-9_-], so quoting it cannot be escaped.
            let found = psql(
                secret,
                &format!("SELECT 1 FROM pg_database WHERE datname = '{name}'"),
            )?;
            if found != "1" {
                psql(secret, &format!("CREATE DATABASE \"{name}\""))?;
            }
            let mut command = postgres("pg_restore", secret, name)?;
            command
                .args([
                    "--clean",
                    "--if-exists",
                    "--no-owner",
                    "--no-privileges",
                    "--exit-on-error",
                ])
                .arg(format!("--dbname={name}"));
            (Tool::new(command, "pg_restore"), found != "1")
        }
        "mysql" | "mariadb" => {
            let found = mysql_query(
                secret,
                &format!("SELECT 1 FROM information_schema.schemata WHERE schema_name = '{name}'"),
            )?;
            if found != "1" {
                mysql_query(secret, &format!("CREATE DATABASE `{name}`"))?;
            }
            let (client, _) = mysql_programs(secret);
            let mut command = mysql(client, secret)?;
            // Binary mode: the client runs no commands of its own that appear in the data.
            command.args(["--binary-mode", &format!("--database={name}")]);
            (Tool::new(command, client), found != "1")
        }
        "mongodb" => {
            if !valid_target("mongodb", source) {
                return Err("the source database name is missing".into());
            }
            if (source == WHOLE) != (name == WHOLE) {
                return Err(
                    format!("a backup of the whole instance is restored to {WHOLE}").into(),
                );
            }
            let (mut command, file) = mongo("mongorestore", secret)?;
            // Not --quiet: it would also hide why a restore failed.
            command.args(["--archive", "--drop", "--stopOnError"]);
            if source == WHOLE {
                // Accounts and the server's own records are the target's: replacing them
                // would end the session that is restoring.
                command
                    .arg("--nsExclude=admin.*")
                    .arg("--nsExclude=config.*");
            } else {
                command
                    .arg(format!("--nsInclude={source}.*"))
                    .arg(format!("--nsFrom={source}.*"))
                    .arg(format!("--nsTo={name}.*"));
            }
            (Tool::holding(command, "mongorestore", file), false)
        }
        "sqlite" => {
            let target = sqlite_path(&sqlite_root()?, &secret.host, name)?;
            let folder = target.parent().ok_or("no folder")?;
            let new = tempfile::Builder::new()
                .prefix(".dbm-restore-")
                .tempfile_in(folder)
                .map_err(|e| format!("cannot write in the folder: {e}"))?
                .into_temp_path();
            let command = sqlite(&new);
            let created = !target.exists();
            let tool = Tool {
                replace: Some((new, target)),
                ..Tool::new(command, "sqlite3")
            };
            (tool, created)
        }
        "redis" => {
            let command = helper(helper::REDIS_RESTORE, secret, name, source)?;
            (Tool::new(command, "redis restore"), false)
        }
        "elasticsearch" | "opensearch" => {
            if !valid_target(&secret.engine, source) {
                return Err("the source index name is missing".into());
            }
            // One index may be restored under another name; a set of them keeps its names.
            if (name == SEARCH_ALL) != (source == SEARCH_ALL) {
                return Err(format!(
                    "restore a backup of all indices to {SEARCH_ALL}, one index to an index name"
                )
                .into());
            }
            let command = helper(helper::SEARCH_RESTORE, secret, name, source)?;
            (Tool::new(command, "index restore"), false)
        }
        _ => return Err("unknown engine".into()),
    };
    tool.command.stdin(Stdio::piped()).stdout(Stdio::null());
    Ok((tool, created))
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
        }
    }

    #[test]
    fn passwords_never_reach_the_command_line() {
        for (engine, name) in [
            ("postgresql", "shop"),
            ("mysql", "shop"),
            ("mariadb", "shop"),
            ("mongodb", "shop"),
            ("redis", "all"),
            ("elasticsearch", "shop"),
            ("opensearch", "_all"),
        ] {
            let tool = dump(&secret(engine), name, false).unwrap();
            let args: Vec<_> = tool.command.get_args().collect();
            assert!(
                !args.iter().any(|a| a.to_string_lossy().contains("hunter2")),
                "{engine}: {args:?}"
            );
        }
        let mut sneaky = secret("mysql");
        sneaky.host = "--plugin-dir=/tmp".into();
        assert!(dump(&sneaky, "shop", false).is_err());
    }

    #[test]
    fn sqlite_stays_inside_its_folder() {
        let top = tempfile::tempdir().unwrap();
        let root = top.path().join("root");
        std::fs::create_dir_all(root.join("app")).unwrap();
        let root = std::fs::canonicalize(root).unwrap();
        std::fs::write(top.path().join("key"), "secret").unwrap();
        std::os::unix::fs::symlink(top.path().join("key"), root.join("app/link")).unwrap();
        std::os::unix::fs::symlink(top.path().join("gone"), root.join("app/dangling")).unwrap();

        assert!(sqlite_path(&root, "app", "new.db").is_ok());
        assert!(sqlite_path(&root, "/app", "new.db").is_ok());
        for (folder, name) in [
            ("..", "key"),
            ("app/../..", "key"),
            ("app", "link"),
            ("app", "dangling"),
        ] {
            assert!(sqlite_path(&root, folder, name).is_err(), "{folder}/{name}");
        }
    }
}
