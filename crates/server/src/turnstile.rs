//! Cloudflare Turnstile. Off unless both keys are configured.

use std::time::Duration;

use serde::Deserialize;

use crate::Error;

pub const VERIFY_URL: &str = "https://challenges.cloudflare.com/turnstile/v0/siteverify";

pub struct Turnstile {
    pub site_key: String,
    secret: String,
    /// Hostnames the widget may be solved on; lowercase, never empty.
    hostnames: Vec<String>,
    verify_url: String,
    client: reqwest::Client,
}

#[derive(Deserialize)]
struct Verdict {
    success: bool,
    #[serde(default)]
    action: String,
    #[serde(default)]
    hostname: String,
    #[serde(default, rename = "error-codes")]
    error_codes: Vec<String>,
}

impl Turnstile {
    /// `None` when `hostnames` (comma separated) has no entry: nothing could ever be accepted.
    pub fn new(
        site_key: String,
        secret: String,
        hostnames: &str,
        verify_url: String,
    ) -> Option<Self> {
        let hostnames: Vec<String> = hostnames
            .split(',')
            .map(|h| h.trim().to_ascii_lowercase())
            .filter(|h| !h.is_empty())
            .collect();
        if hostnames.is_empty() {
            return None;
        }
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("TLS backend");
        Some(Self {
            site_key,
            secret,
            hostnames,
            verify_url,
            client,
        })
    }

    /// Fails closed: a missing token, a rejection or an unreachable verifier all refuse the request.
    pub async fn verify(&self, token: Option<&str>, action: &str, ip: &str) -> Result<(), Error> {
        // Cloudflare caps tokens at 2048 characters.
        let token = token
            .filter(|t| !t.is_empty() && t.len() <= 2048)
            .ok_or(Error::Invalid("captcha"))?;
        let form = [
            ("secret", self.secret.as_str()),
            ("response", token),
            ("remoteip", ip),
        ];
        let verdict: Verdict = async {
            self.client
                .post(&self.verify_url)
                .form(&form)
                .send()
                .await?
                .error_for_status()?
                .json()
                .await
        }
        .await
        .map_err(|e: reqwest::Error| {
            tracing::warn!("Turnstile verification unavailable: {e}");
            Error::Invalid("captcha_unavailable")
        })?;
        // A token solved for another form or on another site must not work here.
        if verdict.success
            && verdict.action == action
            && self
                .hostnames
                .contains(&verdict.hostname.to_ascii_lowercase())
        {
            Ok(())
        } else {
            tracing::info!(
                "Turnstile rejected a request: success={} action={:?} hostname={:?} errors={:?}",
                verdict.success,
                verdict.action,
                verdict.hostname,
                verdict.error_codes
            );
            Err(Error::Invalid("captcha"))
        }
    }
}
