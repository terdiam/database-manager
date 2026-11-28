//! The agent as its own dump or restore tool: stdin and stdout carry the data, stderr the
//! messages for the job log, the exit status says whether it worked.

use std::{env, process::ExitCode};

use dbm_shared::jobs::DatabaseSecret;

use crate::{Result, binlog, redis, search};

pub const TASK: &str = "DBM_HELPER";
pub const SECRET: &str = "DBM_HELPER_SECRET";
pub const NAME: &str = "DBM_HELPER_NAME";
pub const SOURCE: &str = "DBM_HELPER_SOURCE";
/// A JSON list, for the helpers that need more than names.
pub const ARGS: &str = "DBM_HELPER_ARGS";

pub const REDIS_DUMP: &str = "redis-dump";
pub const REDIS_RESTORE: &str = "redis-restore";
pub const SEARCH_DUMP: &str = "search-dump";
pub const SEARCH_RESTORE: &str = "search-restore";
pub const BINLOG_DUMP: &str = "binlog-dump";
pub const BINLOG_APPLY: &str = "binlog-apply";

fn task(task: &str) -> Result<()> {
    let secret: DatabaseSecret = serde_json::from_str(&env::var(SECRET)?)?;
    match task {
        REDIS_DUMP => redis::dump(&secret, std::io::stdout().lock()),
        REDIS_RESTORE => redis::restore(&secret, std::io::stdin().lock()),
        SEARCH_DUMP => search::dump(&secret, &env::var(NAME)?, std::io::stdout().lock()),
        SEARCH_RESTORE => search::restore(
            &secret,
            &env::var(NAME)?,
            &env::var(SOURCE)?,
            std::io::stdin().lock(),
        ),
        BINLOG_DUMP => {
            let files: Vec<String> = serde_json::from_str(&env::var(ARGS)?)?;
            binlog::dump(&secret, &files, std::io::stdout().lock())
        }
        BINLOG_APPLY => {
            let [from, to, until]: [String; 3] = serde_json::from_str(&env::var(ARGS)?)?;
            let (name, source) = (env::var(NAME)?, env::var(SOURCE)?);
            let replay = binlog::Replay {
                source: &source,
                target: &name,
                from: binlog::Position::parse(&from)?,
                to: binlog::Position::parse(&to)?,
                until: &until,
            };
            binlog::apply(&secret, &replay, std::io::stdin().lock())
        }
        other => Err(format!("unknown helper {other}").into()),
    }
}

/// `None` when this process was not started as a helper.
pub fn run() -> Option<ExitCode> {
    let name = env::var(TASK).ok()?;
    Some(match task(&name) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    })
}
