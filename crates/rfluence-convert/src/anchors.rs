//! Heading anchors: Confluence style (`#Install-&-Setup-(v2.0)`) and GitHub style
//! (`#install--setup-v20`). See design.md, "Links" > "Heading anchors".

use std::collections::HashMap;

/// Confluence's anchor for a heading: trimmed, each space replaced by `-`, everything else
/// kept. Duplicates get `.1`, `.2`, ... (verified in the browser on page 295349).
fn confluence_base(text: &str) -> String {
    text.trim().replace(' ', "-")
}

/// The anchors of a document's headings, in both styles, in document order.
#[derive(Debug, Default)]
pub struct Anchors {
    confluence_to_github: HashMap<String, String>,
    github_to_confluence: HashMap<String, String>,
}

impl Anchors {
    /// Build from the plain text of each heading, in document order.
    pub fn new<'a>(headings: impl IntoIterator<Item = &'a str>) -> Self {
        let mut anchors = Anchors::default();
        let mut github = comrak::Anchorizer::new();
        let mut seen: HashMap<String, usize> = HashMap::new();
        for text in headings {
            let base = confluence_base(text);
            let n = seen.entry(base.clone()).or_insert(0);
            let confluence = if *n == 0 {
                base.clone()
            } else {
                format!("{base}.{n}")
            };
            *n += 1;
            // Markdown trims heading text, so the GitHub anchor is computed from the trimmed text.
            let gh = github.anchorize(text.trim());
            anchors
                .confluence_to_github
                .insert(confluence.clone(), gh.clone());
            anchors.github_to_confluence.insert(gh, confluence);
        }
        anchors
    }

    /// GitHub-style anchor for a Confluence anchor. Percent-encoded anchors (from the
    /// browser's "copy link") are decoded first.
    pub fn to_github(&self, confluence: &str) -> Option<&str> {
        let decoded = percent_encoding::percent_decode_str(confluence).decode_utf8_lossy();
        self.confluence_to_github
            .get(decoded.as_ref())
            .map(String::as_str)
    }

    /// Confluence anchor (raw, not percent-encoded) for a GitHub-style anchor.
    pub fn to_confluence(&self, github: &str) -> Option<&str> {
        let decoded = percent_encoding::percent_decode_str(github).decode_utf8_lossy();
        self.github_to_confluence
            .get(decoded.as_ref())
            .map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_page_295349() {
        let a = Anchors::new([
            "Install & Setup (v2.0)",
            "FAQ: What's new?",
            "Ünïcode Héading",
            "Duplicate",
            "Duplicate",
            "  Extra   spaces  ",
        ]);
        assert_eq!(
            a.to_github("Install-&-Setup-(v2.0)"),
            Some("install--setup-v20")
        );
        assert_eq!(a.to_github("FAQ%3A-What's-new%3F"), Some("faq-whats-new"));
        assert_eq!(a.to_github("Ünïcode-Héading"), Some("ünïcode-héading"));
        assert_eq!(a.to_github("Duplicate.1"), Some("duplicate-1"));
        assert_eq!(a.to_github("Extra---spaces"), Some("extra---spaces"));
        assert_eq!(a.to_confluence("duplicate-1"), Some("Duplicate.1"));
        assert_eq!(a.to_confluence("missing"), None);
    }
}
