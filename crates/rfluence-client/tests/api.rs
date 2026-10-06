//! The client against recorded Confluence responses (fixtures/confluence), served by a mock
//! server.

use std::path::PathBuf;

use mockito::Matcher;
use rfluence_client::auth::Credentials;
use rfluence_client::{Client, Error, PageRef};

fn fixture(name: &str, file: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/confluence").join(name).join(file);
    std::fs::read_to_string(path).unwrap()
}

/// The v2 page response: page.json with the body put back.
fn page_response(name: &str) -> String {
    let mut page: serde_json::Value = serde_json::from_str(&fixture(name, "page.json")).unwrap();
    let adf: serde_json::Value = serde_json::from_str(&fixture(name, "adf.json")).unwrap();
    page["body"] = serde_json::json!({ "atlas_doc_format": { "representation": "atlas_doc_format", "value": adf.to_string() } });
    page.to_string()
}

fn client(server: &mockito::Server) -> Client {
    Client::new(&Credentials { base_url: server.url(), email: "me@example.com".into(), token: "secret".into() })
}

#[test]
fn fetches_a_page_and_its_attachments() {
    let mut server = mockito::Server::new();
    // base64("me@example.com:secret")
    let auth = "Basic bWVAZXhhbXBsZS5jb206c2VjcmV0";
    let page = server
        .mock("GET", "/wiki/api/v2/pages/458790")
        .match_query(Matcher::AllOf(vec![
            Matcher::UrlEncoded("body-format".into(), "atlas_doc_format".into()),
            Matcher::UrlEncoded("include-labels".into(), "true".into()),
        ]))
        .match_header("authorization", auth)
        .with_body(page_response("adf-reference"))
        .create();
    let attachments = server
        .mock("GET", "/wiki/api/v2/pages/458790/attachments")
        .match_query(Matcher::Any)
        .match_header("authorization", auth)
        .with_body(fixture("adf-reference", "attachments.json"))
        .create();

    let (page_data, files) = client(&server).page_with_attachments("458790").unwrap();
    page.assert();
    attachments.assert();
    let meta = &page_data.meta;
    assert_eq!(meta.title, "rfluence ADF reference");
    assert_eq!(meta.space_key, "rfluencete");
    assert_eq!(meta.version, 5);
    assert_eq!(meta.url, "https://tech-accounts11.atlassian.net/wiki/spaces/rfluencete/pages/458790");
    assert_eq!(meta.labels, ["two", "words", "ünïcode", "dash-ok", "under_score", "upper", "comma", "label"]);
    assert_eq!(page_data.adf.kind, "doc");
    assert_eq!(files.len(), 4);
    assert!(rfluence_client::file_names(&files).values().any(|n| n == "striped.png"));
}

#[test]
fn follows_label_pages() {
    let mut server = mockito::Server::new();
    let mut page: serde_json::Value = serde_json::from_str(&page_response("emoji")).unwrap();
    page["labels"] = serde_json::json!({ "results": [{ "name": "first", "prefix": "global" }], "meta": { "hasMore": true } });
    server.mock("GET", "/wiki/api/v2/pages/426008").match_query(Matcher::Any).with_body(page.to_string()).create();
    server
        .mock("GET", "/wiki/api/v2/pages/426008/labels")
        .match_query(Matcher::UrlEncoded("limit".into(), "250".into()))
        .with_body(r#"{"results":[{"name":"first","prefix":"global"},{"name":"mine","prefix":"my"}],"_links":{"next":"/api/v2/pages/426008/labels?cursor=abc"}}"#)
        .create();
    server
        .mock("GET", "/wiki/api/v2/pages/426008/labels")
        .match_query(Matcher::UrlEncoded("cursor".into(), "abc".into()))
        .with_body(r#"{"results":[{"name":"second","prefix":"global"}],"_links":{}}"#)
        .create();
    let page = client(&server).page("426008").unwrap();
    assert_eq!(page.meta.labels, ["first", "second"]);
}

#[test]
fn finds_a_page_by_title() {
    let mut server = mockito::Server::new();
    server
        .mock("GET", "/wiki/rest/api/content")
        .match_query(Matcher::AllOf(vec![
            Matcher::UrlEncoded("spaceKey".into(), "rfluencete".into()),
            Matcher::UrlEncoded("title".into(), "rfluence emoji API test".into()),
        ]))
        .with_body(r#"{"results":[{"id":"426008","type":"page"}]}"#)
        .create();
    server.mock("GET", "/wiki/rest/api/content").match_query(Matcher::Any).with_body(r#"{"results":[]}"#).create();
    let c = client(&server);
    let found = PageRef::Title { space_key: "rfluencete".into(), title: "rfluence emoji API test".into() };
    assert_eq!(c.resolve(&found).unwrap(), "426008");
    let missing = PageRef::Title { space_key: "rfluencete".into(), title: "nope".into() };
    assert!(matches!(c.resolve(&missing), Err(Error::NotFound(_))));
}

#[test]
fn maps_http_errors() {
    let mut server = mockito::Server::new();
    server
        .mock("GET", "/wiki/api/v2/pages/1")
        .match_query(Matcher::Any)
        .with_status(404)
        .with_body(r#"{"errors":[{"status":404,"title":"Not Found"}]}"#)
        .create();
    server.mock("GET", "/wiki/api/v2/pages/2").match_query(Matcher::Any).with_status(401).with_body("Unauthorized").create();
    server
        .mock("GET", "/wiki/api/v2/pages/3")
        .match_query(Matcher::Any)
        .with_status(500)
        .with_body(r#"{"message":"boom"}"#)
        .create();
    let c = client(&server);
    assert!(matches!(c.page("1"), Err(Error::NotFound(m)) if m.contains("page 1 not found")));
    assert!(matches!(c.page("2"), Err(Error::Auth(_))));
    assert!(matches!(c.page("3"), Err(Error::Api { status: 500, message }) if message == "boom"));
}

/// The v2 API returns trashed pages too (verified): they count as not found.
#[test]
fn trashed_pages_are_not_found() {
    let mut server = mockito::Server::new();
    let mut page: serde_json::Value = serde_json::from_str(&page_response("adf-reference")).unwrap();
    page["status"] = "trashed".into();
    server.mock("GET", "/wiki/api/v2/pages/458790").match_query(Matcher::Any).with_body(page.to_string()).create();
    let client = client(&server);
    match client.page("458790") {
        Err(Error::NotFound(m)) => assert_eq!(m, "page or folder 458790 is in the trash"),
        other => panic!("{other:?}"),
    }
    assert!(matches!(client.content("458790"), Err(Error::NotFound(_))));
}
