//! Parts of a fetched page, so a large page doesn't flood an LLM's context: `rfluence fetch
//! --section` and `--max-chars`. The result is marked `partial: true` in its frontmatter, so
//! it can't be uploaded (it would delete the rest of the page).

use comrak::nodes::NodeValue;
use comrak::{Arena, parse_document};

use crate::markdown::{inline_text, options};

/// Split `---` frontmatter (including its closing line and the blank line after) from the body.
pub fn split_frontmatter(md: &str) -> (&str, &str) {
    if let Some(rest) = md.strip_prefix("---\n") {
        if let Some(end) = rest.find("\n---\n") {
            let split = 4 + end + 5;
            let body_start = if md[split..].starts_with('\n') { split + 1 } else { split };
            return (&md[..split], &md[body_start..]);
        }
    }
    ("", md)
}

/// Add `partial: true` to the `rfluence:` frontmatter.
pub fn mark_partial(frontmatter: &str) -> String {
    match frontmatter.strip_suffix("---\n") {
        Some(head) if !frontmatter.is_empty() => format!("{head}  partial: true\n---\n"),
        _ => frontmatter.to_string(),
    }
}

struct Heading {
    level: u8,
    text: String,
    anchor: String,
    line: usize,
}

fn headings(body: &str) -> Vec<Heading> {
    let arena = Arena::new();
    let root = parse_document(&arena, body, &options());
    let mut anchorizer = comrak::Anchorizer::new();
    root.children()
        .filter_map(|n| match n.data().value {
            NodeValue::Heading(h) => {
                let text = inline_text(n).trim().to_string();
                Some(Heading { level: h.level, anchor: anchorizer.anchorize(&text), text, line: n.data().sourcepos.start.line })
            }
            _ => None,
        })
        .collect()
}

/// The headings of a body, for listing what `--section` can select.
pub fn heading_titles(body: &str) -> Vec<String> {
    headings(body).into_iter().map(|h| h.text).collect()
}

/// The part of `body` under the heading named `query` (its text, case-insensitive, or its
/// GitHub-style anchor), up to the next heading of the same or a higher level.
pub fn section(body: &str, query: &str) -> Option<String> {
    let q = query.trim().trim_start_matches('#').trim();
    let all = headings(body);
    let i = all.iter().position(|h| h.text.eq_ignore_ascii_case(q) || h.anchor == q.to_lowercase())?;
    let start = all[i].line;
    let end = all[i + 1..].iter().find(|h| h.level <= all[i].level).map(|h| h.line);
    let lines: Vec<&str> = body.lines().collect();
    let slice = &lines[start - 1..end.map_or(lines.len(), |e| e - 1)];
    Some(slice.join("\n").trim_end().to_string() + "\n")
}

/// `body` cut to at most about `max_chars` characters at a block boundary, with a note
/// listing the sections to read the rest. `None` if it already fits.
pub fn truncate(body: &str, max_chars: usize) -> Option<String> {
    let total = body.chars().count();
    if total <= max_chars {
        return None;
    }
    let arena = Arena::new();
    let root = parse_document(&arena, body, &options());
    let lines: Vec<&str> = body.lines().collect();
    // The last top-level block that ends within the limit.
    let mut cut_line = 0;
    for block in root.children() {
        let end = block.data().sourcepos.end.line;
        let chars: usize = lines[..end.min(lines.len())].iter().map(|l| l.chars().count() + 1).sum();
        if chars > max_chars {
            break;
        }
        cut_line = end;
    }
    let mut kept = if cut_line > 0 {
        lines[..cut_line].join("\n")
    } else {
        // Even the first block is too long: cut inside it, at a line.
        let mut out = String::new();
        for line in &lines {
            if out.chars().count() + line.chars().count() + 1 > max_chars {
                break;
            }
            out.push_str(line);
            out.push('\n');
        }
        out
    };
    let shown = kept.chars().count();
    let titles = heading_titles(body);
    let sections = if titles.is_empty() { String::new() } else { format!(" Sections: {}.", titles.join("; ")) };
    kept = kept.trim_end().to_string();
    kept.push_str(&format!(
        "\n\n[Truncated: showing {shown} of {total} characters. Use --section to read one section.{sections}]\n"
    ));
    Some(kept)
}

/// Do two page bodies have the same content? For `rfluence fetch -o`'s check for local
/// edits: compared in normalized form, ignoring link destinations (which change when files
/// are added to the project) and image folders (which follow the file's name).
pub fn same_content(a: &str, b: &str) -> bool {
    comparable(a) == comparable(b)
}

fn comparable(md: &str) -> String {
    let arena = Arena::new();
    let root = parse_document(&arena, md, &options());
    for n in root.descendants() {
        match &mut n.data_mut().value {
            NodeValue::Link(l) => l.url.clear(),
            NodeValue::Image(l) => l.url = l.url.rsplit('/').next().unwrap_or_default().to_string(),
            _ => {}
        }
    }
    crate::normalize(&crate::markdown::render(root))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = "---\nrfluence:\n  id: \"1\"\n---\n\n# Title\n\nIntro.\n\n## Install & Setup\n\nStep one.\n\n### Detail\n\nMore.\n\n## FAQ\n\nAnswers.\n";

    #[test]
    fn splits_frontmatter() {
        let (fm, body) = split_frontmatter(PAGE);
        assert_eq!(fm, "---\nrfluence:\n  id: \"1\"\n---\n");
        assert!(body.starts_with("# Title"));
        assert_eq!(mark_partial(fm), "---\nrfluence:\n  id: \"1\"\n  partial: true\n---\n");
        assert_eq!(split_frontmatter("# No frontmatter\n"), ("", "# No frontmatter\n"));
    }

    #[test]
    fn selects_a_section_with_its_subsections() {
        let (_, body) = split_frontmatter(PAGE);
        assert_eq!(section(body, "install & setup").unwrap(), "## Install & Setup\n\nStep one.\n\n### Detail\n\nMore.\n");
        assert_eq!(section(body, "#install--setup").unwrap(), section(body, "Install & Setup").unwrap());
        assert_eq!(section(body, "FAQ").unwrap(), "## FAQ\n\nAnswers.\n");
        assert!(section(body, "missing").is_none());
        assert_eq!(heading_titles(body), ["Title", "Install & Setup", "Detail", "FAQ"]);
    }

    #[test]
    fn compares_content_ignoring_links_and_image_folders() {
        let fetched = "# T\n\nSee [setup](https://x/wiki/spaces/E/pages/2).\n\n![a](page.assets/a.png)\n";
        assert!(same_content(fetched, "# T\n\nSee [setup](./setup.md).\n\n![a](renamed.assets/a.png)\n"));
        assert!(same_content(fetched, "T\n=\n\nSee [setup](./setup.md).\n\n![a](page.assets/a.png)\n\n\n"));
        assert!(!same_content(fetched, "# T\n\nSee [setup](./setup.md) now.\n\n![a](page.assets/a.png)\n"));
    }

    #[test]
    fn truncates_at_a_block_boundary() {
        let (_, body) = split_frontmatter(PAGE);
        assert!(truncate(body, 10_000).is_none());
        let cut = truncate(body, 30).unwrap();
        assert!(cut.starts_with("# Title\n\nIntro.\n\n[Truncated: showing"), "{cut}");
        assert!(cut.contains("Sections: Title; Install & Setup; Detail; FAQ."), "{cut}");
    }
}
