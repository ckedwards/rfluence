//! `rfluence fetch -o`: writing files, downloading images, links between project files, and
//! not losing local changes. Against a mock site serving captured pages.

use std::path::PathBuf;
use std::process::{Command, Output};

use mockito::Matcher;

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/confluence")
}

fn fixture(name: &str, file: &str) -> String {
    std::fs::read_to_string(fixtures().join(name).join(file)).unwrap()
}

/// A v2 page response from a fixture, optionally with another body and version number.
fn page_json(name: &str, adf_file: &str, version: Option<u64>) -> String {
    let mut page: serde_json::Value = serde_json::from_str(&fixture(name, "page.json")).unwrap();
    let adf: serde_json::Value = serde_json::from_str(&fixture(name, adf_file)).unwrap();
    page["body"] = serde_json::json!({ "atlas_doc_format": { "value": adf.to_string() } });
    if let Some(v) = version {
        page["version"]["number"] = v.into();
    }
    page.to_string()
}

/// The titles of all captured pages, as `GET /wiki/api/v2/pages?id=...` returns them.
fn serve_titles(server: &mut mockito::ServerGuard) {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/confluence");
    let mut results = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        if let Ok(text) = std::fs::read_to_string(entry.path().join("page.json")) {
            let page: serde_json::Value = serde_json::from_str(&text).unwrap();
            results.push(serde_json::json!({ "id": page["id"], "title": page["title"] }));
        }
    }
    server
        .mock("GET", "/wiki/api/v2/pages")
        .match_query(Matcher::Any)
        .with_body(serde_json::json!({ "results": results, "_links": {} }).to_string())
        .create();
}

/// Serve a captured page: its current version, its attachments with their files, and page
/// titles.
fn serve(server: &mut mockito::ServerGuard, name: &str) {
    serve_titles(server);
    let page: serde_json::Value = serde_json::from_str(&fixture(name, "page.json")).unwrap();
    let id = page["id"].as_str().unwrap();
    server
        .mock("GET", format!("/wiki/api/v2/pages/{id}").as_str())
        .match_query(Matcher::Any)
        .with_body(page_json(name, "adf.json", None))
        .create();
    let attachments = std::fs::read_to_string(fixtures().join(name).join("attachments.json"))
        .unwrap_or_else(|_| r#"{"results":[]}"#.into());
    for a in serde_json::from_str::<serde_json::Value>(&attachments).unwrap()["results"]
        .as_array()
        .unwrap()
    {
        let bytes = std::fs::read(
            fixtures()
                .join(name)
                .join("attachments")
                .join(a["title"].as_str().unwrap()),
        )
        .unwrap();
        let path = format!("/wiki{}", a["_links"]["download"].as_str().unwrap());
        server.mock("GET", path.as_str()).with_body(bytes).create();
    }
    server
        .mock(
            "GET",
            format!("/wiki/api/v2/pages/{id}/attachments").as_str(),
        )
        .match_query(Matcher::Any)
        .with_body(attachments)
        .create();
}

struct Project {
    dir: PathBuf,
}

impl Project {
    fn new(name: &str) -> Project {
        let dir =
            std::env::temp_dir().join(format!("rfluence-fetch-file-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Project { dir }
    }

    fn path(&self, file: &str) -> PathBuf {
        self.dir.join(file)
    }

    fn read(&self, file: &str) -> String {
        std::fs::read_to_string(self.path(file)).unwrap()
    }

    fn rf(&self, server: &mockito::ServerGuard, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_rfluence"))
            .args(args)
            .env("CONFLUENCE_BASE_URL", server.url())
            .env("CONFLUENCE_EMAIL", "me@example.com")
            .env("CONFLUENCE_API_KEY", "secret")
            .env("RFLUENCE_CONFIG_DIR", self.dir.join(".config"))
            .env("RFLUENCE_NO_KEYRING", "1")
            .env_remove("RFLUENCE_SITE")
            .output()
            .unwrap()
    }

    fn fetch(&self, server: &mockito::ServerGuard, id: &str, file: &str, extra: &[&str]) -> Output {
        let path = self.path(file);
        let mut args = vec!["fetch", id, "-o", path.to_str().unwrap()];
        args.extend(extra);
        self.rf(server, &args)
    }
}

impl Drop for Project {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn err(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

#[test]
fn writes_the_page_and_its_images() {
    let mut server = mockito::Server::new();
    serve(&mut server, "adf-reference");
    let p = Project::new("images");
    let out = p.fetch(&server, "458790", "ref.md", &[]);
    assert!(out.status.success(), "{}", err(&out));
    assert!(
        err(&out).contains("ref.md written (version 5, 3 of 3 images downloaded"),
        "{}",
        err(&out)
    );
    similar_asserts::assert_eq!(
        p.read("ref.md"),
        fixture("adf-reference", "page.md").replace("](attachments/", "](ref.assets/")
    );
    for image in ["striped.png", "small.png", "rfluence-emoji.png"] {
        assert_eq!(
            std::fs::read(p.path("ref.assets").join(image)).unwrap(),
            std::fs::read(fixtures().join("adf-reference/attachments").join(image)).unwrap()
        );
    }
    // Other files (the PDF file card) stay on Confluence.
    assert!(!p.path("ref.assets/spec.pdf").exists());

    // Fetching again changes nothing and downloads nothing.
    let again = p.fetch(&server, "458790", "ref.md", &[]);
    assert!(
        err(&again).contains("ref.md is up to date (version 5, 3 images up to date)"),
        "{}",
        err(&again)
    );
}

#[test]
fn keeps_other_frontmatter_and_weight() {
    let mut server = mockito::Server::new();
    serve(&mut server, "emoji");
    let p = Project::new("frontmatter");
    assert!(p.fetch(&server, "426008", "emoji.md", &[]).status.success());
    let fetched = p.read("emoji.md");
    let edited = fetched
        .replacen(
            "---\nrfluence:\n",
            "---\ntags: [notes] # mine\nrfluence:\n",
            1,
        )
        .replacen("  labels: []\n", "  labels: []\n  weight: 5\n", 1);
    std::fs::write(p.path("emoji.md"), &edited).unwrap();
    let out = p.fetch(&server, "426008", "emoji.md", &[]);
    assert!(out.status.success(), "{}", err(&out));
    assert_eq!(p.read("emoji.md"), edited);
}

#[test]
fn refuses_to_lose_local_changes() {
    let mut server = mockito::Server::new();
    serve(&mut server, "emoji");
    let p = Project::new("local-changes");
    assert!(p.fetch(&server, "426008", "emoji.md", &[]).status.success());
    let edited = p.read("emoji.md").replace("XXX", "XXX and my local note");
    std::fs::write(p.path("emoji.md"), &edited).unwrap();

    let refused = p.fetch(&server, "426008", "emoji.md", &[]);
    assert_eq!(refused.status.code(), Some(6));
    assert!(
        err(&refused).contains("has local changes that would be lost"),
        "{}",
        err(&refused)
    );
    assert_eq!(p.read("emoji.md"), edited);

    let forced = p.fetch(&server, "426008", "emoji.md", &["--force"]);
    assert!(forced.status.success());
    assert!(!p.read("emoji.md").contains("my local note"));
}

#[test]
fn updates_an_unchanged_file_to_a_newer_version() {
    // Version 1 of the emoji page is its API-saved fixture; version 2 the editor re-save.
    let p = Project::new("newer");
    let mut v1 = mockito::Server::new();
    v1.mock("GET", "/wiki/api/v2/pages/426008")
        .match_query(Matcher::Any)
        .with_body(page_json("emoji", "adf.v1-api-save.json", Some(1)))
        .create();
    v1.mock("GET", "/wiki/api/v2/pages/426008/attachments")
        .match_query(Matcher::Any)
        .with_body(r#"{"results":[]}"#)
        .create();
    assert!(p.fetch(&v1, "426008", "emoji.md", &[]).status.success());
    assert!(p.read("emoji.md").contains("  version: 1\n") && !p.read("emoji.md").contains("XXX"));

    let mut v2 = mockito::Server::new();
    v2.mock("GET", "/wiki/api/v2/pages/426008")
        .match_query(Matcher::UrlEncoded("version".into(), "1".into()))
        .with_body(page_json("emoji", "adf.v1-api-save.json", Some(1)))
        .create();
    serve(&mut v2, "emoji");
    let out = p.fetch(&v2, "426008", "emoji.md", &[]);
    assert!(out.status.success(), "{}", err(&out));
    assert!(p.read("emoji.md").contains("  version: 2\n") && p.read("emoji.md").contains("XXX"));

    // Changed on both sides: refuse.
    std::fs::write(
        p.path("emoji.md"),
        p.read("emoji.md")
            .replace("  version: 2\n", "  version: 1\n")
            .replace("XXX", "local"),
    )
    .unwrap();
    let both = p.fetch(&v2, "426008", "emoji.md", &[]);
    assert_eq!(both.status.code(), Some(6));
    assert!(
        err(&both).contains(
            "has local changes, and the page has changed in Confluence too (version 1 -> 2)"
        ),
        "{}",
        err(&both)
    );
}

#[test]
fn refuses_a_file_holding_another_page() {
    let mut server = mockito::Server::new();
    serve(&mut server, "emoji");
    serve(&mut server, "images-api");
    let p = Project::new("other-page");
    assert!(p.fetch(&server, "426008", "page.md", &[]).status.success());
    let out = p.fetch(&server, "131074", "page.md", &[]);
    assert_eq!(out.status.code(), Some(6));
    assert!(
        err(&out).contains("holds page 426008, not 131074"),
        "{}",
        err(&out)
    );
}

#[test]
fn links_to_project_files_are_relative() {
    let mut server = mockito::Server::new();
    serve(&mut server, "emoji");
    serve(&mut server, "adf-reference");
    let p = Project::new("links");
    std::fs::write(p.path(".rfluence.yaml"), "[]\n").unwrap();
    std::fs::create_dir_all(p.path("docs")).unwrap();
    assert!(p.fetch(&server, "426008", "emoji.md", &[]).status.success());
    assert!(
        p.fetch(&server, "458790", "docs/ref.md", &[])
            .status
            .success()
    );
    let reference = p.read("docs/ref.md");
    // The resized image on the reference page links to the emoji page, now ../emoji.md, and
    // so does its smart link (still marked as one).
    assert!(
        reference.contains("](../emoji.md)<!-- rf: layout=align-end"),
        "{reference}"
    );
    assert!(reference.contains("An inline confluence link [rfluence emoji API test](../emoji.md)<!-- rf: card=inline -->"), "{reference}");
    // Smart links to pages outside the project keep their URLs.
    assert!(reference.contains("[rfluence link API test](https://tech-accounts11.atlassian.net/wiki/spaces/rfluencete/pages/295349)<!-- rf: card=inline -->"));
}

#[test]
fn partial_output_cant_be_written_to_a_file() {
    let server = mockito::Server::new();
    let p = Project::new("partial");
    let out = p.fetch(&server, "1", "x.md", &["--section", "A"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(err(&out).contains("can't be used with -o"));
}
