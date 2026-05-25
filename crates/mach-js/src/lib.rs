//! mach-js — V8 isolate + context lifecycle.
//!
//! This crate intentionally exposes a *minimal* surface in Phase 1A:
//!
//! - One process-wide V8 platform, initialized lazily on first use (Tactic
//!   #1 from architecture doc §0.4: the no-JS `mach fetch` path never pays
//!   the platform-init RSS cost).
//! - One [`JsRuntime`] per page, each owning its own [`v8::OwnedIsolate`].
//!   We choose isolate-per-page over context-per-page in Phase 1A for the
//!   simpler ownership story; the shared-isolate / context-per-page tactic
//!   from §0.4 lands in Phase 1C once the 10-page concurrent crawl path is
//!   wired in.
//! - [`JsRuntime::eval`] compiles + runs a script and returns the result
//!   serialized to a Rust `String`. This is enough to (a) prove V8 works
//!   end-to-end on Windows MSVC + Linux GNU, and (b) back the CLI subcommand
//!   `mach js --eval '<expr>'`.
//!
//! Web API bindings (Document, Element, Window, Navigator, fetch, ...) are
//! Phase 1B+ and intentionally live in a separate crate so they can evolve
//! without churning this isolate-lifecycle code.

use std::sync::OnceLock;

use mach_core::{Error, Result};

/// Lazy, one-shot V8 platform initialization.
///
/// V8 requires the platform + global initialization to happen exactly once
/// per process before any isolate is created. We gate it behind a [`OnceLock`]
/// so:
///
/// - Calling code does not have to remember to call any init function.
/// - Subcommands that never construct a [`JsRuntime`] (e.g. `mach fetch
///   --dump html`) never trigger this init and therefore never pay the
///   ~5 ms / ~10 MB RSS cost (Tactic #1).
fn ensure_v8_initialized() {
    static INIT: OnceLock<()> = OnceLock::new();
    INIT.get_or_init(|| {
        let platform = v8::new_default_platform(0, false).make_shared();
        v8::V8::initialize_platform(platform);
        v8::V8::initialize();
    });
}

/// A V8 runtime sized for a single document / page.
///
/// Owns its own [`v8::OwnedIsolate`]. Drop the runtime → drop the isolate →
/// V8 reclaims the heap. This is the unit of memory accounting in mach.
///
/// Phase 1A: no embedder data, no Web API bindings, no event loop. Just
/// `eval`. Subsequent phases extend this struct with a [`v8::Global`]
/// reference to the page's `Context` plus handles to DOM/Window globals.
pub struct JsRuntime {
    isolate: v8::OwnedIsolate,
}

impl JsRuntime {
    /// Construct a new isolate.
    ///
    /// First call in the process also performs lazy V8 platform init.
    pub fn new() -> Self {
        ensure_v8_initialized();
        let isolate = v8::Isolate::new(v8::CreateParams::default());
        Self { isolate }
    }

    /// Compile and run a JavaScript expression / program, returning the
    /// result coerced to a Rust string (via V8's `ToString`).
    ///
    /// Both compile errors and runtime exceptions are mapped to
    /// [`Error::Other`] so the CLI layer can render them. We deliberately
    /// keep error variants coarse in Phase 1A — finer-grained error
    /// categorization (e.g. `JsCompile` vs `JsThrew`) lands when we wire up
    /// the source-map / inspector story.
    pub fn eval(&mut self, source: &str) -> Result<String> {
        let scope = &mut v8::HandleScope::new(&mut self.isolate);
        let context = v8::Context::new(scope, v8::ContextOptions::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let try_catch = &mut v8::TryCatch::new(scope);

        let code = v8::String::new(try_catch, source)
            .ok_or_else(|| Error::Js("v8::String::new returned None".into()))?;
        let script = v8::Script::compile(try_catch, code, None).ok_or_else(|| {
            let msg = format_exception(try_catch);
            Error::Js(format!("compile: {msg}"))
        })?;
        let result = script.run(try_catch).ok_or_else(|| {
            let msg = format_exception(try_catch);
            Error::Js(format!("run: {msg}"))
        })?;
        Ok(result.to_rust_string_lossy(try_catch))
    }
}

impl Default for JsRuntime {
    fn default() -> Self {
        Self::new()
    }
}

fn format_exception(try_catch: &mut v8::TryCatch<v8::HandleScope>) -> String {
    if let Some(exc) = try_catch.exception() {
        exc.to_rust_string_lossy(try_catch)
    } else {
        "(no exception object)".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arithmetic() {
        let mut rt = JsRuntime::new();
        assert_eq!(rt.eval("21 + 21").unwrap(), "42");
    }

    #[test]
    fn string_concat() {
        let mut rt = JsRuntime::new();
        assert_eq!(rt.eval("'foo' + 'bar'").unwrap(), "foobar");
    }

    #[test]
    fn array_reduce() {
        // Smoke test that the GC + array path actually runs, not just
        // constant folding.
        let mut rt = JsRuntime::new();
        let out = rt
            .eval(
                "const xs = [];
                 for (let i = 0; i < 1000; i++) xs.push(i);
                 xs.reduce((a, b) => a + b, 0)",
            )
            .unwrap();
        assert_eq!(out, "499500");
    }

    #[test]
    fn json_round_trip() {
        let mut rt = JsRuntime::new();
        let out = rt.eval("JSON.stringify({ a: 1, b: [2, 3] })").unwrap();
        assert_eq!(out, "{\"a\":1,\"b\":[2,3]}");
    }

    #[test]
    fn compile_error_surfaces() {
        let mut rt = JsRuntime::new();
        let err = rt.eval("syntax !!! error").unwrap_err();
        match err {
            Error::Js(msg) => assert!(
                msg.contains("compile") || msg.contains("SyntaxError"),
                "expected compile/SyntaxError, got: {msg}"
            ),
            other => panic!("expected Error::Js, got {other:?}"),
        }
    }

    #[test]
    fn runtime_throw_surfaces() {
        let mut rt = JsRuntime::new();
        let err = rt.eval("throw new Error('boom')").unwrap_err();
        match err {
            Error::Js(msg) => assert!(
                msg.contains("boom") || msg.contains("Error"),
                "expected Error/boom in message, got: {msg}"
            ),
            other => panic!("expected Error::Js, got {other:?}"),
        }
    }

    #[test]
    fn each_isolate_is_independent() {
        // Two runtimes in the same process must not share state — that's
        // the property we'll rely on to evaluate two pages concurrently
        // without cross-contamination.
        let mut a = JsRuntime::new();
        let mut b = JsRuntime::new();
        a.eval("globalThis.x = 1").unwrap();
        let in_b = b.eval("typeof globalThis.x").unwrap();
        assert_eq!(in_b, "undefined");
    }
}
