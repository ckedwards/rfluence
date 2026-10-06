//! Re-anchoring inline comments on upload. See design.md, "Inline comments".
//!
//! Markdown has no inline comments, so upload puts each `annotation` mark from the page in
//! Confluence back onto the same text in the new body: matched by the annotated text and which
//! occurrence of it it was, else by being the only occurrence. Comments whose text was changed,
//! or can't be placed unambiguously, are reported (they become detached in Confluence).

use std::collections::BTreeMap;

use crate::adf::{Mark, Node};

/// What happened to the page's inline comments.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Reanchored {
    /// Annotated text placed in the new body.
    pub kept: usize,
    /// Annotated text that couldn't be placed (shortened for messages).
    pub lost: Vec<String>,
}

const ATOM: char = '\u{FFFC}';

fn is_inline(n: &Node) -> bool {
    matches!(
        n.kind.as_str(),
        "text"
            | "hardBreak"
            | "emoji"
            | "mention"
            | "status"
            | "date"
            | "inlineCard"
            | "inlineExtension"
            | "placeholder"
            | "mediaInline"
    )
}

/// The document's text as one string: each text-holding block's content, with other inline
/// nodes as U+FFFC (so matches can't run across them) and blocks separated by newlines; and
/// every text node, in document order, with its byte range in that string.
fn index(doc: &Node) -> (String, Vec<(&Node, usize, usize)>) {
    fn visit<'a>(n: &'a Node, text: &mut String, nodes: &mut Vec<(&'a Node, usize, usize)>) {
        if n.content.iter().any(is_inline) {
            for c in &n.content {
                if let Some(t) = c.text.as_deref().filter(|_| c.is("text")) {
                    let start = text.len();
                    text.push_str(t);
                    nodes.push((c, start, text.len()));
                } else if is_inline(c) {
                    text.push(ATOM);
                } else {
                    visit(c, text, nodes);
                }
            }
            text.push('\n');
        } else {
            for c in &n.content {
                visit(c, text, nodes);
            }
        }
    }
    let mut text = String::new();
    let mut nodes = Vec::new();
    visit(doc, &mut text, &mut nodes);
    (text, nodes)
}

/// Put the inline comments in `remote` (the page as it is in Confluence) onto the same text in
/// `new` (the body about to be uploaded).
pub fn reanchor(remote: &Node, new: &mut Node) -> Reanchored {
    let (old_text, old_nodes) = index(remote);
    // Each comment's annotated ranges, joining adjacent text nodes (formatting splits them).
    let mut ranges: BTreeMap<String, (Mark, Vec<(usize, usize)>)> = BTreeMap::new();
    for (node, start, end) in &old_nodes {
        for mark in node.marks.iter().filter(|m| m.kind == "annotation") {
            let Some(id) = mark.attr_str("id") else {
                continue;
            };
            let entry = ranges
                .entry(id.to_string())
                .or_insert_with(|| (mark.clone(), Vec::new()));
            match entry.1.last_mut() {
                Some(last) if last.1 == *start => last.1 = *end,
                _ => entry.1.push((*start, *end)),
            }
        }
    }

    let (new_text, new_nodes) = index(new);
    let spans: Vec<(usize, usize)> = new_nodes.iter().map(|(_, s, e)| (*s, *e)).collect();
    let mut additions: BTreeMap<usize, Vec<(usize, usize, Mark)>> = BTreeMap::new();
    let mut result = Reanchored::default();
    for (mark, list) in ranges.into_values() {
        for (start, end) in list {
            let needle = &old_text[start..end];
            if needle.trim().is_empty() {
                continue;
            }
            let occurrence = old_text[..start].match_indices(needle).count();
            let found: Vec<usize> = new_text.match_indices(needle).map(|(i, _)| i).collect();
            let at = match (found.get(occurrence), found.as_slice()) {
                (Some(&p), _) => p,
                (None, [only]) => *only,
                _ => {
                    result.lost.push(shorten(needle));
                    continue;
                }
            };
            let (from, to) = (at, at + needle.len());
            for (k, (s, e)) in spans.iter().enumerate() {
                if *s < to && from < *e {
                    additions.entry(k).or_default().push((
                        from.max(*s) - s,
                        to.min(*e) - s,
                        mark.clone(),
                    ));
                }
            }
            result.kept += 1;
        }
    }

    // Apply: split text nodes where comments start or end, in the same order as `index`.
    let mut k = 0;
    apply(new, &additions, &mut k);
    result
}

fn apply(n: &mut Node, additions: &BTreeMap<usize, Vec<(usize, usize, Mark)>>, k: &mut usize) {
    if n.content.iter().any(is_inline) {
        let mut content = Vec::with_capacity(n.content.len());
        for c in std::mem::take(&mut n.content) {
            if c.is("text") && c.text.is_some() {
                match additions.get(k) {
                    Some(adds) => content.extend(split(&c, adds)),
                    None => content.push(c),
                }
                *k += 1;
            } else if is_inline(&c) {
                content.push(c);
            } else {
                let mut c = c;
                apply(&mut c, additions, k);
                content.push(c);
            }
        }
        n.content = content;
    } else {
        for c in &mut n.content {
            apply(c, additions, k);
        }
    }
}

/// A text node split at the edges of `adds` (byte ranges within it), each piece with the
/// annotation marks covering it.
fn split(node: &Node, adds: &[(usize, usize, Mark)]) -> Vec<Node> {
    let text = node.text.as_deref().unwrap_or_default();
    let mut cuts: Vec<usize> = vec![0, text.len()];
    for (s, e, _) in adds {
        cuts.extend([*s, *e]);
    }
    cuts.sort_unstable();
    cuts.dedup();
    cuts.windows(2)
        .filter(|w| w[0] < w[1])
        .map(|w| {
            let mut piece = node.clone();
            piece.text = Some(text[w[0]..w[1]].to_string());
            for (s, e, mark) in adds {
                if *s <= w[0] && w[1] <= *e && !piece.marks.contains(mark) {
                    piece.marks.push(mark.clone());
                }
            }
            piece
        })
        .collect()
}

fn shorten(s: &str) -> String {
    let one_line = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() > 60 {
        format!("{}…", one_line.chars().take(59).collect::<String>())
    } else {
        one_line
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(json: &str) -> Node {
        serde_json::from_str(json).unwrap()
    }

    fn annotated(doc: &Node) -> Vec<(String, String)> {
        let mut out = Vec::new();
        doc.walk(&mut |n| {
            if let Some(m) = n.marks.iter().find(|m| m.kind == "annotation") {
                out.push((
                    n.text.clone().unwrap_or_default(),
                    m.attr_str("id").unwrap_or_default().to_string(),
                ));
            }
        });
        out
    }

    fn para(texts: &str) -> String {
        format!(r#"{{"type":"paragraph","content":[{texts}]}}"#)
    }

    const NOTE: &str =
        r#"{"type":"annotation","attrs":{"annotationType":"inlineComment","id":"c1"}}"#;

    #[test]
    fn keeps_a_comment_on_unchanged_text() {
        let remote = doc(&format!(
            r#"{{"type":"doc","content":[{}]}}"#,
            para(&format!(
                r#"{{"type":"text","text":"Line with "}},{{"type":"text","text":"inline comment","marks":[{NOTE}]}}"#
            ))
        ));
        let mut new = doc(&format!(
            r#"{{"type":"doc","content":[{}]}}"#,
            para(r#"{"type":"text","text":"Line with inline comment"}"#)
        ));
        let r = reanchor(&remote, &mut new);
        assert_eq!((r.kept, r.lost.len()), (1, 0));
        assert_eq!(
            annotated(&new),
            [("inline comment".to_string(), "c1".to_string())]
        );
        assert_eq!(
            new.content[0].content[0].text.as_deref(),
            Some("Line with ")
        );
    }

    #[test]
    fn follows_the_occurrence_when_text_is_added_before() {
        // The second "the cat" is commented; a sentence added in front shifts everything.
        let remote = doc(&format!(
            r#"{{"type":"doc","content":[{}]}}"#,
            para(&format!(
                r#"{{"type":"text","text":"the cat. "}},{{"type":"text","text":"the cat","marks":[{NOTE}]}},{{"type":"text","text":"."}}"#
            ))
        ));
        let mut new = doc(&format!(
            r#"{{"type":"doc","content":[{}]}}"#,
            para(r#"{"type":"text","text":"New intro. the cat. the cat."}"#)
        ));
        reanchor(&remote, &mut new);
        let texts: Vec<_> = new.content[0]
            .content
            .iter()
            .map(|n| (n.text.clone().unwrap(), !n.marks.is_empty()))
            .collect();
        assert_eq!(
            texts,
            [
                ("New intro. the cat. ".into(), false),
                ("the cat".into(), true),
                (".".into(), false)
            ]
        );
    }

    #[test]
    fn spans_formatting_changes() {
        // Commented text crossing a bold run, re-uploaded with different formatting.
        let remote = doc(&format!(
            r#"{{"type":"doc","content":[{}]}}"#,
            para(&format!(
                r#"{{"type":"text","text":"very ","marks":[{NOTE}]}},{{"type":"text","text":"bold","marks":[{{"type":"strong"}},{NOTE}]}},{{"type":"text","text":" claim"}}"#
            ))
        ));
        let mut new = doc(&format!(
            r#"{{"type":"doc","content":[{}]}}"#,
            para(
                r#"{"type":"text","text":"very bold","marks":[{"type":"em"}]},{"type":"text","text":" claim"}"#
            )
        ));
        let r = reanchor(&remote, &mut new);
        assert_eq!(r.kept, 1);
        assert_eq!(
            annotated(&new),
            [("very bold".to_string(), "c1".to_string())]
        );
        assert_eq!(
            new.content[0].content[0].marks.len(),
            2,
            "keeps em and adds the annotation"
        );
    }

    #[test]
    fn reports_changed_or_ambiguous_text() {
        let remote = doc(&format!(
            r#"{{"type":"doc","content":[{}]}}"#,
            para(&format!(
                r#"{{"type":"text","text":"x x "}},{{"type":"text","text":"x","marks":[{NOTE}]}}"#
            ))
        ));
        // Changed: gone.
        let mut changed = doc(&format!(
            r#"{{"type":"doc","content":[{}]}}"#,
            para(r#"{"type":"text","text":"y y y"}"#)
        ));
        assert_eq!(reanchor(&remote, &mut changed).lost, ["x"]);
        // The third "x" no longer exists and two remain: ambiguous.
        let mut ambiguous = doc(&format!(
            r#"{{"type":"doc","content":[{}]}}"#,
            para(r#"{"type":"text","text":"x x"}"#)
        ));
        let r = reanchor(&remote, &mut ambiguous);
        assert_eq!((r.kept, r.lost.len()), (0, 1));
        assert!(annotated(&ambiguous).is_empty());
    }

    #[test]
    fn doesnt_match_across_blocks_or_inline_nodes() {
        let remote = doc(&format!(
            r#"{{"type":"doc","content":[{}]}}"#,
            para(&format!(
                r#"{{"type":"text","text":"ab","marks":[{NOTE}]}}"#
            ))
        ));
        let mut new = doc(&format!(
            r#"{{"type":"doc","content":[{},{}]}}"#,
            para(
                r#"{"type":"text","text":"a"},{"type":"emoji","attrs":{"shortName":":x:"}},{"type":"text","text":"b"}"#
            ),
            para(r#"{"type":"text","text":"a"}"#)
        ));
        assert_eq!(reanchor(&remote, &mut new).lost, ["ab"]);
    }
}
