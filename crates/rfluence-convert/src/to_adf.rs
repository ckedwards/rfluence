//! Markdown -> ADF (the upload direction).

use std::collections::HashMap;

use comrak::nodes::{AlertType, AstNode, ListType, NodeValue, TableAlignment};
use comrak::{Arena, parse_document};
use serde_json::{Value, json};

use crate::adf::{self, Mark, Node};
use crate::anchors::Anchors;
use crate::inline::{self, Item, Leaf, MdMark};
use crate::diagnostics::{Diagnostic, Severity};
use crate::markdown::{self, options};
use crate::settings::Settings;
use crate::{approx, emoji, html_table, language};

/// What upload needs to know beyond the markdown.
#[derive(Debug, Clone, Default)]
pub struct UploadContext {
    /// The page ID, for image `collection`s (`contentId-<page id>`).
    pub page_id: Option<String>,
    /// Attachment `fileId`s by image path as written in the markdown (`page.assets/x.png`).
    pub media: HashMap<String, String>,
    /// Custom emoji on the site: name (without colons) -> emoji `id`.
    pub custom_emoji: HashMap<String, String>,
    /// The merfluence app, for turning ```` ```mermaid ```` fences into diagrams. Without
    /// it, Mermaid fences are uploaded as code blocks.
    pub mermaid: Option<MermaidApp>,
}

/// The merfluence Forge app's IDs (design.md, "Mermaid diagrams (merfluence)").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MermaidApp {
    pub app_id: String,
    pub environment_id: String,
}

impl MermaidApp {
    /// Read the IDs from an existing diagram's `extensionKey` (`<app>/<env>/static/<module>`).
    pub fn from_extension_key(key: &str) -> Option<MermaidApp> {
        let mut parts = key.split('/');
        let (app, env) = (parts.next()?, parts.next()?);
        (parts.next() == Some("static")).then(|| MermaidApp { app_id: app.into(), environment_id: env.into() })
    }
}

/// The converted page body, and what couldn't be converted exactly.
#[derive(Debug, Clone)]
pub struct Upload {
    pub doc: Node,
    /// Warnings (uploaded as the closest equivalent) and errors (can't be represented;
    /// the body contains an approximation). See design.md, "Checking markdown".
    pub diagnostics: Vec<Diagnostic>,
}

impl Upload {
    pub fn has_errors(&self) -> bool {
        self.diagnostics.iter().any(|d| d.severity == Severity::Error)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Error {
    /// Local images with no attachment in [`UploadContext::media`].
    UnresolvedImages(Vec<String>),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::UnresolvedImages(paths) => write!(f, "images without an attachment: {}", paths.join(", ")),
        }
    }
}

impl std::error::Error for Error {}

/// Convert markdown (frontmatter is ignored) to a page body.
pub fn markdown_to_adf(md: &str, ctx: &UploadContext) -> Result<Upload, Error> {
    let (doc, diagnostics, unresolved) = convert(md, ctx, false);
    if !unresolved.is_empty() {
        return Err(Error::UnresolvedImages(unresolved));
    }
    Ok(Upload { doc, diagnostics })
}

/// Report what upload would approximate (warnings) or can't represent (errors), without
/// needing the page's attachments or the merfluence app.
pub fn check(md: &str) -> Vec<Diagnostic> {
    let ctx = UploadContext {
        mermaid: Some(MermaidApp { app_id: "check".into(), environment_id: "check".into() }),
        ..Default::default()
    };
    convert(md, &ctx, true).1
}

/// Local image paths in the markdown (images whose source isn't a URL), with their lines.
pub fn local_images(md: &str) -> Vec<(usize, String)> {
    let arena = Arena::new();
    let root = parse_document(&arena, md, &options());
    root.descendants()
        .filter_map(|n| match &n.data().value {
            NodeValue::Image(l) if !l.url.contains("://") => Some((line(n), l.url.clone())),
            _ => None,
        })
        .collect()
}

fn convert(md: &str, ctx: &UploadContext, check_only: bool) -> (Node, Vec<Diagnostic>, Vec<String>) {
    let arena = Arena::new();
    let root = parse_document(&arena, md, &options());
    let mut diags = Vec::new();
    approx::approximate(&arena, root, &mut diags);
    let headings: Vec<String> = root
        .descendants()
        .filter(|n| matches!(n.data().value, NodeValue::Heading(_)))
        .map(|n| markdown::inline_text(n))
        .collect();
    let mut r = Reader {
        arena: &arena,
        ctx,
        check_only,
        anchors: Anchors::new(headings.iter().map(String::as_str)),
        diags,
        unresolved: Vec::new(),
        place: Place::Top,
    };
    let content = r.blocks_of(&children(root));
    let mut doc = Node::doc(content);
    assign_local_ids(&mut doc);
    let mut diags = r.diags;
    diags.sort_by_key(|d| d.line);
    diags.dedup();
    (doc, diags, r.unresolved)
}

/// Where a block is, for what Confluence allows there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Place {
    Top,
    LayoutColumn,
    Expand,
    TableCell,
    /// Inside a list item, quote, panel, nested expand, ...
    Other,
}

struct Reader<'a, 'c> {
    arena: &'a Arena<'a>,
    ctx: &'c UploadContext,
    /// `rfluence check`: images don't need attachments.
    check_only: bool,
    anchors: Anchors,
    diags: Vec<Diagnostic>,
    unresolved: Vec<String>,
    place: Place,
}

fn children<'a>(node: &'a AstNode<'a>) -> Vec<&'a AstNode<'a>> {
    node.children().collect()
}

fn line(node: &AstNode) -> usize {
    node.data().sourcepos.start.line
}

fn is_details_close(literal: &str) -> bool {
    literal.trim().eq_ignore_ascii_case("</details>")
}

/// Column layout markers. (`layout=` is a different setting: tables', images' and cards'
/// Confluence `layout` attribute.)
fn is_layout_marker(s: &Settings) -> bool {
    s.has("columns") || s.has("column") || s.has("end-columns")
}

/// An `rf:` comment that is a block of its own.
fn block_comment(node: &AstNode) -> Option<Settings> {
    match &node.data().value {
        NodeValue::HtmlBlock(h) => Settings::parse_comment(&h.literal),
        _ => None,
    }
}

impl<'a> Reader<'a, '_> {
    fn warn(&mut self, node: &AstNode, message: impl Into<String>) {
        self.diags.push(Diagnostic::warning(line(node), message));
    }

    fn error(&mut self, node: &AstNode, message: impl Into<String>) {
        self.diags.push(Diagnostic::error(line(node), message));
    }

    /// Blocks in `place` (restoring the current place afterwards).
    fn blocks_in(&mut self, nodes: &[&'a AstNode<'a>], place: Place) -> Vec<Node> {
        let saved = std::mem::replace(&mut self.place, place);
        let out = self.blocks_of(nodes);
        self.place = saved;
        out
    }

    fn blocks_of(&mut self, nodes: &[&'a AstNode<'a>]) -> Vec<Node> {
        let mut out = Vec::new();
        let mut i = 0;
        while i < nodes.len() {
            let node = nodes[i];
            let value = node.data().value.clone();
            match value {
                NodeValue::Table(_) => {
                    // A table's settings are in the `rf:` comment right after it.
                    let settings = nodes.get(i + 1).and_then(|n| block_comment(n)).filter(|s| !is_layout_marker(s));
                    if settings.is_some() {
                        i += 1;
                    }
                    out.push(self.table(node, settings.unwrap_or_default()));
                }
                NodeValue::HtmlBlock(h) => {
                    let literal = h.literal.trim();
                    if literal == "<!-- end list -->" {
                        // comrak's separator between adjacent lists.
                    } else if html_table::is_table_start(literal) {
                        if let Some((table, end)) = html_table::parse(self.arena, nodes, i, &mut self.diags) {
                            let mut settings = table.settings.clone();
                            i = end;
                            if settings.is_none() {
                                settings = nodes.get(i + 1).and_then(|n| block_comment(n)).filter(|s| !is_layout_marker(s));
                                if settings.is_some() {
                                    i += 1;
                                }
                            }
                            out.push(self.html_table(node, table, settings.unwrap_or_default()));
                        }
                    } else if approx::is_details_open(literal) {
                        let end = matching_details(nodes, i);
                        if end.is_none() {
                            self.error(node, "`<details>` without a matching `</details>`");
                        }
                        let j = end.unwrap_or(nodes.len());
                        out.extend(self.expand(node, literal, &nodes[i + 1..j]));
                        i = j;
                    } else if is_details_close(literal) {
                        self.error(node, "`</details>` without a matching `<details>`");
                    } else if let Some(settings) = Settings::parse_comment(literal) {
                        if settings.has("columns") {
                            let (j, closed) = layout_end(nodes, i);
                            if !closed {
                                self.error(node, "layout without `<!-- rf: end-columns -->`");
                            }
                            out.push(self.layout(node, &settings, &nodes[i + 1..j]));
                            // Without an end marker, the next layout marker starts the next layout.
                            i = if closed { j } else { j - 1 };
                        } else if settings.has("column") || settings.has("end-columns") {
                            self.error(node, "`rf:` layout marker outside a layout");
                        } else {
                            self.warn(node, "`rf:` comment not attached to a table; ignored");
                        }
                    } else {
                        self.error(node, "raw HTML block can't be represented in Confluence (uploaded as an html code block)");
                        out.push(code_block(Some("html".into()), literal, vec![]));
                    }
                }
                _ => out.extend(self.block(node)),
            }
            i += 1;
        }
        out
    }

    /// `<details><summary>Title</summary>` ... `</details>` -> an expand, or a nested
    /// expand inside an expand.
    fn expand(&mut self, node: &AstNode, literal: &str, body: &[&'a AstNode<'a>]) -> Vec<Node> {
        let title = approx::details_title(literal).unwrap_or_default();
        let (kind, inner) = match self.place {
            Place::Top | Place::LayoutColumn => ("expand", Place::Expand),
            Place::Expand | Place::TableCell => ("nestedExpand", Place::Other),
            Place::Other => {
                self.error(
                    node,
                    "`<details>` can't be an expand here (Confluence allows expands at the top level, in layout columns and one level inside another expand)",
                );
                return self.blocks_in(body, Place::Other);
            }
        };
        let mut content = self.blocks_in(body, inner);
        if content.is_empty() {
            content.push(Node::new("paragraph"));
        }
        vec![Node::new(kind).with_attr("title", title).with_content(content)]
    }

    /// `<!-- rf: columns=50,50 -->` col `<!-- rf: column -->` col `<!-- rf: end-columns -->`
    fn layout(&mut self, node: &AstNode, settings: &Settings, body: &[&'a AstNode<'a>]) -> Node {
        if self.place != Place::Top {
            self.error(node, "layouts are only allowed at the top level");
        }
        let mut columns: Vec<Vec<&'a AstNode<'a>>> = vec![vec![]];
        for &n in body {
            if block_comment(n).is_some_and(|s| s.has("column")) {
                columns.push(vec![]);
            } else {
                columns.last_mut().expect("never empty").push(n);
            }
        }
        let mut widths: Vec<f64> = settings
            .get("columns")
            .map(|w| w.split(',').filter_map(|x| x.trim().parse().ok()).collect())
            .unwrap_or_default();
        if widths.len() != columns.len() {
            self.error(
                node,
                format!("layout has {} columns but `columns=` lists {} widths", columns.len(), widths.len()),
            );
            let w = (10000.0 / columns.len() as f64).round() / 100.0;
            widths = vec![w; columns.len()];
        }
        for key in settings.keys() {
            if !matches!(key, "columns" | "breakout" | "width") {
                self.warn(node, format!("unknown layout setting `{key}` ignored"));
            }
        }
        let mut section = Node::new("layoutSection");
        section.marks.extend(self.breakout(node, settings));
        for (column, width) in columns.iter().zip(widths) {
            let mut content = self.blocks_in(column, Place::LayoutColumn);
            if content.is_empty() {
                content.push(Node::new("paragraph"));
            }
            section.content.push(Node::new("layoutColumn").with_attr("width", width).with_content(content));
        }
        section
    }

    /// A `breakout` mark from `breakout=` / `width=` settings.
    fn breakout(&mut self, node: &AstNode, settings: &Settings) -> Option<Mark> {
        let width = settings.get_f64("width");
        let mode = match (settings.get("breakout"), width) {
            (None, None) => return None,
            (Some("full-width"), _) => "full-width",
            (Some("wide") | None, _) => "wide",
            (Some(other), _) => {
                self.warn(node, format!("unknown breakout `{other}` ignored"));
                return None;
            }
        };
        let mut mark = Mark::new("breakout").with_attr("mode", mode);
        if let Some(w) = width {
            mark = mark.with_attr("width", w);
        }
        Some(mark)
    }

    fn block(&mut self, node: &'a AstNode<'a>) -> Vec<Node> {
        let value = node.data().value.clone();
        match value {
            NodeValue::FrontMatter(_) => vec![],
            NodeValue::Paragraph => vec![self.paragraph(node)],
            NodeValue::Heading(h) => {
                let (settings, inlines) = trailing_comment(node);
                let mut heading = Node::new("heading")
                    .with_attr("level", h.level)
                    .with_content(self.inlines(&inlines, &[]));
                heading.marks = self.block_marks(node, &settings);
                vec![heading]
            }
            NodeValue::CodeBlock(cb) => self.code(node, &cb.info, &cb.literal),
            NodeValue::BlockQuote => {
                let mut content = Vec::new();
                for child in self.blocks_in(&children(node), Place::Other) {
                    // ADF quotes can't nest; a nested quote's content joins the outer one.
                    if child.is("blockquote") {
                        self.error(node, "nested quote can't be represented in Confluence (flattened)");
                        content.extend(child.content);
                    } else if child.is("panel") {
                        self.error(node, "alert in a quote can't be represented in Confluence (flattened)");
                        content.extend(child.content);
                    } else {
                        content.push(child);
                    }
                }
                vec![Node::new("blockquote").with_content(content)]
            }
            NodeValue::ThematicBreak => vec![Node::new("rule")],
            NodeValue::List(l) => self.list(node, l.list_type, l.start),
            NodeValue::Alert(a) => {
                let panel_type = match a.alert_type {
                    AlertType::Note => "info",
                    AlertType::Important => "note",
                    AlertType::Tip => "success",
                    AlertType::Warning => "warning",
                    AlertType::Caution => "error",
                };
                let content = self.blocks_in(&children(node), Place::Other);
                vec![Node::new("panel").with_attr("panelType", panel_type).with_content(content)]
            }
            NodeValue::FootnoteDefinition(f) => {
                self.error(node, "footnotes can't be represented in Confluence (definition uploaded as a paragraph)");
                let mut content = self.blocks_in(&children(node), Place::Other);
                let label = Node::text(format!("[^{}]: ", f.name));
                match content.first_mut() {
                    Some(p) if p.is("paragraph") => p.content.insert(0, label),
                    _ => content.insert(0, Node::new("paragraph").with_content(vec![label])),
                }
                content
            }
            other => {
                self.error(node, format!("unsupported markdown ({other:?}) uploaded as text"));
                vec![Node::new("paragraph").with_content(vec![Node::text(markdown::inline_text(node))])]
            }
        }
    }

    fn block_marks(&mut self, node: &AstNode, settings: &Settings) -> Vec<Mark> {
        let mut marks = Vec::new();
        for key in settings.keys() {
            match key {
                "align" => marks.push(Mark::new("alignment").with_attr("align", settings.get("align").unwrap_or(""))),
                "indent" => marks.push(Mark::new("indentation").with_attr("level", settings.get_f64("indent").unwrap_or(1.0))),
                other => self.warn(node, format!("unknown setting `{other}` ignored")),
            }
        }
        marks
    }

    fn paragraph(&mut self, node: &'a AstNode<'a>) -> Node {
        let (settings, inlines) = trailing_comment(node);
        if let Some(media) = self.image_paragraph(&inlines, &settings) {
            return media;
        }
        if settings.has("card") {
            if let [link] = inlines.as_slice() {
                if let NodeValue::Link(l) = &link.data().value {
                    let card = match settings.get("card") {
                        Some("embed") => {
                            let mut c = Node::new("embedCard").with_attr("url", l.url.clone());
                            if let Some(layout) = settings.get("layout") {
                                c = c.with_attr("layout", layout);
                            }
                            if let Some(width) = settings.get_f64("width") {
                                c = c.with_attr("width", width);
                            }
                            c
                        }
                        _ => Node::new("blockCard").with_attr("url", l.url.clone()),
                    };
                    return card;
                }
            }
        }
        let mut p = Node::new("paragraph").with_content(self.inlines(&inlines, &[]));
        p.marks = self.block_marks(node, &settings);
        p
    }

    /// A paragraph that is only an image (optionally linked) becomes a `mediaSingle`.
    fn image_paragraph(&mut self, inlines: &[&'a AstNode<'a>], settings: &Settings) -> Option<Node> {
        let (img, href) = match inlines {
            [n] => match &n.data().value {
                NodeValue::Image(_) => (*n, None),
                NodeValue::Link(l) => {
                    let kids = children(n);
                    match kids.as_slice() {
                        [i] if matches!(i.data().value, NodeValue::Image(_)) => (*i, Some(l.url.clone())),
                        _ => return None,
                    }
                }
                _ => return None,
            },
            _ => return None,
        };
        let NodeValue::Image(link) = img.data().value.clone() else { return None };
        let alt = markdown::inline_text(img);
        let mut media = if link.url.contains("://") {
            Node::new("media").with_attr("type", "external").with_attr("url", link.url.clone())
        } else {
            let path = percent_encoding::percent_decode_str(&link.url).decode_utf8_lossy().into_owned();
            let mut id = self.ctx.media.get(&link.url).or_else(|| self.ctx.media.get(&path)).cloned();
            if self.check_only {
                id.get_or_insert_with(String::new);
            }
            if id.is_none() {
                self.unresolved.push(link.url.clone());
            }
            let collection = self.ctx.page_id.as_ref().map(|p| format!("contentId-{p}")).unwrap_or_default();
            Node::new("media")
                .with_attr("type", "file")
                .with_attr("id", id.unwrap_or_default())
                .with_attr("collection", collection)
        };
        if !alt.is_empty() {
            media = media.with_attr("alt", alt);
        }
        if let Some(size) = settings.get_f64("border") {
            let mut border = Mark::new("border").with_attr("size", size);
            if let Some(color) = settings.get("border-color") {
                border = border.with_attr("color", color);
            }
            media.marks.push(border);
        }
        if let Some(href) = href {
            media.marks.push(Mark::new("link").with_attr("href", href));
        }
        let mut single = Node::new("mediaSingle");
        if let Some(layout) = settings.get("layout") {
            single = single.with_attr("layout", layout);
        }
        if let Some(width) = settings.get_f64("width") {
            single = single.with_attr("width", width).with_attr("widthType", settings.get("width-type").unwrap_or("pixel"));
        }
        single.content.push(media);
        if let Some(caption) = settings.get("caption") {
            single.content.push(Node::new("caption").with_content(vec![Node::text(caption)]));
        }
        for key in settings.keys() {
            if !matches!(key, "layout" | "width" | "width-type" | "border" | "border-color" | "caption") {
                self.warn(img, format!("unknown image setting `{key}` ignored"));
            }
        }
        Some(single)
    }

    fn code(&mut self, node: &AstNode, info: &str, literal: &str) -> Vec<Node> {
        let (lang, rest) = markdown::split_info(info);
        let settings = Settings::parse(rest);
        // Fence content ends with a newline; Confluence's text doesn't.
        let text = literal.strip_suffix('\n').unwrap_or(literal);
        match lang {
            Some("adf") => match parse_adf_block(text) {
                Ok(nodes) => nodes,
                Err(message) => {
                    self.error(node, format!("invalid ```adf block: {message}"));
                    vec![]
                }
            },
            Some("mermaid") if self.ctx.mermaid.is_some() => {
                vec![merfluence(self.ctx.mermaid.as_ref().expect("checked"), text, &settings)]
            }
            _ => {
                let marks: Vec<Mark> = self.breakout(node, &settings).into_iter().collect();
                for key in settings.keys() {
                    if !matches!(key, "breakout" | "width") {
                        self.warn(node, format!("unknown code block setting `{key}` ignored"));
                    }
                }
                vec![code_block(lang.and_then(language::canonical), text, marks)]
            }
        }
    }

    fn list(&mut self, node: &'a AstNode<'a>, list_type: ListType, start: usize) -> Vec<Node> {
        let items = children(node);
        if items.iter().any(|i| matches!(i.data().value, NodeValue::TaskItem(_))) {
            return vec![self.task_list(node)];
        }
        let kind = if list_type == ListType::Ordered { "orderedList" } else { "bulletList" };
        let mut list = Node::new(kind);
        if list_type == ListType::Ordered && start != 1 {
            list = list.with_attr("order", start);
        }
        for item in items {
            let mut content = self.blocks_in(&children(item), Place::Other);
            if content.is_empty() {
                content.push(Node::new("paragraph"));
            }
            list.content.push(Node::new("listItem").with_content(content));
        }
        vec![list]
    }

    fn task_list(&mut self, node: &'a AstNode<'a>) -> Node {
        let mut list = Node::new("taskList");
        for item in children(node) {
            let done = matches!(&item.data().value, NodeValue::TaskItem(t) if t.symbol.is_some());
            if !matches!(item.data().value, NodeValue::TaskItem(_)) {
                self.warn(item, "list item without a checkbox in a task list uploaded as a task");
            }
            let mut task = Node::new("taskItem").with_attr("state", if done { "DONE" } else { "TODO" });
            let mut nested = Vec::new();
            for (i, child) in children(item).into_iter().enumerate() {
                let value = child.data().value.clone();
                match value {
                    NodeValue::Paragraph if i == 0 => task.content = self.inlines(&children(child), &[]),
                    // A nested task list follows its parent item in ADF.
                    NodeValue::List(_) if children(child).iter().any(|c| matches!(c.data().value, NodeValue::TaskItem(_))) => {
                        nested.push(self.task_list(child))
                    }
                    _ => self.error(child, "only text and nested task lists can be in a task in Confluence (dropped)"),
                }
            }
            list.content.push(task);
            list.content.extend(nested);
        }
        list
    }

    /// A table node with the settings shared by GFM and HTML tables, and the column widths.
    fn table_node(&mut self, node: &AstNode, settings: &Settings, extra_keys: &[&str]) -> (Node, Vec<Option<f64>>) {
        if self.place == Place::TableCell {
            self.error(node, "table inside a table cell can't be represented in Confluence");
        }
        let widths: Vec<Option<f64>> = match settings.get("colwidths") {
            Some(list) => list.split(',').map(|w| w.trim().parse().ok()).collect(),
            None => vec![],
        };
        let mut table = Node::new("table");
        if settings.has("numbered") {
            table = table.with_attr("isNumberColumnEnabled", true);
        }
        if let Some(layout) = settings.get("layout") {
            table = table.with_attr("layout", layout);
        }
        if let Some(width) = settings.get_f64("width") {
            table = table.with_attr("width", width);
        }
        for key in settings.keys() {
            if !matches!(key, "colwidths" | "numbered" | "layout" | "width") && !extra_keys.contains(&key) {
                self.warn(node, format!("unknown table setting `{key}` ignored"));
            }
        }
        (table, widths)
    }

    fn html_table(&mut self, node: &AstNode, html: html_table::Table<'a>, settings: Settings) -> Node {
        let (mut table, widths) = self.table_node(node, &settings, &[]);
        let spans: Vec<Vec<(usize, usize)>> =
            html.rows.iter().map(|r| r.iter().map(|c| (c.colspan, c.rowspan)).collect()).collect();
        let grid = html_table::grid_columns(&spans);
        for (row, cols) in html.rows.iter().zip(&grid) {
            let mut tr = Node::new("tableRow");
            for (cell, &col) in row.iter().zip(cols) {
                let mut td = Node::new(if cell.header { "tableHeader" } else { "tableCell" });
                if cell.colspan > 1 {
                    td = td.with_attr("colspan", cell.colspan);
                }
                if cell.rowspan > 1 {
                    td = td.with_attr("rowspan", cell.rowspan);
                }
                if let Some(bg) = &cell.background {
                    td = td.with_attr("background", bg.clone());
                }
                let from_table: Option<Vec<f64>> = (col..col + cell.colspan).map(|c| widths.get(c).copied().flatten()).collect();
                if let Some(w) = cell.colwidth.clone().or(from_table) {
                    td = td.with_attr("colwidth", json!(w));
                }
                let mut content = self.blocks_in(&cell.content, Place::TableCell);
                if content.is_empty() {
                    content.push(Node::new("paragraph"));
                }
                tr.content.push(td.with_content(content));
            }
            table.content.push(tr);
        }
        table
    }

    fn table(&mut self, node: &'a AstNode<'a>, settings: Settings) -> Node {
        let NodeValue::Table(t) = node.data().value.clone() else { unreachable!() };
        let (mut table, widths) = self.table_node(node, &settings, &["no-header-row", "header-column"]);
        let header_row = !settings.has("no-header-row");
        let header_column = settings.has("header-column");
        for (r, row) in children(node).into_iter().enumerate() {
            let mut tr = Node::new("tableRow");
            for (c, cell) in children(row).into_iter().enumerate() {
                let header = (r == 0 && header_row) || (c == 0 && header_column);
                let mut p = Node::new("paragraph").with_content(self.inlines(&children(cell), &[]));
                match t.alignments.get(c) {
                    Some(TableAlignment::Center) => p.marks.push(Mark::new("alignment").with_attr("align", "center")),
                    Some(TableAlignment::Right) => p.marks.push(Mark::new("alignment").with_attr("align", "end")),
                    _ => {}
                }
                let mut td = Node::new(if header { "tableHeader" } else { "tableCell" }).with_content(vec![p]);
                if let Some(Some(w)) = widths.get(c) {
                    td = td.with_attr("colwidth", json!([w]));
                }
                tr.content.push(td);
            }
            table.content.push(tr);
        }
        table
    }

    /// Inline nodes (a paragraph's children) as ADF.
    fn inlines(&mut self, nodes: &[&'a AstNode<'a>], marks: &[MdMark]) -> Vec<Node> {
        let at = nodes.first().map(|n| line(n)).unwrap_or(0);
        let items = inline::flatten(nodes, marks);
        let mut out = Vec::new();
        let mut i = 0;
        while i < items.len() {
            let item = &items[i];
            let marks = self.adf_marks(&item.marks);
            match &item.leaf {
                Leaf::Text(text) => {
                    if let Some(card) = autolink(&items, i) {
                        out.push(Node::new("inlineCard").with_attr("url", card));
                    } else {
                        out.extend(self.text_with_emoji(text, &marks));
                    }
                }
                Leaf::Code(c) => {
                    let mut marks = marks;
                    marks.push(Mark::new("code"));
                    out.push(Node::text(c.clone()).with_marks(sorted(marks)));
                }
                Leaf::LineBreak => out.push(Node::new("hardBreak")),
                Leaf::SoftBreak => out.push(Node::text(" ").with_marks(marks)),
                Leaf::Html(h) => {
                    if let Some(node) = adf_span(h) {
                        // Skip the span's visible text, up to its closing tag.
                        let mut depth = 1;
                        while depth > 0 && i + 1 < items.len() {
                            i += 1;
                            if let Leaf::Html(h) = &items[i].leaf {
                                if h.trim_start().starts_with("<span") {
                                    depth += 1;
                                } else if h.trim() == "</span>" {
                                    depth -= 1;
                                }
                            }
                        }
                        match node {
                            Ok(n) => out.push(n),
                            Err(e) => self.diags.push(Diagnostic::error(at, format!("invalid data-adf span ({e}); dropped"))),
                        }
                    } else if matches!(h.trim().to_ascii_lowercase().as_str(), "<br>" | "<br/>" | "<br />") {
                        out.push(Node::new("hardBreak"));
                    } else if Settings::parse_comment(h).is_none() {
                        self.diags.push(Diagnostic::warning(at, format!("inline HTML `{h}` dropped")));
                    }
                }
                Leaf::Other(node) => out.extend(self.other_inline(node, marks)),
            }
            i += 1;
        }
        adf::merge_text_nodes(&mut out);
        out
    }

    fn other_inline(&mut self, node: &'a AstNode<'a>, marks: Vec<Mark>) -> Vec<Node> {
        let value = node.data().value.clone();
        match value {
            NodeValue::Image(l) => {
                self.error(node, "inline images can't be represented in Confluence (uploaded as a link)");
                let mut marks = marks;
                marks.push(Mark::new("link").with_attr("href", l.url.clone()));
                let alt = markdown::inline_text(node);
                vec![Node::text(if alt.is_empty() { l.url.clone() } else { alt }).with_marks(sorted(marks))]
            }
            NodeValue::FootnoteReference(f) => {
                self.error(node, "footnotes can't be represented in Confluence (reference uploaded as text)");
                vec![Node::text(format!("[^{}]", f.name)).with_marks(marks)]
            }
            other => {
                self.error(node, format!("unsupported inline markdown ({other:?}) uploaded as text"));
                vec![Node::text(markdown::inline_text(node)).with_marks(marks)]
            }
        }
    }

    /// Text, with emoji (characters and shortcodes) as emoji nodes (design.md, "Emoji").
    fn text_with_emoji(&self, text: &str, marks: &[Mark]) -> Vec<Node> {
        let standard = |e: emoji::Emoji| {
            Node::new("emoji")
                .with_attr("shortName", emoji::short_name(e))
                .with_attr("id", emoji::id(e))
                .with_attr("text", e.as_str())
        };
        let mut hits: Vec<(std::ops::Range<usize>, Node)> = emoji::find_emoji(text)
            .into_iter()
            .map(|(range, e)| (range, standard(e)))
            .collect();
        for sc in emoji::find_shortcodes(text) {
            let node = if let Some(e) = emoji::lookup(sc.name) {
                standard(e)
            } else if let Some(id) = self.ctx.custom_emoji.get(sc.name) {
                let short = format!(":{}:", sc.name);
                Node::new("emoji").with_attr("shortName", short.clone()).with_attr("id", id.clone()).with_attr("text", short)
            } else {
                continue;
            };
            hits.push((sc.range, node));
        }
        hits.sort_by_key(|(r, _)| r.start);
        let mut out = Vec::new();
        let mut pos = 0;
        for (range, node) in hits {
            if range.start < pos {
                continue;
            }
            if range.start > pos {
                out.push(Node::text(&text[pos..range.start]).with_marks(marks.to_vec()));
            }
            out.push(node);
            pos = range.end;
        }
        if pos < text.len() {
            out.push(Node::text(&text[pos..]).with_marks(marks.to_vec()));
        }
        out
    }

    fn adf_marks(&self, marks: &[MdMark]) -> Vec<Mark> {
        let out = marks
            .iter()
            .map(|m| match m {
                MdMark::Link { url, title } => {
                    let href = match url.strip_prefix('#') {
                        Some(anchor) => self.anchors.to_confluence(anchor).map(|c| format!("#{c}")).unwrap_or_else(|| url.clone()),
                        None => url.clone(),
                    };
                    let mut mark = Mark::new("link").with_attr("href", href);
                    if !title.is_empty() {
                        mark = mark.with_attr("title", title.clone());
                    }
                    mark
                }
                MdMark::Strong => Mark::new("strong"),
                MdMark::Emph => Mark::new("em"),
                MdMark::Strike => Mark::new("strike"),
                MdMark::Underline => Mark::new("underline"),
                MdMark::Sub => Mark::new("subsup").with_attr("type", "sub"),
                MdMark::Sup => Mark::new("subsup").with_attr("type", "sup"),
                MdMark::Color(c) => Mark::new("textColor").with_attr("color", c.clone()),
                MdMark::Background(c) => Mark::new("backgroundColor").with_attr("color", c.clone()),
            })
            .collect();
        sorted(out)
    }
}

/// The index of the `</details>` closing the `<details>` at `open`.
fn matching_details(nodes: &[&AstNode], open: usize) -> Option<usize> {
    let mut depth = 0;
    for (j, n) in nodes.iter().enumerate().skip(open + 1) {
        if let NodeValue::HtmlBlock(h) = &n.data().value {
            if approx::is_details_open(&h.literal) {
                depth += 1;
            } else if is_details_close(&h.literal) {
                if depth == 0 {
                    return Some(j);
                }
                depth -= 1;
            }
        }
    }
    None
}

/// Where the layout at `start` ends: the index of its `end-columns` marker (`true`), or of
/// the next layout marker or the end of the blocks (`false`).
fn layout_end(nodes: &[&AstNode], start: usize) -> (usize, bool) {
    for (j, n) in nodes.iter().enumerate().skip(start + 1) {
        match block_comment(n) {
            Some(s) if s.has("end-columns") => return (j, true),
            Some(s) if s.has("columns") => return (j, false),
            _ => {}
        }
    }
    (nodes.len(), false)
}

/// An autolink (`<url>`, or a link whose text is its URL) is an `inlineCard`: the item's
/// text equals the URL of a link mark that no neighbouring item shares.
fn autolink(items: &[Item], i: usize) -> Option<String> {
    let Leaf::Text(text) = &items[i].leaf else { return None };
    let url = items[i].marks.iter().find_map(|m| match m {
        MdMark::Link { url, title } if url == text && title.is_empty() => Some(url.clone()),
        _ => None,
    })?;
    let link = MdMark::Link { url: url.clone(), title: String::new() };
    let shares = |j: usize| items.get(j).is_some_and(|it| it.marks.contains(&link));
    (!(i > 0 && shares(i - 1)) && !shares(i + 1)).then_some(url)
}

/// `<span data-adf='{json}'>`: the inline node it holds, or the parse error.
fn adf_span(html: &str) -> Option<Result<Node, String>> {
    let rest = html.trim().strip_prefix("<span data-adf='")?;
    let json = rest.strip_suffix("'>")?;
    Some(serde_json::from_str(json).map_err(|e| e.to_string()))
}

/// Split off a trailing `rf:` comment (block settings) from a paragraph or heading.
fn trailing_comment<'a>(node: &'a AstNode<'a>) -> (Settings, Vec<&'a AstNode<'a>>) {
    let mut kids = children(node);
    if let Some(last) = kids.last() {
        if let NodeValue::HtmlInline(h) = &last.data().value {
            if let Some(settings) = Settings::parse_comment(h) {
                kids.pop();
                return (settings, kids);
            }
        }
    }
    (Settings::new(), kids)
}

fn code_block(language: Option<String>, text: &str, marks: Vec<Mark>) -> Node {
    let mut node = Node::new("codeBlock").with_marks(marks);
    if let Some(lang) = language {
        node = node.with_attr("language", lang);
    }
    if !text.is_empty() {
        node.content.push(Node::text(text));
    }
    node
}

/// The node(s) in an ```` ```adf ```` fence: one JSON object, or an array of them.
fn parse_adf_block(text: &str) -> Result<Vec<Node>, String> {
    let value: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let nodes = match value {
        Value::Array(_) => serde_json::from_value(value),
        other => serde_json::from_value(other).map(|n| vec![n]),
    };
    let mut nodes: Vec<Node> = nodes.map_err(|e| e.to_string())?;
    for node in &mut nodes {
        // Page-specific macro data must not point back at the page it was fetched from.
        adf::strip_noise(node);
    }
    Ok(nodes)
}

/// A minimal merfluence node: the source plus the app IDs, no cached SVGs
/// (design.md, "Mermaid diagrams (merfluence)").
fn merfluence(app: &MermaidApp, source: &str, settings: &Settings) -> Node {
    let key = format!("{}/{}/static/mermaid-diagram", app.app_id, app.environment_id);
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
    Node::new("extension")
        .with_attr("layout", "default")
        .with_attr("extensionType", "com.atlassian.ecosystem")
        .with_attr("extensionKey", key.clone())
        .with_attr("text", "Merfluence")
        .with_attr(
            "parameters",
            json!({
                "layout": "extension",
                "guestParams": guest,
                "forgeEnvironment": "PRODUCTION",
                "extensionId": format!("ari:cloud:ecosystem::extension/{key}"),
                "extensionTitle": "Merfluence",
            }),
        )
}

fn sorted(mut marks: Vec<Mark>) -> Vec<Mark> {
    marks.sort_by(|a, b| a.kind.cmp(&b.kind));
    marks
}

/// Give task and decision nodes the `localId`s ADF requires. Deterministic, so the same
/// markdown always produces the same ADF.
fn assign_local_ids(doc: &mut Node) {
    let mut next = 0u64;
    doc.walk_mut(&mut |n| {
        if matches!(n.kind.as_str(), "taskList" | "taskItem" | "decisionList" | "decisionItem") && n.attr("localId").is_none() {
            next += 1;
            n.attrs.insert("localId".into(), format!("{next:012x}").into());
        }
    });
}
