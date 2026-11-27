//! What the agent must remember: its credential, its keypair and the server key it trusts.

use std::{fs, io::Write, os::unix::fs::OpenOptionsExt, path::Path};

use serde::{Deserialize, Serialize};

pub const VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
pub struct State {
    pub version: u32,
    pub secret: String,
    /// X25519 keypair, base64url. Secrets sealed to it are lost with this file.
    pub private_key: String,
    pub public_key: String,
    /// Server transport key, pinned the first time encryption is used.
    pub server_key: Option<String>,
}

pub fn load(path: &Path) -> Option<State> {
    let state: State = serde_json::from_slice(&fs::read(path).ok()?).ok()?;
    (state.version == VERSION).then_some(state)
}

/// Written beside the target and renamed, so a crash never leaves half a file.
pub fn save(path: &Path, state: &State) -> std::io::Result<()> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        fs::create_dir_all(dir)?;
    }
    let temporary = path.with_extension("tmp");
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&temporary)?;
    file.write_all(&serde_json::to_vec(state)?)?;
    file.sync_all()?;
    fs::rename(temporary, path)
}

/// Fails early when the state cannot be written, before a one-time token is spent.
pub fn check_writable(path: &Path) -> std::io::Result<()> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        fs::create_dir_all(dir)?;
    }
    let temporary = path.with_extension("tmp");
    fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&temporary)?;
    fs::remove_file(temporary)
}
