//! HTTP client for `api.frisky.fm/v3`.
//!
//! Everything here runs on the tokio runtime, never on the GTK main thread.
//! Results reach the UI as `AppEvent`s over an async channel.

pub mod model;
pub mod nowplaying;

use anyhow::{Context, Result};
use std::time::Duration;

use crate::channel::{Channel, Quality};
use model::{Artist, Show, StationsResponse, StreamValidation};

pub const API_BASE: &str = "https://api.frisky.fm/v3";

/// Identifies the app honestly. FRISKY does not filter on User-Agent — the 401
/// that looks like bot-blocking is really the subscription paywall on the
/// higher-bitrate mounts — so there is nothing to spoof.
const USER_AGENT: &str = concat!("frisky-gtk/", env!("CARGO_PKG_VERSION"));

/// Generous ceiling for a 1200x1200 JPEG/PNG. The URL is API-controlled, but a
/// bad or compromised endpoint must not be able to grow the process without
/// bound through a chunked response.
const MAX_ARTWORK_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone)]
pub struct FriskyClient {
    http: reqwest::Client,
}

impl FriskyClient {
    pub fn new() -> Result<Self> {
        let http = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(Duration::from_secs(20))
            .connect_timeout(Duration::from_secs(10))
            .build()
            .context("building HTTP client")?;
        Ok(Self { http })
    }

    async fn get_json<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T> {
        let url = format!("{API_BASE}/{path}");
        let response = self
            .http
            .get(&url)
            .send()
            .await
            .with_context(|| format!("requesting {url}"))?
            .error_for_status()
            .with_context(|| format!("unexpected status from {url}"))?;

        response
            .json()
            .await
            .with_context(|| format!("decoding response from {url}"))
    }

    /// All four channels with their currently-airing mix, in one request.
    pub async fn stations(&self) -> Result<StationsResponse> {
        self.get_json("stations").await
    }

    /// A show, which carries the 1200x1200 album art used as cover.
    pub async fn show(&self, id: u64) -> Result<Show> {
        self.get_json(&format!("shows/{id}")).await
    }

    pub async fn artist(&self, id: u64) -> Result<Artist> {
        self.get_json(&format!("artists/{id}")).await
    }

    /// Downloads raw artwork bytes from wherever the API points (S3 or
    /// CloudFront, not the API host).
    pub async fn fetch_bytes(&self, url: &str) -> Result<Vec<u8>> {
        let mut response = self
            .http
            .get(url)
            .send()
            .await
            .with_context(|| format!("requesting {url}"))?
            .error_for_status()
            .with_context(|| format!("unexpected status from {url}"))?;

        if let Some(length) = response.content_length() {
            anyhow::ensure!(
                length <= MAX_ARTWORK_BYTES as u64,
                "artwork response is too large ({length} bytes)"
            );
        }

        let capacity = response.content_length().unwrap_or(0) as usize;
        let mut bytes = Vec::with_capacity(capacity.min(MAX_ARTWORK_BYTES));
        while let Some(chunk) = response.chunk().await? {
            append_bounded(&mut bytes, &chunk, MAX_ARTWORK_BYTES)?;
        }
        Ok(bytes)
    }

    /// Exchanges credentials for a subscriber token.
    ///
    /// The response field name is not documented. The web client just stores
    /// whatever comes back, so accept the shapes it could plausibly be and let
    /// the caller log the raw body if none match.
    pub async fn login(&self, email: &str, password: &str) -> Result<String> {
        let url = format!("{API_BASE}/auth/token");
        let body = serde_json::json!({ "email": email, "password": password });

        let response = self.http.post(&url).json(&body).send().await?;
        let status = response.status();
        let value: serde_json::Value = response.json().await.unwrap_or(serde_json::Value::Null);

        if !status.is_success() {
            anyhow::bail!(
                "login failed ({}): {}",
                status,
                extract_error_message(&value).unwrap_or_else(|| "no message".into())
            );
        }

        extract_token(&value).ok_or_else(|| {
            anyhow::anyhow!(
                "login succeeded but no token field was recognised; body keys: {:?}",
                object_keys(&value)
            )
        })
    }

    /// Asks whether `token` may open a given mount. The free tier answers
    /// `{"allowed": false}` for the premium mounts.
    pub async fn validate_stream(
        &self,
        token: &str,
        channel: Channel,
        quality: Quality,
    ) -> Result<bool> {
        let url = format!("{API_BASE}/subscriptions/validate-streaming");
        let response = self
            .http
            .get(&url)
            .query(&[
                ("token", token),
                ("station", channel.id()),
                ("mount", quality.mount()),
            ])
            .send()
            .await?;

        // An error status means "not entitled", not "app is broken".
        if !response.status().is_success() {
            return Ok(false);
        }
        let validation: StreamValidation = response
            .json()
            .await
            .unwrap_or(StreamValidation { allowed: false });
        Ok(validation.allowed)
    }
}

fn append_bounded(destination: &mut Vec<u8>, chunk: &[u8], limit: usize) -> Result<()> {
    anyhow::ensure!(
        chunk.len() <= limit.saturating_sub(destination.len()),
        "artwork response exceeds {limit} bytes"
    );
    destination.extend_from_slice(chunk);
    Ok(())
}

/// Pulls a token out of the login response, tolerating the shapes the API
/// might use.
fn extract_token(value: &serde_json::Value) -> Option<String> {
    const KEYS: [&str; 4] = ["token", "access_token", "accessToken", "auth_token"];

    // Check the top level, then a `body`/`data` envelope.
    for root in [Some(value), value.get("body"), value.get("data")]
        .into_iter()
        .flatten()
    {
        for key in KEYS {
            if let Some(token) = root.get(key).and_then(|t| t.as_str()) {
                if !token.is_empty() {
                    return Some(token.to_owned());
                }
            }
        }
    }
    // Some endpoints return the token as a bare string.
    value.as_str().filter(|s| !s.is_empty()).map(str::to_owned)
}

fn extract_error_message(value: &serde_json::Value) -> Option<String> {
    for key in ["message", "error", "detail"] {
        if let Some(message) = value.get(key).and_then(|m| m.as_str()) {
            return Some(message.to_owned());
        }
    }
    None
}

fn object_keys(value: &serde_json::Value) -> Vec<String> {
    value
        .as_object()
        .map(|o| o.keys().cloned().collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn chunked_artwork_is_bounded_even_without_a_content_length() {
        let mut bytes = vec![0; 4];
        append_bounded(&mut bytes, &[1, 2], 6).unwrap();
        assert_eq!(bytes.len(), 6);
        assert!(append_bounded(&mut bytes, &[3], 6).is_err());
    }

    #[test]
    fn extracts_token_from_known_shapes() {
        assert_eq!(
            extract_token(&json!({"token": "t1"})).as_deref(),
            Some("t1")
        );
        assert_eq!(
            extract_token(&json!({"access_token": "t2"})).as_deref(),
            Some("t2")
        );
        assert_eq!(
            extract_token(&json!({"body": {"token": "t3"}})).as_deref(),
            Some("t3")
        );
        assert_eq!(
            extract_token(&json!({"data": {"accessToken": "t4"}})).as_deref(),
            Some("t4")
        );
        assert_eq!(extract_token(&json!("t5")).as_deref(), Some("t5"));
    }

    #[test]
    fn rejects_missing_or_empty_tokens() {
        assert_eq!(extract_token(&json!({"token": ""})), None);
        assert_eq!(extract_token(&json!({"unrelated": "x"})), None);
        assert_eq!(extract_token(&json!(null)), None);
    }

    #[test]
    fn reports_body_keys_when_token_is_unrecognised() {
        // Drives the diagnostic that would let us pin the real field name.
        let keys = object_keys(&json!({"member": {}, "expires": 1}));
        assert!(keys.contains(&"member".to_string()));
    }

    #[test]
    fn extracts_error_message() {
        assert_eq!(
            extract_error_message(&json!({"message": "bad creds"})).as_deref(),
            Some("bad creds")
        );
        assert_eq!(extract_error_message(&json!({})), None);
    }
}
