//! Markdown -> ADF: snapshots of what upload sends for the corpus.

mod common;

use std::collections::HashMap;

use common::*;
use rfluence_convert::{MermaidApp, PageRef, UploadContext, local_links, markdown_to_adf};

#[test]
fn corpus_as_adf() {
    for (name, md) in corpus() {
        // Every local image resolves to an attachment named after its path.
        let media: HashMap<String, String> = md
            .split(['(', ')', ' '])
            .filter(|s| s.starts_with(&format!("{name}.assets/")))
            .map(|p| (p.to_string(), format!("file:{p}")))
            .collect();
        // Every linked markdown file is a page, its headings made from the links' anchors.
        let mut pages: HashMap<String, PageRef> = HashMap::new();
        for (_, path) in local_links(&md) {
            let n = pages.len() + 100;
            pages.entry(path.clone()).or_insert_with(|| PageRef {
                url: format!("https://example.atlassian.net/wiki/spaces/ENG/pages/{n}"),
                headings: md
                    .match_indices(&format!("{path}#"))
                    .map(|(i, _)| {
                        let anchor = &md[i + path.len() + 1..];
                        let anchor = &anchor[..anchor.find(')').unwrap()];
                        let mut title = anchor.replace('-', " ");
                        title[..1].make_ascii_uppercase();
                        title
                    })
                    .collect(),
            });
        }
        let ctx = UploadContext {
            page_id: Some("1".into()),
            media,
            custom_emoji: HashMap::new(),
            mermaid: MermaidApp::from_extension_key(MERMAID),
            pages,
            ..Default::default()
        };
        let upload = markdown_to_adf(&md, &ctx).unwrap();
        let json = serde_json::to_string_pretty(&upload.doc).unwrap();
        insta::assert_snapshot!(name.clone(), format!("{json}\n\ndiagnostics: {:#?}", upload.diagnostics));
    }
}
