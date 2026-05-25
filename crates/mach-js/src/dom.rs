//! V8 ↔ `mach_dom` bindings.
//!
//! Phase 1C: **read-only** DOM surface. Lets user JS navigate a parsed
//! page (`document.body.children[0].getAttribute("href")` etc.) but does
//! **not** mutate it. Mutation (`setAttribute`, `appendChild`,
//! `textContent =`, `innerHTML =`, `createElement`, ...) is Phase 1D.
//!
//! Architecture (single load-bearing invariant):
//!
//! - Each DOM node is exposed to JS as a [`v8::Object`] with one internal
//!   field holding the [`mach_dom::NodeId`] as a `v8::Integer`.
//! - A [`HashMap<u32, v8::Global<v8::Object>>`] caches the JS object for
//!   every [`NodeId`] we've ever handed out. The cache is the reason
//!   `el === el.parentNode.children[0]` holds — without it identity
//!   would be lost across reads and the entire DOM contract breaks.
//! - The cache + the owned [`mach_dom::Document`] + the prototype
//!   templates live in a single [`DomInner`] stashed in the isolate's
//!   slot table. Callbacks fetch it via [`v8::HandleScope::get_slot`].
//!
//! Mutation phases (1D+) will reuse this exact infrastructure — the
//! cache becomes write-through, the [`RefCell<Document>`] becomes
//! actually mutable, and the templates gain `set*` methods.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use mach_dom::{Document, NodeId, NodeKind};

/// Internal-field index storing the node id.
const NODE_ID_FIELD: i32 = 0;

/// Total internal-field count on Element / Text / Document instances.
/// Bumped if/when we need to attach more native data per node (e.g. a
/// flag set for live `NodeList` invalidation).
const NODE_INTERNAL_FIELD_COUNT: usize = 1;

/// State shared between all DOM callbacks, stashed in the isolate.
///
/// `Rc` rather than a raw struct so callbacks can `clone()` out of the
/// slot, drop the borrow on the scope, and continue calling V8 APIs that
/// require `&mut HandleScope`. The borrow checker would otherwise reject
/// any V8 call made while `get_slot()`'s borrow is live.
pub(crate) struct DomInner {
    /// Owned page tree. `RefCell` for forward-compat with mutation in
    /// Phase 1D; Phase 1C only borrows immutably.
    pub(crate) document: RefCell<Document>,

    /// Page URL — backs `document.URL` / `document.documentURI`. Kept
    /// here (rather than re-reading `globalThis.location.href`) because
    /// it's the single source of truth for the page identity and
    /// Phase 1D may need to read it from mutation callbacks.
    pub(crate) url: String,

    /// NodeId → JS object identity map. Lazily populated; key set is
    /// the subset of node ids that JS has actually touched.
    pub(crate) cache: RefCell<HashMap<u32, v8::Global<v8::Object>>>,

    /// Template for Element instances. One internal field.
    element_template: v8::Global<v8::ObjectTemplate>,

    /// Template for Text instances. One internal field.
    text_template: v8::Global<v8::ObjectTemplate>,
}

/// Install the DOM surface onto `scope`'s current context.
///
/// Sets the `document` global, stashes [`DomInner`] in the isolate slot,
/// and returns. Callbacks fire lazily as JS reads properties.
pub(crate) fn install(scope: &mut v8::HandleScope, document: Document, url: &str) {
    let element_template = build_element_template(scope);
    let text_template = build_text_template(scope);
    let document_obj = build_document_object(scope);

    let dom = Rc::new(DomInner {
        document: RefCell::new(document),
        url: url.to_string(),
        cache: RefCell::new(HashMap::new()),
        element_template: v8::Global::new(scope, element_template),
        text_template: v8::Global::new(scope, text_template),
    });

    // Slot is keyed by TypeId; Rc<DomInner> is its own type so this
    // never collides with anything else we stash.
    scope.set_slot(dom);

    // Wire `document` onto the global. Always present (even for pages
    // built with `JsRuntimeBuilder` without an explicit document — we
    // only call `install()` when a document was provided, so we can
    // assume the user wants it).
    let context = scope.get_current_context();
    let global = context.global(scope);
    let key = v8::String::new(scope, "document").unwrap();
    global.set(scope, key.into(), document_obj.into());
}

// ---------------------------------------------------------------------------
// templates
// ---------------------------------------------------------------------------

macro_rules! set_accessor {
    ($scope:expr, $tmpl:expr, $name:literal, $cb:ident) => {{
        let key = v8::String::new($scope, $name).unwrap();
        $tmpl.set_accessor(key.into(), $cb);
    }};
}

macro_rules! set_method {
    ($scope:expr, $tmpl:expr, $name:literal, $cb:ident) => {{
        let key = v8::String::new($scope, $name).unwrap();
        let fn_tmpl = v8::FunctionTemplate::new($scope, $cb);
        $tmpl.set(key.into(), fn_tmpl.into());
    }};
}

fn build_element_template<'s>(
    scope: &mut v8::HandleScope<'s>,
) -> v8::Local<'s, v8::ObjectTemplate> {
    let tmpl = v8::ObjectTemplate::new(scope);
    tmpl.set_internal_field_count(NODE_INTERNAL_FIELD_COUNT);

    set_accessor!(scope, tmpl, "tagName", element_tag_name);
    set_accessor!(scope, tmpl, "nodeName", element_tag_name);
    set_accessor!(scope, tmpl, "localName", element_local_name);
    set_accessor!(scope, tmpl, "nodeType", element_node_type);
    set_accessor!(scope, tmpl, "id", element_id);
    set_accessor!(scope, tmpl, "className", element_class_name);
    set_accessor!(scope, tmpl, "parentNode", element_parent_node);
    set_accessor!(scope, tmpl, "parentElement", element_parent_element);
    set_accessor!(scope, tmpl, "children", element_children);
    set_accessor!(scope, tmpl, "childNodes", element_child_nodes);
    set_accessor!(
        scope,
        tmpl,
        "childElementCount",
        element_child_element_count
    );
    set_accessor!(scope, tmpl, "firstChild", element_first_child);
    set_accessor!(scope, tmpl, "lastChild", element_last_child);
    set_accessor!(scope, tmpl, "nextSibling", element_next_sibling);
    set_accessor!(scope, tmpl, "previousSibling", element_previous_sibling);
    set_accessor!(
        scope,
        tmpl,
        "firstElementChild",
        element_first_element_child
    );
    set_accessor!(scope, tmpl, "lastElementChild", element_last_element_child);
    set_accessor!(scope, tmpl, "textContent", element_text_content);
    set_accessor!(scope, tmpl, "innerHTML", element_inner_html);
    set_accessor!(scope, tmpl, "outerHTML", element_outer_html);

    set_method!(scope, tmpl, "getAttribute", element_get_attribute);
    set_method!(scope, tmpl, "hasAttribute", element_has_attribute);
    set_method!(
        scope,
        tmpl,
        "getAttributeNames",
        element_get_attribute_names
    );
    set_method!(scope, tmpl, "hasAttributes", element_has_attributes);

    tmpl
}

fn build_text_template<'s>(scope: &mut v8::HandleScope<'s>) -> v8::Local<'s, v8::ObjectTemplate> {
    let tmpl = v8::ObjectTemplate::new(scope);
    tmpl.set_internal_field_count(NODE_INTERNAL_FIELD_COUNT);

    set_accessor!(scope, tmpl, "nodeType", text_node_type);
    set_accessor!(scope, tmpl, "nodeName", text_node_name);
    set_accessor!(scope, tmpl, "data", text_data);
    set_accessor!(scope, tmpl, "textContent", text_data);
    set_accessor!(scope, tmpl, "parentNode", element_parent_node);
    set_accessor!(scope, tmpl, "parentElement", element_parent_element);
    set_accessor!(scope, tmpl, "nextSibling", element_next_sibling);
    set_accessor!(scope, tmpl, "previousSibling", element_previous_sibling);

    tmpl
}

fn build_document_object<'s>(scope: &mut v8::HandleScope<'s>) -> v8::Local<'s, v8::Object> {
    let tmpl = v8::ObjectTemplate::new(scope);
    tmpl.set_internal_field_count(NODE_INTERNAL_FIELD_COUNT);

    set_accessor!(scope, tmpl, "documentElement", document_document_element);
    set_accessor!(scope, tmpl, "head", document_head);
    set_accessor!(scope, tmpl, "body", document_body);
    set_accessor!(scope, tmpl, "title", document_title);
    set_accessor!(scope, tmpl, "URL", document_url);
    set_accessor!(scope, tmpl, "documentURI", document_url);
    set_accessor!(scope, tmpl, "nodeType", document_node_type);
    set_accessor!(scope, tmpl, "nodeName", document_node_name);
    set_accessor!(scope, tmpl, "childNodes", element_child_nodes);
    set_accessor!(scope, tmpl, "children", element_children);
    set_accessor!(scope, tmpl, "firstChild", element_first_child);
    set_accessor!(scope, tmpl, "lastChild", element_last_child);

    set_method!(scope, tmpl, "getElementById", document_get_element_by_id);

    let obj = tmpl.new_instance(scope).unwrap();
    let id_val = v8::Integer::new(scope, NodeId::ROOT.0 as i32);
    obj.set_internal_field(NODE_ID_FIELD as usize, id_val.into());
    obj
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

/// Pull the shared DOM state out of the isolate slot. Panics if it's
/// missing — that means a callback fired before [`install`] ran, which
/// would be a bug in the binding code rather than user error.
fn dom_state(scope: &mut v8::HandleScope) -> Rc<DomInner> {
    scope
        .get_slot::<Rc<DomInner>>()
        .expect("DOM state not installed; bindings::install was not called with a Document")
        .clone()
}

/// Read the [`NodeId`] embedded in `obj`'s internal field 0. Returns
/// `None` if `obj` isn't a node-typed object (no internal fields), which
/// happens if user JS accidentally calls an accessor with a plain
/// object as receiver.
fn read_node_id(scope: &mut v8::HandleScope, obj: v8::Local<v8::Object>) -> Option<NodeId> {
    if obj.internal_field_count() == 0 {
        return None;
    }
    let field = obj.get_internal_field(scope, NODE_ID_FIELD as usize)?;
    let value: v8::Local<v8::Value> = field.try_into().ok()?;
    let int = value.to_integer(scope)?;
    Some(NodeId(int.value() as u32))
}

/// Look up — or lazily create — the JS object for a node.
///
/// Returns `None` for node kinds we don't expose in Phase 1C
/// (comments, doctypes). The cache is keyed by [`NodeId::index`] so
/// subsequent calls with the same id return the same JS object,
/// preserving DOM identity.
fn get_or_create_node<'s>(
    scope: &mut v8::HandleScope<'s>,
    dom: &Rc<DomInner>,
    id: NodeId,
) -> Option<v8::Local<'s, v8::Object>> {
    let kind_class = {
        let doc = dom.document.borrow();
        match &doc.node(id).kind {
            NodeKind::Element { .. } => NodeKindClass::Element,
            NodeKind::Text(_) => NodeKindClass::Text,
            // Phase 1C does not surface Comment / Doctype to JS. They
            // are valid DOM nodes per spec but no real script reads
            // them, and exposing them is more surface to maintain.
            _ => return None,
        }
    };

    // Cache hit fast path.
    if let Some(global) = dom.cache.borrow().get(&id.0) {
        return Some(v8::Local::new(scope, global));
    }

    // Cache miss: instantiate a fresh object from the right template,
    // stamp the NodeId into its internal field, cache + return.
    let tmpl_global = match kind_class {
        NodeKindClass::Element => &dom.element_template,
        NodeKindClass::Text => &dom.text_template,
    };
    let tmpl = v8::Local::new(scope, tmpl_global);
    let obj = tmpl.new_instance(scope)?;
    let id_val = v8::Integer::new(scope, id.0 as i32);
    obj.set_internal_field(NODE_ID_FIELD as usize, id_val.into());

    dom.cache
        .borrow_mut()
        .insert(id.0, v8::Global::new(scope, obj));
    Some(obj)
}

#[derive(Copy, Clone)]
enum NodeKindClass {
    Element,
    Text,
}

/// Set the return value to a fresh JS string `s`. Convenience for
/// accessors that produce strings.
fn return_string(scope: &mut v8::HandleScope, mut rv: v8::ReturnValue, s: &str) {
    let v = v8::String::new(scope, s).unwrap();
    rv.set(v.into());
}

/// Set the return value to `null`. Used when a navigation accessor has
/// no result (e.g. `.parentNode` on the root).
fn return_null(scope: &mut v8::HandleScope, mut rv: v8::ReturnValue) {
    let n = v8::null(scope);
    rv.set(n.into());
}

// ---------------------------------------------------------------------------
// element accessors
// ---------------------------------------------------------------------------

fn element_tag_name(
    scope: &mut v8::HandleScope,
    _key: v8::Local<v8::Name>,
    args: v8::PropertyCallbackArguments,
    rv: v8::ReturnValue,
) {
    let dom = dom_state(scope);
    let id = match read_node_id(scope, args.this()) {
        Some(id) => id,
        None => return,
    };
    let doc = dom.document.borrow();
    if let NodeKind::Element { name, .. } = &doc.node(id).kind {
        // DOM spec: HTML elements report `tagName` in uppercase. mach_dom
        // stores lowercased names (html5ever normalisation) so we
        // uppercase on the way out.
        let upper = name.to_ascii_uppercase();
        drop(doc);
        return_string(scope, rv, &upper);
    }
}

fn element_local_name(
    scope: &mut v8::HandleScope,
    _key: v8::Local<v8::Name>,
    args: v8::PropertyCallbackArguments,
    rv: v8::ReturnValue,
) {
    let dom = dom_state(scope);
    let id = match read_node_id(scope, args.this()) {
        Some(id) => id,
        None => return,
    };
    let doc = dom.document.borrow();
    if let NodeKind::Element { name, .. } = &doc.node(id).kind {
        let lower = name.clone();
        drop(doc);
        return_string(scope, rv, &lower);
    }
}

fn element_node_type(
    scope: &mut v8::HandleScope,
    _key: v8::Local<v8::Name>,
    _args: v8::PropertyCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let _ = scope;
    rv.set_uint32(1); // ELEMENT_NODE per DOM spec.
}

fn element_id(
    scope: &mut v8::HandleScope,
    _key: v8::Local<v8::Name>,
    args: v8::PropertyCallbackArguments,
    rv: v8::ReturnValue,
) {
    attr_string(scope, args, rv, "id");
}

fn element_class_name(
    scope: &mut v8::HandleScope,
    _key: v8::Local<v8::Name>,
    args: v8::PropertyCallbackArguments,
    rv: v8::ReturnValue,
) {
    attr_string(scope, args, rv, "class");
}

/// Shared back-end for `.id` and `.className` accessors. Returns `""`
/// (not `null`) when the attribute is absent — DOM spec behaviour.
fn attr_string(
    scope: &mut v8::HandleScope,
    args: v8::PropertyCallbackArguments,
    rv: v8::ReturnValue,
    attr_name: &str,
) {
    let dom = dom_state(scope);
    let id = match read_node_id(scope, args.this()) {
        Some(id) => id,
        None => return,
    };
    let value = {
        let doc = dom.document.borrow();
        if let NodeKind::Element { attrs, .. } = &doc.node(id).kind {
            attrs
                .iter()
                .find(|a| a.name == attr_name)
                .map(|a| a.value.clone())
                .unwrap_or_default()
        } else {
            String::new()
        }
    };
    return_string(scope, rv, &value);
}

fn element_parent_node(
    scope: &mut v8::HandleScope,
    _key: v8::Local<v8::Name>,
    args: v8::PropertyCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let dom = dom_state(scope);
    let id = match read_node_id(scope, args.this()) {
        Some(id) => id,
        None => return,
    };
    let parent_id = {
        let doc = dom.document.borrow();
        doc.node(id).parent
    };
    match parent_id {
        Some(pid) if pid == NodeId::ROOT => {
            // Parent is the Document — return the document global.
            let context = scope.get_current_context();
            let global = context.global(scope);
            let key = v8::String::new(scope, "document").unwrap();
            if let Some(doc) = global.get(scope, key.into()) {
                rv.set(doc);
            }
        }
        Some(pid) => {
            if let Some(obj) = get_or_create_node(scope, &dom, pid) {
                rv.set(obj.into());
            }
        }
        None => return_null(scope, rv),
    }
}

fn element_parent_element(
    scope: &mut v8::HandleScope,
    _key: v8::Local<v8::Name>,
    args: v8::PropertyCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let dom = dom_state(scope);
    let id = match read_node_id(scope, args.this()) {
        Some(id) => id,
        None => return,
    };
    let parent_id = {
        let doc = dom.document.borrow();
        doc.node(id).parent
    };
    match parent_id {
        Some(pid) if pid == NodeId::ROOT => return_null(scope, rv),
        Some(pid) => {
            if let Some(obj) = get_or_create_node(scope, &dom, pid) {
                rv.set(obj.into());
            } else {
                return_null(scope, rv);
            }
        }
        None => return_null(scope, rv),
    }
}

fn element_children(
    scope: &mut v8::HandleScope,
    _key: v8::Local<v8::Name>,
    args: v8::PropertyCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let dom = dom_state(scope);
    let id = match read_node_id(scope, args.this()) {
        Some(id) => id,
        None => return,
    };
    let element_kids: Vec<NodeId> = {
        let doc = dom.document.borrow();
        doc.node(id)
            .children
            .iter()
            .copied()
            .filter(|c| matches!(doc.node(*c).kind, NodeKind::Element { .. }))
            .collect()
    };
    let arr = nodes_to_array(scope, &dom, &element_kids);
    rv.set(arr.into());
}

fn element_child_nodes(
    scope: &mut v8::HandleScope,
    _key: v8::Local<v8::Name>,
    args: v8::PropertyCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let dom = dom_state(scope);
    let id = match read_node_id(scope, args.this()) {
        Some(id) => id,
        None => return,
    };
    let kids: Vec<NodeId> = {
        let doc = dom.document.borrow();
        doc.node(id).children.clone()
    };
    let arr = nodes_to_array(scope, &dom, &kids);
    rv.set(arr.into());
}

fn element_child_element_count(
    scope: &mut v8::HandleScope,
    _key: v8::Local<v8::Name>,
    args: v8::PropertyCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let dom = dom_state(scope);
    let id = match read_node_id(scope, args.this()) {
        Some(id) => id,
        None => return,
    };
    let count = {
        let doc = dom.document.borrow();
        doc.node(id)
            .children
            .iter()
            .filter(|c| matches!(doc.node(**c).kind, NodeKind::Element { .. }))
            .count() as u32
    };
    rv.set_uint32(count);
}

fn element_first_child(
    scope: &mut v8::HandleScope,
    _key: v8::Local<v8::Name>,
    args: v8::PropertyCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let dom = dom_state(scope);
    let id = match read_node_id(scope, args.this()) {
        Some(id) => id,
        None => return,
    };
    let first = {
        let doc = dom.document.borrow();
        doc.node(id).children.first().copied()
    };
    match first {
        Some(c) => match get_or_create_node(scope, &dom, c) {
            Some(obj) => rv.set(obj.into()),
            None => return_null(scope, rv),
        },
        None => return_null(scope, rv),
    }
}

fn element_last_child(
    scope: &mut v8::HandleScope,
    _key: v8::Local<v8::Name>,
    args: v8::PropertyCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let dom = dom_state(scope);
    let id = match read_node_id(scope, args.this()) {
        Some(id) => id,
        None => return,
    };
    let last = {
        let doc = dom.document.borrow();
        doc.node(id).children.last().copied()
    };
    match last {
        Some(c) => match get_or_create_node(scope, &dom, c) {
            Some(obj) => rv.set(obj.into()),
            None => return_null(scope, rv),
        },
        None => return_null(scope, rv),
    }
}

fn element_first_element_child(
    scope: &mut v8::HandleScope,
    _key: v8::Local<v8::Name>,
    args: v8::PropertyCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let dom = dom_state(scope);
    let id = match read_node_id(scope, args.this()) {
        Some(id) => id,
        None => return,
    };
    let first = {
        let doc = dom.document.borrow();
        doc.node(id)
            .children
            .iter()
            .copied()
            .find(|c| matches!(doc.node(*c).kind, NodeKind::Element { .. }))
    };
    match first {
        Some(c) => match get_or_create_node(scope, &dom, c) {
            Some(obj) => rv.set(obj.into()),
            None => return_null(scope, rv),
        },
        None => return_null(scope, rv),
    }
}

fn element_last_element_child(
    scope: &mut v8::HandleScope,
    _key: v8::Local<v8::Name>,
    args: v8::PropertyCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let dom = dom_state(scope);
    let id = match read_node_id(scope, args.this()) {
        Some(id) => id,
        None => return,
    };
    let last = {
        let doc = dom.document.borrow();
        doc.node(id)
            .children
            .iter()
            .rev()
            .copied()
            .find(|c| matches!(doc.node(*c).kind, NodeKind::Element { .. }))
    };
    match last {
        Some(c) => match get_or_create_node(scope, &dom, c) {
            Some(obj) => rv.set(obj.into()),
            None => return_null(scope, rv),
        },
        None => return_null(scope, rv),
    }
}

fn element_next_sibling(
    scope: &mut v8::HandleScope,
    _key: v8::Local<v8::Name>,
    args: v8::PropertyCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let dom = dom_state(scope);
    let id = match read_node_id(scope, args.this()) {
        Some(id) => id,
        None => return,
    };
    let sibling = sibling_id(&dom.document.borrow(), id, /*forward=*/ true);
    match sibling {
        Some(c) => match get_or_create_node(scope, &dom, c) {
            Some(obj) => rv.set(obj.into()),
            None => return_null(scope, rv),
        },
        None => return_null(scope, rv),
    }
}

fn element_previous_sibling(
    scope: &mut v8::HandleScope,
    _key: v8::Local<v8::Name>,
    args: v8::PropertyCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let dom = dom_state(scope);
    let id = match read_node_id(scope, args.this()) {
        Some(id) => id,
        None => return,
    };
    let sibling = sibling_id(&dom.document.borrow(), id, /*forward=*/ false);
    match sibling {
        Some(c) => match get_or_create_node(scope, &dom, c) {
            Some(obj) => rv.set(obj.into()),
            None => return_null(scope, rv),
        },
        None => return_null(scope, rv),
    }
}

fn sibling_id(doc: &Document, id: NodeId, forward: bool) -> Option<NodeId> {
    let parent = doc.node(id).parent?;
    let kids = &doc.node(parent).children;
    let pos = kids.iter().position(|&c| c == id)?;
    if forward {
        kids.get(pos + 1).copied()
    } else {
        if pos == 0 {
            return None;
        }
        kids.get(pos - 1).copied()
    }
}

fn element_text_content(
    scope: &mut v8::HandleScope,
    _key: v8::Local<v8::Name>,
    args: v8::PropertyCallbackArguments,
    rv: v8::ReturnValue,
) {
    let dom = dom_state(scope);
    let id = match read_node_id(scope, args.this()) {
        Some(id) => id,
        None => return,
    };
    let s = dom.document.borrow().text_content(id);
    return_string(scope, rv, &s);
}

fn element_inner_html(
    scope: &mut v8::HandleScope,
    _key: v8::Local<v8::Name>,
    args: v8::PropertyCallbackArguments,
    rv: v8::ReturnValue,
) {
    let dom = dom_state(scope);
    let id = match read_node_id(scope, args.this()) {
        Some(id) => id,
        None => return,
    };
    let s = dom.document.borrow().serialize_node_children(id);
    return_string(scope, rv, &s);
}

fn element_outer_html(
    scope: &mut v8::HandleScope,
    _key: v8::Local<v8::Name>,
    args: v8::PropertyCallbackArguments,
    rv: v8::ReturnValue,
) {
    let dom = dom_state(scope);
    let id = match read_node_id(scope, args.this()) {
        Some(id) => id,
        None => return,
    };
    let s = dom.document.borrow().serialize_node(id);
    return_string(scope, rv, &s);
}

// ---------------------------------------------------------------------------
// element methods
// ---------------------------------------------------------------------------

fn element_get_attribute(
    scope: &mut v8::HandleScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let dom = dom_state(scope);
    let id = match read_node_id(scope, args.this()) {
        Some(id) => id,
        None => return,
    };
    if args.length() < 1 {
        return_null(scope, rv);
        return;
    }
    let name = args.get(0).to_rust_string_lossy(scope);
    let value = {
        let doc = dom.document.borrow();
        if let NodeKind::Element { attrs, .. } = &doc.node(id).kind {
            attrs
                .iter()
                .find(|a| a.name == name)
                .map(|a| a.value.clone())
        } else {
            None
        }
    };
    match value {
        Some(v) => {
            let s = v8::String::new(scope, &v).unwrap();
            rv.set(s.into());
        }
        None => return_null(scope, rv),
    }
}

fn element_has_attribute(
    scope: &mut v8::HandleScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let dom = dom_state(scope);
    let id = match read_node_id(scope, args.this()) {
        Some(id) => id,
        None => return,
    };
    if args.length() < 1 {
        rv.set_bool(false);
        return;
    }
    let name = args.get(0).to_rust_string_lossy(scope);
    let present = {
        let doc = dom.document.borrow();
        if let NodeKind::Element { attrs, .. } = &doc.node(id).kind {
            attrs.iter().any(|a| a.name == name)
        } else {
            false
        }
    };
    rv.set_bool(present);
}

fn element_get_attribute_names(
    scope: &mut v8::HandleScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let dom = dom_state(scope);
    let id = match read_node_id(scope, args.this()) {
        Some(id) => id,
        None => return,
    };
    let names: Vec<String> = {
        let doc = dom.document.borrow();
        if let NodeKind::Element { attrs, .. } = &doc.node(id).kind {
            attrs.iter().map(|a| a.name.clone()).collect()
        } else {
            Vec::new()
        }
    };
    let locals: Vec<v8::Local<v8::Value>> = names
        .iter()
        .map(|n| v8::String::new(scope, n).unwrap().into())
        .collect();
    let arr = v8::Array::new_with_elements(scope, &locals);
    rv.set(arr.into());
}

fn element_has_attributes(
    scope: &mut v8::HandleScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let dom = dom_state(scope);
    let id = match read_node_id(scope, args.this()) {
        Some(id) => id,
        None => return,
    };
    let any = {
        let doc = dom.document.borrow();
        if let NodeKind::Element { attrs, .. } = &doc.node(id).kind {
            !attrs.is_empty()
        } else {
            false
        }
    };
    rv.set_bool(any);
}

// ---------------------------------------------------------------------------
// text accessors
// ---------------------------------------------------------------------------

fn text_node_type(
    scope: &mut v8::HandleScope,
    _key: v8::Local<v8::Name>,
    _args: v8::PropertyCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let _ = scope;
    rv.set_uint32(3); // TEXT_NODE per DOM spec.
}

fn text_node_name(
    scope: &mut v8::HandleScope,
    _key: v8::Local<v8::Name>,
    _args: v8::PropertyCallbackArguments,
    rv: v8::ReturnValue,
) {
    return_string(scope, rv, "#text");
}

fn text_data(
    scope: &mut v8::HandleScope,
    _key: v8::Local<v8::Name>,
    args: v8::PropertyCallbackArguments,
    rv: v8::ReturnValue,
) {
    let dom = dom_state(scope);
    let id = match read_node_id(scope, args.this()) {
        Some(id) => id,
        None => return,
    };
    let s = {
        let doc = dom.document.borrow();
        if let NodeKind::Text(s) = &doc.node(id).kind {
            s.clone()
        } else {
            String::new()
        }
    };
    return_string(scope, rv, &s);
}

// ---------------------------------------------------------------------------
// document accessors / methods
// ---------------------------------------------------------------------------

fn document_document_element(
    scope: &mut v8::HandleScope,
    _key: v8::Local<v8::Name>,
    _args: v8::PropertyCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let dom = dom_state(scope);
    let id = dom.document.borrow().find_first_element("html");
    match id {
        Some(i) => match get_or_create_node(scope, &dom, i) {
            Some(obj) => rv.set(obj.into()),
            None => return_null(scope, rv),
        },
        None => return_null(scope, rv),
    }
}

fn document_head(
    scope: &mut v8::HandleScope,
    _key: v8::Local<v8::Name>,
    _args: v8::PropertyCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let dom = dom_state(scope);
    let id = dom.document.borrow().find_first_element("head");
    match id {
        Some(i) => match get_or_create_node(scope, &dom, i) {
            Some(obj) => rv.set(obj.into()),
            None => return_null(scope, rv),
        },
        None => return_null(scope, rv),
    }
}

fn document_body(
    scope: &mut v8::HandleScope,
    _key: v8::Local<v8::Name>,
    _args: v8::PropertyCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let dom = dom_state(scope);
    let id = dom.document.borrow().find_first_element("body");
    match id {
        Some(i) => match get_or_create_node(scope, &dom, i) {
            Some(obj) => rv.set(obj.into()),
            None => return_null(scope, rv),
        },
        None => return_null(scope, rv),
    }
}

fn document_title(
    scope: &mut v8::HandleScope,
    _key: v8::Local<v8::Name>,
    _args: v8::PropertyCallbackArguments,
    rv: v8::ReturnValue,
) {
    let dom = dom_state(scope);
    let title = {
        let doc = dom.document.borrow();
        match doc.find_first_element("title") {
            Some(id) => doc.text_content(id),
            None => String::new(),
        }
    };
    return_string(scope, rv, &title);
}

fn document_url(
    scope: &mut v8::HandleScope,
    _key: v8::Local<v8::Name>,
    _args: v8::PropertyCallbackArguments,
    rv: v8::ReturnValue,
) {
    let dom = dom_state(scope);
    let url = dom.url.clone();
    return_string(scope, rv, &url);
}

fn document_node_type(
    scope: &mut v8::HandleScope,
    _key: v8::Local<v8::Name>,
    _args: v8::PropertyCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let _ = scope;
    rv.set_uint32(9); // DOCUMENT_NODE per DOM spec.
}

fn document_node_name(
    scope: &mut v8::HandleScope,
    _key: v8::Local<v8::Name>,
    _args: v8::PropertyCallbackArguments,
    rv: v8::ReturnValue,
) {
    return_string(scope, rv, "#document");
}

fn document_get_element_by_id(
    scope: &mut v8::HandleScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let dom = dom_state(scope);
    if args.length() < 1 {
        return_null(scope, rv);
        return;
    }
    let needle = args.get(0).to_rust_string_lossy(scope);
    let id = dom.document.borrow().find_by_id(&needle);
    match id {
        Some(i) => match get_or_create_node(scope, &dom, i) {
            Some(obj) => rv.set(obj.into()),
            None => return_null(scope, rv),
        },
        None => return_null(scope, rv),
    }
}

// ---------------------------------------------------------------------------
// shared helpers
// ---------------------------------------------------------------------------

/// Build a JS `Array` of node objects from a slice of [`NodeId`]s.
///
/// Non-Element / non-Text nodes (comments, doctypes) are skipped — they
/// can't be exposed in Phase 1C. The returned array is a plain `Array`,
/// not a live `NodeList` / `HTMLCollection`; live collections are
/// Phase 1E work.
fn nodes_to_array<'s>(
    scope: &mut v8::HandleScope<'s>,
    dom: &Rc<DomInner>,
    ids: &[NodeId],
) -> v8::Local<'s, v8::Array> {
    let mut items: Vec<v8::Local<v8::Value>> = Vec::with_capacity(ids.len());
    for id in ids {
        if let Some(obj) = get_or_create_node(scope, dom, *id) {
            items.push(obj.into());
        }
    }
    v8::Array::new_with_elements(scope, &items)
}
