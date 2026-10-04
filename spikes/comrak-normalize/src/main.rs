//! Spike: is comrak's CommonMark renderer a safe normalizer for rfluence round trips?
//!
//! For every file in fixtures/markdown (or the paths given as arguments) this checks:
//!
//!   1. Fixed point:   normalize(normalize(md)) == normalize(md)
//!   2. Lossless:      parse(normalize(md)) has the same AST as parse(md), ignoring
//!                     presentation (bullet char, setext vs ATX, fence char, ...)
//!   3. rf comments:   every `<!-- rf: ... -->` is parsed as an inline HTML node directly
//!                     after an image or link (or an HTML block directly after a table),
//!                     before and after normalizing
//!
//! Normalized output goes to target/normalized/ for inspection. `-v` prints the diff
//! between each source file and its normalized form.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use comrak::nodes::{AstNode, NodeValue};
use comrak::{Arena, Options, format_commonmark, parse_document};
use similar::TextDiff;

fn options() -> Options<'static> {
    let mut o = Options::default();
    o.extension.strikethrough = true;
    o.extension.table = true;
    o.extension.autolink = true;
    o.extension.tasklist = true;
    o.extension.footnotes = true;
    o.extension.alerts = true;
    o.extension.front_matter_delimiter = Some("---".into());
    o.render.r#unsafe = true;
    // Without this, a fence with no info string is written as an indented code block,
    // and two adjacent indented blocks merge into one on the next parse.
    o.render.prefer_fenced = true;
    // Keep `\` escapes in the AST so `\:tada:` (literal text) can be told apart from `:tada:`.
    o.parse.escaped_char_spans = true;
    o
}

/// Stand-in for the bundled emoji catalog.
const TEST_CATALOG: &[&str] = &["rocket", "tada", "thumbsup", "rotating_light"];

/// Does `text` start with `name:` for a name in the catalog?
fn starts_with_shortcode(text: &str) -> bool {
    text.split_once(':').is_some_and(|(name, _)| TEST_CATALOG.contains(&name))
}

fn text_of<'a>(node: &'a AstNode<'a>) -> Option<String> {
    match &node.data().value {
        NodeValue::Text(t) => Some(t.to_string()),
        NodeValue::Escaped => node.first_child().and_then(text_of),
        _ => None,
    }
}

/// An escaped colon that starts a catalog shortcode: `\:tada:` is literal text, not an emoji.
fn is_escaped_shortcode<'a>(node: &'a AstNode<'a>) -> bool {
    matches!(node.data().value, NodeValue::Escaped)
        && text_of(node).as_deref() == Some(":")
        && starts_with_shortcode(&following_text(node))
}

/// The text after `node` up to the next non-text sibling (comrak splits text at escapes).
fn following_text<'a>(node: &'a AstNode<'a>) -> String {
    let mut text = String::new();
    let mut cur = node.next_sibling();
    while let Some(t) = cur.and_then(text_of) {
        text.push_str(&t);
        cur = cur.and_then(|n| n.next_sibling());
    }
    text
}

fn normalize(md: &str, opts: &Options) -> String {
    let arena = Arena::new();
    let root = parse_document(&arena, md, opts);
    // comrak's renderer ignores source escapes and never escapes `:`, so `\:tada:` would
    // come out as `:tada:`. Turn those escapes into raw inline HTML, which is written verbatim.
    let escapes: Vec<_> = root.descendants().filter(|n| is_escaped_shortcode(n)).collect();
    for node in escapes {
        while let Some(child) = node.first_child() {
            child.detach();
        }
        node.data.borrow_mut().value = NodeValue::HtmlInline("\\:".into());
    }
    let mut out = String::new();
    format_commonmark(root, opts, &mut out).expect("format");
    strip_trailing_whitespace(&out, opts)
}

/// comrak leaves trailing spaces on some lines (`> `, blank lines inside list items).
/// Strip them everywhere except inside code and HTML blocks, where they are content.
/// Hard breaks are rendered as `\`, so trailing spaces never carry meaning elsewhere.
fn strip_trailing_whitespace(md: &str, opts: &Options) -> String {
    let arena = Arena::new();
    let root = parse_document(&arena, md, opts);
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

/// A structural dump of the AST that ignores purely presentational choices.
fn fingerprint(md: &str, opts: &Options) -> String {
    let arena = Arena::new();
    let root = parse_document(&arena, md, opts);
    let mut out = String::new();
    dump(root, 0, &mut out);
    out
}

fn dump<'a>(node: &'a AstNode<'a>, depth: usize, out: &mut String) {
    let pad = "  ".repeat(depth);
    let line = match &node.data().value {
        NodeValue::List(l) => format!(
            "List {:?} start={} tight={} task={}",
            l.list_type, l.start, l.tight, l.is_task_list
        ),
        NodeValue::Heading(h) => format!("Heading {}", h.level),
        NodeValue::CodeBlock(c) => format!("CodeBlock info={:?} literal={:?}", c.info, c.literal),
        NodeValue::Alert(a) => format!("Alert {:?} title={:?}", a.alert_type, a.title),
        NodeValue::TaskItem(t) => format!("TaskItem checked={}", t.symbol.is_some()),
        // comrak inserts this between adjacent lists so they don't merge when re-parsed.
        NodeValue::HtmlBlock(h) if h.literal.trim_end() == "<!-- end list -->" => return,
        NodeValue::HtmlBlock(h) => format!("HtmlBlock {:?}", h.literal.trim_end()),
        NodeValue::FootnoteReference(f) => format!("FootnoteReference {:?}", f.name),
        NodeValue::FootnoteDefinition(f) => format!("FootnoteDefinition {:?}", f.name),
        NodeValue::Link(l) => format!("Link url={:?} title={:?}", l.url, l.title),
        NodeValue::Image(l) => format!("Image url={:?} title={:?}", l.url, l.title),
        NodeValue::Item(_) => "Item".into(),
        NodeValue::Text(_) | NodeValue::Escaped => {
            // Adjacent text and escape nodes are an artifact of parsing; merge them. Escapes
            // only matter before a shortcode, where they are kept as `\:`.
            if node.previous_sibling().and_then(text_of).is_some() {
                return;
            }
            let mut text = String::new();
            let mut cur = Some(node);
            while let Some(n) = cur {
                let Some(t) = text_of(n) else { break };
                text.push_str(if is_escaped_shortcode(n) { "\\:" } else { &t });
                cur = n.next_sibling();
            }
            format!("Text {text:?}")
        }
        other => format!("{other:?}"),
    };
    let _ = writeln!(out, "{pad}{line}");
    for child in node.children() {
        dump(child, depth + 1, out);
    }
}

/// Returns a description of each `<!-- rf: -->` comment and whether it is attached
/// (an inline HTML node whose previous sibling is an image or link, or an HTML block
/// directly after a table).
fn rf_comments(md: &str, opts: &Options) -> Vec<(String, bool)> {
    let arena = Arena::new();
    let root = parse_document(&arena, md, opts);
    let mut found = Vec::new();
    for node in root.descendants() {
        let attached = match &node.data().value {
            NodeValue::HtmlInline(html) if html.starts_with("<!-- rf:") => node
                .previous_sibling()
                .is_some_and(|p| matches!(p.data().value, NodeValue::Image(_) | NodeValue::Link(_))),
            // Block-level form: an `rf:` HTML block directly after a table.
            NodeValue::HtmlBlock(h) if h.literal.contains("<!-- rf:") => node
                .previous_sibling()
                .is_some_and(|p| matches!(p.data().value, NodeValue::Table(_))),
            _ => continue,
        };
        let text = match &node.data().value {
            NodeValue::HtmlInline(h) => h.clone(),
            NodeValue::HtmlBlock(h) => h.literal.trim_end().to_string(),
            _ => unreachable!(),
        };
        found.push((text, attached));
    }
    found
}

fn diff(a_name: &str, a: &str, b_name: &str, b: &str) -> String {
    TextDiff::from_lines(a, b)
        .unified_diff()
        .context_radius(2)
        .header(a_name, b_name)
        .to_string()
}

fn main() -> ExitCode {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let args: Vec<String> = std::env::args().skip(1).collect();
    let verbose = args.iter().any(|a| a == "-v");
    let mut files: Vec<PathBuf> = args.iter().filter(|a| *a != "-v").map(PathBuf::from).collect();
    if files.is_empty() {
        let dir = root.join("fixtures/markdown");
        files = std::fs::read_dir(&dir)
            .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
            .map(|e| e.unwrap().path())
            .filter(|p| p.extension().is_some_and(|x| x == "md"))
            .collect();
        files.sort();
    }

    let out_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/normalized");
    std::fs::create_dir_all(&out_dir).unwrap();
    let opts = options();
    let mut failures = 0;

    for path in &files {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let src = std::fs::read_to_string(path).unwrap();
        let n1 = normalize(&src, &opts);
        let n2 = normalize(&n1, &opts);
        std::fs::write(out_dir.join(&name), &n1).unwrap();

        let mut problems = Vec::new();
        if n1 != n2 {
            problems.push(format!(
                "not a fixed point:\n{}",
                diff("normalize(md)", &n1, "normalize(normalize(md))", &n2)
            ));
        }
        let (f_src, f_n1) = (fingerprint(&src, &opts), fingerprint(&n1, &opts));
        if f_src != f_n1 {
            problems.push(format!(
                "AST changed by normalizing:\n{}",
                diff("ast(md)", &f_src, "ast(normalize(md))", &f_n1)
            ));
        }
        let (c_src, c_n1) = (rf_comments(&src, &opts), rf_comments(&n1, &opts));
        for (text, attached) in &c_src {
            if !attached {
                problems.push(format!("rf comment not attached in source: {text}"));
            }
        }
        if c_src != c_n1 {
            problems.push(format!("rf comments changed by normalizing: {c_src:?} -> {c_n1:?}"));
        }

        let changed = src.lines().zip(n1.lines()).filter(|(a, b)| a != b).count()
            + src.lines().count().abs_diff(n1.lines().count());
        if problems.is_empty() {
            println!(
                "ok    {name}  ({changed} lines restyled, {} rf comments attached)",
                c_src.len()
            );
        } else {
            failures += 1;
            println!("FAIL  {name}");
            for p in &problems {
                println!("{}", p.lines().map(|l| format!("      {l}")).collect::<Vec<_>>().join("\n"));
            }
        }
        if verbose && src != n1 {
            println!("{}", diff(&name, &src, &format!("normalized/{name}"), &n1));
        }
    }

    println!("\nnormalized output: {}", out_dir.display());
    if failures == 0 { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}
