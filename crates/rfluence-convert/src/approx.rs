//! Markdown with a close Confluence equivalent, rewritten into that equivalent.
//!
//! Runs on the parsed markdown before both normalize and upload, so the two agree and
//! `normalize(md) == fetch(upload(md))` still holds. Each rewrite is reported as a warning
//! (design.md, "Checking markdown").

use comrak::nodes::{AstNode, NodeCode, NodeHtmlBlock, NodeTable, NodeValue, TableAlignment};
use comrak::{Arena, parse_document};

use crate::diagnostics::Diagnostic;
use crate::html_table;
use crate::markdown::{self, node, options};
use crate::settings::Settings;

/// Rewrite approximations in place, reporting each as a warning.
pub fn approximate<'a>(arena: &'a Arena<'a>, root: &'a AstNode<'a>, diags: &mut Vec<Diagnostic>) {
    // Tables first: their cells' content is ordinary markdown afterwards.
    gfm_tables_with_lists(arena, root, diags);
    html_tables(arena, root, diags);
    let all: Vec<_> = root.descendants().collect();
    for n in &all {
        let line = n.data().sourcepos.start.line;
        let value = n.data().value.clone();
        match value {
            NodeValue::Alert(a) if a.title.is_some() => {
                let title = a.title.clone().unwrap_or_default();
                if let NodeValue::Alert(a) = &mut n.data_mut().value {
                    a.title = None;
                }
                // The title becomes a bold first line.
                let p = node(arena, NodeValue::Paragraph);
                let strong = node(arena, NodeValue::Strong);
                strong.append(node(arena, NodeValue::Text(title.into())));
                p.append(strong);
                n.prepend(p);
                diags.push(Diagnostic::warning(line, "alert title written as a bold first line (Confluence panels have no title)"));
            }
            NodeValue::Image(l) if !l.title.is_empty() => {
                if let NodeValue::Image(l) = &mut n.data_mut().value {
                    l.title.clear();
                }
                diags.push(Diagnostic::warning(line, "image title dropped (Confluence images have no title)"));
            }
            NodeValue::HtmlBlock(h) if is_details_open(&h.literal) => details(arena, n, &h.literal, diags),
            _ => {}
        }
    }
    let containers: Vec<_> = root
        .descendants()
        .filter(|n| n.children().any(|c| matches!(c.data().value, NodeValue::HtmlInline(_))))
        .collect();
    for container in containers {
        inline_html(arena, container, diags);
    }
}

/// Rewrite HTML tables into the canonical form (html_table.rs).
fn html_tables<'a>(arena: &'a Arena<'a>, root: &'a AstNode<'a>, diags: &mut Vec<Diagnostic>) {
    let containers: Vec<_> = root
        .descendants()
        .filter(|n| n.children().any(|c| matches!(&c.data().value, NodeValue::HtmlBlock(h) if html_table::is_table_start(&h.literal))))
        .collect();
    for container in containers {
        let mut i = 0;
        loop {
            let kids: Vec<_> = container.children().collect();
            let Some(&n) = kids.get(i) else { break };
            let start = matches!(&n.data().value, NodeValue::HtmlBlock(h) if html_table::is_table_start(&h.literal));
            if !start {
                i += 1;
                continue;
            }
            let Some((table, end)) = html_table::parse(arena, &kids, i, diags) else {
                i += 1;
                continue;
            };
            let line = n.data().sourcepos.start.line;
            // Like fetch, use a GFM table when GFM can express it.
            let (rendered, flags) = match as_gfm(arena, &table, line) {
                Some((gfm, flags)) => (vec![gfm], flags),
                None => (html_table::render(arena, &table), Vec::new()),
            };
            // The settings comment: inside the closing HTML block, or the next block.
            let next = kids.get(end + 1).filter(|k| {
                matches!(&k.data().value, NodeValue::HtmlBlock(h)
                    if Settings::parse_comment(&h.literal).is_some_and(|s| !(s.has("columns") || s.has("column") || s.has("end-columns"))))
            });
            let mut settings = table.settings.clone();
            if settings.is_none() && !flags.is_empty() {
                if let Some(next) = next {
                    if let NodeValue::HtmlBlock(h) = &next.data().value {
                        settings = Settings::parse_comment(&h.literal);
                    }
                    next.detach();
                }
            }
            if !flags.is_empty() {
                let s = settings.get_or_insert_with(Settings::new);
                for flag in flags {
                    if !s.has(flag) {
                        s.flag(flag);
                    }
                }
            }
            // New nodes report the table's line in diagnostics.
            for r in &rendered {
                if matches!(r.data().value, NodeValue::HtmlBlock(_)) {
                    set_line(r, line);
                }
            }
            replace(&kids[i..=end], &rendered);
            let mut last = *rendered.last().expect("render always returns the table");
            if let Some(settings) = &settings {
                let comment = node(arena, NodeValue::HtmlBlock(NodeHtmlBlock { block_type: 2, literal: settings.to_comment() }));
                last.insert_after(comment);
                last = comment;
            }
            i = container.children().position(|c| std::ptr::eq(c, last)).expect("just inserted") + 1;
        }
    }
}

/// A GFM table for an HTML table GFM can express (no spans, colours or per-cell widths;
/// each cell at most one paragraph; header cells only in the first row and/or column),
/// with the flags its settings need. Matches fetch's choice (to_md.rs, `TableShape`).
fn set_line(n: &AstNode, line: usize) {
    let mut data = n.data_mut();
    data.sourcepos.start.line = line;
    data.sourcepos.end.line = line;
}

fn as_gfm<'a>(
    arena: &'a Arena<'a>,
    table: &html_table::Table<'a>,
    line: usize,
) -> Option<(&'a AstNode<'a>, Vec<&'static str>)> {
    let ncols = table.rows.first()?.len();
    let header_row = table.rows[0].iter().all(|c| c.header);
    let header_column = table.rows.len() > 1 && table.rows[1..].iter().all(|r| r.first().is_some_and(|c| c.header));
    for (r, row) in table.rows.iter().enumerate() {
        if row.len() != ncols {
            return None;
        }
        for (c, cell) in row.iter().enumerate() {
            let expected = (r == 0 && header_row) || (c == 0 && header_column);
            let simple_content = match cell.content.as_slice() {
                [] => true,
                [p] => {
                    matches!(p.data().value, NodeValue::Paragraph)
                        && !p.last_child().is_some_and(|l| {
                            matches!(&l.data().value, NodeValue::HtmlInline(h) if Settings::parse_comment(h).is_some())
                        })
                }
                _ => false,
            };
            if cell.header != expected || cell.colspan != 1 || cell.rowspan != 1 || cell.background.is_some()
                || cell.colwidth.is_some() || !simple_content
            {
                return None;
            }
        }
    }
    let gfm = node(
        arena,
        NodeValue::Table(Box::new(NodeTable {
            alignments: vec![TableAlignment::None; ncols],
            num_columns: ncols,
            num_rows: table.rows.len(),
            num_nonempty_cells: table.rows.iter().flatten().filter(|c| !c.content.is_empty()).count(),
        })),
    );
    set_line(gfm, line);
    for (r, row) in table.rows.iter().enumerate() {
        let tr = node(arena, NodeValue::TableRow(r == 0));
        set_line(tr, line);
        gfm.append(tr);
        for cell in row {
            let td = node(arena, NodeValue::TableCell);
            set_line(td, line);
            tr.append(td);
            let Some(p) = cell.content.first() else { continue };
            for k in p.children().collect::<Vec<_>>() {
                // A GFM cell is one line.
                let replacement = match k.data().value {
                    NodeValue::LineBreak => Some(NodeValue::HtmlInline("<br>".into())),
                    NodeValue::SoftBreak => Some(NodeValue::Text(" ".into())),
                    _ => None,
                };
                match replacement {
                    Some(v) => td.append(node(arena, v)),
                    None => td.append(k),
                }
            }
        }
    }
    let mut flags = Vec::new();
    if !header_row {
        flags.push("no-header-row");
    }
    if header_column {
        flags.push("header-column");
    }
    Some((gfm, flags))
}

/// Put `new` where `old` is: cell content in `old` is moved, the rest removed.
fn replace<'a>(old: &[&'a AstNode<'a>], new: &[&'a AstNode<'a>]) {
    let first = old[0];
    for n in new {
        first.insert_before(n);
    }
    for o in old {
        if !new.iter().any(|n| std::ptr::eq(*n, *o)) {
            o.detach();
        }
    }
}

/// A GFM table with HTML lists in its cells becomes an HTML table (a GFM cell can't hold a
/// list).
fn gfm_tables_with_lists<'a>(arena: &'a Arena<'a>, root: &'a AstNode<'a>, diags: &mut Vec<Diagnostic>) {
    let tables: Vec<_> = root.descendants().filter(|n| matches!(n.data().value, NodeValue::Table(_))).collect();
    for t in tables {
        let has_list = t.descendants().any(|n| match &n.data().value {
            NodeValue::HtmlInline(h) => {
                let l = h.trim_start().to_ascii_lowercase();
                l.starts_with("<ul") || l.starts_with("<ol")
            }
            _ => false,
        });
        if !has_list {
            continue;
        }
        diags.push(Diagnostic::warning(
            t.data().sourcepos.start.line,
            "list in a table cell: table written as an HTML table",
        ));
        let mut rows = Vec::new();
        for row in t.children() {
            let header = matches!(row.data().value, NodeValue::TableRow(true));
            let mut cells = Vec::new();
            for cell in row.children() {
                // The cell's inline content as markdown text, then as blocks.
                let doc = node(arena, NodeValue::Document);
                let p = node(arena, NodeValue::Paragraph);
                doc.append(p);
                for k in cell.children().collect::<Vec<_>>() {
                    p.append(k);
                }
                let md = html_table::cell_markdown(markdown::render(doc).trim());
                cells.push(html_table::Cell {
                    header,
                    colspan: 1,
                    rowspan: 1,
                    background: None,
                    colwidth: None,
                    content: html_table::parse_at_line(arena, &md, cell.data().sourcepos.start.line),
                });
            }
            rows.push(cells);
        }
        let rendered = html_table::render(arena, &html_table::Table { rows, settings: None });
        for r in &rendered {
            if matches!(r.data().value, NodeValue::HtmlBlock(_)) {
                set_line(r, t.data().sourcepos.start.line);
            }
        }
        replace(&[t], &rendered);
    }
}

pub fn is_details_open(literal: &str) -> bool {
    let l = literal.trim_start().to_ascii_lowercase();
    l.starts_with("<details>") || l.starts_with("<details ")
}

/// The canonical opening line of an expand.
pub fn details_open(title: &str) -> String {
    let escaped = title.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    format!("<details><summary>{escaped}</summary>")
}

/// The title in a canonical `<details><summary>...</summary>` line.
pub fn details_title(literal: &str) -> Option<String> {
    let inner = literal.trim().strip_prefix("<details><summary>")?.strip_suffix("</summary>")?;
    Some(unescape(inner))
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&#39;", "'").replace("&amp;", "&")
}

fn strip_tags(s: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    out
}

fn find_ci(haystack: &str, needle: &str) -> Option<usize> {
    haystack.to_ascii_lowercase().find(needle)
}

/// Rewrite a `<details>` HTML block to the canonical one-line form
/// `<details><summary>Title</summary>`, moving any body written in the same block (no
/// blank lines) out as markdown, followed by its own `</details>`.
fn details<'a>(arena: &'a Arena<'a>, n: &'a AstNode<'a>, literal: &str, diags: &mut Vec<Diagnostic>) {
    let line = n.data().sourcepos.start.line;
    let open_end = literal.find('>').map_or(literal.len(), |i| i + 1);
    if literal[..open_end].to_ascii_lowercase().contains(" open") {
        diags.push(Diagnostic::warning(line, "`<details open>`: Confluence expands always start closed"));
    }
    let mut rest = &literal[open_end..];
    let mut title = String::new();
    if rest.trim_start().to_ascii_lowercase().starts_with("<summary") {
        let start = rest.find('>').map_or(rest.len(), |i| i + 1);
        match find_ci(rest, "</summary>") {
            Some(end) if end >= start => {
                title = unescape(strip_tags(&rest[start..end]).trim());
                rest = &rest[end + "</summary>".len()..];
            }
            _ => {}
        }
    }
    let mut body = rest.trim();
    let closed = body.to_ascii_lowercase().ends_with("</details>");
    if closed {
        body = body[..body.len() - "</details>".len()].trim();
    }
    if let NodeValue::HtmlBlock(h) = &mut n.data_mut().value {
        h.literal = details_open(&title);
    }
    let mut after = n;
    if !body.is_empty() {
        let doc = parse_document(arena, body, &options());
        let kids: Vec<_> = doc.children().collect();
        for kid in kids {
            kid.detach();
            after.insert_after(kid);
            after = kid;
        }
    }
    if closed {
        let close = node(arena, NodeValue::HtmlBlock(NodeHtmlBlock { block_type: 6, literal: "</details>".into() }));
        after.insert_after(close);
    }
}

/// What an inline HTML tag maps to.
enum Tag {
    /// rfluence's own forms (`<u>`, `<sub>`, color spans, `<span data-adf>`, `rf:`
    /// comments) and `<br>`: left alone.
    Keep,
    Code,
    Wrap(NodeValue),
    /// Written as rfluence's `<u>`.
    Underline,
    Drop,
}

fn classify(html: &str, in_table: bool) -> (Tag, String) {
    let h = html.trim();
    let lower = h.to_ascii_lowercase();
    if lower.starts_with("<!--") {
        let rf = crate::settings::Settings::parse_comment(h).is_some();
        return (if rf { Tag::Keep } else { Tag::Drop }, "<!--".into());
    }
    let name: String = lower.trim_start_matches(['<', '/']).chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
    let tag = match name.as_str() {
        "u" | "sub" | "sup" => Tag::Keep,
        "span" if lower.starts_with("</") || lower.starts_with("<span data-adf=") || lower.starts_with("<span style=") => {
            Tag::Keep
        }
        "br" if in_table => Tag::Keep,
        "br" => Tag::Wrap(NodeValue::LineBreak),
        "kbd" | "code" | "samp" | "tt" => Tag::Code,
        "b" | "strong" => Tag::Wrap(NodeValue::Strong),
        "i" | "em" => Tag::Wrap(NodeValue::Emph),
        "s" | "del" | "strike" => Tag::Wrap(NodeValue::Strikethrough),
        "ins" => Tag::Underline,
        _ => Tag::Drop,
    };
    (tag, name)
}

fn is_close(html: &str) -> bool {
    html.trim_start().starts_with("</")
}

/// Map inline HTML tags among `container`'s children to markdown (`<kbd>x</kbd>` -> `` `x` ``,
/// `<b>x</b>` -> `**x**`), and drop tags with no equivalent, keeping their text.
fn inline_html<'a>(arena: &'a Arena<'a>, container: &'a AstNode<'a>, diags: &mut Vec<Diagnostic>) {
    let in_table = matches!(container.data().value, NodeValue::TableCell)
        || container.ancestors().any(|a| matches!(a.data().value, NodeValue::TableCell));
    let line = container.data().sourcepos.start.line;
    let mut i = 0;
    loop {
        let kids: Vec<_> = container.children().collect();
        let Some(&n) = kids.get(i) else { break };
        let NodeValue::HtmlInline(html) = n.data().value.clone() else {
            i += 1;
            continue;
        };
        let (tag, name) = classify(&html, in_table);
        let closing = is_close(&html);
        match tag {
            Tag::Keep => {}
            Tag::Wrap(NodeValue::LineBreak) => {
                n.insert_before(node(arena, NodeValue::LineBreak));
                n.detach();
                diags.push(Diagnostic::warning(line, "`<br>` written as a markdown line break"));
                continue;
            }
            Tag::Drop => {
                n.detach();
                if !closing {
                    diags.push(Diagnostic::warning(line, format!("inline HTML `{}` dropped, text kept", html.trim())));
                }
                continue;
            }
            Tag::Code | Tag::Wrap(_) | Tag::Underline if closing => {
                // A closing tag without its opening tag.
                n.detach();
                continue;
            }
            Tag::Code | Tag::Wrap(_) | Tag::Underline => {
                // Find the matching closing tag among the following siblings.
                let mut depth = 0;
                let mut close = None;
                for (j, k) in kids.iter().enumerate().skip(i + 1) {
                    if let NodeValue::HtmlInline(h) = &k.data().value {
                        let (_, kname) = classify(h, in_table);
                        if kname == name {
                            if !is_close(h) {
                                depth += 1;
                            } else if depth == 0 {
                                close = Some(j);
                                break;
                            } else {
                                depth -= 1;
                            }
                        }
                    }
                }
                let Some(j) = close else {
                    n.detach();
                    diags.push(Diagnostic::warning(line, format!("`{}` without a closing tag dropped", html.trim())));
                    continue;
                };
                let inner: Vec<_> = kids[i + 1..j].to_vec();
                let shown = html.trim().to_string();
                match tag {
                    Tag::Code => {
                        let text: String = inner.iter().map(|k| markdown::inline_text(k)).collect();
                        if !text.is_empty() {
                            n.insert_before(node(arena, NodeValue::Code(NodeCode { num_backticks: 1, literal: text })));
                        }
                        for k in &inner {
                            k.detach();
                        }
                        diags.push(Diagnostic::warning(line, format!("`{shown}` written as inline code")));
                    }
                    Tag::Wrap(value) => {
                        let wrapper = node(arena, value);
                        n.insert_before(wrapper);
                        for k in &inner {
                            k.detach();
                            wrapper.append(k);
                        }
                        diags.push(Diagnostic::warning(line, format!("`{shown}` written as markdown formatting")));
                    }
                    Tag::Underline => {
                        n.insert_before(node(arena, NodeValue::HtmlInline("<u>".into())));
                        kids[j].insert_before(node(arena, NodeValue::HtmlInline("</u>".into())));
                        diags.push(Diagnostic::warning(line, format!("`{shown}` written as `<u>`")));
                    }
                    _ => unreachable!(),
                }
                kids[j].detach();
                n.detach();
                continue;
            }
        }
        i += 1;
    }
}
