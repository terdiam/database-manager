//! Wire types shared by the server and the agent.

pub mod e2ee;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const ENROLL_PATH: &str = "/api/agent/enroll";
pub const HEARTBEAT_PATH: &str = "/api/agent/heartbeat";
pub const E2EE_PATH: &str = "/api/e2ee";
pub const HEARTBEAT_SECS: u64 = 30;
/// An agent counts as online while its last heartbeat is younger than this.
pub const OFFLINE_AFTER_SECS: i64 = 90;

#[derive(Serialize, Deserialize)]
pub struct EnrollRequest {
    pub token: String,
    pub version: String,
    /// X25519 public key that secrets for this agent are sealed to (base64url).
    pub public_key: String,
}

#[derive(Serialize, Deserialize)]
pub struct EnrollResponse {
    pub agent_id: Uuid,
    /// Long-lived credential. Shown once; the server keeps only its hash.
    pub secret: String,
}

#[derive(Serialize, Deserialize)]
pub struct HeartbeatRequest {
    pub version: String,
}

/// Public: tells a client whether to encrypt and to which key.
#[derive(Serialize, Deserialize)]
pub struct E2eeInfo {
    pub required: bool,
    pub public_key: String,
    pub fingerprint: String,
}
