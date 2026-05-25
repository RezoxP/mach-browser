//! Static "Chrome 131 on Linux" profile.
//!
//! Values mirror what `tls.peet.ws` reports for a real Chrome 131 on
//! Ubuntu, verified during the pre-Phase-0 wreq spike. See architecture doc
//! §0.7 (pre-Phase-0 spike result) for the validation details.

use crate::{BrowserProfile, ProfileOS, ScreenProfile, TlsEmulation};

pub(crate) const PROFILE: BrowserProfile = BrowserProfile {
    id: "chrome-linux-131",
    tls: TlsEmulation::Chrome131,
    os: ProfileOS::Linux,
    user_agent: "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 \
                  (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36",
    sec_ch_ua: "\"Google Chrome\";v=\"131\", \"Chromium\";v=\"131\", \"Not_A Brand\";v=\"24\"",
    sec_ch_ua_platform: "\"Linux\"",
    accept_language: "en-US,en;q=0.9",
    languages: &["en-US", "en"],
    hardware_concurrency: 8,
    device_memory_gb: 8,
    screen: ScreenProfile {
        width: 1920,
        height: 1080,
        avail_width: 1920,
        avail_height: 1080,
        color_depth: 24,
        pixel_depth: 24,
    },
    timezone: "America/Los_Angeles",
};
