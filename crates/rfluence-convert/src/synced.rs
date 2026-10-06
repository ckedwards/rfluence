//! Synced blocks: originals (`bodiedSyncBlock`) and copies (`syncBlock`). See design.md, "Tabs
//! and synced blocks".

use std::collections::HashMap;

use crate::adf::Node;

/// The source page and block ID in a copy's `resourceId` (`confluence-page/<page>/<id>`).
pub fn parse_copy(resource_id: &str) -> Option<(&str, &str)> {
    let rest = resource_id.strip_prefix("confluence-page/")?;
    let (page, id) = rest.split_once('/')?;
    (!page.is_empty()
        && page.bytes().all(|b| b.is_ascii_digit())
        && !id.is_empty()
        && !id.contains('/'))
    .then_some((page, id))
}

/// A copy's `resourceId`.
pub fn copy_resource_id(page: &str, id: &str) -> String {
    format!("confluence-page/{page}/{id}")
}

/// The `resourceId`s of the synced block copies in a page.
pub fn copy_ids(doc: &Node) -> Vec<String> {
    let mut ids = Vec::new();
    doc.walk(&mut |n| {
        if let Some(r) = n.attr_str("resourceId").filter(|_| n.is("syncBlock"))
            && parse_copy(r).is_some()
            && !ids.iter().any(|i| i == r)
        {
            ids.push(r.to_string());
        }
    });
    ids
}

/// The original synced blocks in a page, by `resourceId`.
pub fn originals(doc: &Node) -> HashMap<String, Node> {
    let mut found = HashMap::new();
    doc.walk(&mut |n| {
        if let Some(r) = n.attr_str("resourceId").filter(|_| n.is("bodiedSyncBlock")) {
            found.entry(r.to_string()).or_insert_with(|| n.clone());
        }
    });
    found
}

/// The content of each copy (by `resourceId`), from its source pages' bodies (by page ID).
/// Copies whose source page or block isn't there are left out.
pub fn copy_contents(
    ids: &[String],
    sources: &HashMap<String, Node>,
) -> HashMap<String, Vec<Node>> {
    let mut contents = HashMap::new();
    for resource_id in ids {
        let Some((page, id)) = parse_copy(resource_id) else {
            continue;
        };
        if let Some(block) = sources.get(page).and_then(|doc| originals(doc).remove(id)) {
            contents.insert(resource_id.clone(), block.content);
        }
    }
    contents
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_copy_resource_ids() {
        let r = "confluence-page/2392065/b7229247-1d30-4ce9-84dc-accb011d4a6b";
        assert_eq!(
            parse_copy(r),
            Some(("2392065", "b7229247-1d30-4ce9-84dc-accb011d4a6b"))
        );
        assert_eq!(
            copy_resource_id("2392065", "b7229247-1d30-4ce9-84dc-accb011d4a6b"),
            r
        );
        assert_eq!(parse_copy("b7229247"), None);
        assert_eq!(parse_copy("confluence-page/x/y"), None);
    }

    #[test]
    fn finds_copies_content_in_their_sources() {
        let fixture = |name: &str| -> Node {
            let path = format!(
                "{}/../../fixtures/confluence/{name}/adf.json",
                env!("CARGO_MANIFEST_DIR")
            );
            serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
        };
        let copies = fixture("synced-block-copies");
        let ids = copy_ids(&copies);
        assert_eq!(ids.len(), 2);
        let sources = HashMap::from([("2392065".to_string(), fixture("tabs-and-synced-block"))]);
        let contents = copy_contents(&ids, &sources);
        // The second copy's source is a draft, which isn't among the sources.
        assert_eq!(contents.len(), 1);
        assert_eq!(contents[&ids[0]][0].plain_text(), "This is a sync block");
    }
}
