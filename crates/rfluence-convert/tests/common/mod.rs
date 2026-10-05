//! Fixture loading shared by the integration tests.
#![allow(dead_code)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use rfluence_convert::adf::Node;
use rfluence_convert::{FetchContext, MermaidApp, UploadContext, emoji, is_merfluence};

pub fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures")
}

/// Captured pages: directory names under fixtures/confluence (see its README.md).
pub fn pages() -> Vec<String> {
    let mut ids: Vec<String> = std::fs::read_dir(fixtures().join("confluence"))
        .unwrap()
        .map(|e| e.unwrap())
        .filter(|e| e.path().join("page.json").exists())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    ids.sort();
    ids
}

/// The Confluence page ID of a captured page, from its page.json.
pub fn page_id(name: &str) -> String {
    let path = fixtures().join("confluence").join(name).join("page.json");
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    v["id"].as_str().unwrap().to_string()
}

pub fn page_adf(name: &str, file: &str) -> Node {
    let path = fixtures().join("confluence").join(name).join(file);
    serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap()
}

/// fileId -> attachment file name, from the captured attachments response.
pub fn attachments(name: &str) -> HashMap<String, String> {
    let path = fixtures().join("confluence").join(name).join("attachments.json");
    let Ok(json) = std::fs::read_to_string(path) else { return HashMap::new() };
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    v["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| (a["fileId"].as_str().unwrap().to_string(), a["title"].as_str().unwrap().to_string()))
        .collect()
}

/// Images point at the fixture's `attachments/` folder, so its page.md previews with them.
pub fn fetch_ctx(name: &str) -> FetchContext {
    FetchContext { page_id: Some(page_id(name)), assets_dir: "attachments".into(), attachments: attachments(name) }
}

/// The upload context a client would build for a fetched page: its attachments by path,
/// the merfluence app from its diagrams, and its custom emoji.
pub fn upload_ctx(name: &str, doc: &Node) -> UploadContext {
    let media = attachments(name).into_iter().map(|(file_id, name)| (format!("attachments/{name}"), file_id)).collect();
    let mut mermaid = None;
    let mut custom_emoji = HashMap::new();
    doc.walk(&mut |n| {
        if is_merfluence(n) {
            mermaid = n.attr_str("extensionKey").and_then(MermaidApp::from_extension_key);
        }
        if n.is("emoji") {
            if let (Some(short), Some(id), Some(text)) = (n.attr_str("shortName"), n.attr_str("id"), n.attr_str("text")) {
                if !emoji::is_emoji(text) && emoji::lookup(short).is_none() && emoji::from_id(id).is_none() {
                    custom_emoji.insert(short.trim_matches(':').to_string(), id.to_string());
                }
            }
        }
    });
    UploadContext { page_id: Some(page_id(name)), media, custom_emoji, mermaid }
}

/// The corpus of LLM-style markdown.
pub fn corpus() -> Vec<(String, String)> {
    let dir = fixtures().join("markdown");
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "md"))
        .map(|p| (p.file_stem().unwrap().to_string_lossy().into_owned(), std::fs::read_to_string(&p).unwrap()))
        .collect();
    files.sort();
    files
}

pub const MERMAID: &str = "5321c3d1-955d-42ac-9f09-d4d6f0802224/04b85365-6260-47d9-9f03-8f42e258aab7/static/mermaid-diagram";
