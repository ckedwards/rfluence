//! Fixture loading shared by the integration tests.
#![allow(dead_code)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use rfluence_convert::adf::Node;
use rfluence_convert::{FetchContext, UploadContext};

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
/// Smart links get the titles of the other captured pages, as if looked up on the site.
pub fn fetch_ctx(name: &str) -> FetchContext {
    FetchContext {
        page_id: Some(page_id(name)),
        assets_dir: "attachments".into(),
        attachments: attachments(name),
        site_host: Some("tech-accounts11.atlassian.net".into()),
        titles: titles(),
        synced_copies: synced_copies(name),
        ..Default::default()
    }
}

/// The content of a page's synced block copies, from the captured pages they come from (as
/// a client reads them from the source pages).
pub fn synced_copies(name: &str) -> HashMap<String, Vec<Node>> {
    let doc = page_adf(name, "adf.json");
    let sources: HashMap<String, Node> = pages().into_iter().map(|p| (page_id(&p), page_adf(&p, "adf.json"))).collect();
    rfluence_convert::synced::copy_contents(&rfluence_convert::synced::copy_ids(&doc), &sources)
}

/// Every captured page's title, by page ID.
pub fn titles() -> HashMap<String, String> {
    pages()
        .into_iter()
        .map(|name| (page_id(&name), page_meta(&name).title))
        .collect()
}

/// The page metadata a client would read from the page response (page.json).
pub fn page_meta(name: &str) -> rfluence_convert::PageMeta {
    let path = fixtures().join("confluence").join(name).join("page.json");
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let webui = v["_links"]["webui"].as_str().unwrap();
    let space_key = webui.trim_start_matches("/spaces/").split('/').next().unwrap().to_string();
    let id = v["id"].as_str().unwrap().to_string();
    rfluence_convert::PageMeta {
        url: format!("{}/spaces/{space_key}/pages/{id}", v["_links"]["base"].as_str().unwrap()),
        id,
        title: v["title"].as_str().unwrap().to_string(),
        space_key,
        parent: v["parentId"].as_str().map(str::to_string),
        version: v["version"]["number"].as_u64().unwrap(),
        updated: v["version"]["createdAt"].as_str().map(str::to_string),
        labels: v["labels"]["results"].as_array().unwrap().iter().map(|l| l["name"].as_str().unwrap().to_string()).collect(),
    }
}

/// The upload context a client would build for a fetched page: its attachments by path,
/// the merfluence app from its diagrams, and its custom emoji.
pub fn upload_ctx(name: &str, doc: &Node) -> UploadContext {
    let media = attachments(name).into_iter().map(|(file_id, name)| (format!("attachments/{name}"), file_id)).collect();
    let mut ctx = UploadContext { page_id: Some(page_id(name)), media, synced_copies: synced_copies(name), ..Default::default() };
    ctx.learn_from(doc);
    ctx
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
