//! Install browser-shaped globals onto a fresh V8 context.
//!
//! Phase 1B: `window`, `navigator`, `location`, and `console`. No DOM yet
//! (that's Phase 1C). Everything here is either a data object set up from
//! constants in [`BrowserProfile`] or a callback that routes into `tracing`.

use mach_profile::{BrowserProfile, ProfileOS};

/// Install all Phase 1B browser globals onto `scope`'s current context.
///
/// Assumes the caller has already entered the target [`v8::Context`] (via
/// [`v8::ContextScope::new`]).
pub fn install(scope: &mut v8::HandleScope, profile: &BrowserProfile, location_href: &str) {
    let context = scope.get_current_context();
    let global = context.global(scope);

    install_window(scope, &global);
    install_navigator(scope, &global, profile);
    install_location(scope, &global, location_href);
    install_console(scope, &global);
}

/// `window === globalThis` and `window.window === window`.
///
/// This is the simplest of the Phase 1B globals but it's the one Turnstile
/// reads first to disambiguate Node / Deno / random JS runtimes from a
/// "real" browser. The cost is one property write.
fn install_window(scope: &mut v8::HandleScope, global: &v8::Local<v8::Object>) {
    let key = v8::String::new(scope, "window").unwrap();
    global.set(scope, key.into(), (*global).into());
}

/// `navigator` — userAgent, platform, language(s), hardwareConcurrency,
/// deviceMemory, webdriver.
///
/// All values come from [`BrowserProfile`]; nothing here may read the host
/// OS, host CPU count, or any environment that could leak the fact that
/// we're not actually running on the OS we're impersonating (arch doc §5
/// divergence #7).
fn install_navigator(
    scope: &mut v8::HandleScope,
    global: &v8::Local<v8::Object>,
    profile: &BrowserProfile,
) {
    let nav = v8::Object::new(scope);

    set_str(scope, &nav, "userAgent", profile.user_agent);
    set_str(scope, &nav, "appName", "Netscape");
    set_str(scope, &nav, "appCodeName", "Mozilla");
    set_str(
        scope,
        &nav,
        "appVersion",
        strip_mozilla_prefix(profile.user_agent),
    );
    set_str(scope, &nav, "product", "Gecko");
    set_str(scope, &nav, "productSub", "20030107");
    set_str(scope, &nav, "vendor", "Google Inc.");
    set_str(scope, &nav, "vendorSub", "");
    set_str(scope, &nav, "platform", navigator_platform(profile.os));
    set_str(
        scope,
        &nav,
        "language",
        profile.languages.first().copied().unwrap_or("en-US"),
    );

    // languages → frozen Array of strings, matching Chrome's behaviour.
    let lang_locals: Vec<v8::Local<v8::Value>> = profile
        .languages
        .iter()
        .map(|s| v8::String::new(scope, s).unwrap().into())
        .collect();
    let langs = v8::Array::new_with_elements(scope, &lang_locals);
    let key = v8::String::new(scope, "languages").unwrap();
    nav.set(scope, key.into(), langs.into());

    // hardwareConcurrency / deviceMemory are numbers, not strings.
    let key = v8::String::new(scope, "hardwareConcurrency").unwrap();
    let value = v8::Number::new(scope, f64::from(profile.hardware_concurrency));
    nav.set(scope, key.into(), value.into());

    let key = v8::String::new(scope, "deviceMemory").unwrap();
    let value = v8::Number::new(scope, f64::from(profile.device_memory_gb));
    nav.set(scope, key.into(), value.into());

    // navigator.webdriver MUST be false for any stealth play. Phase 2
    // may delete the property outright (Chrome no longer ships it for
    // non-automated sessions in some channels) but this is the safe
    // baseline.
    let key = v8::String::new(scope, "webdriver").unwrap();
    let value = v8::Boolean::new(scope, false);
    nav.set(scope, key.into(), value.into());

    // onLine — true by default; we always have network in mach.
    let key = v8::String::new(scope, "onLine").unwrap();
    let value = v8::Boolean::new(scope, true);
    nav.set(scope, key.into(), value.into());

    // cookieEnabled — pretend yes. We do not actually persist cookies
    // yet, but Turnstile reads this and a `false` value is suspicious.
    let key = v8::String::new(scope, "cookieEnabled").unwrap();
    let value = v8::Boolean::new(scope, true);
    nav.set(scope, key.into(), value.into());

    let key = v8::String::new(scope, "doNotTrack").unwrap();
    let null = v8::null(scope);
    nav.set(scope, key.into(), null.into());

    let key = v8::String::new(scope, "navigator").unwrap();
    global.set(scope, key.into(), nav.into());
}

/// `location` parsed from `href` — protocol, host, origin, pathname,
/// search, hash.
///
/// We use a minimal hand-roll instead of pulling in `url` crate at the
/// binding layer because (a) the parsing surface is tiny, (b) we want to
/// preserve the exact href the caller passed (round-tripping through `url`
/// would normalize things like default ports), and (c) `url` is already
/// reachable in mach-net, which is the right place to bind `URL` /
/// `URLSearchParams` constructors when we install them in Phase 1D.
fn install_location(scope: &mut v8::HandleScope, global: &v8::Local<v8::Object>, href: &str) {
    let parts = parse_location(href);
    let loc = v8::Object::new(scope);

    set_str(scope, &loc, "href", &parts.href);
    set_str(scope, &loc, "protocol", &parts.protocol);
    set_str(scope, &loc, "host", &parts.host);
    set_str(scope, &loc, "hostname", &parts.hostname);
    set_str(scope, &loc, "port", &parts.port);
    set_str(scope, &loc, "pathname", &parts.pathname);
    set_str(scope, &loc, "search", &parts.search);
    set_str(scope, &loc, "hash", &parts.hash);
    set_str(scope, &loc, "origin", &parts.origin);

    let key = v8::String::new(scope, "location").unwrap();
    global.set(scope, key.into(), loc.into());
}

/// `console.log/info/warn/error/debug` — each just stringifies its args
/// and writes them to `tracing` at the matching level. Returns `undefined`.
fn install_console(scope: &mut v8::HandleScope, global: &v8::Local<v8::Object>) {
    let console = v8::Object::new(scope);

    set_callback(scope, &console, "log", console_log);
    set_callback(scope, &console, "info", console_info);
    set_callback(scope, &console, "warn", console_warn);
    set_callback(scope, &console, "error", console_error);
    set_callback(scope, &console, "debug", console_debug);
    set_callback(scope, &console, "trace", console_debug);

    let key = v8::String::new(scope, "console").unwrap();
    global.set(scope, key.into(), console.into());
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

fn set_str(scope: &mut v8::HandleScope, obj: &v8::Local<v8::Object>, name: &str, value: &str) {
    let k = v8::String::new(scope, name).unwrap();
    let v = v8::String::new(scope, value).unwrap();
    obj.set(scope, k.into(), v.into());
}

fn set_callback(
    scope: &mut v8::HandleScope,
    obj: &v8::Local<v8::Object>,
    name: &str,
    callback: impl v8::MapFnTo<v8::FunctionCallback>,
) {
    let k = v8::String::new(scope, name).unwrap();
    let tmpl = v8::FunctionTemplate::new(scope, callback);
    let f = tmpl.get_function(scope).unwrap();
    obj.set(scope, k.into(), f.into());
}

fn stringify_args(scope: &mut v8::HandleScope, args: &v8::FunctionCallbackArguments) -> String {
    let mut parts = Vec::with_capacity(args.length() as usize);
    for i in 0..args.length() {
        let val = args.get(i);
        parts.push(val.to_rust_string_lossy(scope));
    }
    parts.join(" ")
}

fn console_log(
    scope: &mut v8::HandleScope,
    args: v8::FunctionCallbackArguments,
    _rv: v8::ReturnValue,
) {
    let msg = stringify_args(scope, &args);
    tracing::info!(target: "mach_js::console", "{}", msg);
}

fn console_info(
    scope: &mut v8::HandleScope,
    args: v8::FunctionCallbackArguments,
    _rv: v8::ReturnValue,
) {
    let msg = stringify_args(scope, &args);
    tracing::info!(target: "mach_js::console", "{}", msg);
}

fn console_warn(
    scope: &mut v8::HandleScope,
    args: v8::FunctionCallbackArguments,
    _rv: v8::ReturnValue,
) {
    let msg = stringify_args(scope, &args);
    tracing::warn!(target: "mach_js::console", "{}", msg);
}

fn console_error(
    scope: &mut v8::HandleScope,
    args: v8::FunctionCallbackArguments,
    _rv: v8::ReturnValue,
) {
    let msg = stringify_args(scope, &args);
    tracing::error!(target: "mach_js::console", "{}", msg);
}

fn console_debug(
    scope: &mut v8::HandleScope,
    args: v8::FunctionCallbackArguments,
    _rv: v8::ReturnValue,
) {
    let msg = stringify_args(scope, &args);
    tracing::debug!(target: "mach_js::console", "{}", msg);
}

/// `Mozilla/5.0 (X11; Linux x86_64) ...` → `5.0 (X11; Linux x86_64) ...`,
/// to match Chrome's `navigator.appVersion` value.
fn strip_mozilla_prefix(ua: &'static str) -> &'static str {
    ua.strip_prefix("Mozilla/").unwrap_or(ua)
}

/// `navigator.platform` value for the given OS. Differs from
/// [`ProfileOS::ua_token`] — Chrome serializes `platform` as a bare
/// architecture string (e.g. `Linux x86_64`, `Win32`, `MacIntel`) without
/// the parenthesised UA-token framing.
fn navigator_platform(os: ProfileOS) -> &'static str {
    match os {
        ProfileOS::Linux => "Linux x86_64",
        ProfileOS::Windows => "Win32",
        ProfileOS::MacOS => "MacIntel",
    }
}

#[derive(Debug, Default)]
struct LocationParts {
    href: String,
    protocol: String,
    host: String,
    hostname: String,
    port: String,
    pathname: String,
    search: String,
    hash: String,
    origin: String,
}

/// Bare-bones URL splitter for `location.*`.
///
/// Not a full WHATWG URL parser — see crate docs on why we don't pull in
/// `url` here. Recognised shapes:
///
/// - `<scheme>://<host>[:<port>][/path][?search][#hash]`
/// - `about:blank` and other opaque schemes — only `href`/`protocol`
///   meaningfully populated.
fn parse_location(href: &str) -> LocationParts {
    let mut parts = LocationParts {
        href: href.to_string(),
        ..LocationParts::default()
    };

    let Some(scheme_end) = href.find(':') else {
        return parts;
    };
    let scheme = &href[..scheme_end];
    parts.protocol = format!("{scheme}:");

    let after_scheme = &href[scheme_end + 1..];
    let Some(rest) = after_scheme.strip_prefix("//") else {
        // Opaque scheme like `about:blank`. Nothing else to split.
        return parts;
    };

    // Split off fragment first so it doesn't end up in `search`.
    let (rest, hash) = match rest.find('#') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, ""),
    };
    parts.hash = hash.to_string();

    // Then split off query.
    let (rest, search) = match rest.find('?') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, ""),
    };
    parts.search = search.to_string();

    // host + pathname.
    let (host_str, pathname) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    parts.pathname = pathname.to_string();
    parts.host = host_str.to_string();

    // hostname + port.
    if let Some(colon) = host_str.rfind(':') {
        parts.hostname = host_str[..colon].to_string();
        parts.port = host_str[colon + 1..].to_string();
    } else {
        parts.hostname = host_str.to_string();
        parts.port.clear();
    }

    parts.origin = format!("{}//{}", parts.protocol, parts.host);

    parts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_https_with_path_and_query() {
        let p = parse_location("https://example.com/foo?q=1#h");
        assert_eq!(p.protocol, "https:");
        assert_eq!(p.host, "example.com");
        assert_eq!(p.hostname, "example.com");
        assert_eq!(p.port, "");
        assert_eq!(p.pathname, "/foo");
        assert_eq!(p.search, "?q=1");
        assert_eq!(p.hash, "#h");
        assert_eq!(p.origin, "https://example.com");
    }

    #[test]
    fn parse_with_explicit_port() {
        let p = parse_location("http://localhost:8080/");
        assert_eq!(p.host, "localhost:8080");
        assert_eq!(p.hostname, "localhost");
        assert_eq!(p.port, "8080");
        assert_eq!(p.pathname, "/");
    }

    #[test]
    fn parse_about_blank_is_opaque() {
        let p = parse_location("about:blank");
        assert_eq!(p.href, "about:blank");
        assert_eq!(p.protocol, "about:");
        assert_eq!(p.host, "");
        assert_eq!(p.origin, "");
    }

    #[test]
    fn parse_path_without_query_or_hash() {
        let p = parse_location("https://example.com/foo/bar");
        assert_eq!(p.pathname, "/foo/bar");
        assert_eq!(p.search, "");
        assert_eq!(p.hash, "");
    }
}
