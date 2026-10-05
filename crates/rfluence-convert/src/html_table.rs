//! HTML tables with markdown in their cells: the form for tables GFM can't express (lists
//! or code in cells, merged or coloured cells, ...). See design.md, "Element mapping".
//!
//! The canonical form, which fetch writes and normalize produces, has each run of tags on
//! its own lines and each cell's content as markdown between blank lines:
//!
//! ```markdown
//! <table>
//! <tr>
//! <th colspan="2">
//!
//! Cell content as **markdown**
//!
//! </th>
//! </tr>
//! </table>
//! ```
//!
//! Upload also accepts HTML tables as LLMs write them (`<table><tr><td>text</td>...` on
//! one line, `<thead>` / `<tbody>`, `<ul><li>` lists in cells); normalize rewrites those
//! into the canonical form.

use comrak::nodes::{AstNode, NodeHtmlBlock, NodeValue};
use comrak::{Arena, parse_document};

use crate::diagnostics::Diagnostic;
use crate::markdown::{node, options};
use crate::settings::{Settings, fmt_num};

/// A parsed HTML table.
pub struct Table<'a> {
    pub rows: Vec<Vec<Cell<'a>>>,
    /// An `rf:` comment written inside the table's closing HTML block.
    pub settings: Option<Settings>,
}

pub struct Cell<'a> {
    pub header: bool,
    pub colspan: usize,
    pub rowspan: usize,
    pub background: Option<String>,
    /// Per-cell widths (`data-colwidth="120,200"`), when they differ from the table's.
    pub colwidth: Option<Vec<f64>>,
    /// The cell's content: markdown blocks.
    pub content: Vec<&'a AstNode<'a>>,
}

/// The opening tag for a cell.
pub fn cell_open(header: bool, colspan: usize, rowspan: usize, background: Option<&str>, colwidth: Option<&[f64]>) -> String {
    let mut tag = String::from(if header { "<th" } else { "<td" });
    if colspan > 1 {
        tag.push_str(&format!(" colspan=\"{colspan}\""));
    }
    if rowspan > 1 {
        tag.push_str(&format!(" rowspan=\"{rowspan}\""));
    }
    if let Some(bg) = background {
        tag.push_str(&format!(" style=\"background-color: {bg}\""));
    }
    if let Some(w) = colwidth {
        let list: Vec<String> = w.iter().map(|x| fmt_num(*x)).collect();
        tag.push_str(&format!(" data-colwidth=\"{}\"", list.join(",")));
    }
    tag.push('>');
    tag
}

pub fn is_table_start(literal: &str) -> bool {
    let l = literal.trim_start().to_ascii_lowercase();
    l.starts_with("<table>") || l.starts_with("<table ")
}

// ---------- tokenizer ----------

#[derive(Debug, Clone)]
enum Token {
    Open { name: String, attrs: Vec<(String, String)>, raw: String },
    Close { name: String, raw: String },
    Comment(String),
    Text(String),
}

impl Token {
    fn raw(&self) -> &str {
        match self {
            Token::Open { raw, .. } | Token::Close { raw, .. } | Token::Comment(raw) | Token::Text(raw) => raw,
        }
    }
}

fn tokenize(s: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let b = s.as_bytes();
    let mut i = 0;
    let mut text_start = 0;
    let flush = |tokens: &mut Vec<Token>, from: usize, to: usize| {
        if to > from {
            tokens.push(Token::Text(s[from..to].to_string()));
        }
    };
    while i < b.len() {
        if b[i] != b'<' {
            i += 1;
            continue;
        }
        if s[i..].starts_with("<!--") {
            let end = s[i..].find("-->").map_or(s.len(), |e| i + e + 3);
            flush(&mut tokens, text_start, i);
            tokens.push(Token::Comment(s[i..end].to_string()));
            i = end;
            text_start = i;
            continue;
        }
        let closing = s[i..].starts_with("</");
        let name_start = i + if closing { 2 } else { 1 };
        let name_len = s[name_start..].bytes().take_while(|c| c.is_ascii_alphanumeric()).count();
        if name_len == 0 {
            i += 1;
            continue;
        }
        // Find the end of the tag, skipping quoted attribute values.
        let mut j = name_start + name_len;
        let mut quote = None;
        while j < b.len() {
            match (quote, b[j]) {
                (None, b'"' | b'\'') => quote = Some(b[j]),
                (Some(q), c) if c == q => quote = None,
                (None, b'>') => break,
                _ => {}
            }
            j += 1;
        }
        let end = (j + 1).min(s.len());
        flush(&mut tokens, text_start, i);
        let name = s[name_start..name_start + name_len].to_ascii_lowercase();
        let raw = s[i..end].to_string();
        if closing {
            tokens.push(Token::Close { name, raw });
        } else {
            let attrs = parse_attrs(&s[name_start + name_len..j]);
            tokens.push(Token::Open { name, attrs, raw });
        }
        i = end;
        text_start = i;
    }
    flush(&mut tokens, text_start, s.len());
    tokens
}

fn parse_attrs(s: &str) -> Vec<(String, String)> {
    let mut attrs = Vec::new();
    let mut chars = s.trim_end_matches('/').chars().peekable();
    loop {
        while chars.peek().is_some_and(|c| c.is_whitespace()) {
            chars.next();
        }
        let name: String = std::iter::from_fn(|| chars.next_if(|c| !c.is_whitespace() && *c != '=')).collect();
        if name.is_empty() {
            break;
        }
        let mut value = String::new();
        if chars.next_if_eq(&'=').is_some() {
            match chars.peek() {
                Some(&q @ ('"' | '\'')) => {
                    chars.next();
                    value = std::iter::from_fn(|| chars.next_if(|c| *c != q)).collect();
                    chars.next();
                }
                _ => value = std::iter::from_fn(|| chars.next_if(|c| !c.is_whitespace())).collect(),
            }
        }
        attrs.push((name.to_ascii_lowercase(), value));
    }
    attrs
}

// ---------- HTML cell content -> markdown ----------

/// Markdown for a cell's content written as HTML (`text <ul><li>a</li></ul>`).
pub fn cell_markdown(html: &str) -> String {
    html_to_markdown(&tokenize(html))
}

/// Markdown for a cell's HTML content as LLMs write it: `<ul>` / `<ol>` / `<li>` become
/// markdown lists and `<p>` paragraphs; other tags stay as inline HTML for the usual
/// mapping (`<code>` -> inline code, ...).
fn html_to_markdown(tokens: &[Token]) -> String {
    let mut out = String::new();
    // (ordered, next number) per open list
    let mut lists: Vec<(bool, usize)> = Vec::new();
    let indent = |lists: &[(bool, usize)]| -> String {
        lists[..lists.len().saturating_sub(1)].iter().map(|(ordered, _)| if *ordered { "   " } else { "  " }).collect()
    };
    let newline = |out: &mut String, blank: bool| {
        let trimmed = out.trim_end_matches([' ', '\t']).len();
        out.truncate(trimmed);
        if out.is_empty() {
            return;
        }
        let want = if blank { "\n\n" } else { "\n" };
        while !out.ends_with(want) {
            out.push('\n');
        }
    };
    for t in tokens {
        match t {
            Token::Open { name, .. } if name == "ul" || name == "ol" => {
                newline(&mut out, lists.is_empty());
                lists.push((name == "ol", 1));
            }
            Token::Close { name, .. } if (name == "ul" || name == "ol") && !lists.is_empty() => {
                lists.pop();
                newline(&mut out, lists.is_empty());
            }
            Token::Open { name, .. } if name == "li" && !lists.is_empty() => {
                newline(&mut out, false);
                out.push_str(&indent(&lists));
                let (ordered, n) = lists.last_mut().expect("checked");
                if *ordered {
                    out.push_str(&format!("{n}. "));
                    *n += 1;
                } else {
                    out.push_str("- ");
                }
            }
            Token::Close { name, .. } if name == "li" => {}
            Token::Open { name, .. } | Token::Close { name, .. } if name == "p" => newline(&mut out, true),
            Token::Text(text) => {
                // Line breaks in HTML source are just spaces.
                let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
                let leading = out.ends_with('\n') || out.ends_with("- ") || out.ends_with(". ") || out.is_empty();
                if !text.is_empty() {
                    let raw = t.raw();
                    if !leading && raw.starts_with(char::is_whitespace) {
                        out.push(' ');
                    }
                    out.push_str(&text);
                    if raw.ends_with(char::is_whitespace) {
                        out.push(' ');
                    }
                }
            }
            other => out.push_str(other.raw()),
        }
    }
    out.trim().to_string()
}

// ---------- parsing ----------

fn is_structure(name: &str) -> bool {
    matches!(name, "table" | "thead" | "tbody" | "tfoot" | "tr" | "td" | "th" | "caption" | "colgroup" | "col")
}

/// The first tag of an HTML block, if it is table structure.
fn starts_with_structure(literal: &str) -> bool {
    match tokenize(literal.trim_start()).first() {
        Some(Token::Open { name, .. } | Token::Close { name, .. }) => is_structure(name),
        _ => false,
    }
}

enum Piece<'a> {
    /// Cell content written as HTML, and the line of the HTML block it's in.
    Html(Vec<Token>, usize),
    Block(&'a AstNode<'a>),
}

struct OpenCell<'a> {
    cell: Cell<'a>,
    pieces: Vec<Piece<'a>>,
}

/// Parse the HTML table starting at `nodes[start]` (an HTML block for which
/// [`is_table_start`] is true). Returns the table and the index of the block holding
/// `</table>`, or `None` (with an error) if it isn't closed.
pub fn parse<'a>(
    arena: &'a Arena<'a>,
    nodes: &[&'a AstNode<'a>],
    start: usize,
    diags: &mut Vec<Diagnostic>,
) -> Option<(Table<'a>, usize)> {
    let line = |n: &AstNode| n.data().sourcepos.start.line;
    let at = line(nodes[start]);
    let mut table = Table { rows: Vec::new(), settings: None };
    let mut row: Option<Vec<Cell<'a>>> = None;
    let mut cell: Option<OpenCell<'a>> = None;
    let mut depth = 0;
    let mut caption = false;
    let mut closed = false;

    for (idx, &n) in nodes.iter().enumerate().skip(start) {
        let literal = match &n.data().value {
            NodeValue::HtmlBlock(h) => Some(h.literal.clone()),
            _ => None,
        };
        let Some(literal) = literal.filter(|l| cell.is_none() || starts_with_structure(l)) else {
            match &mut cell {
                Some(c) => c.pieces.push(Piece::Block(n)),
                None => diags.push(Diagnostic::error(line(n), "content outside a table cell in an HTML table")),
            }
            continue;
        };
        let mut after_close = Vec::new();
        for token in tokenize(&literal) {
            if closed {
                after_close.push(token);
                continue;
            }
            match &token {
                Token::Open { name, attrs, .. } if is_structure(name) => match name.as_str() {
                    "table" => {
                        depth += 1;
                        if depth > 1 {
                            diags.push(Diagnostic::error(line(n), "table inside a table cell can't be represented in Confluence"));
                        }
                    }
                    "tr" => {
                        close_cell(arena, &mut cell, &mut row);
                        close_row(&mut row, &mut table);
                        row = Some(Vec::new());
                    }
                    "td" | "th" => {
                        close_cell(arena, &mut cell, &mut row);
                        row.get_or_insert_with(Vec::new);
                        cell = Some(OpenCell { cell: new_cell(name == "th", attrs, diags, line(n)), pieces: Vec::new() });
                    }
                    "caption" => {
                        caption = true;
                        diags.push(Diagnostic::warning(line(n), "table caption dropped (Confluence tables have no caption)"));
                    }
                    _ => {} // thead, tbody, tfoot, colgroup, col
                },
                Token::Close { name, .. } if is_structure(name) => match name.as_str() {
                    "table" => {
                        depth -= 1;
                        if depth == 0 {
                            close_cell(arena, &mut cell, &mut row);
                            close_row(&mut row, &mut table);
                            closed = true;
                        }
                    }
                    "tr" => {
                        close_cell(arena, &mut cell, &mut row);
                        close_row(&mut row, &mut table);
                    }
                    "td" | "th" => close_cell(arena, &mut cell, &mut row),
                    "caption" => caption = false,
                    _ => {}
                },
                _ if caption => {}
                _ => match &mut cell {
                    Some(c) => match c.pieces.last_mut() {
                        Some(Piece::Html(tokens, _)) => tokens.push(token),
                        _ => c.pieces.push(Piece::Html(vec![token], line(n))),
                    },
                    None => {
                        if !token.raw().trim().is_empty() && !matches!(token, Token::Comment(_)) {
                            diags.push(Diagnostic::warning(line(n), format!("`{}` outside a table cell dropped", token.raw().trim())));
                        }
                    }
                },
            }
        }
        if closed {
            for token in after_close {
                match &token {
                    Token::Comment(c) => match Settings::parse_comment(c) {
                        Some(s) => table.settings = Some(s),
                        None => diags.push(Diagnostic::warning(line(n), "HTML comment after a table dropped")),
                    },
                    t if t.raw().trim().is_empty() => {}
                    t => diags.push(Diagnostic::error(line(n), format!("`{}` after `</table>` in the same HTML block", t.raw().trim()))),
                }
            }
            return Some((table, idx));
        }
    }
    diags.push(Diagnostic::error(at, "`<table>` without a matching `</table>`"));
    None
}

fn new_cell<'a>(header: bool, attrs: &[(String, String)], diags: &mut Vec<Diagnostic>, line: usize) -> Cell<'a> {
    let mut cell = Cell { header, colspan: 1, rowspan: 1, background: None, colwidth: None, content: Vec::new() };
    for (name, value) in attrs {
        match name.as_str() {
            "colspan" => cell.colspan = value.trim().parse().unwrap_or(1).max(1),
            "rowspan" => cell.rowspan = value.trim().parse().unwrap_or(1).max(1),
            "data-colwidth" => cell.colwidth = value.split(',').map(|w| w.trim().parse().ok()).collect(),
            "style" => {
                for decl in value.split(';') {
                    match decl.split_once(':').map(|(p, v)| (p.trim(), v.trim())) {
                        Some(("background-color" | "background", v)) if !v.is_empty() => cell.background = Some(v.to_string()),
                        Some((p, _)) if !p.is_empty() => {
                            diags.push(Diagnostic::warning(line, format!("cell style `{p}` dropped")));
                        }
                        _ => {}
                    }
                }
            }
            other => diags.push(Diagnostic::warning(line, format!("cell attribute `{other}` dropped"))),
        }
    }
    cell
}

fn close_cell<'a>(arena: &'a Arena<'a>, open: &mut Option<OpenCell<'a>>, row: &mut Option<Vec<Cell<'a>>>) {
    let Some(OpenCell { mut cell, pieces }) = open.take() else { return };
    for piece in pieces {
        match piece {
            Piece::Block(n) => cell.content.push(n),
            Piece::Html(tokens, line) => {
                let md = html_to_markdown(&tokens);
                if md.is_empty() {
                    continue;
                }
                cell.content.extend(parse_at_line(arena, &md, line));
            }
        }
    }
    row.get_or_insert_with(Vec::new).push(cell);
}

/// Parse a markdown snippet whose diagnostics should point at `line` in the page.
pub fn parse_at_line<'a>(arena: &'a Arena<'a>, md: &str, line: usize) -> Vec<&'a AstNode<'a>> {
    let doc = parse_document(arena, md, &options());
    for n in doc.descendants() {
        let mut data = n.data_mut();
        data.sourcepos.start.line = line;
        data.sourcepos.end.line = line;
    }
    doc.children().collect()
}

fn close_row<'a>(row: &mut Option<Vec<Cell<'a>>>, table: &mut Table<'a>) {
    if let Some(r) = row.take() {
        if !r.is_empty() {
            table.rows.push(r);
        }
    }
}

// ---------- rendering ----------

/// Grid column of each cell, accounting for colspans and rowspans.
pub fn grid_columns(rows: &[Vec<(usize, usize)>]) -> Vec<Vec<usize>> {
    let mut occupied: Vec<Vec<bool>> = Vec::new();
    let mut out = Vec::new();
    for (r, row) in rows.iter().enumerate() {
        let mut cols = Vec::new();
        let mut c = 0;
        for &(colspan, rowspan) in row {
            while occupied.get(r).and_then(|o| o.get(c)).copied().unwrap_or(false) {
                c += 1;
            }
            cols.push(c);
            for rr in r..r + rowspan {
                if occupied.len() <= rr {
                    occupied.resize(rr + 1, Vec::new());
                }
                if occupied[rr].len() < c + colspan {
                    occupied[rr].resize(c + colspan, false);
                }
                occupied[rr][c..c + colspan].fill(true);
            }
            c += colspan;
        }
        out.push(cols);
    }
    out
}

/// Canonical nodes for a table: tag runs as HTML blocks, cell content moved in between.
pub fn render<'a>(arena: &'a Arena<'a>, table: &Table<'a>) -> Vec<&'a AstNode<'a>> {
    let mut out = Vec::new();
    let mut buf = String::from("<table>");
    let flush = |buf: &mut String, out: &mut Vec<&'a AstNode<'a>>| {
        if !buf.is_empty() {
            out.push(node(arena, NodeValue::HtmlBlock(NodeHtmlBlock { block_type: 6, literal: std::mem::take(buf) })));
        }
    };
    let push = |buf: &mut String, tag: &str| {
        if !buf.is_empty() {
            buf.push('\n');
        }
        buf.push_str(tag);
    };
    for row in &table.rows {
        push(&mut buf, "<tr>");
        for cell in row {
            push(
                &mut buf,
                &cell_open(cell.header, cell.colspan, cell.rowspan, cell.background.as_deref(), cell.colwidth.as_deref()),
            );
            if !cell.content.is_empty() {
                flush(&mut buf, &mut out);
                out.extend(cell.content.iter().copied());
            }
            push(&mut buf, if cell.header { "</th>" } else { "</td>" });
        }
        push(&mut buf, "</tr>");
    }
    push(&mut buf, "</table>");
    flush(&mut buf, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_cell_html_to_markdown() {
        let md = html_to_markdown(&tokenize("Intro <b>bold</b><ul><li>one</li><li>two<ol><li>a</li></ol></li></ul>after"));
        assert_eq!(md, "Intro <b>bold</b>\n\n- one\n- two\n  1. a\n\nafter");
    }

    #[test]
    fn grid_accounts_for_spans() {
        // | A (rowspan 2) | B | C |
        // |               | D | E |
        assert_eq!(grid_columns(&[vec![(1, 2), (1, 1), (1, 1)], vec![(1, 1), (1, 1)]]), vec![vec![0, 1, 2], vec![1, 2]]);
        assert_eq!(grid_columns(&[vec![(2, 1), (1, 1)], vec![(1, 1), (1, 1), (1, 1)]]), vec![vec![0, 2], vec![0, 1, 2]]);
    }

    #[test]
    fn parses_attributes() {
        let t = tokenize(r#"<td colspan="2" style='background-color: #fff' data-x=y>"#);
        let Token::Open { attrs, .. } = &t[0] else { panic!() };
        assert_eq!(attrs[0], ("colspan".into(), "2".into()));
        assert_eq!(attrs[1], ("style".into(), "background-color: #fff".into()));
        assert_eq!(attrs[2], ("data-x".into(), "y".into()));
    }
}
