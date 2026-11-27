//! The PostgreSQL client tools. Everything about the connection travels in environment
//! variables: never on the command line, and without the agent's own variables.

use std::process::{Command, Stdio};

use dbm_shared::jobs::{DatabaseSecret, valid_database_name};

use crate::Result;

const SSL_MODES: [&str; 6] = [
    "disable",
    "allow",
    "prefer",
    "require",
    "verify-ca",
    "verify-full",
];

fn command(program: &str, secret: &DatabaseSecret, database: &str) -> Result<Command> {
    if !SSL_MODES.contains(&secret.ssl_mode.as_str()) {
        return Err("unknown ssl mode".into());
    }
    let mut command = Command::new(program);
    command
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("PGHOST", &secret.host)
        .env("PGPORT", secret.port.to_string())
        .env("PGUSER", &secret.username)
        .env("PGPASSWORD", &secret.password)
        .env("PGSSLMODE", &secret.ssl_mode)
        .env("PGDATABASE", database)
        .env("PGCONNECT_TIMEOUT", "10")
        .arg("--no-password")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    Ok(command)
}

fn query(secret: &DatabaseSecret, sql: &str) -> Result<String> {
    let output = command("psql", secret, "postgres")?
        .args([
            "--no-psqlrc",
            "--tuples-only",
            "--no-align",
            "--command",
            sql,
        ])
        .output()
        .map_err(|e| format!("cannot run psql: {e}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().into());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

pub fn server_version(secret: &DatabaseSecret) -> Result<String> {
    query(secret, "SHOW server_version")
}

fn checked(name: &str) -> Result<&str> {
    if valid_database_name(name) {
        Ok(name)
    } else {
        Err("invalid database name".into())
    }
}

/// Custom format, uncompressed: compression is done by the agent's own stage.
pub fn dump(secret: &DatabaseSecret, name: &str) -> Result<Command> {
    let mut command = command("pg_dump", secret, checked(name)?)?;
    command.args(["--format=custom", "--compress=0"]);
    Ok(command)
}

/// Creates the target database if it is missing.
pub fn ensure_database(secret: &DatabaseSecret, name: &str) -> Result<bool> {
    // The name is restricted to [A-Za-z0-9_-], so quoting it cannot be escaped.
    let name = checked(name)?;
    if query(
        secret,
        &format!("SELECT 1 FROM pg_database WHERE datname = '{name}'"),
    )? == "1"
    {
        return Ok(false);
    }
    query(secret, &format!("CREATE DATABASE \"{name}\""))?;
    Ok(true)
}

/// Reads the dump from standard input.
pub fn restore(secret: &DatabaseSecret, name: &str) -> Result<Command> {
    let name = checked(name)?;
    let mut command = command("pg_restore", secret, name)?;
    command
        .args([
            "--clean",
            "--if-exists",
            "--no-owner",
            "--no-privileges",
            "--exit-on-error",
        ])
        .arg(format!("--dbname={name}"))
        .stdin(Stdio::piped())
        .stdout(Stdio::null());
    Ok(command)
}
