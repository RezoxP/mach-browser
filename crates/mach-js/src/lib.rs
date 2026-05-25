//! mach-js — V8 isolate + persistent-context lifecycle.
//!
//! Phase 1B surface:
//!
//! - One process-wide V8 platform, initialized lazily on first use (Tactic
//!   #1 from architecture doc §0.4: the no-JS `mach fetch` path never pays
//!   the platform-init RSS cost).
//! - One [`JsRuntime`] per page. Owns one [`v8::OwnedIsolate`] plus one
//!   persistent [`v8::Global<v8::Context>`] — multiple [`JsRuntime::eval`]
//!   calls share the same realm and therefore the same `globalThis`,
//!   declared bindings, prototype chain, etc. (Phase 1A used a throwaway
//!   context per eval, which made any persistent script state impossible.)
//! - Browser globals installed at construction time:
//!     - `window === globalThis`
//!     - `navigator` (userAgent / platform / language / languages /
//!       hardwareConcurrency / deviceMemory — sourced from
//!       [`mach_profile::BrowserProfile`] so HTTP-layer and JS-layer
//!       fingerprints can never drift; arch doc §5 divergence #7).
//!     - `location` (href / origin / protocol / host / pathname — set
//!       from the page URL passed to the builder; defaults to
//!       `about:blank` when unset).
//!     - `console.log/info/warn/error/debug` — route through Rust's
//!       `tracing` so JS output is observable like any other mach log
//!       channel.
//!
//! Web API bindings that need the DOM (Document, Element, EventTarget,
//! fetch, ...) are Phase 1C+ and live in separate crates so they can
//! evolve without churning this isolate-lifecycle code.

#![deny(unsafe_code)]
#![warn(missing_docs)]

use std::sync::OnceLock;

use mach_core::{Error, Result};
use mach_dom::Document;
use mach_profile::{BrowserProfile, Registry};

mod bindings;
mod dom;

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
/// Owns its own [`v8::OwnedIsolate`] + persistent [`v8::Global<v8::Context>`].
/// Drop the runtime → drop the context → drop the isolate → V8 reclaims the
/// heap. This is the unit of memory accounting in mach.
pub struct JsRuntime {
    isolate: v8::OwnedIsolate,
    /// Persistent handle to the page's realm. Held as a `Global` so it can
    /// outlive any individual `HandleScope` while still being routed back
    /// into one in [`eval`].
    context: v8::Global<v8::Context>,
}

impl JsRuntime {
    /// Construct a runtime with the default profile and no page URL
    /// (`location.href === 'about:blank'`).
    ///
    /// First call in the process also performs lazy V8 platform init.
    pub fn new() -> Self {
        Self::builder().build()
    }

    /// Start a builder for a fully-specified runtime.
    pub fn builder() -> JsRuntimeBuilder {
        JsRuntimeBuilder::default()
    }

    /// Compile and run a JavaScript program in the runtime's persistent
    /// context, returning the completion value coerced to a Rust `String`
    /// via V8's `ToString`.
    ///
    /// Compile errors and runtime exceptions map to [`Error::Js`] with the
    /// formatted exception message.
    pub fn eval(&mut self, source: &str) -> Result<String> {
        let scope = &mut v8::HandleScope::new(&mut self.isolate);
        let context = v8::Local::new(scope, self.context.clone());
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

/// Build a [`JsRuntime`] with overridden profile / location / document.
#[derive(Default)]
pub struct JsRuntimeBuilder {
    profile: Option<BrowserProfile>,
    location: Option<String>,
    document: Option<Document>,
}

impl JsRuntimeBuilder {
    /// Override the [`BrowserProfile`] (defaults to [`Registry::default_profile`]).
    pub fn profile(mut self, profile: BrowserProfile) -> Self {
        self.profile = Some(profile);
        self
    }

    /// Set `location.href` for the page. Defaults to `about:blank`.
    pub fn location(mut self, href: impl Into<String>) -> Self {
        self.location = Some(href.into());
        self
    }

    /// Attach a parsed [`Document`] as the page's `document` global.
    ///
    /// When set, the runtime exposes the read-only DOM surface (Phase
    /// 1C): `document`, `Element`, `Text` accessors and methods. When
    /// unset, `document` is not defined and JS that reads it sees
    /// `undefined`.
    pub fn document(mut self, document: Document) -> Self {
        self.document = Some(document);
        self
    }

    /// Finalize and construct the runtime.
    pub fn build(self) -> JsRuntime {
        ensure_v8_initialized();
        let profile = self.profile.unwrap_or_else(Registry::default_profile);
        let location = self.location.unwrap_or_else(|| "about:blank".to_string());
        let document = self.document;

        let mut isolate = v8::Isolate::new(v8::CreateParams::default());
        let context = {
            let scope = &mut v8::HandleScope::new(&mut isolate);
            let context = v8::Context::new(scope, v8::ContextOptions::default());
            let scope = &mut v8::ContextScope::new(scope, context);

            bindings::install(scope, &profile, &location);
            if let Some(doc) = document {
                dom::install(scope, doc, &location);
            }

            v8::Global::new(scope, context)
        };

        JsRuntime { isolate, context }
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

    // Some tests share a runtime to exercise context persistence; others
    // build a fresh one to assert isolation.

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
        // Two runtimes in the same process must not share state.
        let mut a = JsRuntime::new();
        let mut b = JsRuntime::new();
        a.eval("globalThis.x = 1").unwrap();
        let in_b = b.eval("typeof globalThis.x").unwrap();
        assert_eq!(in_b, "undefined");
    }

    #[test]
    fn context_persists_across_eval_calls() {
        // The headline Phase 1B property: state survives between evals
        // because both share the same v8::Global<Context>.
        let mut rt = JsRuntime::new();
        rt.eval("var counter = 0;").unwrap();
        rt.eval("counter += 1; counter += 1;").unwrap();
        rt.eval("counter += 1;").unwrap();
        assert_eq!(rt.eval("counter").unwrap(), "3");
    }

    #[test]
    fn function_declarations_persist() {
        let mut rt = JsRuntime::new();
        rt.eval("function double(n) { return n * 2 }").unwrap();
        assert_eq!(rt.eval("double(21)").unwrap(), "42");
    }

    #[test]
    fn window_aliases_global_this() {
        let mut rt = JsRuntime::new();
        assert_eq!(rt.eval("window === globalThis").unwrap(), "true");
        assert_eq!(rt.eval("window.window === window").unwrap(), "true");
    }

    #[test]
    fn navigator_exposes_profile_fields() {
        let mut rt = JsRuntime::new();
        assert_eq!(rt.eval("typeof navigator").unwrap(), "object");
        assert!(rt
            .eval("navigator.userAgent")
            .unwrap()
            .contains("Chrome/131"));
        assert_eq!(rt.eval("navigator.platform").unwrap(), "Linux x86_64");
        assert_eq!(rt.eval("navigator.language").unwrap(), "en-US");
        // languages is an Array; just check the first element survives
        // the V8 → Rust roundtrip.
        assert_eq!(rt.eval("navigator.languages[0]").unwrap(), "en-US");
        assert_eq!(
            rt.eval("typeof navigator.hardwareConcurrency").unwrap(),
            "number"
        );
        // navigator.webdriver MUST be false for any stealth strategy to
        // have a chance. Phase 2 may go further (delete the property),
        // but this is the baseline.
        assert_eq!(rt.eval("navigator.webdriver").unwrap(), "false");
    }

    #[test]
    fn location_defaults_to_about_blank() {
        let mut rt = JsRuntime::new();
        assert_eq!(rt.eval("location.href").unwrap(), "about:blank");
    }

    #[test]
    fn location_reflects_builder_url() {
        let mut rt = JsRuntime::builder()
            .location("https://example.com/foo/bar?q=1#x")
            .build();
        assert_eq!(
            rt.eval("location.href").unwrap(),
            "https://example.com/foo/bar?q=1#x"
        );
        assert_eq!(rt.eval("location.protocol").unwrap(), "https:");
        assert_eq!(rt.eval("location.host").unwrap(), "example.com");
        assert_eq!(rt.eval("location.pathname").unwrap(), "/foo/bar");
        assert_eq!(rt.eval("location.search").unwrap(), "?q=1");
        assert_eq!(rt.eval("location.hash").unwrap(), "#x");
        assert_eq!(rt.eval("location.origin").unwrap(), "https://example.com");
    }

    #[test]
    fn console_log_does_not_throw_and_returns_undefined() {
        // We route console.log to tracing; from JS it returns undefined.
        let mut rt = JsRuntime::new();
        let out = rt.eval("console.log('hi', 1, { a: 2 }); 42").unwrap();
        assert_eq!(out, "42");
    }

    #[test]
    fn console_all_levels_callable() {
        let mut rt = JsRuntime::new();
        for level in ["log", "info", "warn", "error", "debug"] {
            let out = rt.eval(&format!("console.{level}('x'); 1")).unwrap();
            assert_eq!(out, "1");
        }
    }

    // -----------------------------------------------------------------
    // Phase 1C: DOM bindings
    // -----------------------------------------------------------------

    /// Build a small fixture: `<html><head><title>T</title></head>
    /// <body><p id="x" class="a b">hi <b>there</b></p></body></html>`.
    fn fixture_document() -> mach_dom::Document {
        use mach_dom::{Attr, NodeId, NodeKind};
        let mut d = mach_dom::Document::new();
        let html = d.push(
            NodeId::ROOT,
            NodeKind::Element {
                name: "html".into(),
                attrs: vec![],
            },
        );
        let head = d.push(
            html,
            NodeKind::Element {
                name: "head".into(),
                attrs: vec![],
            },
        );
        let title = d.push(
            head,
            NodeKind::Element {
                name: "title".into(),
                attrs: vec![],
            },
        );
        d.push(title, NodeKind::Text("T".into()));
        let body = d.push(
            html,
            NodeKind::Element {
                name: "body".into(),
                attrs: vec![],
            },
        );
        let p = d.push(
            body,
            NodeKind::Element {
                name: "p".into(),
                attrs: vec![
                    Attr {
                        name: "id".into(),
                        value: "x".into(),
                    },
                    Attr {
                        name: "class".into(),
                        value: "a b".into(),
                    },
                ],
            },
        );
        d.push(p, NodeKind::Text("hi ".into()));
        d.push(
            p,
            NodeKind::Element {
                name: "b".into(),
                attrs: vec![],
            },
        );
        // Note: <b>'s text is added as a sibling of <b>, not as a child,
        // for variety in the test — that's fine, it just exercises a
        // different code path.
        let b = d.node(p).children[1];
        d.push(b, NodeKind::Text("there".into()));
        d
    }

    fn fixture_runtime() -> JsRuntime {
        JsRuntime::builder()
            .location("https://example.com/page?x=1")
            .document(fixture_document())
            .build()
    }

    #[test]
    fn document_is_installed_when_builder_has_document() {
        let mut rt = fixture_runtime();
        assert_eq!(rt.eval("typeof document").unwrap(), "object");
        assert_eq!(rt.eval("document.nodeType").unwrap(), "9");
        assert_eq!(rt.eval("document.nodeName").unwrap(), "#document");
    }

    #[test]
    fn document_is_undefined_without_builder_document() {
        // Phase 1B back-compat: a runtime built without .document()
        // should still work, just with `document === undefined`.
        let mut rt = JsRuntime::new();
        assert_eq!(rt.eval("typeof document").unwrap(), "undefined");
    }

    #[test]
    fn document_url_reflects_location() {
        let mut rt = fixture_runtime();
        assert_eq!(
            rt.eval("document.URL").unwrap(),
            "https://example.com/page?x=1"
        );
        assert_eq!(
            rt.eval("document.documentURI").unwrap(),
            "https://example.com/page?x=1"
        );
    }

    #[test]
    fn document_element_head_body_title() {
        let mut rt = fixture_runtime();
        assert_eq!(rt.eval("document.documentElement.tagName").unwrap(), "HTML");
        assert_eq!(rt.eval("document.head.tagName").unwrap(), "HEAD");
        assert_eq!(rt.eval("document.body.tagName").unwrap(), "BODY");
        assert_eq!(rt.eval("document.title").unwrap(), "T");
    }

    #[test]
    fn element_attributes_read() {
        let mut rt = fixture_runtime();
        // Walk to <p>.
        assert_eq!(
            rt.eval("document.body.firstElementChild.tagName").unwrap(),
            "P"
        );
        assert_eq!(rt.eval("document.body.firstElementChild.id").unwrap(), "x");
        assert_eq!(
            rt.eval("document.body.firstElementChild.className")
                .unwrap(),
            "a b"
        );
        assert_eq!(
            rt.eval("document.body.firstElementChild.getAttribute('id')")
                .unwrap(),
            "x"
        );
        assert_eq!(
            rt.eval("document.body.firstElementChild.getAttribute('missing')")
                .unwrap(),
            "null"
        );
        assert_eq!(
            rt.eval("document.body.firstElementChild.hasAttribute('id')")
                .unwrap(),
            "true"
        );
        assert_eq!(
            rt.eval("document.body.firstElementChild.hasAttribute('missing')")
                .unwrap(),
            "false"
        );
        assert_eq!(
            rt.eval("JSON.stringify(document.body.firstElementChild.getAttributeNames())")
                .unwrap(),
            "[\"id\",\"class\"]"
        );
    }

    #[test]
    fn element_text_content() {
        let mut rt = fixture_runtime();
        // <p> contains "hi " + <b> ("there") = "hi there".
        assert_eq!(
            rt.eval("document.body.firstElementChild.textContent")
                .unwrap(),
            "hi there"
        );
    }

    #[test]
    fn element_inner_and_outer_html() {
        let mut rt = fixture_runtime();
        assert_eq!(
            rt.eval("document.body.firstElementChild.outerHTML")
                .unwrap(),
            "<p id=\"x\" class=\"a b\">hi <b>there</b></p>"
        );
        assert_eq!(
            rt.eval("document.body.firstElementChild.innerHTML")
                .unwrap(),
            "hi <b>there</b>"
        );
    }

    #[test]
    fn navigation_parent_and_children() {
        let mut rt = fixture_runtime();
        // body.parentNode is html; html.parentNode is the document.
        assert_eq!(rt.eval("document.body.parentNode.tagName").unwrap(), "HTML");
        assert_eq!(
            rt.eval("document.body.parentElement.tagName").unwrap(),
            "HTML"
        );
        assert_eq!(
            rt.eval("document.documentElement.parentNode.nodeType")
                .unwrap(),
            "9"
        );
        assert_eq!(
            rt.eval("document.documentElement.parentElement").unwrap(),
            "null"
        );
        // body has one element child (the <p>).
        assert_eq!(rt.eval("document.body.children.length").unwrap(), "1");
        assert_eq!(rt.eval("document.body.childElementCount").unwrap(), "1");
    }

    #[test]
    fn node_identity_preserved_across_reads() {
        // The headline DOM contract: el === el.parentNode.children[0].
        // Without the identity cache, two separate reads would return
        // two different JS wrappers and `===` would fail.
        let mut rt = fixture_runtime();
        let out = rt
            .eval(
                "const p = document.body.firstElementChild;
                   p === p.parentNode.children[0]",
            )
            .unwrap();
        assert_eq!(out, "true");
    }

    #[test]
    fn first_and_last_element_child_skip_text_nodes() {
        let mut rt = fixture_runtime();
        // <p> has children: [text "hi ", <b>]. firstElementChild must
        // skip the text node and return <b>.
        assert_eq!(
            rt.eval("document.body.firstElementChild.firstElementChild.tagName")
                .unwrap(),
            "B"
        );
        // firstChild does NOT skip — should be the text node.
        assert_eq!(
            rt.eval("document.body.firstElementChild.firstChild.nodeType")
                .unwrap(),
            "3"
        );
        assert_eq!(
            rt.eval("document.body.firstElementChild.firstChild.data")
                .unwrap(),
            "hi "
        );
    }

    #[test]
    fn siblings_walk_in_document_order() {
        let mut rt = fixture_runtime();
        // <p>'s children: [text "hi ", <b>].
        // nextSibling of "hi " is <b>; previousSibling of <b> is "hi ".
        assert_eq!(
            rt.eval("document.body.firstElementChild.firstChild.nextSibling.tagName")
                .unwrap(),
            "B"
        );
        assert_eq!(
            rt.eval("document.body.firstElementChild.firstElementChild.previousSibling.data")
                .unwrap(),
            "hi "
        );
        // Bounds: previousSibling of the first child is null.
        assert_eq!(
            rt.eval("document.body.firstElementChild.firstChild.previousSibling")
                .unwrap(),
            "null"
        );
    }

    #[test]
    fn get_element_by_id_finds_match() {
        let mut rt = fixture_runtime();
        assert_eq!(
            rt.eval("document.getElementById('x').tagName").unwrap(),
            "P"
        );
    }

    #[test]
    fn get_element_by_id_returns_null_for_missing() {
        let mut rt = fixture_runtime();
        assert_eq!(rt.eval("document.getElementById('zzz')").unwrap(), "null");
    }

    #[test]
    fn text_node_surface() {
        let mut rt = fixture_runtime();
        // p.firstChild is "hi " text node.
        assert_eq!(
            rt.eval("document.body.firstElementChild.firstChild.nodeType")
                .unwrap(),
            "3"
        );
        assert_eq!(
            rt.eval("document.body.firstElementChild.firstChild.nodeName")
                .unwrap(),
            "#text"
        );
        assert_eq!(
            rt.eval("document.body.firstElementChild.firstChild.textContent")
                .unwrap(),
            "hi "
        );
    }

    #[test]
    fn element_node_type_is_one() {
        let mut rt = fixture_runtime();
        assert_eq!(rt.eval("document.body.nodeType").unwrap(), "1");
        assert_eq!(rt.eval("document.body.nodeName").unwrap(), "BODY");
        assert_eq!(rt.eval("document.body.localName").unwrap(), "body");
    }
}
