// rusty_v8 v130 ships an `etw-jit-win.obj` + `wintz.obj` (from V8 + bundled
// ICU) that reference Windows advapi32 symbols (`EventRegister`,
// `EventSetInformation`, `RegOpenKeyExW`, etc.) without linking advapi32
// itself. Newer rusty_v8 versions fix this in their build script; until we
// move to one of those, link it manually so MSVC linking succeeds.
fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rustc-link-lib=dylib=advapi32");
    }
}
