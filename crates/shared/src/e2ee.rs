//! HPKE (RFC 9180) with X25519, HKDF-SHA256 and AES-256-GCM, used two ways:
//! request/response encryption (response keys derived as in RFC 9458) and
//! sealing secrets to an agent's public key. The browser mirrors this in `web/src/lib/e2ee.ts`.

use aes_gcm::{Aes256Gcm, KeyInit, aead::Aead};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use hkdf::Hkdf;
use hpke::{
    Deserializable, Kem, OpModeR, OpModeS, Serializable, aead::AesGcm256, kdf::HkdfSha256,
    kem::X25519HkdfSha256, setup_receiver, setup_sender,
};
use sha2::{Digest, Sha256};

pub const HEADER: &str = "x-dbm-e2ee";
pub const CONTENT_TYPE: &str = "application/dbm-e2ee";
/// Accepted clock difference between client and server.
pub const MAX_SKEW_SECS: i64 = 120;
pub const KEY_ID: u8 = 1;

const RESPONSE_LABEL: &[u8] = b"dbm/v1 response";
/// KEM, KDF and AEAD identifiers from RFC 9180.
const SUITE: [u8; 6] = [0x00, 0x20, 0x00, 0x01, 0x00, 0x02];

type Dh = X25519HkdfSha256;

#[derive(Debug, PartialEq, Eq)]
pub struct CryptoError;

impl std::fmt::Display for CryptoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("decryption failed")
    }
}
impl std::error::Error for CryptoError {}

#[derive(Clone)]
pub struct Keypair {
    pub public: Vec<u8>,
    pub private: Vec<u8>,
}

impl Keypair {
    pub fn generate() -> Self {
        let (private, public) = Dh::gen_keypair();
        Self {
            public: public.to_bytes().to_vec(),
            private: private.to_bytes().to_vec(),
        }
    }
}

pub fn encode(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

pub fn decode(text: &str) -> Result<Vec<u8>, CryptoError> {
    URL_SAFE_NO_PAD.decode(text).map_err(|_| CryptoError)
}

/// Short form of a public key that people can compare by eye.
pub fn fingerprint(public: &[u8]) -> String {
    let digest = Sha256::digest(public);
    digest[..8]
        .chunks(2)
        .map(|pair| format!("{:02x}{:02x}", pair[0], pair[1]))
        .collect::<Vec<_>>()
        .join(":")
}

/// Length prefixes keep field boundaries unambiguous.
pub fn aad(fields: &[&[u8]]) -> Vec<u8> {
    let mut out = Vec::new();
    for field in fields {
        out.extend_from_slice(&(field.len() as u32).to_be_bytes());
        out.extend_from_slice(field);
    }
    out
}

fn request_info() -> Vec<u8> {
    let mut info = b"dbm/v1 request\0".to_vec();
    info.push(KEY_ID);
    info.extend_from_slice(&SUITE);
    info
}

/// What a request is bound to: it cannot be replayed against another route or later.
pub fn request_aad(method: &str, path_and_query: &str, timestamp: i64) -> Vec<u8> {
    aad(&[
        method.as_bytes(),
        path_and_query.as_bytes(),
        timestamp.to_string().as_bytes(),
    ])
}

/// Kept by the client to open the reply, and by the server to seal it.
#[derive(Clone)]
pub struct Exchange {
    pub enc: Vec<u8>,
    secret: [u8; 32],
}

pub struct SealedRequest {
    pub exchange: Exchange,
    pub ciphertext: Vec<u8>,
}

pub struct OpenedRequest {
    pub exchange: Exchange,
    /// Value for the `Authorization` header, empty when the caller uses cookies.
    pub authorization: String,
    pub body: Vec<u8>,
}

pub fn seal_request(
    server_public: &[u8],
    aad: &[u8],
    authorization: &str,
    body: &[u8],
) -> Result<SealedRequest, CryptoError> {
    let public = <Dh as Kem>::PublicKey::from_bytes(server_public).map_err(|_| CryptoError)?;
    let (enc, mut sender) =
        setup_sender::<AesGcm256, HkdfSha256, Dh>(&OpModeS::Base, &public, &request_info())
            .map_err(|_| CryptoError)?;
    let length = u16::try_from(authorization.len()).map_err(|_| CryptoError)?;
    let mut plain = length.to_be_bytes().to_vec();
    plain.extend_from_slice(authorization.as_bytes());
    plain.extend_from_slice(body);
    let ciphertext = sender.seal(&plain, aad).map_err(|_| CryptoError)?;
    let mut secret = [0; 32];
    sender
        .export(RESPONSE_LABEL, &mut secret)
        .map_err(|_| CryptoError)?;
    Ok(SealedRequest {
        exchange: Exchange {
            enc: enc.to_bytes().to_vec(),
            secret,
        },
        ciphertext,
    })
}

pub fn open_request(
    server_private: &[u8],
    enc: &[u8],
    aad: &[u8],
    ciphertext: &[u8],
) -> Result<OpenedRequest, CryptoError> {
    let private = <Dh as Kem>::PrivateKey::from_bytes(server_private).map_err(|_| CryptoError)?;
    let encapped = <Dh as Kem>::EncappedKey::from_bytes(enc).map_err(|_| CryptoError)?;
    let mut receiver = setup_receiver::<AesGcm256, HkdfSha256, Dh>(
        &OpModeR::Base,
        &private,
        &encapped,
        &request_info(),
    )
    .map_err(|_| CryptoError)?;
    let plain = receiver.open(ciphertext, aad).map_err(|_| CryptoError)?;
    let mut secret = [0; 32];
    receiver
        .export(RESPONSE_LABEL, &mut secret)
        .map_err(|_| CryptoError)?;

    let (length, rest) = plain.split_at_checked(2).ok_or(CryptoError)?;
    let length = usize::from(u16::from_be_bytes([length[0], length[1]]));
    let (authorization, body) = rest.split_at_checked(length).ok_or(CryptoError)?;
    Ok(OpenedRequest {
        exchange: Exchange {
            enc: enc.to_vec(),
            secret,
        },
        authorization: String::from_utf8(authorization.to_vec()).map_err(|_| CryptoError)?,
        body: body.to_vec(),
    })
}

fn response_cipher(exchange: &Exchange, nonce_seed: &[u8]) -> (Aes256Gcm, [u8; 12]) {
    let salt = [exchange.enc.as_slice(), nonce_seed].concat();
    let hkdf = Hkdf::<Sha256>::new(Some(&salt), &exchange.secret);
    let (mut key, mut nonce) = ([0; 32], [0; 12]);
    hkdf.expand(b"key", &mut key)
        .expect("32 bytes is a valid length");
    hkdf.expand(b"nonce", &mut nonce)
        .expect("12 bytes is a valid length");
    (Aes256Gcm::new(&key.into()), nonce)
}

/// The status travels inside the ciphertext.
pub fn seal_response(
    exchange: &Exchange,
    status: u16,
    body: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    let mut seed = [0; 32];
    getrandom::fill(&mut seed).map_err(|_| CryptoError)?;
    let (cipher, nonce) = response_cipher(exchange, &seed);
    let mut plain = status.to_be_bytes().to_vec();
    plain.extend_from_slice(body);
    let sealed = cipher
        .encrypt(&nonce.into(), plain.as_slice())
        .map_err(|_| CryptoError)?;
    Ok([seed.as_slice(), &sealed].concat())
}

pub fn open_response(exchange: &Exchange, data: &[u8]) -> Result<(u16, Vec<u8>), CryptoError> {
    let (seed, sealed) = data.split_at_checked(32).ok_or(CryptoError)?;
    let (cipher, nonce) = response_cipher(exchange, seed);
    let plain = cipher
        .decrypt(&nonce.into(), sealed)
        .map_err(|_| CryptoError)?;
    let (status, body) = plain.split_at_checked(2).ok_or(CryptoError)?;
    Ok((u16::from_be_bytes([status[0], status[1]]), body.to_vec()))
}

/// Everything a sealed secret is tied to, so the server cannot move it elsewhere.
pub struct SecretContext<'a> {
    pub agent_id: &'a str,
    pub secret_id: &'a str,
    pub kind: &'a str,
    pub version: u32,
}

impl SecretContext<'_> {
    fn aad(&self) -> Vec<u8> {
        aad(&[
            self.agent_id.as_bytes(),
            self.secret_id.as_bytes(),
            self.kind.as_bytes(),
            self.version.to_string().as_bytes(),
        ])
    }
}

const SECRET_INFO: &[u8] = b"dbm/v1 agent secret";

/// Seals to an agent's public key: output is `enc (32 bytes) || ciphertext`.
pub fn seal_secret(
    agent_public: &[u8],
    context: &SecretContext,
    plain: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    let public = <Dh as Kem>::PublicKey::from_bytes(agent_public).map_err(|_| CryptoError)?;
    let (enc, mut sender) =
        setup_sender::<AesGcm256, HkdfSha256, Dh>(&OpModeS::Base, &public, SECRET_INFO)
            .map_err(|_| CryptoError)?;
    let sealed = sender
        .seal(plain, &context.aad())
        .map_err(|_| CryptoError)?;
    Ok([&enc.to_bytes()[..], &sealed].concat())
}

pub fn open_secret(
    agent_private: &[u8],
    context: &SecretContext,
    data: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    let (enc, sealed) = data.split_at_checked(32).ok_or(CryptoError)?;
    let private = <Dh as Kem>::PrivateKey::from_bytes(agent_private).map_err(|_| CryptoError)?;
    let encapped = <Dh as Kem>::EncappedKey::from_bytes(enc).map_err(|_| CryptoError)?;
    let mut receiver = setup_receiver::<AesGcm256, HkdfSha256, Dh>(
        &OpModeR::Base,
        &private,
        &encapped,
        SECRET_INFO,
    )
    .map_err(|_| CryptoError)?;
    receiver
        .open(sealed, &context.aad())
        .map_err(|_| CryptoError)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_and_response_round_trip_and_reject_tampering() {
        let server = Keypair::generate();
        let aad = request_aad("POST", "/api/users", 100);
        let sealed = seal_request(&server.public, &aad, "Bearer x", b"{\"a\":1}").unwrap();
        let enc = sealed.exchange.enc.clone();
        let opened = open_request(&server.private, &enc, &aad, &sealed.ciphertext).unwrap();
        assert_eq!(opened.authorization, "Bearer x");
        assert_eq!(opened.body, b"{\"a\":1}");

        for other in [
            request_aad("GET", "/api/users", 100),
            request_aad("POST", "/api/agents", 100),
            request_aad("POST", "/api/users", 101),
        ] {
            assert!(open_request(&server.private, &enc, &other, &sealed.ciphertext).is_err());
        }
        let mut flipped = sealed.ciphertext.clone();
        flipped[0] ^= 1;
        assert!(open_request(&server.private, &enc, &aad, &flipped).is_err());
        assert!(
            open_request(&Keypair::generate().private, &enc, &aad, &sealed.ciphertext).is_err()
        );

        let reply = seal_response(&opened.exchange, 201, b"done").unwrap();
        assert_eq!(
            open_response(&sealed.exchange, &reply).unwrap(),
            (201, b"done".to_vec())
        );
        let mut bad = reply.clone();
        *bad.last_mut().unwrap() ^= 1;
        assert!(open_response(&sealed.exchange, &bad).is_err());
    }

    #[test]
    fn a_secret_opens_only_in_its_own_context() {
        let agent = Keypair::generate();
        let context = SecretContext {
            agent_id: "a1",
            secret_id: "s1",
            kind: "database",
            version: 1,
        };
        let sealed = seal_secret(&agent.public, &context, b"hunter2").unwrap();
        assert_eq!(
            open_secret(&agent.private, &context, &sealed).unwrap(),
            b"hunter2"
        );
        let moved = SecretContext {
            secret_id: "s2",
            ..context
        };
        assert!(open_secret(&agent.private, &moved, &sealed).is_err());
    }
}
