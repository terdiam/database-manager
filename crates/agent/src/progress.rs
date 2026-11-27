//! What a running job shares with the loop that reports it.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

use dbm_shared::jobs::{LogLine, MAX_LOG_LINES};

use crate::Result;

#[derive(Clone, Default)]
pub struct Progress(Arc<Shared>);

#[derive(Default)]
struct Shared {
    bytes: AtomicU64,
    cancelled: AtomicBool,
    logs: Mutex<Vec<LogLine>>,
}

impl Progress {
    pub fn add(&self, bytes: u64) {
        self.0.bytes.fetch_add(bytes, Ordering::Relaxed);
    }

    pub fn bytes(&self) -> i64 {
        i64::try_from(self.0.bytes.load(Ordering::Relaxed)).unwrap_or(i64::MAX)
    }

    pub fn cancel(&self) {
        self.0.cancelled.store(true, Ordering::Relaxed);
    }

    /// Fails once the job has been cancelled.
    pub fn check(&self) -> Result<()> {
        if self.0.cancelled.load(Ordering::Relaxed) {
            return Err("cancelled".into());
        }
        Ok(())
    }

    pub fn log(&self, level: &str, message: impl Into<String>) {
        let mut logs = self.0.logs.lock().unwrap();
        if logs.len() < MAX_LOG_LINES {
            logs.push(LogLine {
                level: level.into(),
                message: message.into(),
            });
        }
    }

    pub fn take_logs(&self) -> Vec<LogLine> {
        std::mem::take(&mut self.0.logs.lock().unwrap())
    }
}
