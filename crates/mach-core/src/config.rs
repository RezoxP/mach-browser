//! Process-wide configuration carried into every subsystem.

use std::time::Duration;

/// Runtime configuration for a single `mach` invocation.
///
/// Built from CLI args (and, later, an optional config file). One `Config`
/// is constructed by `core::App::new` and threaded through `net`, `parser`,
/// `dom`, and `agent`. Subsystems borrow but never mutate.
#[derive(Debug, Clone)]
pub struct Config {
    /// HTTP request timeout. Applies to the full request including redirects.
    pub http_timeout: Duration,

    /// Override of the User-Agent header. When `None`, the BrowserProfile's
    /// default UA is used.
    pub user_agent_override: Option<String>,

    /// Tracing verbosity (re-exported here so subsystems don't depend on
    /// `tracing_subscriber` directly).
    pub log_level: LogLevel,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            http_timeout: Duration::from_secs(20),
            user_agent_override: None,
            log_level: LogLevel::Warn,
        }
    }
}

/// Tracing verbosity for the process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

impl LogLevel {
    /// Render as the directive string `tracing_subscriber::EnvFilter` accepts.
    pub fn as_str(self) -> &'static str {
        match self {
            LogLevel::Error => "error",
            LogLevel::Warn => "warn",
            LogLevel::Info => "info",
            LogLevel::Debug => "debug",
            LogLevel::Trace => "trace",
        }
    }
}
