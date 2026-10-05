//! Shared comrak setup: parse/render options and AST helpers.

use std::cell::RefCell;

use comrak::nodes::{Ast, AstNode, LineColumn, NodeValue};
use comrak::{Arena, Options};

/// comrak options used everywhere: parsing markdown for upload, normalizing, and rendering
/// fetched pages. See design.md, "What 'same content' means".
pub fn options() -> Options<'static> {
    let mut o = Options::default();
    o.extension.strikethrough = true;
    o.extension.table = true;
    o.extension.autolink = true;
    o.extension.tasklist = true;
    o.extension.footnotes = true;
    o.extension.alerts = true;
    o.extension.front_matter_delimiter = Some("---".into());
    o.render.r#unsafe = true;
    // Without this, a fence with no info string is written as an indented code block, and
    // two adjacent indented blocks merge into one on the next parse.
    o.render.prefer_fenced = true;
    o
}

/// Allocate a detached AST node.
pub fn node<'a>(arena: &'a Arena<'a>, value: NodeValue) -> &'a AstNode<'a> {
    arena.alloc(AstNode::new(RefCell::new(Ast::new(value, LineColumn { line: 1, column: 1 }))))
}

/// Allocate a node and append it to `parent`.
pub fn append<'a>(arena: &'a Arena<'a>, parent: &'a AstNode<'a>, value: NodeValue) -> &'a AstNode<'a> {
    let n = node(arena, value);
    parent.append(n);
    n
}

/// Render with comrak, working around its HTML block bug: an HTML block's text is written
/// raw, bypassing the line prefixes of enclosing quotes and list items, so its trailing
/// newline ends a blockquote (`> <!-- c -->` + `>` + `> text` comes out as two quotes).
/// Without the trailing newline comrak starts the next line itself, with the prefix.
/// Multi-line HTML blocks inside quotes or list items are still affected.
pub fn render<'a>(root: &'a AstNode<'a>) -> String {
    for node in root.descendants() {
        if let NodeValue::HtmlBlock(h) = &mut node.data_mut().value {
            while h.literal.ends_with('\n') {
                h.literal.pop();
            }
        }
    }
    let mut out = String::new();
    comrak::format_commonmark(root, &options(), &mut out).expect("formatting to a String can't fail");
    out
}

/// A heading line (`# Title`), escaped by comrak.
pub fn heading(level: u8, text: &str) -> String {
    let arena = Arena::new();
    let root = node(&arena, NodeValue::Document);
    let h = append(&arena, root, NodeValue::Heading(comrak::nodes::NodeHeading { level, setext: false, closed: false }));
    append(&arena, h, NodeValue::Text(text.to_string().into()));
    render(root)
}

/// The text content of an inline subtree (text, code, and breaks as spaces).
pub fn inline_text<'a>(node: &'a AstNode<'a>) -> String {
    let mut out = String::new();
    for d in node.descendants() {
        match &d.data().value {
            NodeValue::Text(t) => out.push_str(t),
            NodeValue::Code(c) => out.push_str(&c.literal),
            NodeValue::SoftBreak | NodeValue::LineBreak => out.push(' '),
            _ => {}
        }
    }
    out
}

/// Split a fence info string into its language and its settings. The language is the first
/// word, unless that word contains `=` (then there is no language, only settings).
pub fn split_info(info: &str) -> (Option<&str>, &str) {
    let info = info.trim();
    let (first, rest) = info.split_once(char::is_whitespace).unwrap_or((info, ""));
    if first.is_empty() || first.contains('=') { (None, info) } else { (Some(first), rest.trim()) }
}

/// Collapse runs of blank lines to one, outside code blocks.
pub fn collapse_blank_lines(md: &str) -> String {
    let arena = Arena::new();
    let root = comrak::parse_document(&arena, md, &options());
    let code: Vec<(usize, usize)> = root
        .descendants()
        .filter_map(|n| {
            let d = n.data();
            matches!(d.value, NodeValue::CodeBlock(_)).then(|| (d.sourcepos.start.line, d.sourcepos.end.line))
        })
        .collect();
    let mut out = String::with_capacity(md.len());
    let mut blank = false;
    for (i, line) in md.lines().enumerate() {
        let in_code = code.iter().any(|&(s, e)| (s..=e).contains(&(i + 1)));
        if line.trim().is_empty() && !in_code {
            if blank {
                continue;
            }
            blank = true;
        } else {
            blank = false;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// Strip trailing whitespace from every line outside code and HTML blocks, where it is
/// content. comrak leaves it on lines like `> ` and blank lines inside list items; hard
/// breaks are rendered as `\`, so it never carries meaning elsewhere.
pub fn strip_trailing_whitespace(md: &str) -> String {
    let opts = options();
    let arena = Arena::new();
    let root = comrak::parse_document(&arena, md, &opts);
    let keep: Vec<(usize, usize)> = root
        .descendants()
        .filter_map(|n| {
            let d = n.data();
            matches!(d.value, NodeValue::CodeBlock(_) | NodeValue::HtmlBlock(_))
                .then(|| (d.sourcepos.start.line, d.sourcepos.end.line))
        })
        .collect();
    let mut out = String::with_capacity(md.len());
    for (i, line) in md.lines().enumerate() {
        let n = i + 1;
        if keep.iter().any(|&(s, e)| (s..=e).contains(&n)) {
            out.push_str(line);
        } else {
            out.push_str(line.trim_end());
        }
        out.push('\n');
    }
    out
}
