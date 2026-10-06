//! The canonical markdown form that round trips are compared in.
//! See design.md, "What 'same content' means".

use comrak::nodes::{AstNode, ListDelimType, NodeValue, TableAlignment};
use comrak::{Arena, parse_document};

use crate::inline::{self, Leaf};
use crate::markdown::{self, options};
use crate::settings::Settings;
use crate::{adf, emoji, language};

/// Normalize markdown: comrak's CommonMark rendering, plus rfluence's canonicalization of
/// things Confluence stores differently (emoji aliases, code languages, left alignment).
pub fn normalize(md: &str) -> String {
    let opts = options();
    let arena = Arena::new();
    let root = parse_document(&arena, md, &opts);
    crate::approx::approximate(&arena, root, &mut Vec::new());
    canonicalize(&arena, root);
    markdown::strip_trailing_whitespace(&markdown::render(root))
}

fn canonicalize<'a>(arena: &'a Arena<'a>, root: &'a AstNode<'a>) {
    let containers: Vec<_> = root
        .descendants()
        .filter(|n| matches!(n.data().value, NodeValue::Paragraph | NodeValue::Heading(_) | NodeValue::TableCell))
        .collect();
    for container in containers {
        let children: Vec<_> = container.children().collect();
        let mut items = inline::flatten(&children, &[]);
        for item in &mut items {
            match &mut item.leaf {
                // Emoji as characters, shortcodes included (design.md, "Emoji").
                Leaf::Text(t) => *t = emoji::canonical_text(t),
                // ADF paragraphs have no soft breaks: a wrapped line is one line.
                Leaf::SoftBreak => item.leaf = Leaf::Text(" ".into()),
                _ => {}
            }
        }
        for child in children {
            child.detach();
        }
        inline::build(arena, container, &items);
    }
    for node in root.descendants() {
        let mut data = node.data_mut();
        match &mut data.value {
            // `rf:` comments in one spacing and key format.
            NodeValue::HtmlBlock(h) => {
                if let Some(settings) = Settings::parse_comment(&h.literal) {
                    h.literal = settings.to_comment();
                }
            }
            NodeValue::HtmlInline(h) => {
                if let Some(settings) = Settings::parse_comment(h) {
                    *h = settings.to_comment();
                }
            }
            NodeValue::CodeBlock(cb) if cb.fenced => {
                cb.info = canonical_info(&cb.info);
                if cb.info == "adf"
                    && let Some(json) = canonical_adf_json(&cb.literal) {
                    cb.literal = json;
                }
            }
            // ADF lists have no loose/tight distinction or marker style.
            NodeValue::List(l) | NodeValue::Item(l) => {
                l.tight = true;
                l.delimiter = ListDelimType::Period;
            }
            // Confluence has no explicit left alignment; `:---` and `---` are the same.
            NodeValue::Table(t) => {
                for a in &mut t.alignments {
                    if *a == TableAlignment::Left {
                        *a = TableAlignment::None;
                    }
                }
            }
            _ => {}
        }
    }
}

/// An ```` ```adf ```` fence's JSON as fetch writes it: one compact line per node, save
/// noise removed. `None` if it isn't valid ADF (upload reports that).
fn canonical_adf_json(literal: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(literal).ok()?;
    let nodes: Vec<adf::Node> = match value {
        serde_json::Value::Array(_) => serde_json::from_value(value).ok()?,
        other => vec![serde_json::from_value(other).ok()?],
    };
    // Several nodes in one fence: keep as written.
    let [mut node] = <[adf::Node; 1]>::try_from(nodes).ok()?;
    adf::strip_noise(&mut node);
    Some(adf::to_compact_json(&node) + "\n")
}

/// A fence info string with the language canonicalized (lowercase, Confluence's renames)
/// and settings re-formatted.
pub fn canonical_info(info: &str) -> String {
    let (lang, rest) = markdown::split_info(info);
    let lang = lang.and_then(language::canonical);
    let settings = Settings::parse(rest).to_string();
    match (lang, settings.is_empty()) {
        (Some(l), true) => l,
        (Some(l), false) => format!("{l} {settings}"),
        (None, _) => settings,
    }
}

#[cfg(test)]
mod tests {
    use super::normalize;

    #[test]
    fn writes_emoji_as_characters() {
        let md = "Yes :+1: and :tada:, 👍🏽, 🎉, ❤, ⛹ and ⛹\u{fe0f}; custom :rfluence:; © 2026\n";
        assert_eq!(normalize(md), "Yes 👍 and 🎉, 👍🏽, 🎉, ❤, ⛹ and ⛹\u{fe0f}; custom :rfluence:; © 2026\n");
        assert_eq!(normalize(&normalize(md)), normalize(md));
    }

    #[test]
    fn canonicalizes_code_languages() {
        assert_eq!(normalize("```Bash\necho\n```\n"), "```shell\necho\n```\n");
        assert_eq!(normalize("```mermaid   theme=dark\nx\n```\n"), "```mermaid theme=dark\nx\n```\n");
    }

    #[test]
    fn canonicalizes_mark_nesting() {
        assert_eq!(normalize("**[a](u)** and [**a**](u)\n"), "[**a**](u) and [**a**](u)\n");
        assert_eq!(normalize("<sup>2</sup> <span style=\"color:#ff5630\">red</span>\n"), "<sup>2</sup> <span style=\"color: #ff5630\">red</span>\n");
    }

    #[test]
    fn drops_left_alignment() {
        assert_eq!(normalize("| a | b |\n|:--|--:|\n| 1 | 2 |\n"), "| a | b |\n| --- | --: |\n| 1 | 2 |\n");
    }

    /// The normalizer on the corpus: fixed point, unchanged AST, `rf:` comments attached.
    mod corpus {
        use std::fmt::Write as _;
        use std::path::PathBuf;

        use comrak::nodes::{AstNode, NodeValue};
        use comrak::{Arena, parse_document};

        use super::super::{canonicalize, normalize};
        use crate::markdown::options;

        fn files() -> Vec<(String, String)> {
            let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/markdown");
            let mut files: Vec<_> = std::fs::read_dir(&dir)
                .unwrap()
                .map(|e| e.unwrap().path())
                .filter(|p| p.extension().is_some_and(|x| x == "md"))
                .map(|p| (p.file_name().unwrap().to_string_lossy().into_owned(), std::fs::read_to_string(&p).unwrap()))
                .collect();
            files.sort();
            assert!(!files.is_empty());
            files
        }

        /// A structural dump of the canonicalized AST, ignoring presentation (bullet char,
        /// setext vs ATX, fence char, how text is split).
        fn fingerprint(md: &str) -> String {
            let arena = Arena::new();
            let root = parse_document(&arena, md, &options());
            // Approximations are intentional changes (design.md, "Checking markdown").
            crate::approx::approximate(&arena, root, &mut Vec::new());
            canonicalize(&arena, root);
            let mut out = String::new();
            dump(root, 0, &mut out);
            out
        }

        fn text_piece<'a>(node: &'a AstNode<'a>) -> Option<String> {
            match &node.data().value {
                NodeValue::Text(t) => Some(t.to_string()),
                _ => None,
            }
        }

        fn dump<'a>(node: &'a AstNode<'a>, depth: usize, out: &mut String) {
            let line = match &node.data().value {
                NodeValue::List(l) => format!("List {:?} start={} tight={} task={}", l.list_type, l.start, l.tight, l.is_task_list),
                NodeValue::Heading(h) => format!("Heading {}", h.level),
                NodeValue::CodeBlock(c) => format!("CodeBlock info={:?} literal={:?}", c.info, c.literal),
                NodeValue::TaskItem(t) => format!("TaskItem checked={}", t.symbol.is_some()),
                NodeValue::HtmlBlock(h) if h.literal.trim_end() == "<!-- end list -->" => return,
                NodeValue::HtmlBlock(h) => format!("HtmlBlock {:?}", h.literal.trim_end()),
                NodeValue::Item(_) => "Item".into(),
                NodeValue::Text(_) => {
                    if node.previous_sibling().and_then(text_piece).is_some() {
                        return;
                    }
                    let mut text = String::new();
                    let mut cur = Some(node);
                    while let Some(t) = cur.and_then(text_piece) {
                        text.push_str(&t);
                        cur = cur.and_then(|n| n.next_sibling());
                    }
                    format!("Text {text:?}")
                }
                other => format!("{other:?}"),
            };
            let _ = writeln!(out, "{}{line}", "  ".repeat(depth));
            for child in node.children() {
                dump(child, depth + 1, out);
            }
        }

        /// Each `<!-- rf: -->` comment and whether it is attached: an inline HTML node right
        /// after an image or link, an HTML block right after a GFM or HTML table, or a
        /// layout marker (a block of its own).
        fn rf_comments(md: &str) -> Vec<(String, bool)> {
            let arena = Arena::new();
            let root = parse_document(&arena, md, &options());
            let mut found = Vec::new();
            for node in root.descendants() {
                let prev = node.previous_sibling().map(|p| p.data().value.clone());
                match &node.data().value {
                    NodeValue::HtmlInline(h) if h.starts_with("<!-- rf:") => found.push((
                        h.clone(),
                        matches!(prev, Some(NodeValue::Image(_) | NodeValue::Link(_))),
                    )),
                    NodeValue::HtmlBlock(h) if h.literal.contains("<!-- rf:") => {
                        let marker = crate::settings::Settings::parse_comment(&h.literal)
                            .is_some_and(|s| s.has("columns") || s.has("column") || s.has("end-columns"));
                        // After an HTML table: in the same HTML block as `</table>`, or the next one.
                        let after_html_table = h.literal.contains("</table>")
                            || matches!(&prev, Some(NodeValue::HtmlBlock(p)) if p.literal.trim_end().ends_with("</table>"));
                        let start = h.literal.find("<!-- rf:").expect("checked");
                        let end = h.literal[start..].find("-->").map_or(h.literal.len(), |e| start + e + 3);
                        found.push((
                            h.literal[start..end].to_string(),
                            marker || after_html_table || matches!(prev, Some(NodeValue::Table(_))),
                        ))
                    }
                    _ => {}
                }
            }
            found
        }

        #[test]
        fn normalize_is_a_fixed_point() {
            for (name, src) in files() {
                let n1 = normalize(&src);
                similar_asserts::assert_eq!(normalize(&n1), n1, "{name}");
            }
        }

        #[test]
        fn normalize_keeps_the_ast() {
            for (name, src) in files() {
                similar_asserts::assert_eq!(fingerprint(&src), fingerprint(&normalize(&src)), "{name}");
            }
        }

        #[test]
        fn rf_comments_stay_attached() {
            for (name, src) in files() {
                let before = rf_comments(&src);
                assert!(before.iter().all(|(_, attached)| *attached), "{name}: {before:?}");
                assert_eq!(rf_comments(&normalize(&src)), before, "{name}");
            }
        }

        #[test]
        fn no_trailing_whitespace_outside_code() {
            for (name, src) in files() {
                let n = normalize(&src);
                let code = n.lines().filter(|l| l.ends_with(' ')).count();
                // Only the edge-cases code block has lines ending in spaces.
                assert!(code <= 2, "{name}: {code} lines with trailing spaces");
            }
        }
    }
}
