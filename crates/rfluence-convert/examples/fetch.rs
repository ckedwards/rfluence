//! Convert a captured page to markdown: `cargo run --example fetch -- fixtures/confluence/<name>`.

use std::collections::HashMap;

fn main() {
    let dir = std::path::PathBuf::from(std::env::args().nth(1).expect("usage: fetch <fixture dir>"));
    let doc: rfluence_convert::adf::Node =
        serde_json::from_str(&std::fs::read_to_string(dir.join("adf.json")).unwrap()).unwrap();
    let mut attachments = HashMap::new();
    if let Ok(json) = std::fs::read_to_string(dir.join("attachments.json")) {
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        for a in v["results"].as_array().unwrap() {
            attachments.insert(a["fileId"].as_str().unwrap().to_string(), a["title"].as_str().unwrap().to_string());
        }
    }
    let page: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join("page.json")).unwrap()).unwrap();
    let id = page["id"].as_str().unwrap().to_string();
    let ctx = rfluence_convert::FetchContext { page_id: Some(id), assets_dir: "page.assets".into(), attachments, simplified: false, ..Default::default() };
    print!("{}", rfluence_convert::adf_to_markdown(&doc, &ctx));
}
