//! Inline content as a flat list of marked leaves, shared by fetch, upload and normalize.
//!
//! ADF stores formatting as a set of marks on each text node, while markdown nests it:
//! `**[a](u)**` and `[**a**](u)` are the same ADF. Flattening markdown into [`Item`]s and
//! rebuilding it with a fixed nesting order ([`build`]) gives one canonical form, so the
//! normalizer and fetch agree however the formatting was nested.

use comrak::nodes::{AstNode, NodeCode, NodeLink, NodeValue};
use comrak::Arena;

use crate::markdown::append;

/// A formatting mark, in markdown terms. Marks without markdown syntax are written as
/// inline HTML (`<u>`, `<sub>`, `<sup>`, `<span style="color: ...">`).
#[derive(Debug, Clone, PartialEq)]
pub enum MdMark {
    Link { url: String, title: String },
    Strong,
    Emph,
    Strike,
    Underline,
    Sub,
    Sup,
    Color(String),
    Background(String),
}

impl MdMark {
    /// Nesting order: lower ranks wrap higher ones.
    fn rank(&self) -> u8 {
        match self {
            MdMark::Link { .. } => 0,
            MdMark::Strong => 1,
            MdMark::Emph => 2,
            MdMark::Strike => 3,
            MdMark::Underline => 4,
            MdMark::Sub => 5,
            MdMark::Sup => 6,
            MdMark::Color(_) => 7,
            MdMark::Background(_) => 8,
        }
    }

    /// Opening and closing HTML for marks written as inline HTML.
    fn html(&self) -> Option<(String, &'static str)> {
        Some(match self {
            MdMark::Underline => ("<u>".into(), "</u>"),
            MdMark::Sub => ("<sub>".into(), "</sub>"),
            MdMark::Sup => ("<sup>".into(), "</sup>"),
            MdMark::Color(c) => (format!("<span style=\"color: {c}\">"), "</span>"),
            MdMark::Background(c) => (format!("<span style=\"background-color: {c}\">"), "</span>"),
            _ => return None,
        })
    }
}

/// An inline leaf.
#[derive(Debug, Clone)]
pub enum Leaf<'a> {
    Text(String),
    Code(String),
    /// Inline HTML written verbatim: tags rfluence doesn't map to marks, `<span data-adf=...>`.
    Html(String),
    LineBreak,
    SoftBreak,
    /// A comrak inline node kept as it is (images, footnote references, ...).
    Other(&'a AstNode<'a>),
}

#[derive(Debug, Clone)]
pub struct Item<'a> {
    pub marks: Vec<MdMark>,
    pub leaf: Leaf<'a>,
}

impl<'a> Item<'a> {
    pub fn new(marks: Vec<MdMark>, leaf: Leaf<'a>) -> Self {
        Item { marks, leaf }
    }
}

enum Open {
    Mark(&'static str, MdMark),
    Verbatim(String),
}

/// Flatten comrak inline nodes into items. Recognised HTML mark tags become marks; other
/// inline HTML is kept verbatim.
pub fn flatten<'a>(nodes: &[&'a AstNode<'a>], marks: &[MdMark]) -> Vec<Item<'a>> {
    let mut items = Vec::new();
    let mut open: Vec<Open> = Vec::new();
    let mut i = 0;
    while i < nodes.len() {
        let node = nodes[i];
        let mut cur = marks.to_vec();
        cur.extend(open.iter().filter_map(|o| match o {
            Open::Mark(_, m) => Some(m.clone()),
            Open::Verbatim(_) => None,
        }));
        let value = node.data().value.clone();
        match value {
            NodeValue::Text(t) => items.push(Item::new(cur, Leaf::Text(t.to_string()))),
            NodeValue::SoftBreak => items.push(Item::new(cur, Leaf::SoftBreak)),
            NodeValue::LineBreak => items.push(Item::new(cur, Leaf::LineBreak)),
            NodeValue::Code(c) => items.push(Item::new(cur, Leaf::Code(c.literal))),
            NodeValue::Emph | NodeValue::Strong | NodeValue::Strikethrough | NodeValue::Link(_) => {
                let mark = match value {
                    NodeValue::Emph => MdMark::Emph,
                    NodeValue::Strong => MdMark::Strong,
                    NodeValue::Strikethrough => MdMark::Strike,
                    NodeValue::Link(l) => MdMark::Link { url: l.url.clone(), title: l.title.clone() },
                    _ => unreachable!(),
                };
                cur.push(mark);
                let children: Vec<_> = node.children().collect();
                items.extend(flatten(&children, &cur));
            }
            NodeValue::HtmlInline(html) => {
                if let Some((tag, mark)) = parse_mark_tag(&html) {
                    open.push(Open::Mark(tag, mark));
                } else if let Some(tag) = close_tag(&html) {
                    match open.iter().rposition(|o| match o {
                        Open::Mark(t, _) => *t == tag,
                        Open::Verbatim(t) => t == tag,
                    }) {
                        Some(pos) => {
                            if let Open::Verbatim(_) = open.remove(pos) {
                                items.push(Item::new(cur, Leaf::Html(html)));
                            }
                        }
                        None => items.push(Item::new(cur, Leaf::Html(html))),
                    }
                } else {
                    if let Some(tag) = open_tag(&html) {
                        open.push(Open::Verbatim(tag.to_string()));
                    }
                    items.push(Item::new(cur, Leaf::Html(html)));
                }
            }
            _ => items.push(Item::new(cur, Leaf::Other(node))),
        }
        i += 1;
    }
    items
}

/// Recognise the inline HTML rfluence writes for marks without markdown syntax.
fn parse_mark_tag(html: &str) -> Option<(&'static str, MdMark)> {
    let h = html.trim();
    match h {
        "<u>" => return Some(("u", MdMark::Underline)),
        "<sub>" => return Some(("sub", MdMark::Sub)),
        "<sup>" => return Some(("sup", MdMark::Sup)),
        _ => {}
    }
    let style = h.strip_prefix("<span style=\"")?.strip_suffix("\">")?;
    let (prop, value) = style.trim().trim_end_matches(';').split_once(':')?;
    let value = value.trim().to_string();
    match prop.trim() {
        "color" => Some(("span", MdMark::Color(value))),
        "background-color" => Some(("span", MdMark::Background(value))),
        _ => None,
    }
}

fn tag_name(s: &str) -> &str {
    let end = s.find(|c: char| !(c.is_ascii_alphanumeric() || c == '-')).unwrap_or(s.len());
    &s[..end]
}

fn close_tag(html: &str) -> Option<&str> {
    let name = tag_name(html.trim().strip_prefix("</")?);
    (!name.is_empty()).then_some(name)
}

fn open_tag(html: &str) -> Option<&str> {
    let h = html.trim();
    if h.ends_with("/>") || h.starts_with("<!") || h.starts_with("<?") {
        return None;
    }
    let name = tag_name(h.strip_prefix('<')?);
    let void = matches!(name.to_ascii_lowercase().as_str(), "br" | "img" | "hr" | "wbr" | "input");
    (!name.is_empty() && !void).then_some(name)
}

/// Rebuild items under `parent` with canonical nesting: the longest run sharing the
/// lowest-ranked mark is wrapped first.
pub fn build<'a>(arena: &'a Arena<'a>, parent: &'a AstNode<'a>, items: &[Item<'a>]) {
    let mut i = 0;
    while i < items.len() {
        let Some(outer) = items[i].marks.iter().min_by_key(|m| m.rank()).cloned() else {
            leaf(arena, parent, &items[i].leaf);
            i += 1;
            continue;
        };
        let mut j = i;
        while j < items.len() && items[j].marks.contains(&outer) {
            j += 1;
        }
        let mut inner: Vec<Item<'a>> = items[i..j]
            .iter()
            .map(|it| Item::new(it.marks.iter().filter(|m| **m != outer).cloned().collect(), it.leaf.clone()))
            .collect();
        // Emphasis delimiters can't open after or close before whitespace (`** a**` isn't
        // bold), so move edge whitespace outside them.
        let delimited = matches!(outer, MdMark::Strong | MdMark::Emph | MdMark::Strike);
        let (lead, trail) = if delimited { trim_edges(&mut inner) } else { (String::new(), String::new()) };
        if !lead.is_empty() {
            leaf(arena, parent, &Leaf::Text(lead));
        }
        if !inner.is_empty() {
            wrap(arena, parent, &outer, &inner);
        }
        if !trail.is_empty() {
            leaf(arena, parent, &Leaf::Text(trail));
        }
        i = j;
    }
}

fn trim_edges(items: &mut Vec<Item>) -> (String, String) {
    let mut lead = String::new();
    while let Some(Leaf::Text(t)) = items.first().map(|i| &i.leaf) {
        let trimmed = t.trim_start();
        lead.push_str(&t[..t.len() - trimmed.len()]);
        if trimmed.is_empty() {
            items.remove(0);
        } else {
            let rest = trimmed.to_string();
            items[0].leaf = Leaf::Text(rest);
            break;
        }
    }
    let mut trail = String::new();
    while let Some(Leaf::Text(t)) = items.last().map(|i| &i.leaf) {
        let trimmed = t.trim_end();
        trail.insert_str(0, &t[trimmed.len()..]);
        if trimmed.is_empty() {
            items.pop();
        } else {
            let rest = trimmed.to_string();
            let n = items.len() - 1;
            items[n].leaf = Leaf::Text(rest);
            break;
        }
    }
    (lead, trail)
}

fn wrap<'a>(arena: &'a Arena<'a>, parent: &'a AstNode<'a>, mark: &MdMark, inner: &[Item<'a>]) {
    if let Some((open, close)) = mark.html() {
        append(arena, parent, NodeValue::HtmlInline(open));
        build(arena, parent, inner);
        append(arena, parent, NodeValue::HtmlInline(close.into()));
        return;
    }
    let value = match mark {
        MdMark::Link { url, title } => NodeValue::Link(Box::new(NodeLink { url: url.clone(), title: title.clone() })),
        MdMark::Strong => NodeValue::Strong,
        MdMark::Emph => NodeValue::Emph,
        MdMark::Strike => NodeValue::Strikethrough,
        _ => unreachable!("HTML marks handled above"),
    };
    let node = append(arena, parent, value);
    build(arena, node, inner);
}

fn leaf<'a>(arena: &'a Arena<'a>, parent: &'a AstNode<'a>, leaf: &Leaf<'a>) {
    match leaf {
        Leaf::Text(t) if t.is_empty() => {}
        Leaf::Text(t) => {
            // A newline in text is a soft break in markdown.
            for (i, line) in t.split('\n').enumerate() {
                if i > 0 {
                    append(arena, parent, NodeValue::SoftBreak);
                }
                if !line.is_empty() {
                    append(arena, parent, NodeValue::Text(line.to_string().into()));
                }
            }
        }
        // comrak's code span renderer can't handle an empty literal.
        Leaf::Code(c) if c.is_empty() => {}
        Leaf::Code(c) => {
            append(arena, parent, NodeValue::Code(NodeCode { num_backticks: 1, literal: c.replace('\n', " ") }));
        }
        Leaf::Html(h) => {
            append(arena, parent, NodeValue::HtmlInline(h.clone()));
        }
        Leaf::LineBreak => {
            append(arena, parent, NodeValue::LineBreak);
        }
        Leaf::SoftBreak => {
            append(arena, parent, NodeValue::SoftBreak);
        }
        Leaf::Other(node) => {
            node.detach();
            parent.append(node);
        }
    }
}
