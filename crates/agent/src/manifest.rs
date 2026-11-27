//! A MAC over what was stored, with a key only this agent has. The server keeps it but
//! cannot make one, so it cannot swap a backup for a file of its own before a restore.

use dbm_shared::e2ee;
use hkdf::Hkdf;
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;

use crate::{Result, state::State};

fn mac(state: &State, object_key: &str, sha256: &str, compression: &str) -> Result<Hmac<Sha256>> {
    let private = e2ee::decode(&state.private_key)?;
    let mut key = [0; 32];
    Hkdf::<Sha256>::new(None, &private)
        .expand(b"dbm/v1 manifest key", &mut key)
        .map_err(|_| "cannot derive the manifest key")?;
    let mut mac = Hmac::<Sha256>::new_from_slice(&key)?;
    mac.update(&e2ee::aad(&[
        object_key.as_bytes(),
        sha256.as_bytes(),
        compression.as_bytes(),
    ]));
    Ok(mac)
}

pub fn sign(state: &State, object_key: &str, sha256: &str, compression: &str) -> Result<String> {
    let tag = mac(state, object_key, sha256, compression)?.finalize();
    Ok(e2ee::encode(&tag.into_bytes()))
}

pub fn verify(
    state: &State,
    object_key: &str,
    sha256: &str,
    compression: &str,
    tag: &str,
) -> Result<()> {
    mac(state, object_key, sha256, compression)?
        .verify_slice(&e2ee::decode(tag)?)
        .map_err(|_| "the backup record was not signed by this agent: not restoring".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_signed_file_verifies() {
        let state = |private: &[u8]| State {
            version: crate::state::VERSION,
            agent_id: "a".into(),
            secret: "s".into(),
            private_key: e2ee::encode(private),
            public_key: String::new(),
            server_key: None,
        };
        let mine = state(&[7; 32]);
        let tag = sign(&mine, "db/x.age", "abc", "zstd").unwrap();
        assert!(verify(&mine, "db/x.age", "abc", "zstd", &tag).is_ok());
        for (key, sha, compression) in [
            ("db/y.age", "abc", "zstd"),
            ("db/x.age", "abd", "zstd"),
            ("db/x.age", "abc", "none"),
        ] {
            assert!(verify(&mine, key, sha, compression, &tag).is_err());
        }
        assert!(verify(&state(&[8; 32]), "db/x.age", "abc", "zstd", &tag).is_err());
    }
}
