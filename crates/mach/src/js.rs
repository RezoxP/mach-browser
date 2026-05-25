//! `mach js` subcommand: evaluate a JavaScript snippet in a fresh V8 isolate.
//!
//! Phase 1A: no DOM, no Web APIs. The point is to (a) prove V8 runs on every
//! supported platform and (b) give CI / users a way to smoke-test it without
//! the cost of a network fetch. Web API surface arrives in Phase 1B+.

use std::io::Read;

use mach_core::{Error, Result};
use mach_js::JsRuntime;

use crate::cli::JsArgs;

/// Run the `mach js` subcommand.
pub fn run(args: JsArgs) -> Result<()> {
    let source = read_source(&args)?;
    let mut rt = JsRuntime::new();
    let value = rt.eval(&source)?;
    println!("{value}");
    Ok(())
}

fn read_source(args: &JsArgs) -> Result<String> {
    match (&args.eval, &args.file) {
        (Some(s), None) => Ok(s.clone()),
        (None, Some(path)) if path == "-" => {
            let mut buf = String::new();
            std::io::stdin()
                .read_to_string(&mut buf)
                .map_err(Error::Io)?;
            Ok(buf)
        }
        (None, Some(path)) => std::fs::read_to_string(path).map_err(Error::Io),
        (None, None) => Err(Error::InvalidArguments(
            "mach js requires either --eval '<src>' or --file <path>".into(),
        )),
        (Some(_), Some(_)) => {
            // clap's `conflicts_with` should already prevent this, but the
            // pattern keeps exhaustiveness checking honest.
            Err(Error::InvalidArguments(
                "--eval and --file are mutually exclusive".into(),
            ))
        }
    }
}
