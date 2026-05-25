//! The shared [`BrowserProfile`] type.
//!
//! See architecture doc §3 and §5 (divergence #7): one struct is read by the
//! HTTP client (for TLS shape + UA + Sec-CH-UA headers) and, in later
//! phases, by every Web API binding that returns environment-derived data
//! (Navigator, Screen, Canvas, WebGL, AudioContext). One source of truth =
//! drift between HTTP-layer and JS-layer fingerprints is a *type error*, not
//! a runtime bug.
//!
//! Phase 0 only populates the HTTP-layer fields. Canvas / WebGL / Audio
//! fingerprint bytes are added in Phase 2 when the JS layer comes online.

#![deny(unsafe_code)]
#![warn(missing_docs)]

mod chrome_linux;

/// The OS half of a profile identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum ProfileOS {
    Linux,
    Windows,
    MacOS,
}

impl ProfileOS {
    /// String the User-Agent grammar expects (`X11; Linux x86_64` etc.).
    pub fn ua_token(self) -> &'static str {
        match self {
            ProfileOS::Linux => "X11; Linux x86_64",
            ProfileOS::Windows => "Windows NT 10.0; Win64; x64",
            ProfileOS::MacOS => "Macintosh; Intel Mac OS X 10_15_7",
        }
    }
}

/// Screen dimensions exposed to the JS layer in later phases.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub struct ScreenProfile {
    pub width: u32,
    pub height: u32,
    pub avail_width: u32,
    pub avail_height: u32,
    pub color_depth: u8,
    pub pixel_depth: u8,
}

/// The wreq-util emulation key that selects the TLS+HTTP/2 ClientHello shape.
///
/// Kept as an enum (not the raw `wreq_util::Emulation`) so this crate stays
/// HTTP-stack agnostic — `mach-net` translates this into the wreq type at
/// the boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum TlsEmulation {
    Chrome131,
}

/// One concrete "who we look like to the wire" profile.
///
/// `'static` everywhere because every profile in v1 is compiled in as
/// static data. User-supplied profiles in later versions will switch the
/// `&'static str` fields to owned `String`s.
#[derive(Debug, Clone)]
pub struct BrowserProfile {
    /// Stable id, e.g. `"chrome-linux-131"`. Used as the CLI flag value
    /// (`--profile=chrome-linux`) and for telemetry.
    pub id: &'static str,

    /// Underlying TLS+HTTP/2 fingerprint shape.
    pub tls: TlsEmulation,

    /// OS half of the identity.
    pub os: ProfileOS,

    /// Full `User-Agent` header value.
    pub user_agent: &'static str,

    /// `Sec-CH-UA` client hints header. Chrome's value is
    /// `"Not?A_Brand";v="99", "Chromium";v="131", "Google Chrome";v="131"`
    /// or similar.
    pub sec_ch_ua: &'static str,

    /// `Sec-CH-UA-Platform` value, e.g. `"Linux"`.
    pub sec_ch_ua_platform: &'static str,

    /// `Accept-Language` header.
    pub accept_language: &'static str,

    /// Languages exposed via `navigator.languages` in later phases.
    pub languages: &'static [&'static str],

    /// `navigator.hardwareConcurrency`.
    pub hardware_concurrency: u32,

    /// `navigator.deviceMemory` in GiB.
    pub device_memory_gb: u8,

    /// Screen geometry.
    pub screen: ScreenProfile,

    /// IANA timezone identifier.
    pub timezone: &'static str,
}

/// Registry of all known profiles. Phase 0 ships one.
pub struct Registry;

impl Registry {
    /// All profiles shipped in this build.
    pub fn all() -> &'static [BrowserProfile] {
        &[chrome_linux::PROFILE]
    }

    /// Look up by id.
    pub fn get(id: &str) -> Option<BrowserProfile> {
        Self::all().iter().find(|p| p.id == id).cloned()
    }

    /// The default profile applied when the user doesn't pass `--profile`.
    pub fn default_profile() -> BrowserProfile {
        chrome_linux::PROFILE.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_profile_is_chrome_linux() {
        let p = Registry::default_profile();
        assert_eq!(p.id, "chrome-linux-131");
        assert!(p.user_agent.contains("Chrome/131"));
        assert!(p.user_agent.contains("Linux x86_64"));
        assert_eq!(p.os, ProfileOS::Linux);
        assert_eq!(p.tls, TlsEmulation::Chrome131);
    }

    #[test]
    fn registry_lookup_roundtrip() {
        for p in Registry::all() {
            assert!(Registry::get(p.id).is_some());
        }
        assert!(Registry::get("does-not-exist").is_none());
    }
}
