//! HTTP client for mach-browser.
//!
//! Wraps [`wreq::Client`] with a Chrome TLS+HTTP/2 fingerprint chosen from
//! [`mach_profile::BrowserProfile`]. Phase 0 ships one client for the
//! `mach fetch` subcommand; later phases add CDP/MCP consumers, layered
//! interceptors, and a shared cookie jar.
//!
//! See architecture doc §3 `net::HttpClient` and §0.7 (pre-Phase-0 spike).

#![deny(unsafe_code)]
#![warn(missing_docs)]

use std::time::Duration;

use mach_core::{Error, Result};
use mach_profile::{BrowserProfile, TlsEmulation};
use tracing::debug;
use url::Url;
use wreq::redirect::Policy;
use wreq_util::{Emulation, EmulationOS, EmulationOption};

/// A single HTTP response after redirects + decompression.
#[derive(Debug, Clone)]
pub struct Response {
    /// HTTP status code.
    pub status: u16,
    /// Final URL after any redirects.
    pub final_url: Url,
    /// Response body bytes (decompressed).
    pub body: Vec<u8>,
    /// `Content-Type`, if the server sent one.
    pub content_type: Option<String>,
}

impl Response {
    /// Try to interpret the body as UTF-8 text.
    ///
    /// Returns the lossily-decoded string. Phase 0 is intentionally
    /// optimistic about encodings; later phases pull in `encoding_rs` and
    /// the `<meta charset>` discovery rules from html5ever.
    pub fn body_text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

/// Wrapper around `wreq::Client` seeded from a [`BrowserProfile`].
///
/// Owns the HTTP/2 connection pool; clone to share. Single instance per
/// `App` for Phase 0.
#[derive(Clone)]
pub struct HttpClient {
    inner: wreq::Client,
    profile: BrowserProfile,
}

impl HttpClient {
    /// Build a client emulating the given profile.
    ///
    /// `timeout` applies per-request including all redirects.
    pub fn new(profile: BrowserProfile, timeout: Duration) -> Result<Self> {
        let emu = EmulationOption::builder()
            .emulation(tls_emulation_to_wreq(profile.tls))
            .emulation_os(EmulationOS::Linux)
            .build();
        let inner = wreq::Client::builder()
            .emulation(emu)
            .timeout(timeout)
            .redirect(Policy::limited(8))
            .build()
            .map_err(|e| Error::Network(format!("build wreq client: {e}")))?;
        Ok(Self { inner, profile })
    }

    /// Returns the profile this client was constructed with.
    pub fn profile(&self) -> &BrowserProfile {
        &self.profile
    }

    /// GET `url`. Returns the body and the final post-redirect URL.
    pub async fn get(&self, url: &str) -> Result<Response> {
        // Validate the URL upfront so error mapping is consistent.
        let parsed: Url = url
            .parse()
            .map_err(|e| Error::InvalidArguments(format!("bad URL {url:?}: {e}")))?;

        debug!(target: "mach_net", %parsed, "GET");
        let resp = self
            .inner
            .get(parsed.as_str())
            .send()
            .await
            .map_err(|e| Error::Network(format!("send: {e}")))?;

        let status = resp.status().as_u16();
        // wreq exposes the final URL as an `http::Uri`; convert to `url::Url`
        // so downstream consumers (`mach_agent::links::collect`) can use it as
        // a join base.
        let final_url: Url = resp
            .uri()
            .to_string()
            .parse()
            .map_err(|e| Error::Network(format!("parse final URL: {e}")))?;
        let content_type = resp
            .headers()
            .get(wreq::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_owned());

        let body = resp
            .bytes()
            .await
            .map_err(|e| Error::Network(format!("read body: {e}")))?
            .to_vec();

        Ok(Response {
            status,
            final_url,
            body,
            content_type,
        })
    }
}

fn tls_emulation_to_wreq(t: TlsEmulation) -> Emulation {
    match t {
        TlsEmulation::Chrome131 => Emulation::Chrome131,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_with_default_profile() {
        let p = mach_profile::Registry::default_profile();
        let c = HttpClient::new(p, Duration::from_secs(5)).expect("build");
        assert_eq!(c.profile().id, "chrome-linux-131");
    }
}
