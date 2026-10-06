//! `rfluence search` against a recorded search response (fixtures/api/search-panel.json).

use std::path::PathBuf;
use std::process::{Command, Output};

use mockito::Matcher;

fn recorded() -> String {
    std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/api/search-panel.json"),
    )
    .unwrap()
}

fn rf(server: &mockito::ServerGuard, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rfluence"))
        .args(args)
        .env("CONFLUENCE_BASE_URL", server.url())
        .env("CONFLUENCE_EMAIL", "me@example.com")
        .env("CONFLUENCE_API_KEY", "secret")
        .env_remove("RFLUENCE_SITE")
        .output()
        .unwrap()
}

fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

#[test]
fn lists_results_for_an_llm() {
    let mut server = mockito::Server::new();
    let search = server
        .mock("GET", "/wiki/rest/api/search")
        .match_query(Matcher::AllOf(vec![
            Matcher::UrlEncoded(
                "cql".into(),
                r#"text ~ "panel" and type = page and space = "rfluencete""#.into(),
            ),
            Matcher::UrlEncoded("limit".into(), "3".into()),
            Matcher::UrlEncoded("expand".into(), "content.metadata.labels".into()),
        ]))
        .with_body(recorded())
        .create();
    let o = rf(
        &server,
        &["search", "panel", "--space", "rfluencete", "--limit", "3"],
    );
    search.assert();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let text = out(&o);
    assert!(text.starts_with(
        "458790  rfluence ADF reference\n  rfluencete · updated 2026-10-04 · labels: two, words, ünïcode, dash-ok, under_score, upper, comma, label\n  https://tech-accounts11.atlassian.net/wiki/spaces/rfluencete/pages/458790\n  API-created reference page for rfluence."
    ), "{text}");
    // Entities decoded, one line per excerpt.
    assert!(text.contains("We've added some suggestions"), "{text}");
    assert!(
        text.ends_with("3 results. Read one with `rfluence fetch <id>`.\n"),
        "{text}"
    );
}

#[test]
fn prints_json() {
    let mut server = mockito::Server::new();
    server
        .mock("GET", "/wiki/rest/api/search")
        .match_query(Matcher::Any)
        .with_body(recorded())
        .create();
    let v: serde_json::Value =
        serde_json::from_slice(&rf(&server, &["search", "panel", "--json"]).stdout).unwrap();
    assert_eq!(v["results"][0]["id"], "458790");
    assert_eq!(v["results"][0]["space_key"], "rfluencete");
    assert_eq!(v["results"].as_array().unwrap().len(), 3);
    assert_eq!(v["total"], 3);
}

#[test]
fn raw_cql_and_no_results() {
    let mut server = mockito::Server::new();
    server
        .mock("GET", "/wiki/rest/api/search")
        .match_query(Matcher::UrlEncoded("cql".into(), "label = nothing".into()))
        .with_body(r#"{"results":[],"totalSize":0,"_links":{"base":"https://x/wiki"}}"#)
        .create();
    let o = rf(&server, &["search", "--cql", "label = nothing"]);
    assert!(o.status.success());
    assert_eq!(out(&o), "No results.\n");
}

#[test]
fn invalid_cql_is_a_usage_error() {
    let mut server = mockito::Server::new();
    server
        .mock("GET", "/wiki/rest/api/search")
        .match_query(Matcher::Any)
        .with_status(400)
        .with_body(r#"{"statusCode":400,"message":"com.atlassian.confluence.api.service.exceptions.api.BadRequestException: Could not parse cql : and and"}"#)
        .create();
    let o = rf(&server, &["search", "--cql", "and and"]);
    assert_eq!(o.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&o.stderr)
            .contains("invalid CQL query: Could not parse cql : and and")
    );
}

#[test]
fn needs_a_query() {
    let server = mockito::Server::new();
    assert_eq!(rf(&server, &["search"]).status.code(), Some(2));
    assert_eq!(
        rf(&server, &["search", "x", "--cql", "y"]).status.code(),
        Some(2)
    );
}
