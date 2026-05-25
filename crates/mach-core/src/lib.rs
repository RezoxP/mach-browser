//! Shared types for the mach-browser workspace.
//!
//! Phase 0: just enough for the no-JS fetch path. Later phases (V8, CDP,
//! MCP) will subscribe to the same [`Notification`] bus and reuse the same
//! [`Error`] hierarchy.

#![deny(unsafe_code)]
#![warn(missing_docs)]

pub mod config;
pub mod error;
pub mod notification;

pub use config::{Config, LogLevel};
pub use error::{Error, Result};
pub use notification::{Notification, NotificationBus, NotificationKind};
