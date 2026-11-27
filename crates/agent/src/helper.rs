//! The agent as its own dump or restore tool: stdin and stdout carry the data, stderr the
//! messages for the job log, the exit status says whether it worked.

use std::{env, process::ExitCode};

use dbm_shared::jobs::DatabaseSecret;

use crate::{Result, redis, search};

pub const TASK: &str = "DBM_HELPER";
pub const SECRET: &str = "DBM_HELPER_SECRET";
pub const NAME: &str = "DBM_HELPER_NAME";
pub const SOURCE: &str = "DBM_HELPER_SOURCE";

pub const REDIS_DUMP: &str = "redis-dump";
pub const REDIS_RESTORE: &str = "redis-restore";
pub const SEARCH_DUMP: &str = "search-dump";
pub const SEARCH_RESTORE: &str = "search-restore";

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
