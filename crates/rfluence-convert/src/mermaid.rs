//! Mermaid diagrams: merfluence diagrams, and Mermaid Diagrams Viewer macros drawn from a code
//! block. See design.md, "Mermaid diagrams (merfluence)" and "Mermaid diagrams on sites without
//! merfluence".

use std::collections::{HashMap, HashSet};

use serde_json::{Value, json};

use crate::adf::{self, Node};
use crate::settings::Settings;

/// merfluence's app ID, from its manifest.
pub const MERFLUENCE_APP_ID: &str = "5321c3d1-955d-42ac-9f09-d4d6f0802224";
/// Mermaid Diagrams Viewer's app ID, from its manifest.
pub const VIEWER_APP_ID: &str = "23392b90-4271-4239-98ca-a3e96c663cbb";
/// Both apps' macro key.
const MODULE: &str = "mermaid-diagram";
/// The title of the expand holding a viewer diagram's source.
pub const SOURCE_TITLE: &str = "Mermaid source";

/// Which app draws the diagrams.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MermaidKind {
    /// The source is in the macro (merfluence, or a fork of it).
    Merfluence,
    /// The macro draws a code block on the page (Mermaid Diagrams Viewer).
    Viewer,
}

/// A Mermaid Forge app's IDs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MermaidApp {
    pub kind: MermaidKind,
    pub app_id: String,
    pub environment_id: String,
}

impl MermaidApp {
    /// Read the IDs from a diagram's `extensionKey` (`<app>/<env>/static/<module>`). The
    /// viewer is recognised by its app ID; anything else is taken to work like merfluence.
    pub fn from_extension_key(key: &str) -> Option<MermaidApp> {
        let mut parts = key.split('/');
        let (app, env) = (parts.next()?, parts.next()?);
        (parts.next() == Some("static")).then(|| MermaidApp::new(app, env))
    }

    pub fn new(app_id: &str, environment_id: &str) -> MermaidApp {
        let kind = if app_id == VIEWER_APP_ID { MermaidKind::Viewer } else { MermaidKind::Merfluence };
        MermaidApp { kind, app_id: app_id.into(), environment_id: environment_id.into() }
    }

    fn key(&self) -> String {
        format!("{}/{}/static/{MODULE}", self.app_id, self.environment_id)
    }

    fn extension(&self, title: &str, guest_params: Option<Value>, local_id: Option<&str>) -> Node {
        let key = self.key();
        let mut parameters = json!({ "extensionId": format!("ari:cloud:ecosystem::extension/{key}"), "extensionTitle": title });
        if let Some(guest) = guest_params {
            parameters["layout"] = "extension".into();
            parameters["guestParams"] = guest;
            parameters["forgeEnvironment"] = "PRODUCTION".into();
        }
        let mut node = Node::new("extension")
            .with_attr("layout", "default")
            .with_attr("extensionType", "com.atlassian.ecosystem")
            .with_attr("extensionKey", key)
            .with_attr("text", title);
        if let Some(id) = local_id {
            parameters["localId"] = id.into();
            node = node.with_attr("localId", id);
        }
        node.with_attr("parameters", parameters)
    }

    /// The nodes for a diagram: a merfluence node, or the viewer's source expand and macro.
    /// `local_id` is the viewer macro's (it finds its place on the page by it).
    pub fn diagram(&self, source: &str, settings: &Settings, local_id: &str) -> Vec<Node> {
        match self.kind {
            MermaidKind::Merfluence => {
                // Minimal: the source and settings, no cached SVGs.
                let mut guest = serde_json::Map::new();
                guest.insert("source".into(), source.into());
                for key in ["theme", "mermaidVersion"] {
                    if let Some(v) = settings.get(key) {
                        guest.insert(key.into(), v.into());
                    }
                }
                if settings.get("useMaxWidth") == Some("false") {
                    guest.insert("useMaxWidth".into(), false.into());
                }
                vec![self.extension("Merfluence", Some(Value::Object(guest)), None)]
            }
            MermaidKind::Viewer => {
                let code = Node::new("codeBlock").with_attr("language", "mermaid").with_content(if source.is_empty() {
                    vec![]
                } else {
                    vec![Node::text(source)]
                });
                vec![
                    Node::new("expand").with_attr("title", SOURCE_TITLE).with_content(vec![code]),
                    self.extension("Mermaid diagram", None, Some(local_id)),
                ]
            }
        }
    }
}

fn app_id(node: &Node) -> Option<&str> {
    if node.kind != "extension" || node.attr_str("extensionType") != Some("com.atlassian.ecosystem") {
        return None;
    }
    let key = node.attr_str("extensionKey")?;
    key.ends_with(&format!("/static/{MODULE}")).then(|| key.split('/').next().unwrap_or_default())
}

fn guest_params(node: &Node) -> Option<&Value> {
    node.attrs.get("parameters").and_then(|p| p.get("guestParams"))
}

/// A merfluence diagram: an ecosystem macro `mermaid-diagram` holding its source.
pub fn is_merfluence(node: &Node) -> bool {
    app_id(node).is_some_and(|id| id != VIEWER_APP_ID) && guest_params(node).and_then(|g| g.get("source")).is_some_and(Value::is_string)
}

/// A Mermaid Diagrams Viewer macro.
pub fn is_viewer(node: &Node) -> bool {
    app_id(node) == Some(VIEWER_APP_ID)
}

/// The code block a viewer macro picked in its settings (`guestParams.index`), if any.
fn picked_index(node: &Node) -> Option<usize> {
    guest_params(node).and_then(|g| g.get("index")).and_then(Value::as_u64).map(|i| i as usize)
}

/// The source of a diagram in the form rfluence uploads for the viewer: an expand titled
/// "Mermaid source" holding only a `mermaid` code block, then a viewer macro that picks no
/// code block (so it draws that one). `None` for anything else.
pub fn viewer_pair<'n>(expand: &'n Node, next: Option<&Node>) -> Option<&'n str> {
    let macro_node = next.filter(|n| is_viewer(n) && picked_index(n).is_none())?;
    let only_noise = |n: &Node, allowed: &[&str]| {
        n.attrs.keys().all(|k| allowed.contains(&k.as_str()) || k == "localId" || k.starts_with("__"))
            && n.marks.iter().all(|m| m.kind == "breakout" && adf::is_default_breakout(m))
    };
    if !(expand.is("expand") && expand.attr_str("title") == Some(SOURCE_TITLE) && only_noise(expand, &["title"])) {
        return None;
    }
    let [code] = expand.content.as_slice() else { return None };
    let extension_ok = macro_node.attrs.keys().all(|k| {
        matches!(k.as_str(), "layout" | "extensionType" | "extensionKey" | "text" | "parameters" | "localId") || k.starts_with("__")
    });
    (code.is("codeBlock")
        && code.attr_str("language") == Some("mermaid")
        && only_noise(code, &["language"])
        && code.content.iter().all(|t| t.is("text") && t.marks.is_empty())
        && extension_ok)
        .then(|| code.content.first().and_then(|t| t.text.as_deref()).unwrap_or(""))
}

/// What the viewer macros on a page draw, worked out as the viewer does: a macro that picks
/// no code block draws the nth Mermaid code block (n = its position among the page's
/// `mermaid-diagram` macros); one that picks the nth code block draws the nth of any
/// language. Returns each macro's source, and the code blocks they draw from (by address).
pub fn viewer_sources(doc: &Node) -> (HashMap<*const Node, String>, HashSet<*const Node>) {
    let mut macros: Vec<&Node> = Vec::new();
    let mut code_blocks: Vec<&Node> = Vec::new();
    doc.walk(&mut |n| {
        if app_id(n).is_some() {
            macros.push(n);
        } else if n.is("codeBlock") {
            code_blocks.push(n);
        }
    });
    let is_mermaid = |c: &&Node| {
        c.attr_str("language").is_some_and(|l| l.eq_ignore_ascii_case("mermaid")) || looks_like_mermaid(&c.plain_text())
    };
    let mermaid_blocks: Vec<&Node> = code_blocks.iter().copied().filter(is_mermaid).collect();
    let mut sources = HashMap::new();
    let mut used = HashSet::new();
    for (position, m) in macros.iter().enumerate() {
        if !is_viewer(m) {
            continue;
        }
        let block = match picked_index(m) {
            Some(i) => code_blocks.get(i),
            None => mermaid_blocks.get(position),
        };
        if let Some(block) = block {
            sources.insert(*m as *const Node, block.plain_text().trim().to_string());
            used.insert(*block as *const Node);
        }
    }
    (sources, used)
}

/// Does the text start like a Mermaid diagram? (After comments and front matter, the first
/// word names a diagram type; Mermaid's own detection works the same way.)
pub fn looks_like_mermaid(text: &str) -> bool {
    const TYPES: &[&str] = &[
        "graph", "flowchart", "flowchart-elk", "sequenceDiagram", "classDiagram", "classDiagram-v2", "stateDiagram",
        "stateDiagram-v2", "erDiagram", "journey", "gantt", "pie", "quadrantChart", "requirementDiagram", "gitGraph",
        "C4Context", "C4Container", "C4Component", "C4Dynamic", "C4Deployment", "mindmap", "timeline", "zenuml", "sankey",
        "sankey-beta", "xychart", "xychart-beta", "block", "block-beta", "packet", "packet-beta", "architecture",
        "architecture-beta", "kanban", "radar-beta", "treemap-beta", "info",
    ];
    let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with("%%"));
    let mut first = lines.next();
    if first == Some("---") {
        first = lines.by_ref().skip_while(|l| *l != "---").nth(1);
    }
    let word = first.and_then(|l| l.split(|c: char| c.is_whitespace() || c == ':' || c == ';').next()).unwrap_or("");
    TYPES.contains(&word)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Node {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/confluence/mermaid-viewer-editor/adf.json");
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    #[test]
    fn recognises_the_apps() {
        let doc = fixture();
        let macros: Vec<&Node> = doc.content.iter().filter(|n| n.is("extension")).collect();
        assert!(macros.iter().all(|m| is_viewer(m) && !is_merfluence(m)));
        let app = MermaidApp::from_extension_key(macros[0].attr_str("extensionKey").unwrap()).unwrap();
        assert_eq!(app.kind, MermaidKind::Viewer);
        assert_eq!(MermaidApp::new(MERFLUENCE_APP_ID, "env").kind, MermaidKind::Merfluence);
    }

    #[test]
    fn editor_made_viewer_diagrams_are_not_rfluence_pairs() {
        // Another expand title, no language: kept as they are.
        let doc = fixture();
        assert_eq!(viewer_pair(&doc.content[0], doc.content.get(1)), None);
    }

    #[test]
    fn pairs_the_uploaded_form() {
        let app = MermaidApp::new(VIEWER_APP_ID, "env");
        let nodes = app.diagram("flowchart TD\n  A --> B", &Settings::new(), "id-1");
        assert_eq!(viewer_pair(&nodes[0], nodes.get(1)), Some("flowchart TD\n  A --> B"));
        assert_eq!(nodes[1].attrs["parameters"]["localId"], "id-1");
        // Not without the macro, or with one that picks a code block.
        assert_eq!(viewer_pair(&nodes[0], None), None);
        let mut picked = nodes[1].clone();
        picked.attrs.get_mut("parameters").unwrap()["guestParams"] = json!({ "index": 0 });
        assert_eq!(viewer_pair(&nodes[0], Some(&picked)), None);
    }

    #[test]
    fn resolves_what_viewer_macros_draw() {
        // Diagram 1 is drawn automatically from the first Mermaid code block; diagram 2 picked
        // code block 0, which is the same one.
        let doc = fixture();
        let (sources, used) = viewer_sources(&doc);
        let flowchart = "flowchart TD\n    A[Start] --> B{Works?}\n    B -- yes --> C[Done]";
        assert_eq!(sources.len(), 2);
        assert!(sources.values().all(|s| s == flowchart));
        assert_eq!(used.len(), 1);
    }

    #[test]
    fn detects_mermaid_text() {
        assert!(looks_like_mermaid("\n\n flowchart TD\n A --> B"));
        assert!(looks_like_mermaid("%% comment\nsequenceDiagram\n A->>B: hi"));
        assert!(looks_like_mermaid("---\ntitle: x\n---\ngraph LR\n A-->B"));
        assert!(!looks_like_mermaid("print('graph')"));
        assert!(!looks_like_mermaid(""));
    }
}
