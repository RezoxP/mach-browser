//! Agent-first dump formats.
//!
//! All exporters operate on a parsed [`mach_dom::Document`] only — no
//! HTTP, no JS. The CLI threads the post-fetch document through whichever
//! exporter `--dump` selected.
//!
//! See architecture doc §3 `agent::*`. Phase 0 ships the four dumps below;
//! `semantic_tree` and `structured_data` (JSON-LD / OpenGraph / microdata)
//! land in Phase 3.

#![deny(unsafe_code)]
#![warn(missing_docs)]

pub mod links;
pub mod markdown;
pub mod text;
