//! Markdown -> ADF: snapshots of what upload sends for the corpus.

mod common;

use std::collections::HashMap;

use common::*;
use rfluence_convert::{MermaidApp, UploadContext, markdown_to_adf};

#[test]
fn corpus_as_adf() {
    for (name, md) in corpus() {
        // Every local image resolves to an attachment named after its path.
        let media: HashMap<String, String> = md
            .split(['(', ')', ' '])
            .filter(|s| s.starts_with(&format!("{name}.assets/")))
            .map(|p| (p.to_string(), format!("file:{p}")))
            .collect();
        let ctx = UploadContext {
            page_id: Some("1".into()),
            media,
            custom_emoji: HashMap::new(),
            mermaid: MermaidApp::from_extension_key(MERMAID),
        };
        let upload = markdown_to_adf(&md, &ctx).unwrap();
        let json = serde_json::to_string_pretty(&upload.doc).unwrap();
        insta::assert_snapshot!(name.clone(), format!("{json}\n\ndiagnostics: {:#?}", upload.diagnostics));
    }
}
