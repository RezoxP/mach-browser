//! Command-line parsing.
//!
//! Kept in its own file so the binary and (future) integration tests can
//! share the same type. See architecture doc §3 `cli::*`.

use std::time::Duration;

use clap::{Args, Parser, Subcommand, ValueEnum};
use mach_core::{Config, LogLevel};

/// Top-level CLI for `mach`.
#[derive(Debug, Parser)]
#[command(
    name = "mach",
    version,
    about = "An ultra-lightweight browser for AI agents and automation.",
    long_about = None,
)]
pub struct Cli {
    /// Increase logging verbosity. Repeat for more (-v, -vv, -vvv).
    #[arg(short = 'v', long = "verbose", action = clap::ArgAction::Count, global = true)]
    verbose: u8,

    /// Silence all non-error logging.
    #[arg(short = 'q', long = "quiet", global = true, conflicts_with = "verbose")]
    quiet: bool,

    /// Subcommand.
    #[command(subcommand)]
    pub command: Command,
}

impl Cli {
    /// Resolve the log level from -v/-q flags.
    pub fn log_level(&self) -> LogLevel {
        if self.quiet {
            return LogLevel::Error;
        }
        match self.verbose {
            0 => LogLevel::Warn,
            1 => LogLevel::Info,
            2 => LogLevel::Debug,
            _ => LogLevel::Trace,
        }
    }

    /// Build the runtime [`Config`] from CLI flags.
    pub fn to_config(&self) -> Config {
        // Per-subcommand overrides are folded in by the subcommand dispatcher;
        // this builds the baseline shared by every command.
        Config {
            log_level: self.log_level(),
            ..Config::default()
        }
    }
}

/// Top-level subcommand.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Fetch a URL and emit the requested representation to stdout.
    Fetch(FetchArgs),

    /// Evaluate a JavaScript expression in a fresh V8 isolate.
    ///
    /// Phase 1A: no DOM, no Web APIs, no network. Just raw V8. This is
    /// here to (a) prove the JS engine works end-to-end on every supported
    /// platform and (b) give scripts/CI a way to smoke-test V8 without
    /// going through a fetched page. Web API surface lands in Phase 1B+.
    Js(JsArgs),
}

/// `mach fetch` arguments.
#[derive(Debug, Args)]
pub struct FetchArgs {
    /// URL to fetch.
    pub url: String,

    /// Output format.
    #[arg(long, value_enum, default_value_t = DumpFormat::Html)]
    pub dump: DumpFormat,

    /// Override the User-Agent header. Defaults to the profile's UA.
    #[arg(long)]
    pub user_agent: Option<String>,

    /// Browser profile id. Defaults to `chrome-linux-131`.
    #[arg(long)]
    pub profile: Option<String>,

    /// HTTP request timeout in seconds.
    #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u64).range(1..=600))]
    pub timeout: u64,
}

impl FetchArgs {
    /// Convenience accessor.
    pub fn timeout_dur(&self) -> Duration {
        Duration::from_secs(self.timeout)
    }
}

/// `mach js` arguments.
#[derive(Debug, Args)]
pub struct JsArgs {
    /// JavaScript source to evaluate.
    #[arg(long, conflicts_with = "file")]
    pub eval: Option<String>,

    /// Path to a JavaScript source file to evaluate. Use `-` for stdin.
    #[arg(long, conflicts_with = "eval")]
    pub file: Option<String>,
}

/// Available `--dump` formats.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum DumpFormat {
    /// Re-serialized HTML after html5ever round-trip.
    Html,
    /// Rough markdown extraction.
    Markdown,
    /// Outbound `<a href>` / `<area href>` URLs, one per line.
    Links,
    /// Visible text content with whitespace collapsed.
    Text,
}
