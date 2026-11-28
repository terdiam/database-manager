//! A MAC over what was stored, with a key only this agent has. The server keeps it but
//! cannot make one, so it cannot swap a backup for a file of its own before a restore.

use dbm_shared::e2ee;
use hkdf::Hkdf;
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;

use crate::{Result, state::State};

/// Log positions of a file in a chain (`from` empty for its full backup), so that the
/// server cannot leave a link out of a restore or put them in another order.
pub type Positions<'a> = Option<(&'a str, &'a str)>;

fn mac(
    state: &State,
    object_key: &str,
    sha256: &str,
    compression: &str,
    positions: Positions,
) -> Result<Hmac<Sha256>> {
    let private = e2ee::decode(&state.private_key)?;
    let mut key = [0; 32];
    Hkdf::<Sha256>::new(None, &private)
        .expand(b"dbm/v1 manifest key", &mut key)
        .map_err(|_| "cannot derive the manifest key")?;
    let mut mac = Hmac::<Sha256>::new_from_slice(&key)?;
    let mut fields = vec![
        object_key.as_bytes(),
        sha256.as_bytes(),
        compression.as_bytes(),
    ];
    // Files outside a chain are signed as before.
    if let Some((from, to)) = positions {
        fields.extend([from.as_bytes(), to.as_bytes()]);
    }
    mac.update(&e2ee::aad(&fields));
    Ok(mac)
}

pub fn sign(
    state: &State,
    object_key: &str,
    sha256: &str,
    compression: &str,
    positions: Positions,
) -> Result<String> {
    let tag = mac(state, object_key, sha256, compression, positions)?.finalize();
    Ok(e2ee::encode(&tag.into_bytes()))
}

pub fn verify(
    state: &State,
    object_key: &str,
    sha256: &str,
    compression: &str,
    positions: Positions,
    tag: &str,
) -> Result<()> {
    mac(state, object_key, sha256, compression, positions)?
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
        let tag = sign(&mine, "db/x.age", "abc", "zstd", None).unwrap();
        assert!(verify(&mine, "db/x.age", "abc", "zstd", None, &tag).is_ok());
        let linked = sign(&mine, "db/x.age", "abc", "zstd", Some(("a", "b"))).unwrap();
        assert!(verify(&mine, "db/x.age", "abc", "zstd", Some(("a", "b")), &linked).is_ok());
        for positions in [None, Some(("a", "c")), Some(("x", "b"))] {
            assert!(verify(&mine, "db/x.age", "abc", "zstd", positions, &linked).is_err());
        }
        for (key, sha, compression) in [
            ("db/y.age", "abc", "zstd"),
            ("db/x.age", "abd", "zstd"),
            ("db/x.age", "abc", "none"),
        ] {
            assert!(verify(&mine, key, sha, compression, None, &tag).is_err());
        }
        assert!(verify(&state(&[8; 32]), "db/x.age", "abc", "zstd", None, &tag).is_err());
    }
}
