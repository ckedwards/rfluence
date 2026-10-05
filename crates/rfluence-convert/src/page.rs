//! A whole fetched page: frontmatter, title, body. See design.md, "Frontmatter" and
//! "Simplified output".

use crate::adf::Node;
use crate::markdown;
use crate::to_md::{FetchContext, adf_to_markdown};

/// What fetch knows about a page besides its body.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PageMeta {
    pub id: String,
    pub title: String,
    pub space_key: String,
    /// Parent page or folder ID.
    pub parent: Option<String>,
    pub version: u64,
    /// When the current version was saved (ISO 8601).
    pub updated: Option<String>,
    /// `https://<site>/wiki/spaces/<KEY>/pages/<id>`.
    pub url: String,
    pub labels: Vec<String>,
}

/// The page as markdown: frontmatter, then the body, with the title as a leading H1 unless
/// the body already starts with one (then the title goes in the frontmatter).
pub fn page_markdown(doc: &Node, meta: &PageMeta, ctx: &FetchContext) -> String {
    let body = adf_to_markdown(doc, ctx);
    let starts_with_h1 = starts_with_h1(doc);
    let frontmatter = if ctx.simplified { simplified_frontmatter(meta) } else { frontmatter(meta, starts_with_h1) };
    let title = if starts_with_h1 { String::new() } else { format!("{}\n", markdown::heading(1, &meta.title)) };
    format!("{frontmatter}\n{title}{body}")
}

/// Does the body start with an H1 (ignoring empty paragraphs)? Then fetch writes the title
/// in the frontmatter, since a leading H1 in the markdown would be taken as the title.
pub fn starts_with_h1(doc: &Node) -> bool {
    doc.content
        .iter()
        .find(|n| !(n.is("paragraph") && n.content.is_empty()))
        .is_some_and(|n| n.is("heading") && n.attr_f64("level") == Some(1.0))
}

/// The page title for upload, and the body to upload: the frontmatter `title` if set (the
/// body as is), else the body's leading H1, which is removed (so Confluence doesn't show the
/// title twice). `None` if neither. See design.md, "Frontmatter" > "Title".
pub fn upload_title<'a>(body: &'a str, frontmatter_title: Option<&str>) -> (Option<String>, &'a str) {
    if let Some(t) = frontmatter_title.filter(|t| !t.trim().is_empty()) {
        return (Some(t.trim().to_string()), body);
    }
    let arena = comrak::Arena::new();
    let root = comrak::parse_document(&arena, body, &markdown::options());
    let Some(first) = root.children().find(|n| !matches!(n.data().value, comrak::nodes::NodeValue::FrontMatter(_))) else {
        return (None, body);
    };
    let level = match first.data().value {
        comrak::nodes::NodeValue::Heading(h) => h.level,
        _ => 0,
    };
    if level != 1 {
        return (None, body);
    }
    let title = markdown::inline_text(first).trim().to_string();
    let end_line = first.data().sourcepos.end.line;
    let offset: usize = body.split_inclusive('\n').take(end_line).map(str::len).sum();
    (Some(title), body[offset.min(body.len())..].trim_start_matches('\n'))
}

/// The `rfluence:` frontmatter block, in the key order of design.md, "Frontmatter".
/// `title` is written only when the body starts with its own H1.
pub fn frontmatter(meta: &PageMeta, title_key: bool) -> String {
    let mut out = String::from("---\nrfluence:\n");
    field(&mut out, "id", &quoted(&meta.id));
    field(&mut out, "space_key", &scalar(&meta.space_key));
    if let Some(parent) = &meta.parent {
        field(&mut out, "parent", &quoted(parent));
    }
    if title_key {
        field(&mut out, "title", &scalar(&meta.title));
    }
    field(&mut out, "version", &meta.version.to_string());
    field(&mut out, "url", &scalar(&meta.url));
    field(&mut out, "labels", &list(&meta.labels));
    out.push_str("---\n");
    out
}

/// Frontmatter for simplified output: context for a reader, and `simplified: true` so the
/// file can't be uploaded.
pub fn simplified_frontmatter(meta: &PageMeta) -> String {
    let mut out = String::from("---\nrfluence:\n");
    field(&mut out, "title", &scalar(&meta.title));
    field(&mut out, "url", &scalar(&meta.url));
    field(&mut out, "space_key", &scalar(&meta.space_key));
    if !meta.labels.is_empty() {
        field(&mut out, "labels", &list(&meta.labels));
    }
    if let Some(updated) = &meta.updated {
        field(&mut out, "updated", &scalar(updated));
    }
    field(&mut out, "simplified", "true");
    out.push_str("---\n");
    out
}

fn field(out: &mut String, key: &str, value: &str) {
    out.push_str(&format!("  {key}: {value}\n"));
}

/// A YAML string, quoted (IDs are strings, not numbers).
fn quoted(s: &str) -> String {
    serde_json::to_string(s).expect("strings serialize")
}

/// A YAML scalar, quoted only when YAML would read it as something else.
fn scalar(s: &str) -> String {
    let plain = !s.is_empty()
        && s.chars().all(|c| c.is_alphanumeric() || " _-./:~+()&?,'".contains(c))
        && !s.starts_with(|c: char| c.is_whitespace() || "-?:,'&~".contains(c))
        && !s.ends_with(char::is_whitespace)
        && !s.contains(": ")
        && !s.contains(" #")
        && s.parse::<f64>().is_err()
        && !matches!(s.to_ascii_lowercase().as_str(), "true" | "false" | "yes" | "no" | "on" | "off" | "null" | "~");
    if plain { s.to_string() } else { quoted(s) }
}

fn list(items: &[String]) -> String {
    format!("[{}]", items.iter().map(|i| scalar(i)).collect::<Vec<_>>().join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta() -> PageMeta {
        PageMeta {
            id: "123456".into(),
            title: "Ingestion: overview".into(),
            space_key: "ENG".into(),
            parent: Some("123000".into()),
            version: 7,
            updated: Some("2026-10-04T05:10:00.000Z".into()),
            url: "https://example.atlassian.net/wiki/spaces/ENG/pages/123456".into(),
            labels: vec!["architecture".into(), "ai-generated".into()],
        }
    }

    #[test]
    fn writes_frontmatter_in_design_order() {
        assert_eq!(
            frontmatter(&meta(), true),
            "---\nrfluence:\n  id: \"123456\"\n  space_key: ENG\n  parent: \"123000\"\n  title: \"Ingestion: overview\"\n  version: 7\n  url: https://example.atlassian.net/wiki/spaces/ENG/pages/123456\n  labels: [architecture, ai-generated]\n---\n"
        );
        assert!(simplified_frontmatter(&meta()).contains("  simplified: true\n"));
    }

    #[test]
    fn quotes_yaml_only_when_needed() {
        assert_eq!(scalar("Plain title"), "Plain title");
        for s in ["true", "123", "a: b", "- x", "", "#tag", "ünïcode ok"] {
            let v = scalar(s);
            if s == "ünïcode ok" {
                assert_eq!(v, s);
            } else {
                assert!(v.starts_with('"'), "{s} -> {v}");
            }
        }
    }

    #[test]
    fn takes_the_upload_title() {
        assert_eq!(upload_title("# The Title\n\nBody\n", None), (Some("The Title".into()), "Body\n"));
        assert_eq!(upload_title("The Title\n=========\n\nBody\n", None), (Some("The Title".into()), "Body\n"));
        assert_eq!(upload_title("# H1\n\nBody\n", Some("Set")), (Some("Set".into()), "# H1\n\nBody\n"));
        assert_eq!(upload_title("Intro\n\n# Later H1\n", None), (None, "Intro\n\n# Later H1\n"));
        assert_eq!(upload_title("## H2\n", None), (None, "## H2\n"));
    }

    #[test]
    fn title_heading_unless_the_body_has_one() {
        let ctx = FetchContext::default();
        let doc: Node = serde_json::from_str(r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"Body"}]}]}"#).unwrap();
        let md = page_markdown(&doc, &meta(), &ctx);
        assert!(md.ends_with("---\n\n# Ingestion: overview\n\nBody\n"), "{md}");
        assert!(!md.contains("  title:"));
        let doc: Node = serde_json::from_str(r#"{"type":"doc","content":[{"type":"heading","attrs":{"level":1},"content":[{"type":"text","text":"Other"}]}]}"#).unwrap();
        let md = page_markdown(&doc, &meta(), &ctx);
        assert!(md.contains("  title: \"Ingestion: overview\"\n") && md.ends_with("---\n\n# Other\n"), "{md}");
    }
}
