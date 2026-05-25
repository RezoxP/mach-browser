# mach-browser — Architecture Proposal (rev 4)

An ultra-lightweight browser built from scratch for AI agents and automation.
**Not** a Chromium fork. **Not** a WebKit patch.

Status of this document: research + architecture proposal only. No code is being
proposed for commit yet — the symbol blocks below are intent declarations to be
reviewed before implementation.

**Rev 4 changes (locked-in answers + cross-platform):** All §9 open questions
are now answered and pinned (see §0.6). Language: **Rust**. License:
**Apache-2.0**. Repo: **`RezoxP/mach-browser`**. JS engine: **V8**. Anti-bot
v1 scope: **Cloudflare Turnstile only** (others Phase-6). `--solver`
integration: **Phase 5/6 optional flag**. Profiles in v1: **one Chrome-Linux
profile** (data-only additions later). **New: cross-platform commitment.**
Linux + Windows are first-class targets from Phase 0; macOS is Phase-6. New
**§0.7 cross-platform notes** with a per-tactic platform matrix and a new
`process::TargetSpawner` abstraction that replaces the Linux-only
`--fork-per-page` design from rev 3 with `ForkSpawner` (Linux) +
`WorkerSpawner` (Windows / macOS / fallback). New companion binary
`mach-worker(.exe)` shipped alongside `mach(.exe)`.

**Rev 3 changes (in response to user feedback on "absolutely lightweight" and
Bun question):** new **§0.4 "Aggressive memory tactics"** with 12 ranked tactics
to drive per-page RSS below the rev 2 targets. New **§0.5** revised memory
budget table with concrete numbers per scenario. Headline targets are now:
< 10 MB RSS for the no-JS markdown path, < 20 MB for plain JS pages, < 35 MB
peak under Turnstile, < 80 MB total for 10 concurrent pages via CDP, binary
60-70 MB. Includes an in-text aside explaining why Bun (a server-side JS
runtime) is not applicable to a browser, and why its underlying engine
(JavaScriptCore) does not justify swapping out V8 once the fingerprint
implications are accounted for. New requirement R0.5 captures the tightened
memory targets.

**Rev 2 changes (in response to user feedback):** binary-size budget pinned to
< 100 MB; **ultra-low memory and overhead** is the primary product
requirement; **Cloudflare Turnstile must pass** as a Phase-2 acceptance gate.
The biggest architectural consequences are (a) stealth / fingerprint shaping
is promoted from a v2 feature flag to v1 core, (b) the network stack is
pinned to a Chrome-fingerprint-shaped TLS+HTTP/2 client (`wreq`/`rquest` or
equivalent) — libcurl and stock hyper/reqwest are both rejected, (c) the Web
API surface must include enough of Canvas2D / WebGL / AudioContext / Web
Crypto to survive a Turnstile JS challenge, and (d) faux-layout becomes
default-on. Details in §0 (new) and §3, §4, §5, §7.

---

## 0. Hard product requirements (rev 2)

These are pinned constraints from the user. The rest of the document is
constructed to satisfy them; conflicts elsewhere must defer to this section.

- **R0.1 Memory & overhead come first.** Every other design choice is
  subordinate to "stay tiny and fast." Concrete budgets in §0.3.
- **R0.2 Binary ≤ 100 MB.** Not a top priority but a hard cap. Obscura is
  ~70 MB statically linked with V8; we have headroom but not infinite.
- **R0.3 Cloudflare Turnstile must pass.** Defined as: a Turnstile "managed"
  / "non-interactive" challenge embedded in a third-party page completes
  end-to-end and yields a valid `cf-turnstile-response` token, with no
  external solver service. "Interactive" mode (the user-must-click variant)
  is best-effort; integrating an external solver is an optional flag.
- **R0.4 Headless-only.** No headed/windowed mode, ever.
- **R0.5 (rev 3) "Absolutely lightweight."** Memory is the headline metric.
  Process RSS targets (revised down from rev 2 by exploiting paths real
  agents actually take):
  - `mach fetch --dump markdown <url>` (typical agent call, no JS needed):
    **< 10 MB RSS**.
  - `mach fetch <url>` (JS enabled, plain page): **< 20 MB RSS**.
  - `mach fetch <url>` (JS enabled, Turnstile challenge live): **< 35 MB RSS
    peak**.
  - 10 concurrent pages in a CDP session: **< 80 MB RSS total** (shared
    isolate, context-per-page).

### 0.1 What Turnstile actually checks (and what each check forces on us)

The Turnstile challenge is the de facto "minimum hard test" for whether a
non-Chromium browser is viable for real automation work. The specific checks
it performs translate directly into architectural constraints:

| Turnstile probe | What it reads | Architectural consequence |
|---|---|---|
| TLS ClientHello (JA3 / JA4) | cipher list, extensions, curves, ALPN | TLS stack must replay Chrome's ClientHello byte-for-byte. **Eliminates libcurl and stock rustls.** Forces `wreq` / `rquest` / BoringSSL-with-Chrome-profile. |
| HTTP/2 SETTINGS, WINDOW_UPDATE, frame order, pseudo-header order, HPACK table | first 3 frames after connection preface | HTTP/2 client must match Chrome's frame emission. `wreq` / `rquest` already do this. **Eliminates stock hyper.** |
| `User-Agent`, `Sec-CH-UA*` client hints, `Accept`, `Accept-Language`, `Accept-Encoding` order | request headers | Header set + order pinned to a real Chrome build. Cheap once the HTTP client supports it. |
| `navigator.userAgent`, `navigator.platform`, `navigator.languages`, `navigator.hardwareConcurrency`, `navigator.deviceMemory`, `navigator.webdriver`, `navigator.plugins`, `navigator.mimeTypes`, `navigator.connection`, `navigator.userAgentData` | ~50 props on Navigator | Hand-rolled Navigator with **all** these properties returning Chrome-realistic values. `navigator.webdriver` must be `false` or absent. |
| `window.chrome` object | typeof, common subprops | The whole `window.chrome` shape must be present (the existence-check, not full functionality). |
| `screen.width/height/availWidth/availHeight/colorDepth/pixelDepth`, `window.innerWidth/innerHeight`, `document.documentElement.clientWidth` | viewport / display geometry | **Faux-layout default-on.** Fixed viewport values (e.g. 1920×1080) returned consistently across all geometry APIs. |
| `Date.now()`, `performance.now()`, `Intl.DateTimeFormat().resolvedOptions().timeZone`, `Intl.Collator`, `new Date().getTimezoneOffset()` | timing & locale | Real `Intl` (V8 ships it), real `performance.now()` with monotonic + per-process random offset; timezone pluggable via config. |
| `Notification.permission`, `navigator.permissions.query(...)` | permissions state | Tiny stub returning Chrome-default values ("default" / "granted" / "denied" as appropriate). |
| `WebGLRenderingContext.getParameter(UNMASKED_VENDOR_WEBGL / UNMASKED_RENDERER_WEBGL)` | GPU vendor + renderer strings | **No real WebGL needed.** Return hardcoded strings from the profile ("Google Inc. (NVIDIA)" / "ANGLE (NVIDIA, NVIDIA GeForce RTX 3060 …)"). |
| `HTMLCanvasElement.toDataURL()` / `getImageData()` after drawing | canvas fingerprint | Either a tiny software 2D rasterizer (cairo-rs is ~600 KB, raqote is pure-Rust ~400 KB) or a **deterministic spoofed PNG** keyed by the profile. We pick deterministic-spoof in v1 (smaller, more robust). |
| `OfflineAudioContext.startRendering()` → buffer hash | audio fingerprint | Deterministic spoofed Float32Array per profile. No real audio DSP. |
| `crypto.subtle.digest / sign / encrypt`, `crypto.getRandomValues`, `crypto.randomUUID` | **real crypto output** | **Must be real.** The Turnstile challenge HMACs/SHA-256s real data and the server validates. RustCrypto (`sha2`, `hmac`, `aes-gcm`, `p256`) covers it; Lightpanda already has `AES.zig`, `HMAC.zig`, `RSA.zig`, `X25519.zig`. |
| `Function.prototype.toString.call(navigator.webdriver)`, `Error().stack` formatting | engine identity | Must be V8 (not QuickJS / Boa / SpiderMonkey). V8 was already the right choice; Turnstile makes it the *only* choice. |
| `requestAnimationFrame`, `setTimeout` jitter, microtask ordering | timing fingerprint | rAF must fire on a fake 60 Hz vsync; `performance.now()` must be coarse (Spectre-style 100 µs quantization matches Chrome). |
| Mouse movement / focus events (interactive mode only) | behavioral | Out of scope for managed/non-interactive mode; with `--solver` flag we shell out to 2captcha/anti-captcha for interactive challenges. |

### 0.2 What we are explicitly **not** trying to defeat

To keep scope honest:

- **Active TLS interception by enterprise middleboxes** (corporate MITM). Out
  of scope. Use `--ca-bundle` / system store and accept what the network
  gives us.
- **Bot-management products that rely on residential-IP heuristics** (most
  of Akamai BMP, parts of DataDome). Fingerprint shaping does not buy us an
  IP; users bring their own proxies via `--proxy`. We are *fingerprint*
  honest, not *network identity* honest.
- **Behavioural / mouse-trajectory analysis** (e.g. Distil, PerimeterX
  interactive). Phase-3 problem at earliest. If/when in scope, it's a
  separate "behavioural simulator" module, not a browser concern.

### 0.3 Memory & binary budgets (concrete numbers we will hold ourselves to)

Per-page resident set, idle after navigation:

| Component | Budget | Notes |
|---|---|---|
| V8 platform (process-singleton) | 5 MB | One copy across all pages |
| V8 isolate (one per Page) | 10-12 MB | With `--max-old-space-size=128 --max-semi-space-size=2` |
| DOM arena | 2-5 MB | Arena-per-page, dropped on navigation |
| HTTP / TLS connection pool | 3-5 MB | Keep-alive sockets + cookie jar |
| Stealth profile (strings + spoofed canvas PNG + spoofed audio buffer) | 1-2 MB | Hardcoded into binary, mmapped read-only |
| Web API binding wrappers (live) | 1-2 MB | Grows with DOM size |
| **Total per active page** | **~25-30 MB** | Comparable to Obscura's 30 MB; ~6-8× under headless Chrome |

Process-level (with one active page):

- **RSS target:** < 50 MB steady-state, < 80 MB peak during JS challenge.
- **Cold start:** < 50 ms to first-byte-of-response (V8 snapshot mandatory).
- **Binary size:** target 60-80 MB, hard cap 100 MB. V8 static link is the
  dominant term (~50 MB); BoringSSL is ~3 MB; html5ever ~500 KB; everything
  else fits in single-digit MB.

V8 flags pinned at startup (one place, `js::Runtime::init`):

```
--max-old-space-size=128
--max-semi-space-size=2
--no-expose-wasm        # we expose it via binding only if needed
--no-flush-bytecode     # smaller code cache, faster re-runs across nav
--turbo-fast-api-calls  # cheap interop for hot DOM ops
```

Note: **`--lite-mode` / `--jitless` are tempting for size but rejected** —
the Turnstile challenge does CPU-heavy crypto+math in JS and times out
without JIT. We pay the JIT memory cost (~5-8 MB additional under load) to
pass Turnstile.

### 0.6 Locked-in decisions (rev 4)

All the §9 open questions from rev 1-3 are now answered. They are pinned
here so the rest of the doc reads against fixed ground.

| Decision | Value | Notes |
|---|---|---|
| Implementation language | **Rust** (edition 2021) | `rusty_v8`, tokio, html5ever, wreq, mimalloc all native |
| License | **Apache-2.0** | Compatible with all dependency licenses (BSD/MIT/MPL-2.0) |
| Canonical repo | **`github.com/RezoxP/mach-browser`** | Single workspace, not multi-repo |
| JS engine | **V8** (via `rusty_v8`) | Turnstile pins us to V8; JSC/QuickJS/Boa all rejected |
| HTML parser | **html5ever** | Servo crate, native Rust |
| CSS selector matcher | **Servo `selectors` + `cssparser`** | `querySelector*` only; no layout |
| HTTP/TLS client | **`wreq` + `wreq_util`** | Chrome 131+ TLS+HTTP/2 fingerprint impersonation. Fallback `rquest`. |
| Async runtime | **tokio** | Single-threaded scheduler per process |
| Allocator | **mimalloc** (global) | Eager `purge_on_idle` |
| v1 anti-bot scope | **Cloudflare Turnstile (managed mode) only** | hCaptcha / DataDome / Akamai / PerimeterX are Phase-6 |
| `--solver` external captcha integration | **Phase 5/6 optional flag** | Out of core; thin HTTP POST adapter for 2captcha/anti-captcha |
| Profiles shipped in v1 | **One Chrome-Linux profile** | Windows/Mac profile data shipped later as static files; no code change required |
| Platform support, v1 | **Linux x86_64 + Windows x86_64** (both first-class) | macOS x86_64 + aarch64 in Phase-6; Linux aarch64 follows |
| Distribution | **Two binaries:** `mach` + `mach-worker` (Obscura pattern) | `.exe` on Windows; ELF on Linux |
| Headless mode | **Headless only, forever** | No headed/windowed build target |

### 0.7 Cross-platform notes (Linux + Windows first-class, macOS later)

The v1 platform commitment is Linux and Windows together. Lightpanda punts
to WSL on Windows; we don't. The user's tactic-by-platform table below is
the authoritative summary; this section explains the architectural seams
that make it work.

| Tactic | Linux | macOS (Phase-6) | Windows |
|---|---|---|---|
| #1 Lazy V8 isolate | works | works | works |
| #2 Shared isolate, context-per-page | works | works | works |
| #3 mmap'd V8 snapshot | `mmap` direct | `mmap` direct | `CreateFileMapping`+`MapViewOfFile` via `memmap2` |
| #4 mimalloc + `purge_on_idle` | `madvise(DONTNEED)` | `madvise(FREE)`, tuning needed | `VirtualFree(MEM_DECOMMIT)` |
| #5 Lazy bindings | works | works | works |
| #6 Lazy DOM | works | works | works |
| #7 String interning | works | works | works |
| #8 `--single-threaded` V8 | works | works | works |
| #9 LTO + strip | gcc-style `-Wl,--gc-sections -Wl,--icf=safe` | gcc/clang-style same | MSVC-style `/OPT:REF /OPT:ICF` |
| #10 fork-per-page COW | works — best case | no (deprecated post-`exec`); use worker | no `fork()`; use worker |
| #11 HTTP/2 conn-pool caps | works | works | works |
| #12 Intl support | works | works | works |

**The one tactic that has to split: #10.** Everywhere else, the same Rust
code compiles to all three platforms with no `cfg`-gated divergence. For
spawning CDP targets we introduce a trait so the rest of the codebase
doesn't know which platform it's on:

```rust
// In crates/process/src/lib.rs
trait TargetSpawner: Send + Sync {
    fn spawn(&self, profile: &BrowserProfile) -> Result<TargetHandle>;
    fn wait(&self, t: TargetHandle) -> Result<ExitStatus>;
}

// Linux only: fork() the parent, child inherits the warm isolate.
#[cfg(target_os = "linux")]
struct ForkSpawner { /* parent_pid, warm_context_data, … */ }

// Windows + macOS + Linux fallback: spawn `mach-worker` and talk JSON-RPC.
struct WorkerSpawner { worker_path: PathBuf, profile: Arc<BrowserProfile> }

fn default_spawner() -> Box<dyn TargetSpawner> {
    #[cfg(target_os = "linux")]
    { if env::var("MACH_NO_FORK").is_err() { return Box::new(ForkSpawner::new()) } }
    Box::new(WorkerSpawner::new())
}
```

The CDP server constructs one `default_spawner()` at startup and calls
`.spawn()` per new target. On Linux this is a `fork()` with COW memory; on
Windows it's `tokio::process::Command::new("mach-worker.exe")` plus JSON-RPC
over stdin/stdout. Same external behavior, completely different memory
profile. The `--spawner=fork|worker` CLI override is for testing parity
between paths.

**`mach-worker` companion binary.** Same crate workspace, separate
binary target. Loads the same `js`, `dom`, `webapi`, `browser` crates as
`mach` but boots into a JSON-RPC loop instead of a CLI. Roughly Obscura's
`obscura-worker` pattern. Communicates with the parent over stdio; one
worker = one Page. The worker dies when its page closes; the parent
`waits()` and reaps.

**Build flags by target.** `.cargo/config.toml`:

```toml
[profile.release]
lto = "fat"
codegen-units = 1
strip = "symbols"
panic = "abort"

[target.x86_64-unknown-linux-gnu]
rustflags = ["-C", "link-arg=-Wl,--gc-sections",
             "-C", "link-arg=-Wl,--icf=safe"]

[target.x86_64-pc-windows-msvc]
rustflags = ["-C", "link-arg=/OPT:REF",
             "-C", "link-arg=/OPT:ICF"]
```

**Path handling.** Every internal path is `PathBuf`. Config dirs come from
the `dirs` crate (`%APPDATA%\mach\` on Windows, `~/.config/mach/` on Linux,
`~/Library/Application Support/mach/` on macOS). A workspace-level Clippy
lint will reject any function signature taking `&str` for a path argument.

**Profile portability.** The single Chrome-Linux `BrowserProfile` we ship
in v1 works regardless of the host OS mach is running on — anti-bot servers
inspect the network and JS surface we emit, not what kernel is underneath
the mach process. Shipping additional profiles (Chrome-Win, Chrome-Mac,
Firefox-Linux, …) in later versions is **data-only**: drop a new
`profile.toml` + canvas PNG + audio Float32Array into `crates/profile/data/`
and register it in the profile registry. No architecture change.

**Pre-Phase-0 spike — done, passed (rev 4-spike-1).** A minimal `wreq 6.0.0-rc.28`
+ `wreq-util 3.0.0-rc.11` binary was built on `x86_64-pc-windows-msvc` with
`rustc 1.95` against an embedded BoringSSL (via `boring-sys2 5.0.0-alpha.13`).
Build succeeded; final stripped+LTO binary is 5.8 MB. Findings:

- **Build prerequisites on Windows (load-bearing for CI):** rustup (MSVC host
  toolchain) + Visual Studio 2022 Build Tools with the **VCTools** workload
  (`cl.exe` + `link.exe` v14.44) + Windows SDK 10.0.26100 + **NASM** on PATH
  (BoringSSL's perlasm output requires it) + **LLVM/Clang's `libclang.dll`**
  with `LIBCLANG_PATH` exported (bindgen needs it to walk BoringSSL headers).
  `cmake` ships with VS Build Tools, no separate install. Build time on a
  cold cache: ~10 min total (~8 min in BoringSSL via the VS 2022 cmake
  generator), ~1 min warm. CI should `actions/cache` the cargo registry +
  `target/.../boring-sys2-*/out` to keep this under 2 min.
- **`prefix-symbols` feature** must be gated to Linux/Android only (matches
  obscura's split): enabling it on Windows produces unresolved
  `build_script_main_*` symbols at link time.
- **TLS handshake reads as real Chrome 131.** Round-tripping through
  `tls.peet.ws/api/all` returns JA4 `t13d1516h2_8daaf6152771_02713d6af862`
  (stable across runs) and HTTP/2 Akamai fingerprint
  `1:65536;2:0;4:6291456;6:262144|15663105|0|m,a,s,p`
  (hash `52d84b11737d980aef856699f885ca86`). Cipher list, ALPS (ext 17513),
  ECH (ext 65037), and the X25519MLKEM768 hybrid post-quantum group are all
  present. JA3 hash *varies* between runs because Chrome (and `wreq`) shuffle
  TLS extensions via GREASE per RFC 8701 — a stable JA3 would itself be a
  fingerprinting tell.
- **Cloudflare gate at the HTTP layer is open.** `demo.turnstile.workers.dev`
  returns HTTP 200 with the Turnstile widget HTML embedded; no 1015 (rate
  limit) or 403 (hard block). The classic `nowsecure.nl` probe returns the
  standard "Just a moment…" managed challenge page — the same response real
  Chrome receives on first visit, i.e. Cloudflare is advancing us to the JS
  interrogation rather than pre-blocking on TLS+HTTP/2 alone. Beating the JS
  challenge is Phase 2's job; the HTTP layer is no longer a risk.

**Decision: `wreq` is locked in. `rquest` fallback dropped.** The spike
project lives at `C:/Users/Administrator/work/wreq-spike/` outside the repo
(no commits, no `.git`) and exists as a reference for Phase 0 scaffolding.

**CI matrix.** GitHub Actions, from Phase 0:

| Job | OS | What runs |
|---|---|---|
| lint | ubuntu-22.04 | `cargo fmt --check`, `cargo clippy -- -D warnings` |
| build-linux | ubuntu-22.04 | `cargo build --release`; assert binary < 100 MB |
| build-windows | windows-2022 (MSVC) | install NASM + LLVM, `LIBCLANG_PATH=…`, `cargo build --release`; assert binary < 100 MB |
| test-linux | ubuntu-22.04 | `cargo test --workspace` |
| test-windows | windows-2022 | `cargo test --workspace` |
| turnstile-nightly | ubuntu-22.04 | Live Turnstile test page; gate on token retrieval |
| memory-bench | ubuntu-22.04 | RSS smoke test against R0.5 budgets |

### 0.4 Aggressive memory tactics (rev 3, in priority order)

The R0.5 targets above are not achievable with the rev 2 design as-stated.
They require deliberate tactics, listed in order of leverage:

1. **Lazy V8 isolate creation.** For `mach fetch --no-js` (HTML-only crawl),
   *never instantiate V8*. Process RSS drops from ~25 MB to ~8 MB — that's
   html5ever + DOM + HTTP client only. Most agent crawl tasks (markdown
   extraction, link harvesting, structured-data scraping) need zero JS.
   - **CLI default:** when the user passes `--dump markdown|links|text` and
     the page does not redirect via JS, never start V8.
   - **`--no-js` explicit flag** forces JS off and fails closed if a page
     refuses to render.
   - This is the single biggest memory win available. Lightpanda's
     "isolate-per-page" model leaves this on the floor.

2. **One V8 isolate per *process*, one *context* per Page.** (Diverges from
   both references — Lightpanda uses one isolate per Page; Obscura uses one
   isolate total but assumes one Page at a time.) A V8 isolate baseline is
   ~10 MB; a V8 Context is ~1-2 MB. For an N-page concurrent crawl, sharing
   the isolate saves ~(N-1) × 10 MB. Requires careful identity-map
   discipline so cross-context JS object access works correctly. The
   `BrowserProfile` is set once per isolate; contexts inherit.

3. **Memory-mapped V8 snapshot via `v8_use_external_startup_data=true`.**
   The startup snapshot (~3-4 MB) is mmap'd read-only from a sidecar file
   instead of baked into the binary. Multiple `mach` processes on the same
   box share the same physical pages. Saves RSS per process beyond the
   first.

4. **`mimalloc` with eager `purge_on_idle`.** System malloc holds onto
   freed pages indefinitely; over a 1000-page crawl that's the difference
   between flat RSS and a leak-shaped curve. mimalloc's purge-on-idle
   returns pages to the OS the moment a Page arena drops. ~1 MB binary
   cost; lower steady-state RSS by 20-40% on long crawls.

5. **Lazy Web API binding registration.** Don't install Canvas / WebGL /
   AudioContext / SubtleCrypto bindings into a Context until JS first
   touches them. Each unused binding family saves ~50-100 KB of V8 function
   templates. Most pages never touch WebGL. Tracked via a "first access"
   hook on `Window` / `globalThis` properties.

6. **Lazy DOM materialization.** Don't allocate `Text` node strings until
   something queries `.textContent` / `.nodeValue` / `.innerText`. Keep
   them as `(offset, length)` slices into the source HTML buffer (which
   we're already holding for the parser). Saves 30-50% of DOM memory on
   typical content-heavy pages. Wrap html5ever's `TreeSink` to never copy
   text content.

7. **String interning for tag names, attribute names, common attribute
   values** (`"true"`, `"false"`, classes that recur). One pool per
   Session, dropped on Session close. ~1-2 MB saved on link-heavy pages.

8. **`--single-threaded` V8** (skip background compile / GC threads). Saves
   ~3 MB of thread stacks per isolate and ~1 MB of internal queues. Slightly
   slower JIT warmup (~50 ms on a heavy challenge); acceptable.

9. **Strip-unused linker flags + LTO.** In CI release builds:
   `-Wl,--gc-sections -Wl,--icf=safe`, `RUSTFLAGS="-C lto=fat -C
   codegen-units=1 -C strip=symbols"`. Cuts binary 15-25%.

10. **Optional `--fork-per-page` mode** (Linux-only) for the CDP server.
    Each new CDP target = `fork()` of a parent process whose isolate is
    already primed with the BrowserProfile bindings. COW memory means the
    kernel only allocates pages that the new target actually writes; on
    target close, just `wait()` the child and the kernel reclaims all
    pages instantly — no GC needed. This is what gives Chromium's
    process-per-tab model its memory locality; we get the same shape for
    free because we have no IPC surface to maintain.

11. **HTTP/2 connection-pool caps.** Per-host max 2 idle connections (vs
    reqwest's default of unbounded). Idle-timeout 30 s. Per-host max-streams
    follows Chrome (100). Saves 1-3 MB per active host pair on long crawls.

12. **No `v8_enable_i18n_support=false`.** Tempting — would save 5-8 MB —
    but Turnstile uses `Intl.DateTimeFormat` / `Intl.Collator`. We pay for
    Intl.

**Aside on Bun / JavaScriptCore.** Bun is a *server-side JavaScript runtime*
(Node/Deno class), not a JS engine and not a browser. We cannot "use Bun" in
a browser the way we use V8. The engine Bun is built on, JavaScriptCore,
*could* be embedded in mach instead of V8 — but the analysis is unfavorable:
JSC saves only ~3-5 MB per isolate and ~20 MB of binary size, in exchange
for (a) redoing all binding work (`rusty_v8` doesn't speak JSC), (b) forcing
the entire `BrowserProfile` to be Safari-on-macOS, which is a smaller share
of legitimate traffic and therefore more suspicious to anti-bot systems,
(c) inheriting JSC's engine-specific fingerprint (Error.stack format,
`Function.prototype.toString` output, RegExp.toString) which Turnstile's
JS detector probes. The memory savings are smaller than what tactics #1-#3
above buy us, and the risk surface is much larger. **Recommendation
stands: V8.**

### 0.5 Revised memory budget (rev 3, with tactics applied)

Per-page resident set with the §0.4 tactics in place:

| Scenario | Tactics in play | Target RSS |
|---|---|---|
| `mach fetch --dump markdown <url>` (no JS) | #1 (lazy isolate), #4 (mimalloc), #6 (lazy DOM), #7 (intern) | **< 10 MB** |
| `mach fetch <url>` (JS, plain page) | #2 (shared isolate), #3 (mmap snapshot), #5 (lazy bindings), #4, #6, #7 | **< 20 MB** |
| `mach fetch <url>` (JS, Turnstile active) | as above + Canvas/WebGL/Audio/SubtleCrypto bindings live | **< 35 MB peak** |
| 10 concurrent pages via CDP | #2 (shared isolate across all), #10 (optional fork-per-page) | **< 80 MB total** |

Binary size projection with tactics #9 + #12:

- V8 statically linked: ~45-50 MB
- BoringSSL (via wreq): ~3 MB
- html5ever + dependencies: ~2 MB
- mach own code: ~3-5 MB
- Embedded profile data (canvas PNG + audio Float32Array + WebGL strings, x3 profiles): ~3 MB
- Everything else (tokio, hyper-h2, mimalloc, …): ~5 MB
- **Total: 60-70 MB. Well under the 100 MB hard cap.**

---

## 1. Core facts I'm starting from

Enumerated up front so the proposal can be challenged on premises, not just
conclusions.

1. **The "consumer" is a non-human agent.** No human is looking at pixels. We
   never need a compositor, GPU pipeline, font shaper, audio playback, video
   decoding, WebGPU, accessibility tree for screen readers, IME, printing,
   or extensions. **Caveat from rev 2:** we *do* need the *shape* of
   Canvas2D, WebGL, and AudioContext APIs because anti-bot challenges
   (Turnstile) read fingerprints from them. We satisfy the shape with
   deterministic spoofed outputs — no real rasterizer, no real GPU, no real
   audio DSP.
2. **The two reference implementations have already validated the thesis.**
   - **Lightpanda** (Zig + V8 + html5ever via FFI, AGPLv3): ~123 MB peak for
     100 pages vs Chrome's ~2 GB, ~9× faster on a real crawl benchmark. Single
     binary, custom DOM in Zig, CDP server, native MCP, "WebMCP" CDP domain,
     semantic-tree dump, libcurl-based networking with cache / interception /
     robots / WebBotAuth layers. ~85 Web API files implemented by hand.
   - **Obscura** (Rust + `deno_core` V8 + html5ever + reqwest, Apache-2.0):
     ~30 MB resident, ~70 MB binary, ~85 ms page load. Workspace split into
     `obscura-dom / -net / -js / -browser / -cdp / -mcp / -cli` crates,
     stealth mode + tracker blocklist as a feature flag, CDP + MCP-over-HTTP,
     SOP-policed sub-resource fetcher, V8 snapshot embedded at build time,
     separate `obscura-worker` binary for parallel scraping.
3. **Writing a JS engine from scratch is not viable.** V8, JSC, SpiderMonkey
   each represent ~10⁵ engineering-years. Both references took V8. So will
   we — the only honest question is *how* we embed it.
4. **Writing a parser from scratch is not viable either.** html5ever (Servo)
   is the spec-compliant HTML5 tokenizer + tree builder used in production by
   both references. The CSS selector matcher in Servo (`selectors`,
   `cssparser`) is similarly the de facto choice.
5. **No layout engine is needed for agents** — `querySelector`,
   `getBoundingClientRect` (synthesized from the DOM tree, not from real
   layout), accessibility-style traversal, and text/markdown serialization
   cover ~all agent use cases. We explicitly drop CSSOM box model, flexbox,
   grid, fragmentation, painting.
6. **CDP is the integration point.** Puppeteer, Playwright, and every existing
   automation tool already speak CDP. Implementing a subset of CDP is the
   cheapest way to be a drop-in for Headless Chrome.
7. **MCP is the *agent-native* integration point.** A locally-spawned MCP
   server lets an LLM call `navigate`, `read_markdown`, `click`, `eval`
   without needing a Puppeteer-style harness in between. Lightpanda has this
   over stdio; Obscura has it over HTTP — both are valuable.
8. **The repo `RezoxP/mach-browser` is empty.** This is greenfield. The
   "symbols I intend to modify" section below is therefore a *new-module*
   plan, not an edit plan.

### What I deliberately do **not** know yet, and need from you

- Preferred implementation language (the choice has consequences — see §6).
- License posture (AGPLv3 like Lightpanda vs Apache-2.0/MIT like Obscura).
- Target platforms day-one (Linux x86_64 only? + aarch64? + macOS? Windows
  native or WSL-only as Lightpanda has chosen?).
- ~~Whether stealth / anti-detection is in-scope for v1 or deferred.~~
  **Answered in rev 2: stealth is in-scope for v1 (R0.3).**
- ~~Whether headed (windowed) browsing must *ever* be supported.~~
  **Answered in rev 2: never (R0.4).**

I am not blocking on these — the design below is structured so language and
license can be picked at the very last moment without redoing the topology.

---

## 2. What we keep, what we drop, what we redesign

| Subsystem               | Chromium does it… | mach-browser will… |
|-------------------------|-------------------|--------------------|
| JS engine               | V8 (in-process)   | **Reuse V8** via a thin native binding |
| HTML parsing            | Blink             | **Reuse html5ever** (FFI / native) |
| CSS parsing + selectors | Blink             | **Reuse Servo `selectors` + `cssparser`** for `querySelector` *only* |
| CSSOM / style cascade   | Blink             | **Skip.** Provide stubs on Element that return zeroed `DOMRect` etc. unless explicitly opted in |
| Layout (flexbox/grid/…) | Blink             | **Skip entirely.** No box tree. No fragmentation. |
| Painting / Compositing  | Blink + Skia      | **Skip entirely.** No display list. No GPU. |
| Networking              | Chromium net      | **Chrome-fingerprint-shaped client** built on `wreq` / `rquest` (BoringSSL + curl_cffi-style TLS impersonation); HTTP/1.1+HTTP/2, no QUIC v1 (HTTP/3 phase 2), cookies, IPv6, proxies, robots.txt. **TLS fingerprint shaping is v1 core, not optional** — required to pass Turnstile. |
| DOM                     | Blink C++         | **Custom slab-allocated DOM** (NodeId-indexed arena, like Obscura) |
| Web APIs                | Blink C++         | **Hand-written, minimal-but-correct.** v1 surface (Turnstile-driven): `Document`, `Element`, `Node`, `Window`, `Location`, `History`, **full `Navigator` (~50 props)**, **`window.chrome` stub**, `Screen`, `XHR`, `fetch`, `URL`, `Timers`, `Console`, `Storage`, `EventTarget`, `MutationObserver`, `IntersectionObserver` (stubbed), **real `Crypto.subtle` (digest/sign/verify/encrypt/decrypt via RustCrypto)**, **`HTMLCanvasElement` with spoofed-`toDataURL`**, **`WebGLRenderingContext` returning hardcoded profile strings**, **`OfflineAudioContext` with spoofed `startRendering`**, `Permissions.query`, `Notification.permission`, `Intl.*` (V8 built-in). Grow on demand. |
| Multi-process sandbox   | Yes               | **No.** Single-process, single-binary. Optional `--worker N` spawns sibling processes for *crawling concurrency*, not for sandbox isolation. |
| GPU / Skia              | Yes               | **No.** |
| Media / WebRTC          | Yes               | **No.** Return capability=false. |
| Extensions / Web Store  | Yes               | **No.** |
| Service workers         | Yes               | **No in v1.** Defer; revisit if real sites break. |
| WASM                    | Yes               | **Yes** (V8 has it for free). |
| Automation protocol     | CDP (1st-class)   | **CDP subset** + **native MCP server** (stdio *and* streamable HTTP) |
| Output format           | Pixels            | **HTML, text, markdown, links, structured-data (JSON-LD / microdata / OG), semantic accessibility tree** |

This is the same shape Lightpanda and Obscura converged on independently, with
explicit and labeled choices.

---

## 3. Proposed module layout (control & data flow order)

A *single binary* called `mach`, internally a workspace of small modules.
Layout shown in the order data flows during a `mach fetch https://example.com`
invocation — the same order you'd read it to debug a bug.

```
mach/
├── cli/          # argv → Config; subcommands: fetch, serve, scrape, mcp
├── core/         # App, Config, Logger, Notification bus
├── profile/      # BrowserProfile registry (Chrome 131 Linux/Mac/Win, …)
│                #   — single source of truth shared by net/ and webapi/
├── net/          # HTTP client (wreq/rquest, Chrome TLS+HTTP2 fingerprint),
│                #   cookies, robots, blocklist, interception
├── parser/       # html5ever wrapper → DOM events
├── dom/          # NodeId arena, tree, selectors, serializer
├── js/           # V8 platform + isolate + binding bridge
├── webapi/       # Web API surface (Document, Element, Navigator, Canvas,
│                #   WebGL, AudioContext, SubtleCrypto, fetch, XHR, …)
├── fingerprint/  # Profile-driven spoof generators: canvas PNG bytes,
│                #   webgl strings, audio Float32Array, plus realm bootstrap
│                #   that hides webdriver / installs window.chrome
├── browser/      # Page, Frame, Session, ScriptManager, lifecycle
├── agent/        # markdown dump, semantic tree, links, structured data
├── cdp/          # CDP server + per-domain handlers
├── mcp/          # MCP server (stdio + streamable HTTP)
└── main.rs/.zig  # entrypoint
```

Two new modules vs rev 1: **`profile/`** (the shared `BrowserProfile`
registry; ~200 LoC of static data + a config picker) and **`fingerprint/`**
(generators that read a profile and produce the bytes Web APIs hand back to
JS). Both are tiny in source size but are the load-bearing pieces that make
the HTTP layer and the JS layer agree on "who we are."

A `mach fetch` call traces top-to-bottom; a `mach serve` (CDP) call adds
`cdp/` as an additional consumer of `browser/`; an `mcp` call swaps in `mcp/`
instead. **All three commands share the same `browser/`, `dom/`, `js/`,
`net/` stack.** That is what makes the system small.

### 3.1 Key symbol signatures (intent, language-agnostic pseudocode)

Comments under each symbol describe the *change* I'm proposing — i.e. what
exists in Lightpanda/Obscura that I'm keeping, dropping, or rethinking.

#### `core::App`
```rust
struct App {
    config: Config,
    allocator: Allocator,       // arena-per-page; arena-per-session
    notification_bus: Notification,
}
impl App {
    fn new(cfg: Config) -> Self;
    fn shutdown(self);
}
```
- **Change vs Lightpanda's `App.zig`:** keep the arena-per-page lifetime
  discipline (this is why Lightpanda's memory is flat across pages); drop
  Lightpanda's telemetry by default.
- **Change vs Obscura:** introduce an explicit `Notification` bus so CDP
  events, MCP progress, and internal logging share one fan-out and we don't
  re-grow the unstructured `tracing::info!` call sites Obscura accumulates.

#### `net::HttpClient`
```rust
struct HttpClient {
    inner: WreqClient,                // BoringSSL + Chrome TLS/HTTP-2 fingerprint
    profile: BrowserProfile,          // pinned at startup; Chrome 131 by default
    cookies: Arc<CookieJar>,
    interceptors: Vec<Box<dyn Interceptor>>,  // robots, cache, blocklist, user
    layers: LayerStack,                       // composable; order is config'd
}
impl HttpClient {
    fn new(profile: BrowserProfile, proxy: Option<Url>) -> Self;
    async fn request(req: Request) -> Response;
    fn add_layer<L: Layer>(&mut self, l: L);  // pre + post hooks per layer
}

// Pinned at startup; same struct read from by `webapi::Navigator`,
// `webapi::Screen`, the canvas/webgl/audio spoof generators, and the
// HTTP client header builder. ONE source of truth.
struct BrowserProfile {
    chrome_version: SemVer,           // e.g. 131.0.6778.85
    os: ProfileOS,                    // Linux | MacOS | Windows
    arch: ProfileArch,
    user_agent: &'static str,
    sec_ch_ua: &'static str,
    languages: &'static [&'static str],
    hardware_concurrency: u32,
    device_memory_gb: u8,
    screen: ScreenProfile,            // 1920×1080 default
    timezone: &'static str,           // "America/Los_Angeles" default
    webgl_vendor: &'static str,
    webgl_renderer: &'static str,
    canvas_fingerprint: &'static [u8],   // pre-rendered PNG bytes
    audio_fingerprint: &'static [f32],   // pre-computed Float32Array
}
```
- **TLS fingerprint is now a v1 core requirement.** `wreq` (with
  `wreq_util::Emulation::Chrome131` or similar) is the leading Rust
  option — Obscura's `obscura-net/src/wreq_client.rs` already validates it
  against real anti-bot stacks. `rquest` is the alternative.
- **Reject libcurl** (Lightpanda's choice). libcurl's ClientHello does not
  match Chrome; it is detected by JA3/JA4 within milliseconds.
- **Reject stock hyper/reqwest** (Obscura's non-stealth path). Same problem
  on HTTP/2 SETTINGS frame and header order.
- **Layered design** stays (Lightpanda's `network/layer/` shape). New
  policies (rate-limit, retry-with-backoff, host-blocklist) become 30-line
  `Layer`s, not forks. The fingerprint shaping lives *below* the layer
  stack inside `WreqClient`, so policy layers don't have to know about TLS.
- **`BrowserProfile` is the single source of truth for everything that has
  to look identical across HTTP and JS surfaces.** Mismatch between
  `User-Agent` header and `navigator.userAgent` is the most common stealth
  bug; the shared struct eliminates that class entirely.

#### `parser::Html`
```rust
fn parse(bytes: &[u8], base_url: &Url, sink: &mut DomTreeSink) -> ParseResult;
```
- **No new parser.** Wrap html5ever (Rust) or, if the host language is Zig,
  link html5ever as a static archive the way Lightpanda does.
- **Streamed:** parser pushes into `DomTreeSink` so the JS engine can begin
  scripting before the whole document has arrived (matches Chromium's
  speculative parser at a fraction of the complexity).

#### `dom::DomTree`
```rust
struct DomTree {
    nodes: Vec<Node>,            // slab; NodeId = u32 index
    version: u64,                // bumped on mutation; live-collection cache key
    string_pool: StringInterner, // tag names, attr names, classes
}
struct Node {
    parent: Option<NodeId>,
    first_child: Option<NodeId>,
    next_sibling: Option<NodeId>,
    prev_sibling: Option<NodeId>,
    data: NodeData,              // Element { tag, attrs }, Text, Comment, …
}
```
- **Match Obscura's `obscura-dom::tree.rs` shape exactly.** It is the right
  shape: NodeId indirection means JS objects can hold a stable handle that
  outlives mutations; a global `version` counter invalidates live
  `HTMLCollection` caches in O(1). Lightpanda's `dom_version` on `Page`
  proves this design at scale.
- **Add string interning** for tag/attr names. Both references re-allocate
  these; interning is ~free memory win on long crawls.

#### `js::Runtime`
```rust
struct Runtime {
    platform: V8Platform,        // process-singleton, init'd with v8_flags() (see below)
    isolate: V8Isolate,          // per Page; max_old_space_size=128, max_semi_space=2
    context: V8Context,          // per Frame; injects window.chrome stub at creation
    bindings: BindingRegistry,   // tag → Web API class
    identity: IdentityMap,       // Node* → V8 wrapper, monotonic
    profile: &'static BrowserProfile,  // same instance as net::HttpClient
}
impl Runtime {
    fn eval(&mut self, script: &str) -> Result<Value>;
    fn call_function(&mut self, name: &str, args: &[Value]) -> Result<Value>;
    fn pump_microtasks(&mut self);
    fn flush_animation_frames(&mut self);  // fake 60Hz vsync, called by waiter
    fn snapshot() -> &'static [u8];        // built-in startup snapshot
}

// V8 flags pinned at startup. Memory-tuned; JIT stays ON (Turnstile needs it).
fn v8_flags() -> &'static str {
    "--max-old-space-size=128 --max-semi-space-size=2 \
     --no-flush-bytecode --turbo-fast-api-calls"
}
```
- **Big architectural choice:** bind V8 directly (Lightpanda-style) or via
  `deno_core` (Obscura-style)?
  - **Recommendation: direct bindings (e.g. `rusty_v8` if Rust).** `deno_core`
    is excellent but brings Deno's ops + extension model, which assumes
    server-side semantics (CommonJS + ESM + worker pool). For a *browser-side*
    JS host where we need realm-per-iframe and a tight identity map between
    DOM nodes and V8 wrappers, the abstraction works against us. Lightpanda
    spent real effort building `Identity.zig`/`Origin.zig`; we want that
    control too.
- **Embedded V8 snapshot** at build time (both references do this). Cuts
  cold-start from ~200 ms to ~5 ms.
- **One isolate per Page, one context per Frame.** Same-origin iframes share
  an identity map (the Lightpanda `Page.identity` pattern); cross-origin
  frames get a fresh context.

#### `webapi::*` — the v1 Turnstile-driven surface
```rust
// Each Web API is a thin struct holding a NodeId + back-pointer to DomTree,
// bound into V8 via a generated dispatcher.

impl Element {
    fn query_selector(&self, sel: &str) -> Option<ElementRef>;
    fn query_selector_all(&self, sel: &str) -> NodeList;
    fn get_bounding_client_rect(&self) -> DOMRect;  // faux-layout: returns profile-derived values
    fn click(&mut self) -> Result<()>;              // dispatches synthetic MouseEvent
    fn focus(&mut self);
    // attribute / text / classlist ops…
}

// Navigator must look like real Chrome on inspection — Turnstile reads all of these.
impl Navigator {
    fn user_agent(&self) -> &str;                   // from BrowserProfile
    fn platform(&self) -> &str;
    fn languages(&self) -> &[&str];
    fn hardware_concurrency(&self) -> u32;
    fn device_memory(&self) -> u8;
    fn webdriver(&self) -> bool { false }           // hard-coded false; never true
    fn plugins(&self) -> PluginArray;               // Chrome-default plugins
    fn mime_types(&self) -> MimeTypeArray;
    fn user_agent_data(&self) -> NavigatorUAData;   // Sec-CH-UA equivalent
    fn permissions(&self) -> Permissions;
}

// window.chrome — Turnstile probes typeof window.chrome and a few subprops.
// Full functionality NOT needed; existence-shape IS.
fn install_window_chrome_stub(ctx: &mut V8Context);

// Screen + viewport — faux-layout default-on. All geometry APIs derive from
// BrowserProfile.screen so they're internally consistent.
impl Screen { /* width, height, availWidth, availHeight, colorDepth, pixelDepth */ }
impl Window {
    fn inner_width(&self) -> u32;   // from profile
    fn inner_height(&self) -> u32;
    fn device_pixel_ratio(&self) -> f32 { 1.0 }
    fn request_animation_frame(&mut self, cb: JsCallback) -> u32;  // queued; flushed on tick
}

// Canvas2D — spoofed fingerprint, NOT a real rasterizer.
// toDataURL/getImageData return profile.canvas_fingerprint, deterministically.
// Real drawing ops are no-ops on a 2D buffer the user never sees.
impl HTMLCanvasElement {
    fn get_context(&mut self, kind: &str) -> CanvasContext;
    fn to_data_url(&self, kind: &str) -> String;   // ← returns spoofed PNG
    fn to_blob(&self, cb: JsCallback);
}
impl CanvasRenderingContext2D { /* all drawing ops are recorded-but-not-rendered */ }

// WebGL — spoofed strings. getParameter(UNMASKED_*) returns profile values.
// All other gl.* calls succeed with sane defaults; no real GPU.
impl WebGLRenderingContext {
    fn get_parameter(&self, pname: u32) -> JsValue;  // ← spoof per profile
}

// AudioContext / OfflineAudioContext — spoofed audio fingerprint.
impl OfflineAudioContext {
    fn start_rendering(&mut self) -> Promise<AudioBuffer>;  // ← returns profile.audio_fingerprint
}

// Web Crypto — REAL implementations (RustCrypto: sha2, hmac, aes-gcm, p256, …).
// Turnstile actually validates HMAC/SHA output server-side; cannot spoof.
impl SubtleCrypto {
    fn digest(&self, alg: &str, data: &[u8]) -> Promise<Vec<u8>>;       // real SHA-256/384/512
    fn sign(&self, key: &CryptoKey, data: &[u8]) -> Promise<Vec<u8>>;   // real HMAC, ECDSA, RSA
    fn encrypt(&self, alg: AlgParams, key: &CryptoKey, data: &[u8]) -> Promise<Vec<u8>>;
    fn decrypt(&self, alg: AlgParams, key: &CryptoKey, data: &[u8]) -> Promise<Vec<u8>>;
    fn generate_key(&self, alg: AlgParams) -> Promise<CryptoKey>;
    fn import_key(&self, format: &str, data: &[u8], alg: AlgParams) -> Promise<CryptoKey>;
}
```
- **Surface is hand-rolled, but the v1 minimum is now Turnstile-driven, not
  agent-driven.** Roughly the union of (a) the surface real sites assume
  exists, (b) the surface Turnstile probes.
- **Spoofing strategy: deterministic per `BrowserProfile`.** Same profile →
  same canvas/audio/webgl fingerprints across runs. This matches real
  machines (which also produce the same fingerprint repeatedly) and avoids
  the "randomized fingerprint = bot" detector. Profiles ship as static
  bytes in the binary; switching profile is `--profile=chrome-131-linux` etc.
- **Use a binding *generator* from day one.** Both references hand-write the
  V8 plumbing per class; that's where their bug surface concentrates.
  Generate the V8 accessor/method shims from a small DSL or directly from a
  subset of WebIDL. **This is the single biggest leverage point for being
  smaller than them.**
- **Lightpanda already has Canvas/WebGL/AES/HMAC/RSA/X25519 modules** —
  proves the surface is implementable at the binary-size budget. We adopt
  the same shape, replace real rendering with profile-driven spoofing
  where it's safe to do so.

#### `browser::Page`, `browser::Frame`, `browser::Session`
```rust
struct Session { browser_arena: Arena, cookie_jar: CookieJar, pages: Vec<Page> }
struct Page    { frame: Frame, factory: Factory, identity: IdentityMap, dom_version: u64 }
struct Frame   { url: Url, document: NodeId, js_context: V8Context, script_mgr: ScriptManager }
```
- **Match Lightpanda's three-tier (`Browser → Session → Page → Frame`)
  taxonomy.** It maps cleanly onto CDP's BrowserContext / Target / Frame and
  costs nothing.
- **One arena per Page lifetime.** Everything allocated during navigation
  goes into the page arena; navigation = drop the arena. This is why
  Lightpanda doesn't leak across 100 pages.

#### `agent::SemanticTree`, `agent::markdown`, `agent::structured_data`
```rust
fn dump_html(page: &Page) -> String;
fn dump_text(page: &Page) -> String;
fn dump_markdown(page: &Page) -> String;
fn dump_links(page: &Page) -> Vec<Link>;
fn dump_semantic_tree(page: &Page, opts: SemanticOpts) -> SemanticNode;
fn dump_structured_data(page: &Page) -> StructuredData; // JSON-LD, microdata, OG
```
- **This is the agent-first differentiator vs Headless Chrome.** Lightpanda
  ships exactly this (`SemanticTree.zig`, `markdown.zig`, `links.zig`,
  `structured_data.zig`); we should ship the same surface and treat it as a
  *first-class output format alongside HTML*.
- The semantic tree is essentially the accessibility tree minus
  presentational nodes — what an LLM actually wants to read.

#### `cdp::Server`
```rust
struct CdpServer { listener: TcpListener, sessions: HashMap<SessionId, BrowserSession> }
fn dispatch(method: &str, params: Value) -> Result<Value>;
// domains: Browser, Target, Page, DOM, Runtime, Network, Fetch, Input,
//          Storage, Emulation, Log, Performance, Accessibility, WebMCP
```
- **Implement only the domains Puppeteer and Playwright actually call in
  their default flows.** Lightpanda's domain list (see
  `src/cdp/domains/`) is a good v1 cutoff.
- **`WebMCP` CDP domain (Lightpanda invention).** Re-implement; it's how an
  agent can call MCP tools *from inside the page's JS context*. Without
  this, MCP and CDP are two separate worlds that don't compose.

#### `mcp::Server`
```rust
fn serve_stdio()  -> !;  // JSON-RPC 2.0 over stdin/stdout
fn serve_http(addr: SocketAddr) -> !;
// tools: navigate, dump_html, dump_markdown, dump_links, eval, click,
//        fill, screenshot(unimpl), wait_for, list_cookies, set_cookie
```
- **Offer both transports.** Stdio for "LLM spawns mach as a subprocess" (the
  Lightpanda model); HTTP for "long-running agent server" (the Obscura
  model). They share the same tool registry.
- **Tools must be safe-by-default.** `eval` returns a *string* (not a JS
  object handle) unless explicitly enabled, because LLMs cannot juggle
  refcounts.

---

## 4. End-to-end control & data flow (one page fetch, Turnstile-aware)

```
[ CLI ] argv → Config (incl. --profile=chrome-131-linux)
   │
[ Profile ] resolve BrowserProfile (UA + screen + WebGL strings + canvas PNG
   │         + audio buffer + TLS fingerprint preset). Single instance.
   │
[ App ] open Session, open Page (arena), open Frame, open V8 isolate+context
   │     ├─ js: install window.chrome stub
   │     ├─ js: install Navigator/Screen properties from profile
   │     └─ js: install spoofed Canvas/WebGL/Audio bindings
   │
[ Net ] HttpClient.request(GET https://example.com)
   │     ├─ wreq emits Chrome-shaped ClientHello (JA3/JA4 match)
   │     ├─ wreq emits Chrome-shaped HTTP/2 SETTINGS + WINDOW_UPDATE
   │     ├─ wreq emits Chrome header order incl. Sec-CH-UA*
   │     ├─ RobotsLayer.allow?
   │     ├─ InterceptionLayer.maybe_rewrite_or_block
   │     ├─ CacheLayer.lookup → miss
   │     └─ Forward → Response{200, text/html, body, set-cookie: cf_*}
   │
[ Parser ] html5ever.parse(body) → events → DomTreeSink → DomTree
   │
[ JS ] for each <script> (incl. Turnstile widget):
   │     ScriptManager schedules (defer/async/sync rules)
   │     v8.eval in Frame context
   │     ├─ Turnstile probes navigator.* / window.chrome / screen.*
   │     │   → served from BrowserProfile, consistent with HTTP UA
   │     ├─ Turnstile creates Canvas / WebGL / OfflineAudio
   │     │   → spoofed outputs match profile fingerprint
   │     ├─ Turnstile runs crypto.subtle.digest / hmac
   │     │   → REAL RustCrypto output
   │     └─ Turnstile schedules rAF + setTimeout → flushed by js::Runtime
   │
[ Wait ] until { networkidle0 | networkidle2 | load | selector | script | ms }
   │
[ JS ] Turnstile POSTs challenge response via fetch(); HttpClient re-emits
   │   Chrome-shaped TLS/HTTP-2 for *that* request too. Server returns token.
   │
[ Agent ] dump_markdown(page) | dump_semantic_tree(page) | …
   │
[ Out ] stdout or HTTP response or CDP `Page.captureSnapshot` reply
```

The arrows from `Profile` are the key new dependency: HTTP-layer
fingerprint and JS-layer fingerprint **must** be derived from the same
struct, or sites cross-correlate them and detect the mismatch. This is the
design mistake every from-scratch automation browser eventually makes.

---

## 5. Where my design diverges from Lightpanda *and* Obscura

These are the deliberate bets — call them out so we can argue about each one.

1. **Binding generator over hand-written V8 glue.** Both references hand-write
   the V8 ↔ DOM glue per class. That is their single largest source code
   surface and their single largest bug surface. Generating it from a small
   IDL subset trades a few weeks of tooling work for an order-of-magnitude
   smaller `webapi/` directory long-term.
2. **Both stdio *and* streamable-HTTP MCP, sharing one tool registry.**
   Lightpanda is stdio-only; Obscura is HTTP-only. Agents legitimately want
   both depending on whether they spawn-and-pipe or connect to a long-lived
   service.
3. **CDP "WebMCP" domain mandatory in v1.** Lightpanda invented this and it's
   the one piece of glue that lets an in-page script talk to the agent
   *through the browser*, not around it. Without it MCP and CDP keep
   diverging.
4. **Stealth is v1 core, not a feature flag.** (Rev 2 reversal.) Both
   references treat anti-detection as optional — Obscura via
   `--features stealth`, Lightpanda by being deliberately honest about
   itself. We can't do that because **passing Turnstile is a v1 acceptance
   gate** (R0.3). Stealth-as-default has architectural consequences:
     - the network stack is built on `wreq`/`rquest` (Chrome-fingerprinted)
       from day one, not on stock hyper/curl;
     - the `BrowserProfile` struct is the single source of truth for both
       the HTTP header builder *and* the JS Navigator/Screen/Canvas/WebGL
       bindings, making cross-surface mismatch impossible at compile time;
     - `navigator.webdriver === false` is wired in at V8 context creation,
       not exposed as a runtime toggle.
   The `--obey-robots` flag still exists and is on by default — stealth
   means "undetectable as bot," not "hostile to good-faith site policy."
5. **`Notification` bus instead of ad-hoc `tracing::info!` calls.** Forces a
   structured event model that CDP and MCP both consume. Costs ~200 lines;
   buys correctness for free across the lifetime of the project.
6. **Aggressive arena reuse:** one arena per Page, one per Session, one per
   request. No per-allocation `free` paths anywhere on the hot crawl loop.
   Lightpanda gets this from Zig naturally; in Rust we'd use `bumpalo` or a
   custom slab.
7. **Single shared `BrowserProfile` between HTTP and JS surfaces.** Neither
   reference enforces this. Obscura's stealth UA lives in
   `wreq_client::STEALTH_USER_AGENT` while its JS `navigator.userAgent` is
   set elsewhere via ops; nothing prevents drift. We make drift a *type
   error*.

---

## 6. Implementation language — the honest tradeoff

| Aspect | Rust | Zig | C++ | Go |
|--------|------|-----|-----|----|
| V8 binding | `rusty_v8` / `v8` crate (mature) | hand-rolled C ABI binding (Lightpanda did it) | native | cgo nightmare |
| html5ever | native | FFI (already proven by Lightpanda) | FFI | FFI |
| Memory model fit | ownership + arenas (bumpalo) | arenas first-class | manual | GC fights arena story |
| Async story | tokio is excellent | new + small std async | none std | excellent |
| Hireability / contributor pool | large | very small | large but selects against this domain | large |
| Binary size | medium (LTO 30-70 MB) | smallest (~30-40 MB) | smallest with effort | largest |
| Cross-compile to musl/aarch64 | great | great | painful | great |

**Recommendation: Rust.** Obscura's stack (deno-style V8 binding optional,
hyper/reqwest, tokio, html5ever) is closer to "industry standard" than
Lightpanda's, which means more drive-by contributors. We keep Lightpanda's
*ideas* (arenas, layer-stack networking, semantic tree, WebMCP) but in a
language that more people on the team can move in confidently.

**Decision in rev 4: Rust is locked in.** The cross-platform requirement
added in rev 4 makes Rust an even better fit — `rusty_v8`, `wreq`, `tokio`,
`memmap2`, `mimalloc`, `html5ever`, and `dirs` all build cleanly on both
Linux and Windows MSVC out of the box, which is not true of the
equivalent Zig/Go/C++ toolchains for at least one of the dependencies.

---

## 7. Key technical challenges (ordered by how likely they are to bite)

0. **Turnstile pass is the v1 acceptance gate, not a stretch goal.** Every
   other risk below is one we can incrementally fix; failing Turnstile means
   the product is invisible to half of real-world automation work. The
   architecture must be measured end-to-end against a Turnstile test page
   from Phase 2 onward, on *every* CI run. If a fingerprint regresses, CI
   fails.
0a. **Fingerprint drift between HTTP and JS surfaces.** The single most
   common stealth bug: HTTP `User-Agent: Chrome/131` but `navigator.userAgent`
   says `Chrome/127`. The `BrowserProfile` shared-struct discipline in
   `profile/` exists to make this *impossible at compile time* — if the
   field doesn't live on `BrowserProfile`, it doesn't get used.
0b. **`wreq` / `rquest` track real Chrome on a moving target.** Both crates
   need updates within days of a major Chrome release that changes
   ClientHello extensions or HTTP/2 SETTINGS. CI must run the Turnstile test
   nightly so we catch upstream drift early.
1. **JS event-loop semantics without a render loop.** Real browsers tie
   `requestAnimationFrame`, `IntersectionObserver`, `ResizeObserver`, and
   microtask scheduling to the compositor's vsync. We have no vsync. Many
   modern frameworks (React, Vue) will hang or busy-loop if `rAF` never
   fires. Turnstile's challenge schedules work via rAF too. Lightpanda's
   solution is a virtual clock + opt-in `rAF` flush per step; we must do
   the same. **This is the #1 source of "site X just hangs" bugs.**
2. **Identity & lifetime across V8 GC and DOM mutations.** A JS variable
   holding `document.body` must keep returning the same object after the
   page mutates around it. Cross-realm identity (same-origin iframes) is the
   subtle case — Lightpanda has a whole `Identity.zig`/`Origin.zig` pair
   precisely for this. Getting it wrong = mysterious `null` returns and
   double-frees.
3. **CDP wire compatibility is a long tail.** Puppeteer and Playwright issue
   *dozens* of CDP commands during a single `goto`. Missing one returns
   `MethodNotFound` and the client falls back to raw JS, which usually means
   "site doesn't render correctly under mach but works in Chrome." Coverage
   has to be measured against a Puppeteer/Playwright test corpus from day
   one, not vibes.
4. **No layout means `getBoundingClientRect`, `offsetTop`, `scrollIntoView`
   all lie.** Sites that *gate* behavior on geometry (lazy-load images,
   carousel autoplay, "scroll-into-view to load more") will misbehave. Plan:
   ship a *faux-layout* mode behind `--faux-layout=approx` that returns
   sentinel non-zero rects and synthesizes scroll events, off by default,
   tested per-site.
5. **TLS / HTTP/2 fingerprinting and the anti-bot industry.** Cloudflare,
   Akamai, DataDome, and PerimeterX all fingerprint at the TLS/HTTP layer
   *before* any JS runs. A from-scratch HTTP client is trivially detectable.
   **Rev 2: solved-by-design** via `wreq` + `BrowserProfile`. Risk
   downgrades from architectural to operational — keeping the fingerprint
   in sync with real Chrome is now a recurring maintenance cost, not a
   one-time engineering task.
6. **Streaming HTML parser ↔ scripting interleaving.** Inline `<script>`
   execution must block the parser at exactly the right offset, and the
   parser must re-enter after the script completes. Doing this on top of
   html5ever's streaming API is fiddly. Lightpanda has a `ScriptManager`;
   we'll need the equivalent, and it'll be hard to write.
7. **`fetch` / `XHR` semantics: CORS, credentials, redirects, ReadableStream.**
   Lightpanda's status table flags CORS as still open. CORS without an
   actual rendering origin is partly a fiction, but agents that care about
   correctness need it. Plan: implement the "block on simple-vs-non-simple"
   rules correctly, skip preflight caching in v1.
8. **WPT (Web Platform Tests) coverage.** Lightpanda runs against a forked
   WPT. Without continuous WPT signal we will silently regress DOM
   correctness with every refactor. Setting WPT up *first*, before any Web
   API code, is non-negotiable.
9. **V8 snapshot drift on upgrade.** When V8 bumps (every ~4 weeks), the
   embedded snapshot must be regenerated. Both references gate this in CI.
   Plan: pinned V8 version + reproducible snapshot job.
10. **Single-process means a crash kills the whole crawl.** Chromium has
    process-per-tab. We don't. Either accept it (mach is one binary, you
    `--workers N` for parallelism) or build a thin parent supervisor that
    respawns dead workers. Obscura already chose the latter shape with
    `obscura-worker`; mach should too.
11. **License compatibility.** V8 is BSD-3; html5ever is MPL-2.0; Servo
    `selectors` / `cssparser` are MPL-2.0; libcurl is curl-license; hyper /
    reqwest / rustls are MIT/Apache-2.0. None block AGPLv3 *or* Apache-2.0
    *outbound*. Just pick and document.
12. **Maintenance cost of a hand-rolled Web API surface is uncapped.** This
    is the slow-bleed risk Lightpanda is paying off forever (`webapi/` has
    ~85 files and growing). The IDL-generator bet in §5 is mostly to bound
    *this* risk.

---

## 8. Phased delivery plan (rev 2 — Turnstile is now a gate, not a stretch)

Each phase is a shippable milestone with a hard acceptance test.

- **Phase 0 — Skeleton (1 wk).** Workspace, CI, V8 vendored + snapshot built,
  html5ever wired, `wreq` integrated with `Emulation::Chrome131`,
  `BrowserProfile` registry with one profile, `mach fetch --dump html`
  returns body for a static page (no JS yet).
  *Gate:* binary builds under 100 MB; cold start < 100 ms.
- **Phase 1 — JS-enabled fetch + minimum Web API (3-4 wk).** V8
  isolate-per-page, the v1 Turnstile-driven Web API surface (Document,
  Element, Window, Navigator, Screen, Console, Timers, fetch, XHR, URL,
  Location, MutationObserver, IntersectionObserver-stub, EventTarget).
  *Gate:* `mach fetch --dump html` returns post-JS HTML for the lightpanda
  demo corpus; RSS < 50 MB on a single navigation.
- **Phase 2 — Stealth + Turnstile pass (2-3 wk).** `fingerprint/` module
  with Canvas/WebGL/Audio spoofing, real `SubtleCrypto` via RustCrypto,
  `window.chrome` stub, `Permissions` stub, full Navigator surface,
  faux-layout defaults, rAF virtual-vsync scheduler.
  *Gate:* **Cloudflare Turnstile managed/non-interactive challenge passes
  end-to-end on a third-party test page, producing a valid
  `cf-turnstile-response` token.** This is the v1 release criterion.
- **Phase 3 — Agent outputs (1-2 wk).** markdown, links, structured-data,
  semantic-tree exporters. This is where we *visibly* beat headless Chrome
  for agents — ship a benchmark blog post comparing against Lightpanda,
  Obscura, and Headless Chrome on (a) memory, (b) Turnstile pass rate.
- **Phase 4 — CDP subset (3-4 wk).** Enough CDP to run Puppeteer's `goto`,
  `evaluate`, `click`, `waitForSelector`, `cookies`, `setCookie`, basic
  `Network.requestWillBeSent` events. Validated against Puppeteer's own
  test corpus.
- **Phase 5 — MCP server + WebMCP (1 wk).** Tool registry, stdio + streamable
  HTTP, CDP-WebMCP bridge.
- **Phase 6 — Robustness (ongoing).** WPT coverage, more Web APIs by demand,
  more profiles (Firefox / Safari / iOS Chrome), hCaptcha / DataDome eval,
  optional `--solver` flag for interactive captchas, HTTP/3, service workers
  (maybe).

**CI must include a Turnstile-pass test from Phase 2 onward and run it
nightly.** This is the only way to catch fingerprint regressions when
`wreq` / V8 / our own bindings move.

---

## 9. Open questions — closed out (rev 4)

All answered. Recorded here for the project history; the rest of the doc
is aligned to these answers.

(a) ~~Language?~~ **Rust** (edition 2021).
(b) ~~License?~~ **Apache-2.0**.
(c) ~~Stealth in v1?~~ **Yes, mandatory** (Turnstile pass).
(d) ~~Repo / name?~~ **`RezoxP/mach-browser`**.
(e) ~~JS engine?~~ **V8** (locked by Turnstile).
(f) ~~Scope beyond Turnstile?~~ **Turnstile only in v1.** hCaptcha,
    DataDome, Akamai BMP, PerimeterX are **Phase-6** goals or sponsored
    work. Out of v1 gate.
(g) ~~`--solver` integration?~~ **Phase 5/6 optional flag.** Tiny HTTP
    adapter for 2captcha / anti-captcha; not in core.
(h) ~~Profiles in v1?~~ **One** Chrome-Linux profile. Additional profiles
    (Chrome-Win, Chrome-Mac, Firefox-Linux, …) are data-only contributions
    in later versions; no architecture change.
(i) ~~Platforms?~~ **Linux + Windows first-class in v1**, macOS in Phase-6.
    See §0.7.

---

## 10. What I'm *not* proposing, and why

For completeness — these came up in research and were rejected.

- **Servo as a base.** Tempting (we'd reuse html5ever, selectors,
  cssparser, layout-2020). Rejected: pulls in SpiderMonkey, WebRender, and a
  layout engine we explicitly don't want. The savings vanish.
- **QuickJS instead of V8.** ~3× smaller binary, but real sites assume
  V8-grade performance and spec coverage. Sites using modern syntax via
  Babel will mostly work; sites using `Intl`, weak refs, top-level await,
  decorators will not. Wrong tradeoff for "drop-in for Headless Chrome".
- **Writing our own HTML parser.** It would take longer than every other
  module combined and would never be more correct than html5ever.
- **Multi-process renderer sandboxing.** Sandboxing exists to protect the
  user from the page. Agents do not need that protection; they are the user.
- **GUI / headed mode "just for debugging."** Once it exists it never goes
  away and becomes the source of half of every PR's complexity. Use CDP
  inspector clients (Chrome DevTools frontend speaks CDP, you can attach it
  to mach) for debugging instead.

---

## Appendix A — Reference architecture comparison (raw facts)

### Lightpanda (`lightpanda-io/browser`)

- **Language:** Zig 0.15.2
- **JS:** V8 (direct C-ABI binding in `src/browser/js/`)
- **HTML:** html5ever via FFI (vendored in `src/html5ever/`)
- **HTTP:** libcurl + layered `network/layer/` (Cache, Interception, Robots,
  WebBotAuth, Forward)
- **DOM:** custom Zig, NodeId-style, `dom_version` on Page for live-collection
  invalidation
- **WebAPI:** ~85 hand-written `.zig` files under `src/browser/webapi/`
- **CDP:** full set of domains in `src/cdp/domains/` including the
  Lightpanda-invented **`WebMCP`** domain
- **MCP:** stdio-only, `src/mcp/`
- **Agent outputs:** `SemanticTree.zig`, `markdown.zig`, `links.zig`,
  `structured_data.zig`, `interactive.zig` (for clickable surface dump),
  `forms.zig`
- **Tests:** custom test runner + WPT fork
- **License:** AGPL-3.0
- **Benchmark:** 123 MB / 5 s for 100 pages (vs Chrome 2 GB / 46 s)

### Obscura (`h4ckf0r0day/obscura`)

- **Language:** Rust 2021
- **JS:** V8 via `deno_core` + embedded snapshot at `OBSCURA_SNAPSHOT_PATH`
- **HTML:** html5ever native + Servo `selectors` + `cssparser`
- **HTTP:** `reqwest` with cookies / gzip / brotli / deflate / socks /
  native-tls-vendored, on tokio; SOP-policed sub-resource fetch in
  `obscura-browser::page::subresource_allowed`
- **DOM:** `obscura-dom`, NodeId-indexed `Vec<Node>`, html5ever `TreeSink`
- **WebAPI:** thinner than Lightpanda, via `obscura-js::ops` extension
- **CDP:** `obscura-cdp` with Accessibility, Browser, DOM, Fetch, Input, LP,
  Network, Page, Runtime, Storage, Target
- **MCP:** **HTTP** transport (`obscura-mcp::http`), JSON-RPC over POST `/mcp`
- **Stealth:** `--features stealth` flag + `pgl_domains.txt` tracker list
- **Concurrency:** parallel scrape via separate `obscura-worker` binary
- **License:** Apache-2.0
- **Numbers:** ~30 MB RSS, ~70 MB binary, ~85 ms page load

### Convergent design choices (both projects independently picked these)

- V8 (not a from-scratch JS engine)
- html5ever (not a from-scratch HTML parser)
- Slab-allocated, NodeId-indexed DOM (not pointer-graph)
- No layout / paint / GPU / media
- CDP server as the integration surface
- MCP server built in
- Markdown / structured-data dump as first-class output
- Embedded V8 snapshot at build time
- Single-binary distribution
- Stealth / anti-detect treated as a separable concern

The convergence is itself the strongest evidence that the proposed shape for
mach-browser is the right one.
