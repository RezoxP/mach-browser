//! `mach` CLI entrypoint.
//!
//! Phase 0 ships one subcommand, `fetch`. Later phases add `serve` (CDP),
//! `mcp`, and `scrape`. The argument parser lives in `cli.rs`; this file
//! is the async runtime + dispatch + exit-code mapping.

#![deny(unsafe_code)]
#![warn(missing_docs)]

mod cli;
mod fetch;

use std::process::ExitCode;

use clap::Parser;
use mach_core::{Error, LogLevel};
use tracing::error;
use tracing_subscriber::EnvFilter;

use crate::cli::{Cli, Command};

fn main() -> ExitCode {
    let args = Cli::parse();
    init_tracing(args.log_level());

    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("mach: tokio init failed: {e}");
            return ExitCode::from(1);
        }
    };

    match rt.block_on(run(args)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            error!("{e}");
            eprintln!("mach: {e}");
            ExitCode::from(e.exit_code() as u8)
        }
    }
}

fn init_tracing(level: LogLevel) {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        EnvFilter::new(format!("mach={lvl},mach_net={lvl}", lvl = level.as_str()))
    });
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .with_writer(std::io::stderr)
        .try_init();
}

async fn run(cli: Cli) -> Result<(), Error> {
    let config = cli.to_config();
    match cli.command {
        Command::Fetch(args) => fetch::run(args, config).await,
    }
}
