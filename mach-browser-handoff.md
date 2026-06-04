# mach-browser — Context Handoff

Single-page brief for an AI agent picking up this project cold. Read top to bottom; do **not** ask the user questions answered here.

---

## 1. What this project is

`mach-browser` is a **new, ultra-lightweight headless browser engine built in Rust from scratch** — not a Chromium fork. Target audience is AI agents and automation, not human users. Repo: **github.com/RezoxP/mach-browser**.

It must:

1. Hit **< 10 MB RSS** for the no-JS page-extraction path (`mach fetch --dump markdown` etc.), **< 35 MB peak** during a Cloudflare Turnstile JS challenge, and **< 100 MB binary**.
2. **Pass Cloudflare Turnstile** end-to-end on a real site. This is the v1 release gate. hCaptcha / DataDome / Akamai BMP are explicitly out of v1 scope.
3. Work on **Linux x86_64 (Ubuntu 22.04)** and **Windows x86_64 (MSVC, Server 2022)**. macOS is post-v1. The single in-tree fingerprint profile in v1 is **Chrome 131 on Linux** — it impersonates that to the wire regardless of the host OS we actually run on.

It must **not**:

- Be a Chromium / WebKit / Servo fork. Greenfield only.
- Implement a render pipeline (no layout, paint, GPU, media, accessibility, screen-reader, extensions, service workers in v1).
- Add code that protects against humans (sandbox, CORS enforcement, etc. — pointless for an automation client).

Architecture proposal lives at **`docs/architecture.md` in the repo**. It's authoritative. §0 has hard product requirements, §0.4 has the 12 memory tactics, §0.7 has the cross-platform plan, §8 has the phased roadmap with acceptance gates.

---

## 2. Where things live

| Path | What |
|---|---|
| `crates/mach` | CLI binary (`mach.exe`). Subcommands: `fetch`, `js`. |
| `crates/mach-core` | `Config`, `Error`, `NotificationBus`. |
| `crates/mach-profile` | `BrowserProfile` struct (UA, TLS shape, Sec-CH-UA, languages, hardwareConcurrency, screen, timezone). Source of truth for fingerprinting — HTTP layer and JS layer both read it. |
| `crates/mach-net` | `HttpClient` wrapping `wreq` v6 + BoringSSL. JA4 / HTTP/2-Akamai fingerprints validated against `tls.peet.ws` as real Chrome 131. |
| `crates/mach-parser` | `html5ever` wrapper that emits into `mach-dom`. |
| `crates/mach-dom` | Arena-backed DOM (`Vec<Node>`, `NodeId(u32)`). Serializer, text-content, `find_by_id`, `find_first_element`. |
| `crates/mach-js` | V8 isolate + persistent `v8::Global<v8::Context>` per page. Globals: `window`, `navigator`, `location`, `console`. **Phase 1C done** — `document` / `Element` / `Text` (read-only) wired through `JsRuntimeBuilder::document(doc)`. See `crates/mach-js/src/dom.rs`. |
| `crates/mach-agent` | `--dump markdown|links|text` exporters from a parsed DOM. |
| `.cargo/config.toml` | Linker flags split per target (gcc-style for Linux, MSVC `/OPT:REF /OPT:ICF` for Windows). |
| `.github/workflows/ci.yml` | Matrix: `ubuntu-22.04` + `windows-2022`. Three jobs: lint, build-linux, build-windows-msvc. Required prereqs on the Windows runner: NASM (via `ilammy/setup-nasm`), LLVM (via `choco install llvm`), `LIBCLANG_PATH=C:\Program Files\LLVM\bin`. |
| `.github/workflows/release.yml` | Builds release binaries on push-to-Jules--only + on `v*` tag; uploads as workflow artifacts + GitHub Release. |
| `docs/architecture.md` | Rev-4 architecture proposal. Always re-read before scoping a new phase. |

---

## 3. What's done (merged to Jules--only)

| PR | Phase | What landed |
|---|---|---|
| #1 | 0 | Workspace skeleton; `wreq` HTTP stack validated as real Chrome 131 on JA4 + HTTP/2-Akamai; `html5ever` parser; arena DOM; `mach fetch --dump html\|markdown\|links\|text` works end-to-end. CI matrix green Linux + Windows MSVC. Binary 7.2 MB. |
| #2 | 0.5 | Release workflow: builds binaries on push-to-Jules--only + `v*` tags; artifacts downloadable from Actions UI. |
| #3 | 1A | V8 wired up. New `mach-js` crate. `OnceLock`-gated lazy V8 platform init (Tactic #1: `mach fetch` never pays V8 cost). `mach js --eval '<src>'` subcommand. Binary 32 MB (25 MB for V8 static lib). 20 unit tests. **Windows linker fix in `crates/mach-js/build.rs`: links `advapi32.lib` to satisfy V8's ETW + ICU `wintz.obj` references.** |
| #4 | 1B | Persistent `v8::Global<v8::Context>` per page (Phase 1A used throwaway contexts per eval — broke state). `JsRuntime::builder()` API. Globals installed at construction: `window === globalThis`, `navigator` (userAgent / platform / language(s) / hardwareConcurrency / deviceMemory / webdriver=false / cookieEnabled=true / onLine=true), `location` (full parsed-URL surface), `console.log/info/warn/error/debug/trace` routed through `tracing`. 32 unit tests total. Binary still 32 MB. |
| #5 | 1C | Read-only DOM bindings. New `crates/mach-js/src/dom.rs` (~720 lines). `JsRuntimeBuilder::document(doc)` opt-in API — runtime built without it has `document === undefined` (no Phase 1B regression). Surface: `document` (`documentElement`/`head`/`body`/`title`/`URL`/`getElementById`/`nodeType=9`), `Element` (`tagName`/`localName`/`id`/`className`/`parentNode`/`parentElement`/`children`/`childNodes`/`firstChild`/`lastChild`/`nextSibling`/`previousSibling`/`firstElementChild`/`lastElementChild`/`childElementCount`/`textContent`/`innerHTML`/`outerHTML`/`getAttribute`/`hasAttribute`/`getAttributeNames`/`hasAttributes`/`nodeType=1`), `Text` (`data`/`textContent`/`nodeType=3`/`nodeName="#text"`/parent + sibling nav). **Node-identity cache** (`RefCell<HashMap<u32, v8::Global<v8::Object>>>` in isolate slot) preserves `el === el.parentNode.children[0]`. 34 mach-js + 11 mach-dom unit tests. Binary still 32 MB. |

**Verified properties as of #5 merged:**

- `mach fetch --dump html https://example.com` works, ~5.5 ms cold start, no V8 init.
- `mach js --eval '21 + 21'` returns `42`, ~9 ms cold start.
- `mach js --eval 'navigator.userAgent'` returns Chrome 131 / Linux UA.
- `mach js --eval 'window === globalThis'` returns `true`.
- `mach js --eval 'navigator.webdriver'` returns `false`.
- `mach js --eval 'typeof document'` returns `undefined` (CLI path doesn't pass a document yet — that's Phase 1G).
- Library-level: `JsRuntime::builder().document(doc).build()` exposes a working read-only DOM and `el === el.parentNode.children[0]` is preserved.
- CI green on Linux + Windows MSVC.

---

## 4. What is being worked on right now (Phase 1D, planned)

**Goal of next PR:** Mutation API on the DOM bindings from Phase 1C. `document.body.appendChild(document.createElement('div'))` actually changes the tree; subsequent `outerHTML` reads reflect the change.

**Pre-reading before you write any code:**

- `crates/mach-js/src/dom.rs` (already on `Jules--only`). The accessor/method patterns, identity cache, and isolate-slot dance are all there. You'll extend the existing `DomInner` rather than redesign anything.
- `crates/mach-dom/src/lib.rs`. `Document::push` already exists for appending; you'll likely need `insert_before`, `remove`, `replace`, and `set_attribute` methods on `Document` — add them as small focused PRs to `mach-dom` first if they don't fit cleanly inline.

**Concrete shape of Phase 1D:**

1. **Mutate `mach-dom::Document`.** Add methods that the bindings will call:
   - `pub fn insert_child(&mut self, parent: NodeId, kind: NodeKind, before: Option<NodeId>) -> NodeId`
   - `pub fn remove_child(&mut self, parent: NodeId, child: NodeId) -> Option<NodeKind>`
   - `pub fn set_attribute(&mut self, node: NodeId, name: &str, value: &str)`
   - `pub fn remove_attribute(&mut self, node: NodeId, name: &str)`
   - `pub fn set_text_data(&mut self, node: NodeId, value: String)`
   - `pub fn replace_children_with_parsed(&mut self, parent: NodeId, fragment: &str)` — used by `innerHTML =`; parses with `html5ever` in fragment-context-element mode (use `parse_fragment` from `html5ever`, fragment-context = parent's tag, e.g. `body` for `<body>` parents). Wipes parent's children first.
   - `pub fn create_element(&mut self, name: &str) -> NodeId` — orphan node, no parent, returns NodeId. Needed for `document.createElement('div')` which must produce a detached node.
   - `pub fn create_text_node(&mut self, data: &str) -> NodeId` — same.
   - All of these must update `Node::parent` consistently. Add tests in `mach-dom` for each.

2. **Extend `crates/mach-js/src/dom.rs`** with the JS methods:
   - On `Element`: `setAttribute(name, value)`, `removeAttribute(name)`, `appendChild(node)`, `insertBefore(node, refNode)`, `removeChild(child)`, `replaceChild(newChild, oldChild)`. Setters: `textContent`, `innerHTML`. Note: V8 130's `ObjectTemplate::set_accessor_with_setter(name, getter, setter)` is the equivalent of `set_accessor` for read+write — check the rusty_v8 source for the exact signature before guessing.
   - On `Document`: `createElement(tagName)`, `createTextNode(data)`.
   - On `Text`: writable `data` / `textContent`.

3. **Cache discipline.** When a node is removed, the JS wrapper *stays alive* as long as user JS holds a reference — that's required by spec (a removed `<div>` can be re-appended elsewhere and the same wrapper must surface). Don't evict from the cache on remove. **Only** evict when the underlying NodeId is actually freed in `mach-dom` (currently never — arena is monotonic). If you add NodeId recycling to `mach-dom`, you'll need a v8 weak handle / finalization-callback story to evict the cache, which is non-trivial — defer recycling.

4. **Tests** — minimum coverage:
   - `setAttribute` round-trip: set, then `getAttribute` returns the new value, then `outerHTML` contains it.
   - `appendChild` round-trip: create, append, `el.children.length` increased, `outerHTML` contains the new element.
   - `removeChild`: remove, `el.children.length` decreased, `outerHTML` no longer contains it.
   - Identity after move: `const x = ...createElement('div'); body.appendChild(x); body.firstElementChild === x` is `true` (cache must survive append).
   - `innerHTML = '<p>x</p>'`: serializes to `<div><p>x</p></div>` etc.
   - `textContent =` wipes children and replaces with a single text node.
   - `createElement('DIV')` (uppercase) produces an element whose `tagName === 'DIV'` and whose serialized form is `<div></div>` (lowercase on the wire — DOM spec quirk; `localName` is always lowercased, `tagName` is uppercased on read).

5. **fmt + clippy + workspace tests + release build on Windows MSVC.** Same drill as previous phases (see §7).

6. **Commit, push, open PR**, wait for CI green on both runners, share PR URL with user.

**Out of scope for Phase 1D PR:** selectors (querySelector), events (EventTarget), fetch / XHR, CLI `--execute-js`. Each gets its own PR.

**Pitfalls picked up during Phase 1C (avoid re-discovering):**

- **`Local<Data>` → `Local<Value>`** conversion is `field.try_into().ok()?`, *not* `field.to_integer(scope)`. `Data` is the supertype; you must downcast first. Used in `read_node_id` — copy the pattern.
- **Accessor callback signature in v8 130** is `fn(scope: &mut HandleScope, _key: Local<Name>, args: PropertyCallbackArguments, rv: ReturnValue)`. Note the `Name` (not `String`) key parameter; this is different from v149's PinScope-based API.
- **`scope.set_slot(Rc<DomInner>)`** keys by `TypeId::of::<Rc<DomInner>>()`. If you wrap state in a different type alias the slot lookup will silently miss — keep it as `Rc<DomInner>` everywhere.
- **Borrow-checker dance:** every accessor / method clones the `Rc` out of the slot *first* (drops the immutable borrow on the scope's annex), *then* calls V8 APIs that need `&mut HandleScope`. See `dom_state(scope)` in `dom.rs`. Holding the slot borrow across V8 calls will not compile.
- **`tagName` is uppercased, `localName` is lowercased.** `mach-dom` stores names lowercased (html5ever normalisation); uppercase only at the binding boundary for `tagName` / `nodeName`. WHATWG DOM spec quirk — easy to get wrong.
- **`getAttribute(missing)` returns `null`, not `""`.** Spec. But `element.id` / `element.className` return `""` for missing attributes, also spec. Two different patterns in the same module; double-check when adding new attribute-backed accessors.
- **Comment + Doctype nodes are NOT exposed in Phase 1C** — `get_or_create_node` returns `None` for them. If Phase 1D wants to expose them, decide explicitly rather than silently changing behaviour.

---

## 5. Phased plan after Phase 1D

| Phase | Scope | Acceptance gate |
|---|---|---|
| **1E** | Querying: `querySelector`, `querySelectorAll`, `getElementsByTagName`, `getElementsByClassName`. Integrates `selectors` + `cssparser` crates from the Servo stack. | jsdom-style selector tests pass on a fixture HTML. |
| **1F** | EventTarget: `addEventListener`, `removeEventListener`, `dispatchEvent`, `Event` constructor. No actual events fire (no timers, no IO yet) — this is just plumbing. | `el.addEventListener('x', cb); el.dispatchEvent(new Event('x'))` invokes `cb`. |
| **1G** | `mach fetch --execute-js <URL>`: HTTP → parse → build DOM → JsRuntime with document → run inline `<script>` tags in document order → re-serialize. | `mach fetch --execute-js --dump html` against a server-rendered page that has client-side JS produces post-JS HTML. |
| **2** | Fingerprint spoofing for Turnstile: real `SubtleCrypto` via RustCrypto, spoofed `Canvas.toDataURL` / `WebGL.getParameter` / `AudioContext` (hardcoded byte tables per profile), `window.chrome` stub, full Permissions API, `Intl.DateTimeFormat` timezone from profile, faux-layout for `getBoundingClientRect` / `clientWidth`. Turnstile-pass nightly CI job (Linux only). | Cloudflare Turnstile managed-mode challenge passes end-to-end on a third-party test page from a clean IP. |
| **3** | Timers (`setTimeout`/`setInterval`/`requestAnimationFrame`), `fetch` (route through `mach-net`'s wreq client), `XMLHttpRequest`, `URL` + `URLSearchParams` constructors, `MutationObserver`. | Real-world SPA bootstrap completes. |
| **4** | CDP subset (network doJules--only, DOM doJules--only, Runtime doJules--only, Page doJules--only — enough to drive from Puppeteer). | `puppeteer.connect()` works against `mach serve`. |
| **5** | MCP server (stdio + streamable HTTP), `mach scrape` agent commands, structured-data exporters. Lightpanda's WebMCP CDP doJules--only. | Claude Desktop / Cursor can drive mach via MCP. |
| **6** | Optional `--solver` flag for 2captcha / anti-captcha integration (interactive-mode Turnstile, hCaptcha later). macOS profile + macOS CI. Additional anti-bot products (hCaptcha / DataDome / Akamai BMP) become explicit goals here. | Sponsored / community contribution territory. |

Don't skip ahead. Each phase has dependencies on the previous one.

---

## 6. Hard constraints — re-read whenever in doubt

1. **Never break the `mach fetch` lazy-V8 path.** It must stay under ~6 ms cold start and never link V8 platform init. Tactic #1 in `docs/architecture.md` §0.4. The current architecture (V8 init guarded by `OnceLock` in `JsRuntime::new`, no other path touches it) enforces this — preserve the invariant.
2. **Binary stays under 100 MB.** Current: 32 MB. DOM bindings add ~100 KB. Fingerprint tables (Phase 2) will be the next big chunk (~5 MB hardcoded byte arrays for Canvas + WebGL + Audio profiles). LTO + strip are configured in `Cargo.toml`; do not disable.
3. **One `BrowserProfile`, two consumers.** HTTP layer (`mach-net`) and JS layer (`mach-js`) **must** read from the same `mach_profile::BrowserProfile`. Never read host OS / host CPU / host environment in a binding. Anti-bot detection looks for drift between HTTP fingerprint and JS surface — drift = type error, not runtime bug. Arch doc §5 divergence #7.
4. **Cross-platform from day one.** Every PR must build on Windows MSVC. Verify locally with the build wrapper script before pushing. CI also enforces this. Windows-only gotcha: `crates/mach-js/build.rs` links `advapi32.lib` for V8 130's `etw-jit-win.obj` + `wintz.obj` ETW + ICU references. Do not remove.
5. **rusty_v8 pinned at v130** to match Chrome 130/131 (the profile we impersonate). Upgrading is fine but is a separate, focused PR — v149 has the `PinnedRef`/Scope API rework.
6. **No `Any`, no `getattr`/`setattr`, no laziness.** Understand types fully. The arch doc is authoritative on data model.
7. **Use the builtin git tools** (`git_create_pr`, `git_view_pr`, `git_pr_checks`, `git_update_pr`, `git_ci_job_logs`) — never use `gh pr create` directly. **Always call `fetch_pr_template` before `git_create_pr`** — the tool rejects PRs without it.
8. **Never push to Jules--only.** Always work on `devin/<timestamp>-<topic>` branches and PR into Jules--only. User merges.
9. **CI must be green before reporting completion.** Use `git_pr_checks wait_mode="all"` after creating a PR.

---

## 7. How to run things

```bash
# Local dev (Windows VM)
cmd //c 'C:\Users\Administrator\work\mach-build.bat build --workspace --target x86_64-pc-windows-msvc'
cmd //c 'C:\Users\Administrator\work\mach-build.bat fmt --all -- --check'
cmd //c 'C:\Users\Administrator\work\mach-build.bat clippy --workspace --all-targets -- -D warnings'
cmd //c 'C:\Users\Administrator\work\mach-build.bat test --workspace --target x86_64-pc-windows-msvc --no-fail-fast'

# Smoke tests
target/x86_64-pc-windows-msvc/release/mach.exe fetch --dump text https://example.com
target/x86_64-pc-windows-msvc/release/mach.exe js --eval '21 + 21'
target/x86_64-pc-windows-msvc/release/mach.exe js --eval 'navigator.userAgent'
```

A `cmd //c` call from a Git-Bash shell is required to launch the MSVC environment correctly. Calling `cargo` directly from Git-Bash will fail to find `cl.exe`.

---

## 8. Communication norms with the user

- User is **@RezoxP**. Comments on PRs in English, terse, low-bandwidth.
- After each merge the user usually says "continue upgrading the mach browser". That means: pick the next phase from §5 above, branch, build, PR. Don't ask what to do next.
- The user wants memory minimisation above all else; don't propose anything that conflicts with §6.1 / §6.2 without explicit pushback first.
- The user already rejected Bun and JavaScriptCore (memory savings too small vs the fingerprint-consistency cost of switching from a Chrome profile to a Safari profile). Do not relitigate.

---

## 9. If you get stuck

- The architecture doc (`docs/architecture.md`) answers most "why" questions. §0 is requirements, §3 is module layout, §5 is divergences from Obscura / Lightpanda, §7 is known technical risks, §8 is the phased plan.
- Existing merged PRs (`#1`, `#3`, `#4`) are good examples of the **PR description style** the user prefers — measurements table, deliberate out-of-scope list, risk colour, testing checklist.
- The Lightpanda repo (`lightpanda-io/browser`) and Obscura (`h4ckf0r0day/obscura`) are the two reference projects. Use them as design oracles when in doubt. Do **not** copy their code (license + style mismatch) — read for ideas, write fresh.

---

End of brief. **Don't ask the user to repeat any of this** — re-read this file and `docs/architecture.md` if you need more context.
