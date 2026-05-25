//! `--dump links` exporter.
//!
//! Walks the document, collects every `<a href>` (and `<area href>`),
//! resolves relative URLs against a base URL, and deduplicates in document
//! order.

use std::collections::HashSet;

use mach_dom::{Document, NodeKind};
use url::Url;

/// Collect outbound links from `doc`, resolved against `base`.
///
/// Returns the absolute URLs in document order, deduplicated. Malformed
/// hrefs are silently skipped.
pub fn collect(doc: &Document, base: &Url) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for id in doc.iter_pre_order() {
        if let NodeKind::Element { name, attrs } = &doc.node(id).kind {
            if name == "a" || name == "area" {
                for a in attrs {
                    if a.name == "href" {
                        if let Ok(resolved) = base.join(&a.value) {
                            let s = resolved.into();
                            if seen.insert(String::clone(&s)) {
                                out.push(s);
                            }
                        }
                    }
                }
            }
        }
    }
    out
}
