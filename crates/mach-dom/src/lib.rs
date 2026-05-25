//! Arena-backed DOM tree.
//!
//! Nodes are stored in a `Vec<Node>` and addressed by [`NodeId`]. The arena
//! gives us O(1) parent/sibling traversal without `Rc<RefCell<_>>` overhead
//! and makes per-Page memory cleanup a single `drop`. See architecture doc
//! §3 `dom::DomTree` and §0.4 tactic #6 (lazy DOM materialization — Phase
//! 0 stores everything eagerly; Phase 2 introduces text-slice deferral).
//!
//! Phase 0 keeps the surface small: enough to round-trip `html5ever` output
//! and run the markdown/links/text extractors. Selectors, mutation events,
//! `live` collections, and the V8 identity map are all deferred.

#![deny(unsafe_code)]
#![warn(missing_docs)]

use std::fmt::Write;

/// Index into [`Document::nodes`]. Cheap to copy; outlives no specific
/// `Document` borrow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NodeId(pub u32);

impl NodeId {
    /// The id of the document root.
    pub const ROOT: NodeId = NodeId(0);

    /// Raw index for debugging/printing.
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// Element attribute.
#[derive(Debug, Clone)]
pub struct Attr {
    /// Name (lowercased ASCII for HTML).
    pub name: String,
    /// Value (raw, undecoded entities).
    pub value: String,
}

/// One DOM node. Discriminated by [`NodeKind`].
#[derive(Debug, Clone)]
pub struct Node {
    /// Parent or `None` if this is the root.
    pub parent: Option<NodeId>,
    /// Children, in document order.
    pub children: Vec<NodeId>,
    /// Discriminator + payload.
    pub kind: NodeKind,
}

/// Node payloads.
#[derive(Debug, Clone)]
#[allow(missing_docs)]
pub enum NodeKind {
    /// Root document node.
    Document,
    /// HTML element. `name` is lowercased.
    Element { name: String, attrs: Vec<Attr> },
    /// Text node.
    Text(String),
    /// HTML comment (kept for fidelity; ignored by extractors).
    Comment(String),
    /// Doctype declaration (kept for round-trip serializers).
    Doctype {
        name: String,
        public_id: String,
        system_id: String,
    },
}

/// Owns a tree of [`Node`]s.
#[derive(Debug, Clone, Default)]
pub struct Document {
    /// Node arena. Index 0 is always [`NodeId::ROOT`] and has
    /// [`NodeKind::Document`].
    nodes: Vec<Node>,
}

impl Document {
    /// Construct an empty document with a `Document` root at id 0.
    pub fn new() -> Self {
        let mut d = Document { nodes: Vec::new() };
        d.nodes.push(Node {
            parent: None,
            children: Vec::new(),
            kind: NodeKind::Document,
        });
        d
    }

    /// Total number of nodes (including the root).
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// True iff the document only contains the root.
    pub fn is_empty(&self) -> bool {
        self.nodes.len() <= 1
    }

    /// Look up a node by id. Panics on out-of-range ids — ids are produced
    /// only by `push` so this should be impossible without unsafe.
    pub fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id.index()]
    }

    /// Append a new node under `parent`. Returns the new id.
    pub fn push(&mut self, parent: NodeId, kind: NodeKind) -> NodeId {
        let id = NodeId(self.nodes.len() as u32);
        self.nodes.push(Node {
            parent: Some(parent),
            children: Vec::new(),
            kind,
        });
        self.nodes[parent.index()].children.push(id);
        id
    }

    /// Iterate over node ids in document order (depth-first pre-order
    /// starting at root).
    pub fn iter_pre_order(&self) -> PreOrderIter<'_> {
        PreOrderIter {
            doc: self,
            stack: vec![NodeId::ROOT],
        }
    }

    /// Re-serialize the document as HTML.
    ///
    /// Round-trip is *not* byte-identical to the input — html5ever
    /// normalizes attribute quoting, lowercases element names, drops
    /// extraneous whitespace inside tags, and reorders attributes. That's
    /// fine for Phase 0; later phases that need fidelity will record source
    /// offsets in `Text` nodes.
    pub fn serialize_html(&self) -> String {
        let mut out = String::new();
        write_node(self, NodeId::ROOT, &mut out);
        out
    }

    /// Serialize a single node (and its subtree) to HTML — i.e. the value
    /// returned by `element.outerHTML` in the DOM spec.
    pub fn serialize_node(&self, id: NodeId) -> String {
        let mut out = String::new();
        write_node(self, id, &mut out);
        out
    }

    /// Serialize a node's children — i.e. `element.innerHTML`.
    pub fn serialize_node_children(&self, id: NodeId) -> String {
        let mut out = String::new();
        for c in &self.node(id).children {
            write_node(self, *c, &mut out);
        }
        out
    }

    /// Recursive text content (DOM `textContent` semantics: concatenate
    /// all descendant text nodes in document order; ignore comments and
    /// element tags themselves).
    pub fn text_content(&self, id: NodeId) -> String {
        let mut out = String::new();
        collect_text(self, id, &mut out);
        out
    }

    /// Walk the document looking for an element with `id="..."` matching
    /// `value`. Returns the first match in document order (matches the
    /// DOM spec).
    ///
    /// Linear scan. Phase 1+ will memoize / index this when JS workloads
    /// start hammering `getElementById`.
    pub fn find_by_id(&self, value: &str) -> Option<NodeId> {
        for id in self.iter_pre_order() {
            if let NodeKind::Element { attrs, .. } = &self.node(id).kind {
                if attrs.iter().any(|a| a.name == "id" && a.value == value) {
                    return Some(id);
                }
            }
        }
        None
    }

    /// First element in document order whose lowercased tag name equals
    /// `name`. Used by the JS bindings to wire up `document.documentElement`,
    /// `.head`, `.body`.
    pub fn find_first_element(&self, name: &str) -> Option<NodeId> {
        for id in self.iter_pre_order() {
            if let NodeKind::Element { name: n, .. } = &self.node(id).kind {
                if n == name {
                    return Some(id);
                }
            }
        }
        None
    }
}

fn collect_text(doc: &Document, id: NodeId, out: &mut String) {
    match &doc.node(id).kind {
        NodeKind::Text(s) => out.push_str(s),
        NodeKind::Comment(_) | NodeKind::Doctype { .. } => {}
        NodeKind::Document | NodeKind::Element { .. } => {
            for c in &doc.node(id).children {
                collect_text(doc, *c, out);
            }
        }
    }
}

fn write_node(doc: &Document, id: NodeId, out: &mut String) {
    let n = doc.node(id);
    match &n.kind {
        NodeKind::Document => {
            for c in &n.children {
                write_node(doc, *c, out);
            }
        }
        NodeKind::Doctype {
            name,
            public_id,
            system_id,
        } => {
            out.push_str("<!DOCTYPE ");
            out.push_str(name);
            if !public_id.is_empty() {
                let _ = write!(out, " PUBLIC \"{}\"", public_id);
            }
            if !system_id.is_empty() {
                let _ = write!(out, " \"{}\"", system_id);
            }
            out.push('>');
        }
        NodeKind::Element { name, attrs } => {
            out.push('<');
            out.push_str(name);
            for a in attrs {
                out.push(' ');
                out.push_str(&a.name);
                out.push('=');
                out.push('"');
                escape_attr_into(&a.value, out);
                out.push('"');
            }
            out.push('>');
            for c in &n.children {
                write_node(doc, *c, out);
            }
            if !is_void_element(name) {
                out.push_str("</");
                out.push_str(name);
                out.push('>');
            }
        }
        NodeKind::Text(s) => {
            escape_text_into(s, out);
        }
        NodeKind::Comment(s) => {
            out.push_str("<!--");
            out.push_str(s);
            out.push_str("-->");
        }
    }
}

fn is_void_element(name: &str) -> bool {
    matches!(
        name,
        "area"
            | "base"
            | "br"
            | "col"
            | "embed"
            | "hr"
            | "img"
            | "input"
            | "link"
            | "meta"
            | "param"
            | "source"
            | "track"
            | "wbr"
    )
}

fn escape_text_into(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(c),
        }
    }
}

fn escape_attr_into(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
}

/// Depth-first pre-order iterator over [`Document`] node ids.
pub struct PreOrderIter<'a> {
    doc: &'a Document,
    stack: Vec<NodeId>,
}

impl Iterator for PreOrderIter<'_> {
    type Item = NodeId;

    fn next(&mut self) -> Option<NodeId> {
        let id = self.stack.pop()?;
        // Push children in reverse so they pop in document order.
        for c in self.doc.node(id).children.iter().rev() {
            self.stack.push(*c);
        }
        Some(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn elem(name: &str) -> NodeKind {
        NodeKind::Element {
            name: name.into(),
            attrs: Vec::new(),
        }
    }

    #[test]
    fn round_trip_simple_tree() {
        let mut d = Document::new();
        let html = d.push(NodeId::ROOT, elem("html"));
        let body = d.push(html, elem("body"));
        d.push(body, NodeKind::Text("hello".into()));
        assert_eq!(d.serialize_html(), "<html><body>hello</body></html>");
    }

    #[test]
    fn void_element_has_no_close_tag() {
        let mut d = Document::new();
        let html = d.push(NodeId::ROOT, elem("html"));
        let head = d.push(html, elem("head"));
        d.push(
            head,
            NodeKind::Element {
                name: "meta".into(),
                attrs: vec![Attr {
                    name: "charset".into(),
                    value: "utf-8".into(),
                }],
            },
        );
        let s = d.serialize_html();
        assert!(s.contains("<meta charset=\"utf-8\">"));
        assert!(!s.contains("</meta>"));
    }

    #[test]
    fn text_content_concatenates_descendants() {
        let mut d = Document::new();
        let html = d.push(NodeId::ROOT, elem("html"));
        let body = d.push(html, elem("body"));
        let p = d.push(body, elem("p"));
        d.push(p, NodeKind::Text("hi ".into()));
        let b = d.push(p, elem("b"));
        d.push(b, NodeKind::Text("there".into()));
        assert_eq!(d.text_content(p), "hi there");
        assert_eq!(d.text_content(b), "there");
    }

    #[test]
    fn serialize_node_emits_just_subtree() {
        let mut d = Document::new();
        let html = d.push(NodeId::ROOT, elem("html"));
        let body = d.push(html, elem("body"));
        d.push(body, NodeKind::Text("x".into()));
        assert_eq!(d.serialize_node(body), "<body>x</body>");
        assert_eq!(d.serialize_node_children(body), "x");
    }

    #[test]
    fn find_by_id_walks_in_document_order() {
        let mut d = Document::new();
        let html = d.push(NodeId::ROOT, elem("html"));
        let body = d.push(html, elem("body"));
        let _first = d.push(
            body,
            NodeKind::Element {
                name: "div".into(),
                attrs: vec![Attr {
                    name: "id".into(),
                    value: "target".into(),
                }],
            },
        );
        let _second = d.push(
            body,
            NodeKind::Element {
                name: "span".into(),
                attrs: vec![Attr {
                    name: "id".into(),
                    value: "target".into(),
                }],
            },
        );
        // Spec: first match in tree order.
        let found = d.find_by_id("target").unwrap();
        assert!(matches!(
            &d.node(found).kind,
            NodeKind::Element { name, .. } if name == "div"
        ));
        assert_eq!(d.find_by_id("missing"), None);
    }

    #[test]
    fn find_first_element_returns_root_match() {
        let mut d = Document::new();
        let html = d.push(NodeId::ROOT, elem("html"));
        d.push(html, elem("head"));
        d.push(html, elem("body"));
        assert_eq!(d.find_first_element("body"), Some(NodeId(3)));
        assert_eq!(d.find_first_element("nope"), None);
    }

    #[test]
    fn pre_order_walks_document_order() {
        let mut d = Document::new();
        let a = d.push(NodeId::ROOT, elem("a"));
        let b = d.push(a, elem("b"));
        let c = d.push(a, elem("c"));
        let _ = b;
        let _ = c;
        let names: Vec<String> = d
            .iter_pre_order()
            .filter_map(|id| match &d.node(id).kind {
                NodeKind::Element { name, .. } => Some(name.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(names, vec!["a", "b", "c"]);
    }
}
