//! Calls to the server, encrypted when a server key is pinned.

use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use dbm_shared::{E2EE_PATH, E2eeInfo, e2ee};
use reqwest::{Client, StatusCode, Url, header};
use serde::Serialize;

use crate::Result;

pub struct Reply {
    pub status: u16,
    pub body: Vec<u8>,
}

impl Reply {
    pub fn error_code(&self) -> Option<String> {
        let value: serde_json::Value = serde_json::from_slice(&self.body).ok()?;
        value.get("error")?.as_str().map(str::to_owned)
    }
}

pub struct Api {
    client: Client,
    server: Url,
    /// Whether the last encrypted reply said the server reads compressed requests.
    server_gzip: AtomicBool,
}

impl Api {
    pub fn new(server: Url) -> Result<Self> {
        // Redirects are off so the credential is never replayed elsewhere.
        let client = Client::builder()
            .timeout(Duration::from_secs(15))
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        Ok(Self {
            client,
            server,
            server_gzip: AtomicBool::new(false),
        })
    }

    pub async fn e2ee_info(&self) -> Result<E2eeInfo> {
        let url = self.server.join(E2EE_PATH)?;
        Ok(self
            .client
            .get(url)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?)
    }

    /// `server_key` encrypts the exchange; `None` sends it as ordinary TLS traffic.
    pub async fn post(
        &self,
        path: &str,
        server_key: Option<&[u8]>,
        bearer: Option<&str>,
        body: &impl Serialize,
    ) -> Result<Reply> {
        let url = self.server.join(path)?;
        let body = serde_json::to_vec(body)?;
        let authorization = bearer.map_or_else(String::new, |token| format!("Bearer {token}"));

        let Some(server_key) = server_key else {
            let mut request = self
                .client
                .post(url)
                .header(header::CONTENT_TYPE, "application/json")
                .body(body);
            if !authorization.is_empty() {
                request = request.header(header::AUTHORIZATION, authorization);
            }
            let response = request.send().await?;
            let status = response.status().as_u16();
            return Ok(Reply {
                status,
                body: response.bytes().await?.to_vec(),
            });
        };

        let timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as i64;
        let aad = e2ee::request_aad("POST", path, timestamp);
        let gzip = self.server_gzip.load(Ordering::Relaxed);
        let sealed = e2ee::seal_request(server_key, &aad, &authorization, &body, gzip)?;
        let value = format!("1.{timestamp}.{}", e2ee::encode(&sealed.exchange.enc));
        let response = self
            .client
            .post(url)
            .header(e2ee::HEADER, value)
            .header(e2ee::GZIP_HEADER, "1")
            .header(header::CONTENT_TYPE, e2ee::CONTENT_TYPE)
            .body(sealed.ciphertext)
            .send()
            .await?;
        let encrypted = response
            .headers()
            .get(header::CONTENT_TYPE)
            .is_some_and(|v| v.as_bytes() == e2ee::CONTENT_TYPE.as_bytes());
        let status = response.status();
        let gzip = response.headers().contains_key(e2ee::GZIP_HEADER);
        self.server_gzip.store(gzip, Ordering::Relaxed);
        let bytes = response.bytes().await?;
        if encrypted {
            let (status, body) = e2ee::open_response(&sealed.exchange, &bytes)?;
            return Ok(Reply { status, body });
        }
        // Rejected before decryption (or by a proxy). Never treat plain text as success.
        if status == StatusCode::OK || status == StatusCode::NO_CONTENT {
            return Err("server answered an encrypted request in plain text".into());
        }
        Ok(Reply {
            status: status.as_u16(),
            body: bytes.to_vec(),
        })
    }
}
