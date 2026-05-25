//! `--dump markdown` exporter.
//!
//! Phase 0 ships a deliberately rough markdown rendering: headings,
//! paragraphs, ordered + unordered lists, links, code, strong/em. Tables,
//! images, and nested-list edge cases are explicitly Phase 3 territory;
//! once the semantic tree exists we'll rewrite this on top of it.

use mach_dom::{Document, NodeId, NodeKind};

/// Render `doc` as Markdown.
pub fn render(doc: &Document) -> String {
    let mut buf = String::new();
    walk(doc, NodeId::ROOT, &mut buf, &Ctx::default());
    normalize_blank_lines(&buf)
}

#[derive(Default, Clone, Copy)]
struct Ctx {
    suppress_text: bool,
    inside_pre: bool,
}

fn walk(doc: &Document, id: NodeId, out: &mut String, ctx: &Ctx) {
    let n = doc.node(id);
    match &n.kind {
        NodeKind::Document => {
            for c in &n.children {
                walk(doc, *c, out, ctx);
            }
        }
        NodeKind::Text(s) if !ctx.suppress_text => {
            if ctx.inside_pre {
                out.push_str(s);
            } else {
                // Collapse internal whitespace; full normalization happens
                // in `normalize_blank_lines`.
                let mut last_space = false;
                for ch in s.chars() {
                    if ch.is_whitespace() {
                        if !last_space {
                            out.push(' ');
                            last_space = true;
                        }
                    } else {
                        out.push(ch);
                        last_space = false;
                    }
                }
            }
        }
        NodeKind::Element { name, attrs } => {
            let mut child_ctx = *ctx;
            match name.as_str() {
                "script" | "style" | "noscript" | "template" | "head" => {
                    child_ctx.suppress_text = true;
                    for c in &n.children {
                        walk(doc, *c, out, &child_ctx);
                    }
                }
                tag @ ("h1" | "h2" | "h3" | "h4" | "h5" | "h6") => {
                    let n_hash = tag[1..].parse::<usize>().unwrap_or(1);
                    out.push_str("\n\n");
                    for _ in 0..n_hash {
                        out.push('#');
                    }
                    out.push(' ');
                    for c in &n.children {
                        walk(doc, *c, out, &child_ctx);
                    }
                    out.push_str("\n\n");
                }
                "p" | "div" | "section" | "article" => {
                    out.push_str("\n\n");
                    for c in &n.children {
                        walk(doc, *c, out, &child_ctx);
                    }
                    out.push_str("\n\n");
                }
                "br" => {
                    out.push_str("  \n");
                }
                "ul" | "ol" => {
                    out.push_str("\n\n");
                    for c in &n.children {
                        walk(doc, *c, out, &child_ctx);
                    }
                    out.push_str("\n\n");
                }
                "li" => {
                    out.push_str("\n- ");
                    for c in &n.children {
                        walk(doc, *c, out, &child_ctx);
                    }
                }
                "strong" | "b" => {
                    out.push_str("**");
                    for c in &n.children {
                        walk(doc, *c, out, &child_ctx);
                    }
                    out.push_str("**");
                }
                "em" | "i" => {
                    out.push('*');
                    for c in &n.children {
                        walk(doc, *c, out, &child_ctx);
                    }
                    out.push('*');
                }
                "code" => {
                    out.push('`');
                    for c in &n.children {
                        walk(doc, *c, out, &child_ctx);
                    }
                    out.push('`');
                }
                "pre" => {
                    child_ctx.inside_pre = true;
                    out.push_str("\n\n```\n");
                    for c in &n.children {
                        walk(doc, *c, out, &child_ctx);
                    }
                    out.push_str("\n```\n\n");
                }
                "a" => {
                    let href = attrs
                        .iter()
                        .find(|a| a.name == "href")
                        .map(|a| a.value.clone())
                        .unwrap_or_default();
                    out.push('[');
                    for c in &n.children {
                        walk(doc, *c, out, &child_ctx);
                    }
                    out.push(']');
                    if !href.is_empty() {
                        out.push('(');
                        out.push_str(&href);
                        out.push(')');
                    }
                }
                _ => {
                    for c in &n.children {
                        walk(doc, *c, out, &child_ctx);
                    }
                }
            }
        }
        _ => {}
    }
}

fn normalize_blank_lines(s: &str) -> String {
    // Collapse 3+ consecutive newlines into 2 (a single blank line).
    let mut out = String::with_capacity(s.len());
    let mut newlines = 0;
    for ch in s.chars() {
        if ch == '\n' {
            newlines += 1;
            if newlines <= 2 {
                out.push('\n');
            }
        } else {
            newlines = 0;
            out.push(ch);
        }
    }
    out.trim().to_owned() + "\n"
}

#[cfg(test)]
mod tests {
    use super::*;
    use mach_dom::{Attr, NodeKind};

    fn elem(name: &str, attrs: Vec<Attr>) -> NodeKind {
        NodeKind::Element {
            name: name.into(),
            attrs,
        }
    }

    #[test]
    fn headings_paragraph_link() {
        let mut d = Document::new();
        let body = d.push(NodeId::ROOT, elem("body", vec![]));
        let h1 = d.push(body, elem("h1", vec![]));
        d.push(h1, NodeKind::Text("Title".into()));
        let p = d.push(body, elem("p", vec![]));
        d.push(p, NodeKind::Text("hello ".into()));
        let a = d.push(
            p,
            elem(
                "a",
                vec![Attr {
                    name: "href".into(),
                    value: "https://e.com/".into(),
                }],
            ),
        );
        d.push(a, NodeKind::Text("world".into()));
        let md = render(&d);
        assert!(md.contains("# Title"));
        assert!(md.contains("[world](https://e.com/)"));
    }
}
