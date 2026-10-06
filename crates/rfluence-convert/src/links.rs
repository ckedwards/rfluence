//! Confluence page URLs: which page a link points at. See design.md, "Links".

/// A page URL, resolved: the page ID and the anchor (still percent-encoded, if it was).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageLink {
    pub id: String,
    pub anchor: Option<String>,
}

/// The page a URL on `host` points at: `/wiki/spaces/KEY/pages/ID[/title][#anchor]`,
/// `?pageId=ID`, or a tiny link `/wiki/x/CODE`. Links are matched by page ID only (the space
/// key and title slug change when pages are moved or renamed). `None` for other URLs.
pub fn page_link(url: &str, host: &str) -> Option<PageLink> {
    let (scheme, rest) = url.split_once("://")?;
    if !matches!(scheme, "http" | "https") {
        return None;
    }
    let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
    if !authority.eq_ignore_ascii_case(host) {
        return None;
    }
    let (path, anchor) = match path.split_once('#') {
        Some((p, a)) => (p, (!a.is_empty()).then(|| a.to_string())),
        None => (path, None),
    };
    let digits = |s: &str| -> Option<String> {
        let id: String = s.chars().take_while(char::is_ascii_digit).collect();
        (!id.is_empty()).then_some(id)
    };
    // `/wiki/pages/viewpage.action?pageId=` also contains `/pages/`, so try each form.
    let id = path
        .split("/pages/")
        .nth(1)
        .and_then(digits)
        .or_else(|| path.split("pageId=").nth(1).and_then(digits))
        .or_else(|| {
            let code = path.strip_prefix("wiki/x/")?.split(['/', '?']).next()?;
            Some(tiny_link_id(code)?.to_string())
        })?;
    Some(PageLink { id, anchor })
}

/// The IDs of pages on `host` that a page's smart links (inline, block and embed cards) point
/// at, to look up their titles.
pub fn card_page_ids(doc: &crate::adf::Node, host: &str) -> Vec<String> {
    let mut ids = Vec::new();
    doc.walk(&mut |n| {
        if matches!(n.kind.as_str(), "inlineCard" | "blockCard" | "embedCard")
            && let Some(link) = n.attr_str("url").and_then(|u| page_link(u, host))
            && !ids.contains(&link.id)
        {
            ids.push(link.id);
        }
    });
    ids
}

/// The IDs of pages on `host` that a page links to: text and image links, and smart links.
pub fn linked_page_ids(doc: &crate::adf::Node, host: &str) -> Vec<String> {
    let mut ids = card_page_ids(doc, host);
    doc.walk(&mut |n| {
        for mark in n.marks.iter().filter(|m| m.kind == "link") {
            if let Some(link) = mark.attr_str("href").and_then(|u| page_link(u, host))
                && !ids.contains(&link.id)
            {
                ids.push(link.id);
            }
        }
    });
    ids
}

/// The page ID in a tiny link code (`/x/tYEE` -> 295349): the ID's little-endian bytes in
/// base64, with `/` and `+` written as `-` and `_`, and trailing zero bytes dropped.
pub fn tiny_link_id(code: &str) -> Option<u64> {
    if code.is_empty() || code.len() > 11 {
        return None;
    }
    let mut bits: u128 = 0;
    let mut nbits = 0;
    let mut bytes = Vec::new();
    for c in code.chars() {
        let v = match c {
            'A'..='Z' => c as u32 - 'A' as u32,
            'a'..='z' => c as u32 - 'a' as u32 + 26,
            '0'..='9' => c as u32 - '0' as u32 + 52,
            '_' | '+' => 62,
            '-' | '/' => 63,
            _ => return None,
        };
        bits = (bits << 6) | u128::from(v);
        nbits += 6;
        if nbits >= 8 {
            nbits -= 8;
            bytes.push((bits >> nbits) as u8);
            bits &= (1 << nbits) - 1;
        }
    }
    let mut id = 0u64;
    for (i, b) in bytes.iter().take(8).enumerate() {
        id |= u64::from(*b) << (8 * i);
    }
    (id > 0).then_some(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOST: &str = "x.atlassian.net";

    #[test]
    fn resolves_page_urls() {
        let link = |u: &str| page_link(u, HOST);
        assert_eq!(
            link("https://x.atlassian.net/wiki/spaces/ENG/pages/123")
                .unwrap()
                .id,
            "123"
        );
        let with_anchor =
            link("https://x.atlassian.net/wiki/spaces/ENG/pages/123/Some+Title#Install-&-Setup")
                .unwrap();
        assert_eq!(
            (with_anchor.id.as_str(), with_anchor.anchor.as_deref()),
            ("123", Some("Install-&-Setup"))
        );
        assert_eq!(
            link("https://x.atlassian.net/wiki/pages/viewpage.action?pageId=456")
                .unwrap()
                .id,
            "456"
        );
        assert_eq!(
            link("https://x.atlassian.net/wiki/x/tYEE").unwrap().id,
            "295349"
        );
        assert!(link("https://other.atlassian.net/wiki/spaces/ENG/pages/123").is_none());
        assert!(link("https://x.atlassian.net/wiki/spaces/ENG/overview").is_none());
        assert!(link("#anchor").is_none());
    }

    #[test]
    fn finds_linked_pages() {
        let doc: crate::adf::Node = serde_json::from_str(r#"{"type":"doc","content":[{"type":"paragraph","content":[
            {"type":"text","text":"a","marks":[{"type":"link","attrs":{"href":"https://x.atlassian.net/wiki/spaces/E/pages/1/T#A"}}]},
            {"type":"text","text":"b","marks":[{"type":"link","attrs":{"href":"https://other.net/wiki/spaces/E/pages/2"}}]},
            {"type":"inlineCard","attrs":{"url":"https://x.atlassian.net/wiki/x/tYEE"}},
            {"type":"text","text":"c","marks":[{"type":"link","attrs":{"href":"https://x.atlassian.net/wiki/spaces/E/pages/1"}}]}
        ]}]}"#).unwrap();
        assert_eq!(linked_page_ids(&doc, HOST), ["295349", "1"]);
    }

    #[test]
    fn decodes_tiny_links() {
        // From captured page responses (`_links.tinyui`).
        for (code, id) in [
            ("JgAH", 458790),
            ("ZIAB", 98404),
            ("CAAL", 720904),
            ("GIAG", 426008),
            ("AgAC", 131074),
            ("tYEE", 295349),
            ("AwAH", 458755),
            ("rYEE", 295341),
        ] {
            assert_eq!(tiny_link_id(code), Some(id), "{code}");
        }
        assert_eq!(tiny_link_id(""), None);
        assert_eq!(tiny_link_id("a!"), None);
    }
}
