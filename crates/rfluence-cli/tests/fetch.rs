//! `rfluence fetch` against recorded Confluence responses (fixtures/confluence), served by a mock
//! server through the CONFLUENCE_* variables.

use std::path::PathBuf;
use std::process::{Command, Output};

use mockito::Matcher;

fn fixture(name: &str, file: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/confluence").join(name).join(file);
    std::fs::read_to_string(path).unwrap()
}

/// A mock site serving one captured page (and its attachments).
fn site(name: &str) -> mockito::ServerGuard {
    let mut server = mockito::Server::new();
    let mut page: serde_json::Value = serde_json::from_str(&fixture(name, "page.json")).unwrap();
    let id = page["id"].as_str().unwrap().to_string();
    let adf: serde_json::Value = serde_json::from_str(&fixture(name, "adf.json")).unwrap();
    page["body"] = serde_json::json!({ "atlas_doc_format": { "value": adf.to_string() } });
    server.mock("GET", format!("/wiki/api/v2/pages/{id}").as_str()).match_query(Matcher::Any).with_body(page.to_string()).create();
    let attachments = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/confluence").join(name).join("attachments.json"),
    )
    .unwrap_or_else(|_| r#"{"results":[],"_links":{}}"#.into());
    server
        .mock("GET", format!("/wiki/api/v2/pages/{id}/attachments").as_str())
        .match_query(Matcher::Any)
        .with_body(attachments)
        .create();
    server
}

fn rf(server: &mockito::ServerGuard, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rfluence"))
        .args(args)
        .env_remove("RFLUENCE_SITE")
        .env("CONFLUENCE_BASE_URL", server.url())
        .env("CONFLUENCE_EMAIL", "me@example.com")
        .env("CONFLUENCE_API_KEY", "secret")
        .output()
        .unwrap()
}

fn stdout(o: &Output) -> String {
    String::from_utf8(o.stdout.clone()).unwrap()
}

/// The fixture's reference output, with images in the folder `rfluence fetch` names after the page.
fn expected(name: &str, file: &str, assets: &str) -> String {
    fixture(name, file).replace("](attachments/", &format!("]({assets}/"))
}

#[test]
fn prints_the_round_trip_form() {
    let server = site("adf-reference");
    let out = rf(&server, &["fetch", "458790"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    similar_asserts::assert_eq!(stdout(&out), expected("adf-reference", "page.md", "rfluence-adf-reference.assets"));
}

#[test]
fn prints_the_simplified_form() {
    let server = site("adf-reference");
    let out = rf(&server, &["fetch", "--simplified", "458790"]);
    assert!(out.status.success());
    similar_asserts::assert_eq!(stdout(&out), expected("adf-reference", "page.simplified.md", "rfluence-adf-reference.assets"));
}

#[test]
fn accepts_page_urls_and_tiny_links() {
    let server = site("emoji");
    let by_id = stdout(&rf(&server, &["fetch", "426008"]));
    // A page URL picks its own site's account: here, the mock site's.
    for path in ["/wiki/spaces/rfluencete/pages/426008/rfluence+emoji+API+test", "/wiki/x/GIAG"] {
        let reference = format!("{}{path}", server.url());
        assert_eq!(stdout(&rf(&server, &["fetch", &reference])), by_id, "{reference}");
    }
}

#[test]
fn selects_a_section() {
    let server = site("adf-reference");
    let out = stdout(&rf(&server, &["fetch", "458790", "--section", "Images and files"]));
    assert!(out.contains("  partial: true\n---\n\n## Images and files\n"), "{out}");
    assert!(!out.contains("## Inline nodes"));
    let missing = rf(&server, &["fetch", "458790", "--section", "Nope"]);
    assert_eq!(missing.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&missing.stderr).contains("headings: rfluence ADF reference; Code blocks;"));
}

#[test]
fn truncates_long_pages() {
    let server = site("adf-reference");
    let out = stdout(&rf(&server, &["fetch", "458790", "--max-chars", "400"]));
    assert!(out.contains("  partial: true\n"));
    assert!(out.contains("[Truncated: showing "), "{out}");
    assert!(out.len() < 2000);
}

#[test]
fn prints_json() {
    let server = site("emoji");
    let out = rf(&server, &["fetch", "426008", "--json"]);
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["title"], "rfluence emoji API test");
    assert_eq!(v["space_key"], "rfluencete");
    assert_eq!(v["version"], 2);
    assert_eq!(v["partial"], false);
    assert!(v["markdown"].as_str().unwrap().starts_with("---\nrfluence:\n  id: \"426008\""));
}

#[test]
fn exit_codes() {
    let mut server = mockito::Server::new();
    server.mock("GET", "/wiki/api/v2/pages/1").match_query(Matcher::Any).with_status(404).with_body("{}").create();
    server.mock("GET", "/wiki/api/v2/pages/1/attachments").match_query(Matcher::Any).with_status(404).with_body("{}").create();
    server.mock("GET", "/wiki/api/v2/pages/2").match_query(Matcher::Any).with_status(401).with_body("").create();
    server.mock("GET", "/wiki/api/v2/pages/2/attachments").match_query(Matcher::Any).with_status(401).with_body("").create();
    assert_eq!(rf(&server, &["fetch", "1"]).status.code(), Some(3));
    assert_eq!(rf(&server, &["fetch", "2"]).status.code(), Some(4));
    assert_eq!(rf(&server, &["fetch", "not a page"]).status.code(), Some(2));
    let partial_env = Command::new(env!("CARGO_BIN_EXE_rfluence"))
        .args(["fetch", "1"])
        .env("CONFLUENCE_BASE_URL", server.url())
        .env_remove("CONFLUENCE_EMAIL")
        .env_remove("CONFLUENCE_API_KEY")
        .output()
        .unwrap();
    assert_eq!(partial_env.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&partial_env.stderr).contains("missing: CONFLUENCE_EMAIL, CONFLUENCE_API_KEY"));
}
