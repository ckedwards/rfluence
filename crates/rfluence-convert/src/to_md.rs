//! ADF -> markdown (the fetch direction).
//!
//! Builds a comrak AST and renders it, so escaping is comrak's and the output is already in
//! normalized form. Anything without a markdown form is kept losslessly: block nodes as
//! ```` ```adf ```` fences, inline nodes as `<span data-adf='...'>` (design.md, "Content
//! Confluence has that markdown doesn't").

use std::collections::HashMap;

use comrak::nodes::{
    AlertType, AstNode, ListDelimType, ListType, NodeAlert, NodeCodeBlock, NodeHeading, NodeHtmlBlock, NodeLink,
    NodeList, NodeTable, NodeTaskItem, NodeValue, TableAlignment,
};
use comrak::Arena;
use serde_json::Value;

use crate::adf::{self, Mark, Node};
use crate::anchors::Anchors;
use crate::inline::{self, Item, Leaf, MdMark};
use crate::markdown::append;
use crate::settings::{Settings, fmt_num};
use crate::{emoji, normalize};

/// What fetch needs to know beyond the page body.
#[derive(Debug, Clone, Default)]
pub struct FetchContext {
    /// The page ID; images whose `collection` is another page's are kept as ```` ```adf ````.
    pub page_id: Option<String>,
    /// Directory images are written to, relative to the markdown file (e.g. `page.assets`).
    pub assets_dir: String,
    /// Attachment file names by `fileId`, from the attachments API. Falls back to the
    /// `__fileName` attribute that API-created images have.
    pub attachments: HashMap<String, String>,
    /// Simplified output for reading, not round trips (design.md, "Simplified output").
    pub simplified: bool,
    /// The site's host (`example.atlassian.net`), to recognise links to its pages.
    pub site_host: Option<String>,
    /// Pages in the local project, by page ID: text links to them are written as relative
    /// paths (`rfluence fetch -o`; design.md, "Links").
    pub links: HashMap<String, LinkTarget>,
}

/// A local markdown file for a page.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LinkTarget {
    /// Relative to the markdown file being written (`./setup.md`, `../api/auth.md`).
    pub path: String,
    /// The file's headings, to translate anchors to GitHub style.
    pub headings: Vec<String>,
}

/// Convert a page body to markdown.
pub fn adf_to_markdown(doc: &Node, ctx: &FetchContext) -> String {
    let arena = Arena::new();
    let root = crate::markdown::node(&arena, NodeValue::Document);
    let mut headings = Vec::new();
    doc.walk(&mut |n| {
        if n.is("heading") {
            headings.push(n.plain_text());
        }
    });
    let w = Writer { arena: &arena, ctx, anchors: Anchors::new(headings.iter().map(String::as_str)) };
    w.blocks(root, &doc.content);
    if ctx.simplified {
        // Never compared in a round trip, so no normalize pass (it would escape the
        // simplified markers, e.g. `[IN PROGRESS]` as `\[IN PROGRESS\]`).
        crate::markdown::collapse_blank_lines(&crate::markdown::strip_trailing_whitespace(&crate::markdown::render(root)))
    } else {
        normalize(&crate::markdown::render(root))
    }
}

struct Writer<'a, 'c> {
    arena: &'a Arena<'a>,
    ctx: &'c FetchContext,
    anchors: Anchors,
}

impl<'a> Writer<'a, '_> {
    fn blocks(&self, parent: &'a AstNode<'a>, nodes: &[Node]) {
        for node in nodes {
            self.block(parent, node);
        }
    }

    fn block(&self, parent: &'a AstNode<'a>, node: &Node) {
        let converted = match node.kind.as_str() {
            "paragraph" => self.paragraph(parent, node),
            "heading" => self.heading(parent, node),
            "codeBlock" => self.code_block(parent, node),
            "blockquote" if node.marks.is_empty() => {
                let quote = append(self.arena, parent, NodeValue::BlockQuote);
                self.blocks(quote, &node.content);
                true
            }
            "rule" if node.marks.is_empty() => {
                append(self.arena, parent, NodeValue::ThematicBreak);
                true
            }
            "bulletList" | "orderedList" => self.list(parent, node),
            "taskList" => self.task_list(parent, node),
            "panel" => self.panel(parent, node),
            "mediaSingle" => self.media_single(parent, node),
            "blockCard" | "embedCard" => self.card(parent, node),
            "extension" if is_merfluence(node) => self.mermaid(parent, node),
            "table" => self.table(parent, node),
            "expand" | "nestedExpand" => self.expand(parent, node),
            "layoutSection" => self.layout(parent, node),
            _ => false,
        };
        if !converted {
            if self.simple() {
                self.simplified_block(parent, node);
            } else {
                self.adf_block(parent, node);
            }
        }
    }

    fn simple(&self) -> bool {
        self.ctx.simplified
    }

    /// Text written as is (not escaped), for simplified output's markers.
    fn marker(&self, parent: &'a AstNode<'a>, text: &str) {
        let p = append(self.arena, parent, NodeValue::Paragraph);
        append(self.arena, p, NodeValue::HtmlInline(text.to_string()));
    }

    /// Simplified output for a node without a markdown form: its text, or a short marker.
    fn simplified_block(&self, parent: &'a AstNode<'a>, node: &Node) {
        match node.kind.as_str() {
            "extension" | "bodiedExtension" => {
                if !node.content.is_empty() {
                    self.blocks(parent, &node.content);
                    return;
                }
                let params = node.attrs.get("parameters");
                let title = params
                    .and_then(|p| p.pointer("/macroMetadata/title"))
                    .or_else(|| params.and_then(|p| p.get("extensionTitle")))
                    .and_then(Value::as_str)
                    .or_else(|| node.attr_str("text"))
                    .or_else(|| node.attr_str("extensionKey"))
                    .unwrap_or("macro");
                let title = match node.attr_str("extensionKey") {
                    Some("toc") => "Table of contents",
                    Some("children") => "Child pages",
                    _ => title,
                };
                self.marker(parent, &format!("[{title}]"));
            }
            "decisionList" => {
                // As a list of the decisions.
                let items = node
                    .content
                    .iter()
                    .map(|d| Node::new("listItem").with_content(vec![Node::new("paragraph").with_content(d.content.clone())]))
                    .collect();
                self.list(parent, &Node::new("bulletList").with_content(items));
            }
            "panel" => {
                let quote = append(self.arena, parent, NodeValue::BlockQuote);
                self.blocks(quote, &node.content);
            }
            "mediaGroup" => {
                for media in &node.content {
                    let name = media
                        .attr_str("id")
                        .and_then(|id| self.ctx.attachments.get(id).map(String::as_str))
                        .or_else(|| media.attr_str("__fileName"))
                        .unwrap_or("attachment");
                    self.marker(parent, &format!("[file: {name}]"));
                }
            }
            _ if node.content.iter().any(|c| !c.is("text")) => self.blocks(parent, &node.content),
            _ => {
                let text = node.plain_text();
                if !text.trim().is_empty() {
                    let p = append(self.arena, parent, NodeValue::Paragraph);
                    append(self.arena, p, NodeValue::Text(text.into()));
                }
            }
        }
    }

    /// Keep a node losslessly as an ```` ```adf ```` fence, without save noise.
    fn adf_block(&self, parent: &'a AstNode<'a>, node: &Node) {
        let mut node = node.clone();
        adf::strip_noise(&mut node);
        strip_annotations(&mut node);
        // Compact, one node per fence: pretty-printed JSON would flood an LLM's context.
        self.code(parent, "adf".into(), adf::to_compact_json(&node) + "\n");
    }

    fn code(&self, parent: &'a AstNode<'a>, info: String, literal: String) {
        append(
            self.arena,
            parent,
            NodeValue::CodeBlock(Box::new(NodeCodeBlock {
                fenced: true,
                fence_char: b'`',
                fence_length: 3,
                fence_offset: 0,
                info,
                literal,
                closed: true,
            })),
        );
    }

    /// Block-level settings from `alignment` / `indentation` marks; `None` if the node has
    /// other marks.
    fn block_settings(node: &Node) -> Option<Settings> {
        let mut settings = Settings::new();
        for mark in &node.marks {
            match mark.kind.as_str() {
                "alignment" => settings.set("align", mark.attr_str("align")?),
                "indentation" => settings.set("indent", fmt_num(mark.attr_f64("level")?)),
                _ => return None,
            }
        }
        Some(settings)
    }

    fn paragraph(&self, parent: &'a AstNode<'a>, node: &Node) -> bool {
        let settings = match Self::block_settings(node) {
            Some(_) if self.simple() => Settings::new(),
            None if self.simple() => Settings::new(),
            Some(s) => s,
            None => return false,
        };
        let mut content: &[Node] = &node.content;
        // Markdown can't end a paragraph with a hard break.
        while content.last().is_some_and(|n| n.is("hardBreak")) {
            content = &content[..content.len() - 1];
        }
        let Some(items) = self.items(content) else { return false };
        if items.is_empty() {
            // Markdown has no empty paragraph (design.md, "What Confluence rewrites on save").
            return true;
        }
        let p = append(self.arena, parent, NodeValue::Paragraph);
        inline::build(self.arena, p, &items);
        if !settings.is_empty() {
            append(self.arena, p, NodeValue::HtmlInline(settings.to_comment()));
        }
        true
    }

    fn heading(&self, parent: &'a AstNode<'a>, node: &Node) -> bool {
        let settings = match Self::block_settings(node) {
            _ if self.simple() => Settings::new(),
            Some(s) => s,
            None => return false,
        };
        let level = node.attr_f64("level").unwrap_or(1.0).clamp(1.0, 6.0) as u8;
        let Some(items) = self.items(&node.content) else { return false };
        let h = append(self.arena, parent, NodeValue::Heading(NodeHeading { level, setext: false, closed: false }));
        inline::build(self.arena, h, &items);
        if !settings.is_empty() {
            append(self.arena, h, NodeValue::HtmlInline(settings.to_comment()));
        }
        true
    }

    fn code_block(&self, parent: &'a AstNode<'a>, node: &Node) -> bool {
        if self.simple() {
            self.code(parent, node.attr_str("language").unwrap_or("").to_string(), node.plain_text() + "\n");
            return true;
        }
        if node.attrs.keys().any(|k| !matches!(k.as_str(), "language" | "localId") && !k.starts_with("__")) {
            return false;
        }
        let mut settings = Settings::new();
        for mark in &node.marks {
            if mark.kind != "breakout" || !breakout_settings(mark, &mut settings) {
                return false;
            }
        }
        let lang = node.attr_str("language").unwrap_or("");
        let info = match (lang.is_empty(), settings.is_empty()) {
            (_, true) => lang.to_string(),
            (true, false) => settings.to_string(),
            (false, false) => format!("{lang} {settings}"),
        };
        self.code(parent, info, node.plain_text() + "\n");
        true
    }

    fn html_block(&self, parent: &'a AstNode<'a>, literal: String) {
        append(self.arena, parent, NodeValue::HtmlBlock(NodeHtmlBlock { block_type: 6, literal }));
    }

    /// An expand as `<details><summary>Title</summary>` ... `</details>`.
    fn expand(&self, parent: &'a AstNode<'a>, node: &Node) -> bool {
        if self.simple() {
            // The title as a bold line, then the content.
            let title = node.attr_str("title").unwrap_or("");
            if !title.is_empty() {
                let p = append(self.arena, parent, NodeValue::Paragraph);
                let strong = append(self.arena, p, NodeValue::Strong);
                append(self.arena, strong, NodeValue::Text(title.to_string().into()));
            }
            self.blocks(parent, &node.content);
            return true;
        }
        if node.attrs.keys().any(|k| !matches!(k.as_str(), "title" | "localId") && !k.starts_with("__"))
            || node.marks.iter().any(|m| !(m.kind == "breakout" && adf::is_default_breakout(m)))
        {
            return false;
        }
        self.html_block(parent, crate::approx::details_open(node.attr_str("title").unwrap_or("")));
        self.blocks(parent, &node.content);
        self.html_block(parent, "</details>".into());
        true
    }

    /// A layout as `<!-- rf: columns=50,50 -->`, the columns separated by
    /// `<!-- rf: column -->`, then `<!-- rf: end-columns -->` (design.md, "Element mapping").
    fn layout(&self, parent: &'a AstNode<'a>, node: &Node) -> bool {
        if self.simple() {
            // The columns' content, in order.
            for column in &node.content {
                self.blocks(parent, &column.content);
            }
            return true;
        }
        let mut settings = Settings::new();
        let mut widths = Vec::new();
        for column in &node.content {
            if !column.is("layoutColumn")
                || column.attrs.keys().any(|k| !matches!(k.as_str(), "width" | "localId") && !k.starts_with("__"))
            {
                return false;
            }
            widths.push(fmt_num(column.attr_f64("width").unwrap_or(0.0)));
        }
        settings.set("columns", widths.join(","));
        for mark in &node.marks {
            if mark.kind != "breakout" || !breakout_settings(mark, &mut settings) {
                return false;
            }
        }
        self.html_block(parent, settings.to_comment());
        for (i, column) in node.content.iter().enumerate() {
            if i > 0 {
                let mut marker = Settings::new();
                marker.flag("column");
                self.html_block(parent, marker.to_comment());
            }
            self.blocks(parent, &column.content);
        }
        let mut end = Settings::new();
        end.flag("end-columns");
        self.html_block(parent, end.to_comment());
        true
    }

    fn list(&self, parent: &'a AstNode<'a>, node: &Node) -> bool {
        if !node.marks.is_empty() || node.content.iter().any(|i| !i.is("listItem") || !i.marks.is_empty()) {
            return false;
        }
        let ordered = node.is("orderedList");
        let info = NodeList {
            list_type: if ordered { ListType::Ordered } else { ListType::Bullet },
            marker_offset: 0,
            padding: if ordered { 3 } else { 2 },
            start: node.attr_f64("order").unwrap_or(1.0) as usize,
            delimiter: ListDelimType::Period,
            bullet_char: b'-',
            tight: true,
            is_task_list: false,
        };
        let list = append(self.arena, parent, NodeValue::List(info));
        for item in &node.content {
            let li = append(self.arena, list, NodeValue::Item(info));
            self.blocks(li, &item.content);
        }
        true
    }

    fn task_list(&self, parent: &'a AstNode<'a>, node: &Node) -> bool {
        let info = NodeList {
            list_type: ListType::Bullet,
            marker_offset: 0,
            padding: 2,
            start: 1,
            delimiter: ListDelimType::Period,
            bullet_char: b'-',
            tight: true,
            is_task_list: true,
        };
        // Check first, so a list we can't convert isn't half-written.
        let ok = node.content.iter().all(|c| match c.kind.as_str() {
            "taskItem" => c.marks.is_empty() && self.items(&c.content).is_some(),
            "taskList" => true,
            _ => false,
        });
        if !ok || node.content.first().is_some_and(|c| c.is("taskList")) {
            return false;
        }
        let list = append(self.arena, parent, NodeValue::List(info));
        let mut last_item = None;
        for child in &node.content {
            if child.is("taskList") {
                // A nested task list follows its parent item in ADF; in markdown it goes inside it.
                if let Some(item) = last_item {
                    self.block(item, child);
                }
                continue;
            }
            let done = child.attr_str("state") == Some("DONE");
            let item = append(
                self.arena,
                list,
                NodeValue::TaskItem(NodeTaskItem { symbol: done.then_some('x'), symbol_sourcepos: (1, 1, 1, 1).into() }),
            );
            let items = self.items(&child.content).unwrap_or_default();
            if !items.is_empty() {
                let p = append(self.arena, item, NodeValue::Paragraph);
                inline::build(self.arena, p, &items);
            }
            last_item = Some(item);
        }
        true
    }

    fn panel(&self, parent: &'a AstNode<'a>, node: &Node) -> bool {
        // GitHub alerts and Confluence panels, matched by colour.
        let alert_type = match node.attr_str("panelType") {
            Some("info") => AlertType::Note,
            Some("note") => AlertType::Important,
            Some("success") => AlertType::Tip,
            Some("warning") => AlertType::Warning,
            Some("error") => AlertType::Caution,
            _ => return false,
        };
        if node.attrs.keys().any(|k| !matches!(k.as_str(), "panelType" | "localId") && !k.starts_with("__"))
            || !node.marks.is_empty()
        {
            return false;
        }
        let alert = append(
            self.arena,
            parent,
            NodeValue::Alert(Box::new(NodeAlert { alert_type, title: None, multiline: false, fence_length: 0, fence_offset: 0 })),
        );
        self.blocks(alert, &node.content);
        true
    }

    fn media_single(&self, parent: &'a AstNode<'a>, node: &Node) -> bool {
        let Some(media) = node.content.first().filter(|m| m.is("media")) else { return false };
        if self.simple() {
            self.simplified_image(parent, node, media);
            return true;
        }
        let caption = match &node.content[1..] {
            [] => None,
            [c] if c.is("caption") && c.content.iter().all(|t| t.is("text") && t.marks.is_empty()) => Some(c.plain_text()),
            _ => return false,
        };
        let known_ms = ["layout", "width", "widthType", "localId"];
        let known_media = ["type", "id", "collection", "alt", "url", "width", "height", "localId"];
        if node.attrs.keys().any(|k| !known_ms.contains(&k.as_str()) && !k.starts_with("__"))
            || media.attrs.keys().any(|k| !known_media.contains(&k.as_str()) && !k.starts_with("__"))
            || !node.marks.is_empty()
        {
            return false;
        }
        let src = match media.attr_str("type") {
            Some("file") => {
                let Some(id) = media.attr_str("id") else { return false };
                if let Some(page) = &self.ctx.page_id {
                    if media.attr_str("collection") != Some(&format!("contentId-{page}")) {
                        return false;
                    }
                }
                let name = self.ctx.attachments.get(id).map(String::as_str).or_else(|| media.attr_str("__fileName"));
                let Some(name) = name else { return false };
                if self.ctx.assets_dir.is_empty() { name.to_string() } else { format!("{}/{name}", self.ctx.assets_dir) }
            }
            Some("external") => match media.attr_str("url") {
                Some(url) => url.to_string(),
                None => return false,
            },
            _ => return false,
        };

        let mut settings = Settings::new();
        if let Some(layout) = node.attr_str("layout").filter(|l| *l != "align-start") {
            settings.set("layout", layout);
        }
        let width_type = node.attr_str("widthType").unwrap_or("pixel");
        if let Some(width) = node.attr_f64("width") {
            let natural = media.attr_f64("width");
            if !(width_type == "pixel" && natural == Some(width)) {
                settings.set("width", fmt_num(width));
                if width_type != "pixel" {
                    settings.set("width-type", width_type);
                }
            }
        }
        let mut link = None;
        for mark in &media.marks {
            match mark.kind.as_str() {
                "border" => {
                    settings.set("border", fmt_num(mark.attr_f64("size").unwrap_or(1.0)));
                    if let Some(color) = mark.attr_str("color") {
                        settings.set("border-color", color);
                    }
                }
                "link" if mark.attr_str("href").is_some() => link = mark.attr_str("href"),
                "annotation" => {}
                _ => return false,
            }
        }
        if let Some(caption) = caption {
            settings.set("caption", caption);
        }

        let p = append(self.arena, parent, NodeValue::Paragraph);
        let target = match link {
            Some(href) => {
                let url = self.local_link(href).unwrap_or_else(|| href.to_string());
                append(self.arena, p, NodeValue::Link(Box::new(NodeLink { url, title: String::new() })))
            }
            None => p,
        };
        let img = append(self.arena, target, NodeValue::Image(Box::new(NodeLink { url: src, title: String::new() })));
        if let Some(alt) = media.attr_str("alt").filter(|a| !a.is_empty()) {
            append(self.arena, img, NodeValue::Text(alt.to_string().into()));
        }
        if !settings.is_empty() {
            append(self.arena, p, NodeValue::HtmlInline(settings.to_comment()));
        }
        true
    }

    /// An image in simplified output: external images keep their URL; attached ones become
    /// `[image: alt]`, since the file isn't there.
    fn simplified_image(&self, parent: &'a AstNode<'a>, node: &Node, media: &Node) {
        let alt = media.attr_str("alt").filter(|a| !a.is_empty());
        let caption = node.content.iter().find(|c| c.is("caption")).map(Node::plain_text).filter(|c| !c.is_empty());
        let p = append(self.arena, parent, NodeValue::Paragraph);
        if let Some(url) = media.attr_str("url").filter(|_| media.attr_str("type") == Some("external")) {
            let img = append(self.arena, p, NodeValue::Image(Box::new(NodeLink { url: url.into(), title: String::new() })));
            if let Some(alt) = alt {
                append(self.arena, img, NodeValue::Text(alt.to_string().into()));
            }
        } else {
            let name = media
                .attr_str("id")
                .and_then(|id| self.ctx.attachments.get(id).map(String::as_str))
                .or_else(|| media.attr_str("__fileName"));
            let label = alt.or(name).unwrap_or("image");
            append(self.arena, p, NodeValue::HtmlInline(format!("[image: {label}]")));
        }
        if let Some(caption) = caption {
            append(self.arena, p, NodeValue::Text(format!(" ({caption})").into()));
        }
    }

    fn card(&self, parent: &'a AstNode<'a>, node: &Node) -> bool {
        let Some(url) = node.attr_str("url") else { return false };
        if self.simple() {
            let p = append(self.arena, parent, NodeValue::Paragraph);
            let link = append(self.arena, p, NodeValue::Link(Box::new(NodeLink { url: url.into(), title: String::new() })));
            append(self.arena, link, NodeValue::Text(url.to_string().into()));
            return true;
        }
        let embed = node.is("embedCard");
        let allowed: &[&str] = if embed { &["url", "layout", "width", "originalWidth", "originalHeight", "localId"] } else { &["url", "localId"] };
        if node.attrs.keys().any(|k| !allowed.contains(&k.as_str()) && !k.starts_with("__")) || !node.marks.is_empty() {
            return false;
        }
        let mut settings = Settings::new();
        settings.set("card", if embed { "embed" } else { "block" });
        if let Some(layout) = node.attr_str("layout") {
            settings.set("layout", layout);
        }
        if let Some(width) = node.attr_f64("width") {
            settings.set("width", fmt_num(width));
        }
        let p = append(self.arena, parent, NodeValue::Paragraph);
        let link = append(self.arena, p, NodeValue::Link(Box::new(NodeLink { url: url.into(), title: String::new() })));
        append(self.arena, link, NodeValue::Text(url.to_string().into()));
        append(self.arena, p, NodeValue::HtmlInline(settings.to_comment()));
        true
    }

    fn mermaid(&self, parent: &'a AstNode<'a>, node: &Node) -> bool {
        let guest = node.attrs.get("parameters").and_then(|p| p.get("guestParams"));
        let Some(source) = guest.and_then(|g| g.get("source")).and_then(Value::as_str) else { return false };
        let mut settings = Settings::new();
        let guest = guest.expect("checked above");
        for (key, default) in [("theme", "auto"), ("mermaidVersion", "auto")] {
            if let Some(v) = guest.get(key).and_then(Value::as_str).filter(|v| *v != default) {
                settings.set(key, v);
            }
        }
        if guest.get("useMaxWidth").and_then(Value::as_bool) == Some(false) {
            settings.set("useMaxWidth", "false");
        }
        let info = if settings.is_empty() || self.simple() { "mermaid".to_string() } else { format!("mermaid {settings}") };
        // `source` has no trailing newline; fence content does (design.md, "Mermaid diagrams").
        self.code(parent, info, format!("{source}\n"));
        true
    }

    fn table(&self, parent: &'a AstNode<'a>, node: &Node) -> bool {
        let Some(t) = TableShape::read(node, self) else { return self.html_table(parent, node) };
        let num_columns = t.alignments.len();
        let table = append(
            self.arena,
            parent,
            NodeValue::Table(Box::new(NodeTable {
                alignments: t.alignments.clone(),
                num_columns,
                num_rows: t.rows.len(),
                num_nonempty_cells: t.rows.iter().flatten().filter(|c| !c.is_empty()).count(),
            })),
        );
        for (i, row) in t.rows.iter().enumerate() {
            let r = append(self.arena, table, NodeValue::TableRow(i == 0));
            for cell in row {
                let c = append(self.arena, r, NodeValue::TableCell);
                inline::build(self.arena, c, cell);
            }
        }
        if !t.settings.is_empty() && !self.simple() {
            append(
                self.arena,
                parent,
                NodeValue::HtmlBlock(NodeHtmlBlock { block_type: 2, literal: t.settings.to_comment() }),
            );
        }
        true
    }

    /// A table GFM can't express, as an HTML table with markdown cells (html_table.rs).
    fn html_table(&self, parent: &'a AstNode<'a>, node: &Node) -> bool {
        let known = ["isNumberColumnEnabled", "layout", "width", "localId", "displayMode"];
        if node.attrs.keys().any(|k| !known.contains(&k.as_str()) && !k.starts_with("__"))
            || node.attr_str("displayMode").is_some_and(|m| m != "default")
            || !node.marks.is_empty()
            || node.content.iter().any(|r| !r.is("tableRow") || r.content.is_empty())
        {
            return false;
        }
        let cell_attrs = ["colspan", "rowspan", "colwidth", "background", "localId"];
        let strict = !self.simple();
        let span = |c: &Node, key: &str| c.attr_f64(key).unwrap_or(1.0).max(1.0) as usize;
        let mut spans = Vec::new();
        for row in &node.content {
            let mut r = Vec::new();
            for cell in &row.content {
                if !(cell.is("tableCell") || cell.is("tableHeader"))
                    || strict && cell.attrs.keys().any(|k| !cell_attrs.contains(&k.as_str()) && !k.starts_with("__"))
                {
                    return false;
                }
                r.push((span(cell, "colspan"), span(cell, "rowspan")));
            }
            spans.push(r);
        }
        let grid = crate::html_table::grid_columns(&spans);

        // Column widths: one list for the table if every cell agrees, else per cell.
        let cell_widths = |cell: &Node| -> Option<Vec<f64>> {
            cell.attr("colwidth")?.as_array()?.iter().map(Value::as_f64).collect()
        };
        let mut columns: Vec<Option<f64>> = Vec::new();
        let mut consistent = true;
        let mut any = false;
        for (row, cols) in node.content.iter().zip(&grid) {
            for (cell, &col) in row.content.iter().zip(cols) {
                let colspan = span(cell, "colspan");
                let widths = cell_widths(cell);
                any |= widths.is_some();
                for k in 0..colspan {
                    let w = widths.as_ref().and_then(|w| w.get(k).copied());
                    if columns.len() <= col + k {
                        columns.resize(col + k + 1, None);
                        columns[col + k] = w;
                    } else if columns[col + k] != w {
                        consistent = false;
                    }
                }
            }
        }
        let table_widths = any && consistent && columns.iter().all(Option::is_some);

        let mut settings = Settings::new();
        let layout = node.attr_str("layout").unwrap_or("default");
        if layout != "default" {
            settings.set("layout", layout);
        }
        if let Some(width) = node.attr_f64("width").filter(|w| Some(*w) != adf::default_table_width(layout)) {
            settings.set("width", fmt_num(width));
        }
        if table_widths {
            let list: Vec<String> = columns.iter().map(|w| fmt_num(w.expect("checked"))).collect();
            settings.set("colwidths", list.join(","));
        }
        if node.attr("isNumberColumnEnabled") == Some(&Value::Bool(true)) {
            settings.flag("numbered");
        }

        // Tag runs as HTML blocks, cell content as markdown between them.
        let mut buf = String::from("<table>");
        let push = |buf: &mut String, tag: &str| {
            buf.push('\n');
            buf.push_str(tag);
        };
        for row in &node.content {
            push(&mut buf, "<tr>");
            for cell in &row.content {
                let header = cell.is("tableHeader");
                let own_widths = if table_widths || self.simple() { None } else { cell_widths(cell) };
                let background = if self.simple() { None } else { cell.attr_str("background") };
                let open = crate::html_table::cell_open(
                    header,
                    span(cell, "colspan"),
                    span(cell, "rowspan"),
                    background,
                    own_widths.as_deref(),
                );
                push(&mut buf, &open);
                // Render the content aside first: a cell whose content renders to nothing
                // (an empty paragraph) has no blank lines inside.
                let scratch = crate::markdown::node(self.arena, NodeValue::Document);
                self.blocks(scratch, &cell.content);
                let kids: Vec<_> = scratch.children().collect();
                // Simplified output puts one-line content on the tags' line: shorter to read,
                // though markdown there isn't parsed (it can't be uploaded anyway).
                let one_line = (self.simple() && !kids.is_empty())
                    .then(|| crate::markdown::render(scratch).trim().to_string())
                    .filter(|md| !md.contains('\n'));
                if let Some(md) = one_line {
                    buf.push_str(&md);
                    buf.push_str(if header { "</th>" } else { "</td>" });
                    continue;
                }
                if !kids.is_empty() {
                    self.html_block(parent, std::mem::take(&mut buf));
                    for k in kids {
                        parent.append(k);
                    }
                }
                push(&mut buf, if header { "</th>" } else { "</td>" });
            }
            push(&mut buf, "</tr>");
        }
        push(&mut buf, "</table>");
        self.html_block(parent, buf.trim_start_matches('\n').to_string());
        if !settings.is_empty() && !self.simple() {
            append(self.arena, parent, NodeValue::HtmlBlock(NodeHtmlBlock { block_type: 2, literal: settings.to_comment() }));
        }
        true
    }

    /// A link to a page as a relative path, if the page is in the local project (or is this
    /// page, with an anchor).
    fn local_link(&self, href: &str) -> Option<String> {
        let link = crate::links::page_link(href, self.ctx.site_host.as_deref()?)?;
        if self.ctx.page_id.as_deref() == Some(link.id.as_str()) {
            let gh = self.anchors.to_github(link.anchor.as_deref()?)?;
            return Some(format!("#{gh}"));
        }
        let target = self.ctx.links.get(&link.id)?;
        let anchor = match &link.anchor {
            None => String::new(),
            Some(a) => match Anchors::new(target.headings.iter().map(String::as_str)).to_github(a) {
                Some(gh) => format!("#{gh}"),
                None => format!("#{a}"),
            },
        };
        Some(format!("{}{anchor}", target.path))
    }

    /// Inline ADF nodes as items; `None` if they contain marks rfluence can't write.
    fn items(&self, nodes: &[Node]) -> Option<Vec<Item<'a>>> {
        let mut items = Vec::new();
        for node in nodes {
            let marks = self.md_marks(&node.marks)?;
            match node.kind.as_str() {
                "text" => {
                    let text = node.text.clone().unwrap_or_default();
                    if node.mark("code").is_some() {
                        items.push(Item::new(marks, Leaf::Code(text)));
                    } else {
                        items.push(Item::new(marks, Leaf::Text(text)));
                    }
                }
                "hardBreak" => items.push(Item::new(marks, Leaf::LineBreak)),
                "emoji" => match emoji_text(node) {
                    Some(text) => items.push(Item::new(marks, Leaf::Text(text))),
                    None if self.simple() => items.extend(simplified_inline(node)),
                    None => items.extend(span(node)),
                },
                "inlineCard" if node.attr_str("url").is_some() => {
                    let url = node.attr_str("url").expect("checked").to_string();
                    let mut marks = marks;
                    marks.push(MdMark::Link { url: url.clone(), title: String::new() });
                    items.push(Item::new(marks, Leaf::Text(url)));
                }
                _ if self.simple() => items.extend(simplified_inline(node)),
                _ => items.extend(span(node)),
            }
        }
        Some(items)
    }

    fn md_marks(&self, marks: &[Mark]) -> Option<Vec<MdMark>> {
        let mut out = Vec::new();
        for mark in marks {
            // Simplified output keeps only marks with markdown syntax.
            let markdown = matches!(mark.kind.as_str(), "strong" | "em" | "strike" | "link" | "code");
            if self.simple() && !markdown {
                continue;
            }
            out.push(match mark.kind.as_str() {
                "strong" => MdMark::Strong,
                "em" => MdMark::Emph,
                "strike" => MdMark::Strike,
                "underline" => MdMark::Underline,
                "subsup" if mark.attr_str("type") == Some("sub") => MdMark::Sub,
                "subsup" if mark.attr_str("type") == Some("sup") => MdMark::Sup,
                "textColor" => MdMark::Color(mark.attr_str("color")?.to_string()),
                "backgroundColor" => MdMark::Background(mark.attr_str("color")?.to_string()),
                "link" => {
                    let href = mark.attr_str("href")?;
                    if mark.attrs.keys().any(|k| !matches!(k.as_str(), "href" | "title") && !k.starts_with("__")) {
                        return None;
                    }
                    let url = match href.strip_prefix('#') {
                        Some(anchor) => match self.anchors.to_github(anchor) {
                            Some(gh) => format!("#{gh}"),
                            None => href.to_string(),
                        },
                        None => self.local_link(href).unwrap_or_else(|| href.to_string()),
                    };
                    MdMark::Link { url, title: mark.attr_str("title").unwrap_or("").to_string() }
                }
                // Code is a leaf, not a wrapper; inline comments are re-anchored on upload
                // (design.md, "Inline comments").
                "code" | "annotation" => continue,
                _ => return None,
            });
        }
        Some(out)
    }
}

/// The parts of a table that can be written as a GFM table.
struct TableShape<'a> {
    rows: Vec<Vec<Vec<Item<'a>>>>,
    alignments: Vec<TableAlignment>,
    settings: Settings,
}

impl<'a> TableShape<'a> {
    fn read(node: &Node, w: &Writer<'a, '_>) -> Option<TableShape<'a>> {
        let known = ["isNumberColumnEnabled", "layout", "width", "localId", "displayMode"];
        if node.attrs.keys().any(|k| !known.contains(&k.as_str()) && !k.starts_with("__"))
            || node.attr_str("displayMode").is_some_and(|m| m != "default")
            || !node.marks.is_empty()
            || node.content.is_empty()
        {
            return None;
        }
        let ncols = node.content[0].content.len();
        let mut header_row = true;
        let mut header_column = node.content.len() > 1;
        for (r, row) in node.content.iter().enumerate() {
            if !row.is("tableRow") || row.content.len() != ncols || ncols == 0 {
                return None;
            }
            for (c, cell) in row.content.iter().enumerate() {
                let header = cell.is("tableHeader");
                if r == 0 && !header {
                    header_row = false;
                }
                if c == 0 && r > 0 && !header {
                    header_column = false;
                }
            }
        }
        let mut rows = Vec::new();
        let mut alignments: Vec<Option<Option<&str>>> = vec![None; ncols];
        let mut widths: Vec<Option<Option<f64>>> = vec![None; ncols];
        for (r, row) in node.content.iter().enumerate() {
            let mut cells = Vec::new();
            for (c, cell) in row.content.iter().enumerate() {
                let header = cell.is("tableHeader");
                let expected = (r == 0 && header_row) || (c == 0 && header_column);
                if header != expected || !(cell.is("tableCell") || header) {
                    return None;
                }
                let known = ["colspan", "rowspan", "colwidth", "localId"];
                if cell.attrs.keys().any(|k| !known.contains(&k.as_str()) && !k.starts_with("__"))
                    || cell.attr_f64("colspan").unwrap_or(1.0) != 1.0
                    || cell.attr_f64("rowspan").unwrap_or(1.0) != 1.0
                {
                    return None;
                }
                let width = match cell.attr("colwidth") {
                    None => None,
                    Some(Value::Array(a)) if a.len() == 1 => Some(a[0].as_f64()?),
                    Some(_) => return None,
                };
                if *widths[c].get_or_insert(width) != width {
                    return None;
                }
                let (align, items) = match cell.content.as_slice() {
                    [] => (None, Vec::new()),
                    [p] if p.is("paragraph") => {
                        let mut align = None;
                        for mark in &p.marks {
                            match (mark.kind.as_str(), mark.attr_str("align")) {
                                ("alignment", Some(a @ ("center" | "end"))) => align = Some(a),
                                _ => return None,
                            }
                        }
                        // A GFM table cell is one line; line breaks are written as `<br>`.
                        let items = w
                            .items(&p.content)?
                            .into_iter()
                            .map(|it| match it.leaf {
                                Leaf::LineBreak => Item::new(it.marks, Leaf::Html("<br>".into())),
                                _ => it,
                            })
                            .collect();
                        (align, items)
                    }
                    _ => return None,
                };
                // Empty cells take the column's alignment; others must agree.
                if (!items.is_empty() || align.is_some()) && *alignments[c].get_or_insert(align) != align {
                    return None;
                }
                cells.push(items);
            }
            rows.push(cells);
        }

        let mut settings = Settings::new();
        let layout = node.attr_str("layout").unwrap_or("default");
        if layout != "default" {
            settings.set("layout", layout);
        }
        if let Some(width) = node.attr_f64("width").filter(|w| Some(*w) != adf::default_table_width(layout)) {
            settings.set("width", fmt_num(width));
        }
        if widths.iter().any(|w| matches!(w, Some(Some(_)))) {
            let list: Option<Vec<String>> = widths.iter().map(|w| w.flatten().map(fmt_num)).collect();
            settings.set("colwidths", list?.join(","));
        }
        if node.attr("isNumberColumnEnabled") == Some(&Value::Bool(true)) {
            settings.flag("numbered");
        }
        if !header_row {
            settings.flag("no-header-row");
        }
        if header_column {
            settings.flag("header-column");
        }
        let alignments = alignments
            .into_iter()
            .map(|a| match a.flatten() {
                Some("center") => TableAlignment::Center,
                Some("end") => TableAlignment::Right,
                _ => TableAlignment::None,
            })
            .collect();
        Some(TableShape { rows, alignments, settings })
    }
}

/// `breakout` mark as fence settings; false if it can't be expressed.
fn breakout_settings(mark: &Mark, settings: &mut Settings) -> bool {
    let mode = mark.attr_str("mode").unwrap_or("");
    let width = mark.attr_f64("width");
    match (mode, width) {
        ("wide", Some(760.0)) => {}
        ("wide", Some(w)) => settings.set("width", fmt_num(w)),
        ("wide", None) => settings.set("breakout", "wide"),
        ("full-width", w) => {
            settings.set("breakout", "full-width");
            if let Some(w) = w.filter(|w| Some(*w) != adf::default_breakout_width("full-width")) {
                settings.set("width", fmt_num(w));
            }
        }
        _ => return false,
    }
    true
}

/// How fetch writes an emoji node (design.md, "Emoji"); `None` to keep it as a span.
fn emoji_text(node: &Node) -> Option<String> {
    let short = node.attr_str("shortName");
    // A standard emoji: its characters, from `text`, or `id`, or (for nodes written by other
    // tools with only a name) `shortName`.
    let standard = node
        .attr_str("text")
        .filter(|t| emoji::is_emoji(t))
        .and_then(emoji::get)
        .or_else(|| node.attr_str("id").and_then(emoji::from_id))
        .or_else(|| short.and_then(emoji::lookup));
    if let Some(e) = standard {
        return Some(e.as_str().to_string());
    }
    // A custom emoji (no characters): its shortcode.
    short.filter(|s| emoji::find_shortcodes(s).first().is_some_and(|sc| sc.range == (0..s.len()))).map(str::to_string)
}

/// An inline node in simplified output: its visible text (`[IN PROGRESS]` for a status).
fn simplified_inline<'a>(node: &Node) -> Vec<Item<'a>> {
    let text = match node.kind.as_str() {
        "status" => format!("[{}]", node.attr_str("text").unwrap_or("")),
        "mention" => node.attr_str("text").unwrap_or("").to_string(),
        "date" => node.attr_str("timestamp").and_then(|t| t.parse::<i64>().ok()).map(iso_date).unwrap_or_default(),
        "emoji" => node.attr_str("shortName").unwrap_or("").to_string(),
        "inlineExtension" => node.attr_str("text").or_else(|| node.attr_str("extensionKey")).map(|t| format!("[{t}]")).unwrap_or_default(),
        _ => node.plain_text(),
    };
    if text.is_empty() { vec![] } else { vec![Item::new(vec![], Leaf::Html(text))] }
}

/// An inline node without a markdown form, as `<span data-adf='{json}'>visible text</span>`.
fn span<'a>(node: &Node) -> Vec<Item<'a>> {
    let mut node = node.clone();
    adf::strip_noise(&mut node);
    strip_annotations(&mut node);
    let visible = match node.kind.as_str() {
        "mention" | "status" => node.attr_str("text").unwrap_or("").to_string(),
        "date" => node.attr_str("timestamp").and_then(|t| t.parse::<i64>().ok()).map(iso_date).unwrap_or_default(),
        "inlineExtension" => node.attr_str("text").or_else(|| node.attr_str("extensionKey")).unwrap_or("").to_string(),
        _ => node.plain_text(),
    };
    // JSON in a single-quoted attribute: `'` must not appear raw.
    let json = adf::to_compact_json(&node).replace('\'', "\\u0027");
    let mut items = vec![Item::new(vec![], Leaf::Html(format!("<span data-adf='{json}'>")))];
    if !visible.is_empty() {
        items.push(Item::new(vec![], Leaf::Text(visible)));
    }
    items.push(Item::new(vec![], Leaf::Html("</span>".into())));
    items
}

fn strip_annotations(node: &mut Node) {
    node.walk_mut(&mut |n| n.marks.retain(|m| m.kind != "annotation"));
}

/// `YYYY-MM-DD` (UTC) for milliseconds since the epoch.
fn iso_date(ms: i64) -> String {
    // Howard Hinnant's civil_from_days.
    let z = ms.div_euclid(86_400_000) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

/// A merfluence diagram: `extensionType` `com.atlassian.ecosystem` and a key ending in
/// `/static/mermaid-diagram` (design.md, "Mermaid diagrams (merfluence)").
pub fn is_merfluence(node: &Node) -> bool {
    node.attr_str("extensionType") == Some("com.atlassian.ecosystem")
        && node.attr_str("extensionKey").is_some_and(|k| k.ends_with("/static/mermaid-diagram"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_dates() {
        assert_eq!(iso_date(1_798_761_600_000), "2027-01-01");
        assert_eq!(iso_date(0), "1970-01-01");
    }

    #[test]
    fn writes_links_to_project_pages_as_relative_paths() {
        let doc: Node = serde_json::from_str(
            r##"{"type":"doc","content":[
                {"type":"heading","attrs":{"level":2},"content":[{"type":"text","text":"Here"}]},
                {"type":"paragraph","content":[
                    {"type":"text","text":"setup","marks":[{"type":"link","attrs":{"href":"https://x.atlassian.net/wiki/spaces/ENG/pages/22/Setup#Install-&-Run"}}]},
                    {"type":"text","text":", "},
                    {"type":"text","text":"self","marks":[{"type":"link","attrs":{"href":"https://x.atlassian.net/wiki/spaces/ENG/pages/11#Here"}}]},
                    {"type":"text","text":", "},
                    {"type":"text","text":"other","marks":[{"type":"link","attrs":{"href":"https://x.atlassian.net/wiki/spaces/ENG/pages/33"}}]},
                    {"type":"text","text":", "},
                    {"type":"inlineCard","attrs":{"url":"https://x.atlassian.net/wiki/spaces/ENG/pages/22"}}
                ]}]}"##,
        )
        .unwrap();
        let ctx = FetchContext {
            page_id: Some("11".into()),
            site_host: Some("x.atlassian.net".into()),
            links: HashMap::from([(
                "22".to_string(),
                LinkTarget { path: "../guides/setup.md".into(), headings: vec!["Setup".into(), "Install & Run".into()] },
            )]),
            ..Default::default()
        };
        let md = adf_to_markdown(&doc, &ctx);
        assert!(md.contains("[setup](../guides/setup.md#install--run)"), "{md}");
        assert!(md.contains("[self](#here)"), "{md}");
        // Pages outside the project, and smart links, keep their URLs.
        assert!(md.contains("[other](https://x.atlassian.net/wiki/spaces/ENG/pages/33)"), "{md}");
        assert!(md.contains("<https://x.atlassian.net/wiki/spaces/ENG/pages/22>"), "{md}");
    }
}
