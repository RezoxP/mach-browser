//! `--dump text` exporter.
//!
//! Visible text content with whitespace collapsed. Skips `<script>`,
//! `<style>`, `<noscript>`, `<template>`, and comments — the same set the
//! browsers strip when computing `Node.innerText` (informally; we don't try
//! to be CSS-aware in Phase 0).

use mach_dom::{Document, NodeId, NodeKind};

/// Render the document as plain text.
pub fn render(doc: &Document) -> String {
    let mut buf = String::new();
    walk(doc, NodeId::ROOT, &mut buf, false);
    collapse_whitespace(&buf)
}

fn walk(doc: &Document, id: NodeId, out: &mut String, suppressed: bool) {
    let n = doc.node(id);
    match &n.kind {
        NodeKind::Element { name, .. } => {
            let now_suppressed = suppressed
                || matches!(
                    name.as_str(),
                    "script" | "style" | "noscript" | "template" | "head" | "title"
                );
            for c in &n.children {
                walk(doc, *c, out, now_suppressed);
            }
            // Add a newline after block-level elements to keep text from
            // running together. The list below is the minimal set needed
            // for sensible Phase 0 output; a CSS-aware version comes later.
            if matches!(
                name.as_str(),
                "p" | "br"
                    | "div"
                    | "section"
                    | "article"
                    | "header"
                    | "footer"
                    | "nav"
                    | "aside"
                    | "li"
                    | "tr"
                    | "h1"
                    | "h2"
                    | "h3"
                    | "h4"
                    | "h5"
                    | "h6"
            ) {
                out.push('\n');
            }
        }
        NodeKind::Text(s) if !suppressed => {
            out.push_str(s);
        }
        NodeKind::Document => {
            for c in &n.children {
                walk(doc, *c, out, suppressed);
            }
        }
        _ => {}
    }
}

fn collapse_whitespace(s: &str) -> String {
    // Collapse runs of whitespace, but preserve single newlines as paragraph
    // breaks. Empirically good enough for the Phase 0 corpus.
    let mut out = String::with_capacity(s.len());
    let mut last_was_space = true; // suppress leading whitespace
    let mut last_was_newline = true;
    for ch in s.chars() {
        if ch == '\n' {
            if !last_was_newline {
                out.push('\n');
                last_was_newline = true;
                last_was_space = true;
            }
        } else if ch.is_whitespace() {
            if !last_was_space {
                out.push(' ');
                last_was_space = true;
            }
        } else {
            out.push(ch);
            last_was_space = false;
            last_was_newline = false;
        }
    }
    out.trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use mach_dom::{Attr, NodeKind};

    fn elem(name: &str) -> NodeKind {
        NodeKind::Element {
            name: name.into(),
            attrs: Vec::new(),
        }
    }

    #[test]
    fn skips_script_and_style() {
        let mut d = Document::new();
        let html = d.push(NodeId::ROOT, elem("html"));
        let head = d.push(html, elem("head"));
        let script = d.push(head, elem("script"));
        d.push(script, NodeKind::Text("alert(1)".into()));
        let body = d.push(html, elem("body"));
        d.push(body, NodeKind::Text("hello".into()));
        assert_eq!(render(&d), "hello");
    }

    #[test]
    fn block_elements_separate_text() {
        let mut d = Document::new();
        let html = d.push(NodeId::ROOT, elem("html"));
        let body = d.push(html, elem("body"));
        let p1 = d.push(body, elem("p"));
        d.push(p1, NodeKind::Text("one".into()));
        let p2 = d.push(body, elem("p"));
        d.push(p2, NodeKind::Text("two".into()));
        let s = render(&d);
        assert!(s.contains("one"));
        assert!(s.contains("two"));
        assert!(s.contains('\n'));
    }

    #[test]
    fn ignores_attrs_argument() {
        // Attr import only exists to keep test file consistent if we extend.
        let _ = Attr {
            name: "x".into(),
            value: "y".into(),
        };
    }
}
