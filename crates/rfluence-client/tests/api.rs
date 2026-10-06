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

fn quick(server: &mockito::Server) -> Client {
    client(server).with_retry_unit(std::time::Duration::from_millis(1))
}

/// Reads are retried on 429, 502-504 and no response; writes only when Confluence says it
/// didn't process them (429, or 503 with Retry-After). Up to 3 retries.
#[test]
fn retries_when_confluence_is_busy() {
    let mut server = mockito::Server::new();
    let ok = r#"{"displayName":"Me"}"#;
    // A read: 503, then 429 with Retry-After, then it works.
    let first = server.mock("GET", "/wiki/rest/api/user/current").with_status(503).expect(1).create();
    let second = server.mock("GET", "/wiki/rest/api/user/current").with_status(429).with_header("Retry-After", "2").expect(1).create();
    let third = server.mock("GET", "/wiki/rest/api/user/current").with_body(ok).create();
    assert_eq!(quick(&server).current_user().unwrap(), "Me");
    first.assert();
    second.assert();
    third.assert();

    // Retries run out: the last answer is the error.
    let mut server = mockito::Server::new();
    let busy = server.mock("GET", "/wiki/rest/api/user/current").with_status(502).with_body(r#"{"message":"bad gateway"}"#).expect(4).create();
    assert!(matches!(quick(&server).current_user(), Err(Error::Api { status: 502, .. })));
    busy.assert();

    // Not retried: a 500 (a bug or failure on Confluence's side that waiting won't fix).
    let mut server = mockito::Server::new();
    let failed = server.mock("GET", "/wiki/rest/api/user/current").with_status(500).expect(1).create();
    assert!(quick(&server).current_user().is_err());
    failed.assert();
}

#[test]
fn retries_writes_only_when_confluence_did_nothing() {
    let doc = rfluence_convert::adf::Node::doc(vec![]);
    let page = r#"{"title":"T","parentId":null,"version":{"number":2}}"#;
    // 429: retried.
    let mut server = mockito::Server::new();
    let limited = server.mock("PUT", "/wiki/api/v2/pages/1").with_status(429).expect(1).create();
    let ok = server.mock("PUT", "/wiki/api/v2/pages/1").with_body(page).expect(1).create();
    assert_eq!(quick(&server).update_page("1", "T", &doc, 2).unwrap().version, 2);
    limited.assert();
    ok.assert();
    // 503 with Retry-After: retried.
    let mut server = mockito::Server::new();
    let unavailable = server.mock("PUT", "/wiki/api/v2/pages/1").with_status(503).with_header("Retry-After", "1").expect(1).create();
    let ok = server.mock("PUT", "/wiki/api/v2/pages/1").with_body(page).expect(1).create();
    assert!(quick(&server).update_page("1", "T", &doc, 2).is_ok());
    unavailable.assert();
    ok.assert();
    // 503 without Retry-After, or 502: not retried (the update may have been saved).
    for (status, header) in [(503, None), (502, None)] {
        let mut server = mockito::Server::new();
        let mut m = server.mock("PUT", "/wiki/api/v2/pages/1").with_status(status).expect(1);
        if let Some(h) = header {
            m = m.with_header("Retry-After", h);
        }
        let m = m.create();
        assert!(matches!(quick(&server).update_page("1", "T", &doc, 2), Err(Error::Api { .. })), "{status}");
        m.assert();
    }
}

#[test]
fn retries_reads_without_a_response() {
    // Nothing listens on this port: connection refused, retried, then a network error.
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let c = Client::new(&Credentials { base_url: format!("http://127.0.0.1:{port}"), email: "me@example.com".into(), token: "secret".into() })
        .with_retry_unit(std::time::Duration::from_millis(1));
    assert!(matches!(c.current_user(), Err(Error::Network(_))));
}

/// A 403 means the login works but the account isn't allowed: not "authentication failed".
#[test]
fn reports_403_as_permission_denied() {
    let mut server = mockito::Server::new();
    server.mock("PUT", "/wiki/api/v2/pages/1").with_status(403).with_body(r#"{"message":"Not permitted to update"}"#).create();
    let err = quick(&server).update_page("1", "T", &rfluence_convert::adf::Node::doc(vec![]), 2).unwrap_err();
    assert!(matches!(err, Error::Forbidden(_)), "{err:?}");
    let message = err.to_string();
    assert!(message.starts_with("permission denied: HTTP 403 from"), "{message}");
    assert!(message.contains("Not permitted to update") && message.contains("isn't allowed to do this"), "{message}");
}

/// A read that timed out isn't repeated: it already waited, and won't do better at once.
#[test]
fn doesnt_retry_timeouts() {
    // A server that accepts connections and never answers.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let accepted = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let count = accepted.clone();
    std::thread::spawn(move || {
        let mut held = Vec::new();
        for stream in listener.incoming().flatten() {
            count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            held.push(stream);
        }
    });
    let c = Client::new(&Credentials { base_url: format!("http://127.0.0.1:{port}"), email: "me@example.com".into(), token: "secret".into() })
        .with_retry_unit(std::time::Duration::from_millis(1))
        .with_timeout(std::time::Duration::from_millis(300));
    let err = c.current_user().unwrap_err();
    assert!(matches!(&err, Error::Network(m) if m.starts_with(&format!("timed out waiting for 127.0.0.1:{port} to answer"))), "{err:?}");
    assert_eq!(accepted.load(std::sync::atomic::Ordering::SeqCst), 1, "one attempt");
}

/// Confluence runs requests with a bad API token as an anonymous user (verified): pages are
/// 404, other requests 403. The client checks who it is and blames the token instead.
#[test]
fn reports_rejected_tokens() {
    let mut server = mockito::Server::new();
    server.mock("GET", "/wiki/api/v2/pages/1").match_query(Matcher::Any).with_status(404).with_body(r#"{"errors":[{"title":"Not Found"}]}"#).create();
    let whoami = server
        .mock("GET", "/wiki/rest/api/user/current")
        .with_status(403)
        .with_body(r#"{"statusCode":403,"message":"Request rejected because caller cannot access Confluence"}"#)
        // Once for the check (then remembered), once for current_user itself.
        .expect(2)
        .create();
    let c = quick(&server);
    let err = c.page("1").unwrap_err();
    assert!(matches!(err, Error::Auth(_)), "{err:?}");
    let message = err.to_string();
    assert!(
        message.contains("Confluence didn't accept the API token for me@example.com on 127.0.0.1:")
            && message.contains("expired, been revoked, or be mistyped")
            && message.contains("https://id.atlassian.com/manage-profile/security/api-tokens"),
        "{message}"
    );
    // Checked once per client: a second 404 doesn't ask again.
    assert!(matches!(c.page("1"), Err(Error::Auth(_))));
    assert!(matches!(c.current_user(), Err(Error::Auth(_))));
    whoami.assert();
}

#[test]
fn a_real_404_stays_not_found() {
    let mut server = mockito::Server::new();
    server.mock("GET", "/wiki/api/v2/pages/1").match_query(Matcher::Any).with_status(404).create();
    server.mock("GET", "/wiki/rest/api/user/current").with_body(r#"{"type":"known","displayName":"Me"}"#).create();
    assert!(matches!(quick(&server).page("1"), Err(Error::NotFound(_))));
}

#[test]
fn an_anonymous_user_means_the_token_was_ignored() {
    // Sites that allow anonymous access answer "who am I" for an anonymous user.
    let mut server = mockito::Server::new();
    server.mock("GET", "/wiki/rest/api/search").match_query(Matcher::Any).with_status(403).with_body(r#"{"message":"Current user not permitted to use Confluence"}"#).create();
    server.mock("GET", "/wiki/rest/api/user/current").with_body(r#"{"type":"anonymous","displayName":"Anonymous"}"#).create();
    let err = quick(&server).search("type = page", 1).unwrap_err();
    assert!(err.to_string().contains("didn't accept the API token"), "{err}");
}
