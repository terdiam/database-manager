//! Encryption of sensitive values stored in the database: XChaCha20-Poly1305 under a
//! master key that lives outside it.

use chacha20poly1305::{
    KeyInit, XChaCha20Poly1305,
    aead::{Aead, Payload},
};
use dbm_shared::e2ee::{self, Keypair};
use sqlx::PgPool;

const VERSION: u8 = 1;
/// Which master key sealed a value; lets a later rotation keep old values readable.
const MASTER_KEY_ID: u8 = 1;

pub struct MasterKey(XChaCha20Poly1305);

impl MasterKey {
    /// 32 random bytes, base64 (standard or URL safe).
    pub fn parse(text: &str) -> Result<Self, String> {
        let cleaned = text
            .trim()
            .trim_end_matches('=')
            .replace('+', "-")
            .replace('/', "_");
        let bytes = e2ee::decode(&cleaned).map_err(|_| "the master key is not valid base64")?;
        let cipher = XChaCha20Poly1305::new_from_slice(&bytes)
            .map_err(|_| "the master key must be 32 bytes (openssl rand -base64 32)")?;
        Ok(Self(cipher))
    }

    /// Reads `DBM_MASTER_KEY`, or the file named by `DBM_MASTER_KEY_FILE`.
    pub fn from_env() -> Result<Self, String> {
        let direct = std::env::var("DBM_MASTER_KEY")
            .ok()
            .filter(|v| !v.is_empty());
        let text = match direct {
            Some(text) => text,
            None => {
                let path = std::env::var("DBM_MASTER_KEY_FILE")
                    .map_err(|_| "DBM_MASTER_KEY is required (openssl rand -base64 32)")?;
                std::fs::read_to_string(&path).map_err(|e| format!("cannot read {path}: {e}"))?
            }
        };
        Self::parse(&text)
    }

    /// `row` ties the ciphertext to its place, so it cannot be copied to another record.
    fn context(table: &str, column: &str, row: &str) -> Vec<u8> {
        e2ee::aad(&[
            &[VERSION, MASTER_KEY_ID],
            table.as_bytes(),
            column.as_bytes(),
            row.as_bytes(),
        ])
    }

    /// Output: `version | key id | nonce (24) | ciphertext`.
    pub fn seal(
        &self,
        table: &str,
        column: &str,
        row: &str,
        plain: &[u8],
    ) -> Result<Vec<u8>, String> {
        let nonce: [u8; 24] = rand::random();
        let aad = Self::context(table, column, row);
        let sealed = self
            .0
            .encrypt(
                &nonce.into(),
                Payload {
                    msg: plain,
                    aad: &aad,
                },
            )
            .map_err(|_| "encryption failed")?;
        Ok([&[VERSION, MASTER_KEY_ID][..], &nonce, &sealed].concat())
    }

    pub fn open(
        &self,
        table: &str,
        column: &str,
        row: &str,
        data: &[u8],
    ) -> Result<Vec<u8>, String> {
        let fail = || "cannot decrypt: wrong DBM_MASTER_KEY or damaged data".to_string();
        let [VERSION, MASTER_KEY_ID, rest @ ..] = data else {
            return Err(fail());
        };
        let (nonce, sealed) = rest.split_first_chunk::<24>().ok_or_else(fail)?;
        let aad = Self::context(table, column, row);
        self.0
            .decrypt(
                &(*nonce).into(),
                Payload {
                    msg: sealed,
                    aad: &aad,
                },
            )
            .map_err(|_| fail())
    }
}

/// The server's transport key, created on first start. Opening it also proves the
/// master key matches this database, so a wrong key stops the server at once.
pub async fn load_transport_key(db: &PgPool, master: &MasterKey) -> Result<Keypair, String> {
    let fail = |e: sqlx::Error| e.to_string();
    let mut tx = db.begin().await.map_err(fail)?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext('dbm-server-keys'))")
        .execute(&mut *tx)
        .await
        .map_err(fail)?;
    let stored: Option<(Vec<u8>, Vec<u8>)> =
        sqlx::query_as("SELECT public_key, secret FROM server_keys WHERE name = 'transport'")
            .fetch_optional(&mut *tx)
            .await
            .map_err(fail)?;
    let key = match stored {
        Some((public, secret)) => Keypair {
            private: master.open("server_keys", "secret", "transport", &secret)?,
            public,
        },
        None => {
            let key = Keypair::generate();
            let sealed = master.seal("server_keys", "secret", "transport", &key.private)?;
            sqlx::query(
                "INSERT INTO server_keys (name, public_key, secret) VALUES ('transport', $1, $2)",
            )
            .bind(&key.public)
            .bind(sealed)
            .execute(&mut *tx)
            .await
            .map_err(fail)?;
            key
        }
    };
    tx.commit().await.map_err(fail)?;
    Ok(key)
}
