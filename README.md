# mach-browser

An ultra-lightweight browser built from scratch for AI agents and automation.

**Not** a Chromium fork. **Not** a WebKit patch.

Headline targets:

- < 10 MB RSS for the no-JS `mach fetch --dump markdown` path
- < 20 MB RSS for JS-enabled fetches on plain pages
- < 35 MB peak RSS under a Cloudflare Turnstile challenge
- < 100 MB binary (60-70 MB target with LTO + strip)
- < 100 ms cold start

## Status — Phase 0 (skeleton)

This is the Phase 0 skeleton: workspace layout, CI matrix (Linux + Windows
MSVC), the validated `wreq` HTTP stack with Chrome 131 TLS+HTTP/2 fingerprint,
an `html5ever`-backed parser, and a no-JS `fetch` command that dumps HTML,
links, text, or rough markdown. **No JavaScript yet** — V8 lands in Phase 1.

See the architecture proposal in `docs/architecture.md` for the full plan.

## Building

### Linux

```
cargo build --release
./target/release/mach fetch --dump html https://example.com
```

### Windows (MSVC)

Phase 0 requires the following toolchain components on Windows:

1. `rustup` with the `x86_64-pc-windows-msvc` host toolchain.
2. **Visual Studio 2022 Build Tools** with the **VCTools** workload
   (`cl.exe`, `link.exe`, Windows SDK, `cmake`).
3. **NASM** on `PATH` (BoringSSL's perlasm output needs it).
4. **LLVM/Clang** with `LIBCLANG_PATH` exported (`bindgen` needs `libclang.dll`).

Then from a Developer Command Prompt for VS 2022:

```
set LIBCLANG_PATH=C:\Program Files\LLVM\bin
cargo build --release
target\release\mach.exe fetch --dump html https://example.com
```

These prereqs are why the Windows CI job in `.github/workflows/ci.yml`
installs `nasm` and `llvm` via Chocolatey before invoking `cargo build`.

## CLI

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

Exit codes: `0` success, `1` HTTP error, `2` parse error, `3` argument error.

JavaScript is not yet supported. JS-rendered pages will return whatever the
server emits server-side. JS support arrives in Phase 1.

## License

Apache 2.0. See `LICENSE-APACHE` and `NOTICE`.
