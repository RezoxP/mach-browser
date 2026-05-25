# mach-browser

An ultra-lightweight browser built from scratch for AI agents and automation.

**Not** a Chromium fork. **Not** a WebKit patch.

Headline targets:

- < 10 MB RSS for the no-JS `mach fetch --dump markdown` path
- < 20 MB RSS for JS-enabled fetches on plain pages
- < 35 MB peak RSS under a Cloudflare Turnstile challenge
- < 100 MB binary (60-70 MB target with LTO + strip)
- < 100 ms cold start

## Status — Phase 1A (V8 wired up)

What works today:

- Workspace + CI matrix (Linux + Windows MSVC).
- `wreq` HTTP stack with Chrome 131 TLS+HTTP/2 fingerprint.
- `html5ever`-backed parser + arena-backed DOM.
- `mach fetch` returns HTML, markdown, links, or visible text from any
  static page. JS is never loaded for this path — confirmed `< 6 ms` cold
  start (Tactic #1, lazy V8 init).
- `mach js --eval '<expr>'` evaluates an arbitrary JS expression in a
  fresh V8 isolate (~9 ms cold). No DOM, no Web APIs, no network — just
  raw V8. This is here to prove the engine works end-to-end on every
  supported platform before DOM bindings land.

Not yet:

- DOM / EventTarget / Window / Navigator JS bindings (Phase 1B+).
- `mach fetch --execute-js <URL>` — the post-JS-evaluation render path
  (Phase 1B).
- Fingerprint spoofing, `window.chrome`, real `SubtleCrypto`, Canvas /
  WebGL / Audio shims (Phase 2).
- Cloudflare Turnstile pass (Phase 2 gate, the v1 release criterion).

See the architecture proposal in `docs/architecture.md` for the full plan.

## Building

### Linux

```
cargo build --release
./target/release/mach fetch --dump html https://example.com
```

### Windows (MSVC)

Phase 1A requires the following toolchain components on Windows:

1. `rustup` with the `x86_64-pc-windows-msvc` host toolchain.
2. **Visual Studio 2022 Build Tools** with the **VCTools** workload
   (`cl.exe`, `link.exe`, Windows SDK, `cmake`).
3. **NASM** on `PATH` (BoringSSL's perlasm output needs it).
4. **LLVM/Clang** with `LIBCLANG_PATH` exported (`bindgen` needs `libclang.dll`).

(V8 itself ships as a prebuilt static library via the `v8` crate — no
`depot_tools`, GN, ninja, or Python 3 needed.)

Then from a Developer Command Prompt for VS 2022:

```
set LIBCLANG_PATH=C:\Program Files\LLVM\bin
cargo build --release
target\release\mach.exe fetch --dump html https://example.com
```

These prereqs are why the Windows CI job in `.github/workflows/ci.yml`
installs `nasm` and `llvm` via Chocolatey before invoking `cargo build`.

## Pre-built binaries

You don't need to build from source — every push to `main` produces a
downloadable binary, and tagged releases produce permanent GitHub Release
assets.

- **Latest dev build:** open the [Actions tab][actions], pick the most recent
  `Release` workflow run, scroll to the "Artifacts" section, and download
  `mach-<short-sha>-x86_64-unknown-linux-gnu` (Linux) or
  `mach-<short-sha>-x86_64-pc-windows-msvc` (Windows). Each artifact ships
  the binary plus a `.sha256` checksum file. Dev artifacts are retained for
  90 days.
- **Tagged release:** see the [Releases page][releases] for permanent
  download URLs of `mach-vX.Y.Z-<target>.{tar.gz,zip}`.
- **Trigger a build manually:** any maintainer can hit "Run workflow" on
  the [Release workflow][workflow] to produce fresh artifacts off any
  branch.

[actions]: https://github.com/RezoxP/mach-browser/actions
[releases]: https://github.com/RezoxP/mach-browser/releases
[workflow]: https://github.com/RezoxP/mach-browser/actions/workflows/release.yml

## CLI

### `mach fetch` — HTTP-only page download

```
mach fetch [--dump html|markdown|links|text]
           [--user-agent STR]
           [--timeout SECS]
           <URL>
```

| `--dump`   | Output                                                   |
|------------|----------------------------------------------------------|
| `html`     | Re-serialized HTML after `html5ever` round-trip (default) |
| `markdown` | Rough markdown extraction (Phase 0: link + heading text) |
| `links`    | One `href` per line, deduplicated, in document order     |
| `text`     | Visible text content with whitespace collapsed           |

This path **does not initialize V8**. Cold start is ~5-6 ms on a release
build, RSS stays in single-digit MB. JS-rendered pages will return whatever
the server emits server-side — the post-JS render path lands in Phase 1B
as `mach fetch --execute-js`.

### `mach js` — evaluate a JavaScript expression

```
mach js --eval '<source>'
mach js --file <path>      # use `-` for stdin
```

Compiles and runs the source in a fresh V8 isolate. The result is coerced
to a string (V8's `ToString`) and printed to stdout. Exceptions print to
stderr and exit with code 1.

```
$ mach js --eval '21 + 21'
42
$ mach js --eval 'JSON.stringify({a: 1, b: [2, 3]})'
{"a":1,"b":[2,3]}
$ echo 'Math.sqrt(2)' | mach js --file -
1.4142135623730951
```

No DOM, no Web APIs, no `fetch`, no `document` — those land in Phase 1B+.
This subcommand exists to (a) prove V8 itself works on every supported
platform and (b) let CI smoke-test the JS engine.

### Exit codes

`0` success, `1` HTTP / JS / I/O error, `2` parse error, `3` argument error.

## License

Apache 2.0. See `LICENSE-APACHE` and `NOTICE`.
