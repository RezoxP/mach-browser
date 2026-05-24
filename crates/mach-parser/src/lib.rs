//! html5ever wrapper that emits into [`mach_dom::Document`].
//!
//! Phase 0 uses `markup5ever_rcdom` as an intermediate representation: it
//! ships with `html5ever`, is well-tested, and lets us focus on the
//! `Rc<Node>` → arena conversion. Phase 1+ swaps in a custom `TreeSink`
//! that writes directly into the arena, dropping the `Rc<RefCell<_>>`
//! traffic. See architecture doc §3 `parser::Html`.

#![deny(unsafe_code)]
#![warn(missing_docs)]

use std::cell::RefCell;
use std::default::Default;

use html5ever::driver::ParseOpts;
use html5ever::tendril::TendrilSink;
use html5ever::tree_builder::TreeBuilderOpts;
use html5ever::{parse_document, QualName};
use mach_core::{Error, Result};
use mach_dom::{Attr, Document, NodeId, NodeKind};
use markup5ever_rcdom::{Handle, NodeData, RcDom};

/// Parse a full HTML document into a [`mach_dom::Document`].
pub fn parse_html(bytes: &[u8]) -> Result<Document> {
    let opts = ParseOpts {
        tree_builder: TreeBuilderOpts {
            drop_doctype: false,
            ..Default::default()
        },
        ..Default::default()
    };
    // tendril needs the bytes as UTF-8; do a lossy decode for Phase 0.
    let s = std::str::from_utf8(bytes)
        .map(|s| s.to_owned())
        .unwrap_or_else(|_| String::from_utf8_lossy(bytes).into_owned());

    let dom: RcDom = parse_document(RcDom::default(), opts)
        .from_utf8()
        .read_from(&mut s.as_bytes())
        .map_err(|e| Error::Parse(format!("html5ever: {e}")))?;

    let mut doc = Document::new();
    walk(&dom.document, NodeId::ROOT, &mut doc);
    Ok(doc)
}

fn walk(handle: &Handle, parent: NodeId, out: &mut Document) {
    let kind = match &handle.data {
        NodeData::Document => None, // root already exists; just recurse
        NodeData::Doctype {
            name,
            public_id,
            system_id,
        } => Some(NodeKind::Doctype {
            name: name.to_string(),
            public_id: public_id.to_string(),
            system_id: system_id.to_string(),
        }),
        NodeData::Element { name, attrs, .. } => {
            let attrs_v: Vec<Attr> = attrs
                .borrow()
                .iter()
                .map(|a| Attr {
                    name: qual_name_to_local(&a.name),
                    value: a.value.to_string(),
                })
                .collect();
            Some(NodeKind::Element {
                name: qual_name_to_local(name),
                attrs: attrs_v,
            })
        }
        NodeData::Text { contents } => {
            let t: &RefCell<_> = contents;
            Some(NodeKind::Text(t.borrow().to_string()))
        }
        NodeData::Comment { contents } => Some(NodeKind::Comment(contents.to_string())),
        NodeData::ProcessingInstruction { .. } => None, // ignore for Phase 0
    };

    let child_parent = if let Some(k) = kind {
        out.push(parent, k)
    } else {
        parent
    };

    for child in handle.children.borrow().iter() {
        walk(child, child_parent, out);
    }
}

fn qual_name_to_local(q: &QualName) -> String {
    q.local.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_minimal() {
        let html = b"<!DOCTYPE html><html><body><p>hi</p></body></html>";
        let doc = parse_html(html).expect("parse");
        assert!(doc.len() >= 5); // doctype, html, head (synthesized), body, p, text
        let s = doc.serialize_html();
        assert!(s.to_lowercase().contains("<html>"));
        assert!(s.contains("hi"));
    }

    #[test]
    fn parse_invalid_utf8_lossily() {
        let bytes: &[u8] = &[0xff, 0xfe, b'<', b'b', b'>', b'x', b'<', b'/', b'b', b'>'];
        let doc = parse_html(bytes).expect("parse");
        let s = doc.serialize_html();
        assert!(s.contains('x'));
    }
}
