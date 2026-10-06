//! ADF (Atlassian Document Format) as a generic tree.
//!
//! Nodes are kept generic rather than typed per node kind: Confluence pages contain node
//! types and attributes rfluence doesn't understand (app macros, new editor features), and
//! those must survive a fetch/upload cycle unchanged. The converter reads only the
//! attributes it needs and passes everything else through.

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};

/// An ADF node: `doc`, `paragraph`, `text`, `extension`, ...
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Node {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(
        default,
        skip_serializing_if = "Map::is_empty",
        deserialize_with = "null_as_empty"
    )]
    pub attrs: Map<String, Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub content: Vec<Node>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub marks: Vec<Mark>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Any other top-level keys (e.g. `version` on `doc`).
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// An ADF mark on a node: `strong`, `link`, `annotation`, `breakout`, ...
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Mark {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(
        default,
        skip_serializing_if = "Map::is_empty",
        deserialize_with = "null_as_empty"
    )]
    pub attrs: Map<String, Value>,
}

fn null_as_empty<'de, D: Deserializer<'de>>(d: D) -> Result<Map<String, Value>, D::Error> {
    Ok(Option::<Map<String, Value>>::deserialize(d)?.unwrap_or_default())
}

impl Node {
    pub fn new(kind: &str) -> Self {
        Node {
            kind: kind.to_string(),
            ..Default::default()
        }
    }

    /// A `doc` node with `version: 1`.
    pub fn doc(content: Vec<Node>) -> Self {
        let mut doc = Node::new("doc").with_content(content);
        doc.extra.insert("version".into(), 1.into());
        doc
    }

    pub fn text(text: impl Into<String>) -> Self {
        Node {
            kind: "text".into(),
            text: Some(text.into()),
            ..Default::default()
        }
    }

    pub fn with_attr(mut self, key: &str, value: impl Into<Value>) -> Self {
        self.attrs.insert(key.to_string(), value.into());
        self
    }

    pub fn with_content(mut self, content: Vec<Node>) -> Self {
        self.content = content;
        self
    }

    pub fn with_marks(mut self, marks: Vec<Mark>) -> Self {
        self.marks = marks;
        self
    }

    pub fn is(&self, kind: &str) -> bool {
        self.kind == kind
    }

    pub fn attr(&self, key: &str) -> Option<&Value> {
        self.attrs.get(key)
    }

    pub fn attr_str(&self, key: &str) -> Option<&str> {
        self.attrs.get(key).and_then(Value::as_str)
    }

    pub fn attr_f64(&self, key: &str) -> Option<f64> {
        self.attrs.get(key).and_then(Value::as_f64)
    }

    pub fn mark(&self, kind: &str) -> Option<&Mark> {
        self.marks.iter().find(|m| m.kind == kind)
    }

    /// The node's text content, concatenated (text nodes only).
    pub fn plain_text(&self) -> String {
        let mut out = String::new();
        self.collect_text(&mut out);
        out
    }

    fn collect_text(&self, out: &mut String) {
        if let Some(t) = &self.text {
            out.push_str(t);
        }
        for child in &self.content {
            child.collect_text(out);
        }
    }

    /// Visit this node and all descendants, depth first.
    pub fn walk<'a>(&'a self, f: &mut impl FnMut(&'a Node)) {
        f(self);
        for child in &self.content {
            child.walk(f);
        }
    }

    pub fn walk_mut(&mut self, f: &mut impl FnMut(&mut Node)) {
        f(self);
        for child in &mut self.content {
            child.walk_mut(f);
        }
    }
}

impl Mark {
    pub fn new(kind: &str) -> Self {
        Mark {
            kind: kind.to_string(),
            attrs: Map::new(),
        }
    }

    pub fn with_attr(mut self, key: &str, value: impl Into<Value>) -> Self {
        self.attrs.insert(key.to_string(), value.into());
        self
    }

    pub fn attr_str(&self, key: &str) -> Option<&str> {
        self.attrs.get(key).and_then(Value::as_str)
    }

    pub fn attr_f64(&self, key: &str) -> Option<f64> {
        self.attrs.get(key).and_then(Value::as_f64)
    }
}

/// Default `breakout` width for a mode: what an editor save fills in when the width is
/// missing (design.md, "Code block languages and widths").
pub fn default_breakout_width(mode: &str) -> Option<f64> {
    match mode {
        "wide" => Some(760.0),
        "full-width" => Some(1800.0),
        _ => None,
    }
}

/// Default table width for a table `layout`.
pub fn default_table_width(layout: &str) -> Option<f64> {
    match layout {
        "default" => Some(760.0),
        "wide" => Some(960.0),
        "full-width" => Some(1800.0),
        _ => None,
    }
}

/// Is this `breakout` mark the default (`wide` at 760)?
pub fn is_default_breakout(mark: &Mark) -> bool {
    mark.attr_str("mode") == Some("wide") && mark.attr_f64("width") == Some(760.0)
}

/// Remove what Confluence adds on save and what an editor save rewrites, so that two
/// versions of a page that differ only in this noise compare equal, and so ```` ```adf ````
/// blocks written by fetch don't change when someone edits the page in Confluence.
///
/// See design.md, "What Confluence rewrites on save".
pub fn strip_noise(node: &mut Node) {
    node.walk_mut(&mut |n| {
        n.attrs.remove("localId");
        n.attrs.retain(|k, _| !k.starts_with("__"));
        for mark in &mut n.marks {
            mark.attrs.retain(|k, _| !k.starts_with("__"));
        }
        match n.kind.as_str() {
            "mention" if n.attr_str("accessLevel") == Some("") => {
                n.attrs.remove("accessLevel");
            }
            "embedCard" => {
                n.attrs.remove("originalWidth");
                n.attrs.remove("originalHeight");
            }
            "extension" | "bodiedExtension" | "inlineExtension" => {
                if let Some(Value::Object(params)) = n.attrs.get_mut("parameters") {
                    if let Some(Value::Object(meta)) = params.get_mut("macroMetadata") {
                        meta.remove("macroId");
                    }
                    if let Some(Value::Object(macro_params)) = params.get_mut("macroParams") {
                        macro_params.remove("_parentId");
                    }
                }
            }
            "orderedList" if n.attr_f64("order") == Some(1.0) => {
                n.attrs.remove("order");
            }
            "tableCell" | "tableHeader" => {
                for key in ["colspan", "rowspan"] {
                    if n.attr_f64(key) == Some(1.0) {
                        n.attrs.remove(key);
                    }
                }
            }
            "table" => {
                let layout = n.attr_str("layout").unwrap_or("default").to_string();
                if n.attr_f64("width").is_some()
                    && n.attr_f64("width") == default_table_width(&layout)
                {
                    n.attrs.remove("width");
                }
                if layout == "default" {
                    n.attrs.remove("layout");
                }
            }
            "mediaSingle" => {
                // Natural width in pixels is the default (design.md, "Images and attachments").
                let natural = n
                    .content
                    .first()
                    .filter(|m| m.is("media"))
                    .and_then(|m| m.attr_f64("width"));
                if natural.is_some()
                    && n.attr_f64("width") == natural
                    && n.attr_str("widthType").unwrap_or("pixel") == "pixel"
                {
                    n.attrs.remove("width");
                    n.attrs.remove("widthType");
                }
                if n.attr_str("layout") == Some("align-start") {
                    n.attrs.remove("layout");
                }
            }
            _ => {}
        }
        n.marks
            .retain(|m| !(m.kind == "breakout" && is_default_breakout(m)));
        for mark in &mut n.marks {
            if mark.kind == "breakout" {
                let mode = mark.attr_str("mode").unwrap_or("").to_string();
                if mark.attr_f64("width").is_some()
                    && mark.attr_f64("width") == default_breakout_width(&mode)
                {
                    mark.attrs.remove("width");
                }
            }
        }
        // Confluence reorders marks on save, differently for API and editor saves.
        n.marks.sort_by_key(mark_sort_key);
    });
    merge_text_nodes_deep(node);
    // The natural media size is filled in by Confluence; drop it after mediaSingle used it.
    node.walk_mut(&mut |n| {
        if n.is("media") && n.attr_str("type") == Some("file") {
            n.attrs.remove("width");
            n.attrs.remove("height");
        }
    });
}

fn mark_sort_key(mark: &Mark) -> (String, String) {
    (
        mark.kind.clone(),
        Value::Object(mark.attrs.clone()).to_string(),
    )
}

fn merge_text_nodes_deep(node: &mut Node) {
    for child in &mut node.content {
        merge_text_nodes_deep(child);
    }
    merge_text_nodes(&mut node.content);
}

/// Merge adjacent text nodes with the same marks and drop empty ones. Confluence does
/// both on save, so the converter must not depend on how text is split.
pub fn merge_text_nodes(nodes: &mut Vec<Node>) {
    let mut out: Vec<Node> = Vec::with_capacity(nodes.len());
    for node in nodes.drain(..) {
        if node.is("text") && node.text.as_deref().unwrap_or("").is_empty() {
            continue;
        }
        if let Some(prev) = out.last_mut()
            && node.is("text")
            && prev.is("text")
            && prev.marks == node.marks
        {
            prev.text
                .get_or_insert_with(String::new)
                .push_str(node.text.as_deref().unwrap_or(""));
            continue;
        }
        out.push(node);
    }
    *nodes = out;
}

/// Compact JSON for a node: `type` first (the struct's field order), attribute keys sorted
/// so the output doesn't depend on the order Confluence returned them in.
pub fn to_compact_json(node: &Node) -> String {
    let mut node = node.clone();
    node.walk_mut(&mut |n| {
        n.attrs = sorted_map(std::mem::take(&mut n.attrs));
        n.extra = sorted_map(std::mem::take(&mut n.extra));
        for m in &mut n.marks {
            m.attrs = sorted_map(std::mem::take(&mut m.attrs));
        }
    });
    serde_json::to_string(&node).expect("ADF serializes")
}

fn sorted_map(map: Map<String, Value>) -> Map<String, Value> {
    match sorted(Value::Object(map)) {
        Value::Object(m) => m,
        _ => unreachable!(),
    }
}

/// Recursively sort object keys.
fn sorted(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut entries: Vec<_> = map.into_iter().collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            Value::Object(entries.into_iter().map(|(k, v)| (k, sorted(v))).collect())
        }
        Value::Array(a) => Value::Array(a.into_iter().map(sorted).collect()),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_unknown_fields() {
        let json = r#"{"type":"doc","version":1,"content":[{"type":"someAppNode","attrs":{"x":{"y":[1,2]}},"custom":true}]}"#;
        let doc: Node = serde_json::from_str(json).unwrap();
        assert_eq!(doc.content[0].extra.get("custom"), Some(&Value::Bool(true)));
        let back: Value = serde_json::to_value(&doc).unwrap();
        assert_eq!(back, serde_json::from_str::<Value>(json).unwrap());
    }

    #[test]
    fn strip_noise_merges_text_and_sorts_marks() {
        let mut p = Node::new("paragraph").with_content(vec![
            Node::text("a").with_marks(vec![Mark::new("strong"), Mark::new("em")]),
            Node::text("b").with_marks(vec![Mark::new("em"), Mark::new("strong")]),
            Node::text(""),
        ]);
        strip_noise(&mut p);
        assert_eq!(p.content.len(), 1);
        assert_eq!(p.content[0].text.as_deref(), Some("ab"));
        assert_eq!(p.content[0].marks[0].kind, "em");
    }
}
