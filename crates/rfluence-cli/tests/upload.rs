//! `rfluence upload`: updating a page from a fetched file, against a mock site.

use std::path::PathBuf;
use std::process::{Command, Output};

use mockito::Matcher;
use serde_json::json;

const ID: &str = "131074";

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/confluence/images-api")
}

fn fixture(file: &str) -> String {
    std::fs::read_to_string(fixtures().join(file)).unwrap()
}

/// The images-api page (two images, version 2) on the mock site, with another body and
/// version if given.
fn page_json(server: &mockito::ServerGuard, adf: Option<&serde_json::Value>, version: u64) -> String {
    let mut page: serde_json::Value = serde_json::from_str(&fixture("page.json")).unwrap();
    let adf = adf.cloned().unwrap_or_else(|| serde_json::from_str(&fixture("adf.json")).unwrap());
    page["body"] = json!({ "atlas_doc_format": { "value": adf.to_string() } });
    page["version"]["number"] = version.into();
    page["_links"]["base"] = format!("{}/wiki", server.url()).into();
    page.to_string()
}

struct Site {
    server: mockito::ServerGuard,
    dir: PathBuf,
}

impl Site {
    /// A mock site serving the page (at `version`, with `adf` if given), its attachments
    /// and their files, and the page's `rfluence` property if given.
    fn new(name: &str, adf: Option<serde_json::Value>, version: u64, property: Option<serde_json::Value>) -> Site {
        let mut server = mockito::Server::new();
        let body = page_json(&server, adf.as_ref(), version);
        server.mock("GET", format!("/wiki/api/v2/pages/{ID}").as_str()).match_query(Matcher::Any).with_body(body).create();
        let attachments = fixture("attachments.json");
        for a in serde_json::from_str::<serde_json::Value>(&attachments).unwrap()["results"].as_array().unwrap() {
            let bytes = std::fs::read(fixtures().join("attachments").join(a["title"].as_str().unwrap())).unwrap();
            let path = format!("/wiki{}", a["_links"]["download"].as_str().unwrap());
            server.mock("GET", path.as_str()).with_body(bytes).create();
        }
        server
            .mock("GET", format!("/wiki/api/v2/pages/{ID}/attachments").as_str())
            .match_query(Matcher::Any)
            .with_body(attachments)
            .create();
        server
            .mock("GET", format!("/wiki/api/v2/pages/{ID}/properties").as_str())
            .match_query(Matcher::UrlEncoded("key".into(), "rfluence".into()))
            .with_body(json!({ "results": property.into_iter().collect::<Vec<_>>(), "_links": {} }).to_string())
            .create();
        let dir = std::env::temp_dir().join(format!("rfluence-upload-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Site { server, dir }
    }

    fn path(&self, file: &str) -> PathBuf {
        self.dir.join(file)
    }

    fn read(&self) -> String {
        std::fs::read_to_string(self.path("page.md")).unwrap()
    }

    fn write(&self, md: &str) {
        std::fs::write(self.path("page.md"), md).unwrap();
    }

    fn rf(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_rfluence"))
            .args(args)
            .env("CONFLUENCE_BASE_URL", self.server.url())
            .env("CONFLUENCE_EMAIL", "me@example.com")
            .env("CONFLUENCE_API_KEY", "secret")
            .env("RFLUENCE_CONFIG_DIR", self.dir.join(".config"))
            .env("RFLUENCE_NO_KEYRING", "1")
            .env_remove("RFLUENCE_SITE")
            .output()
            .unwrap()
    }

    /// `rfluence fetch -o page.md`.
    fn fetch(&self) {
        let o = self.rf(&["fetch", ID, "-o", self.path("page.md").to_str().unwrap()]);
        assert!(o.status.success(), "{}", err(&o));
    }

    fn upload(&self, extra: &[&str]) -> Output {
        let path = self.path("page.md");
        let mut args = vec!["upload", path.to_str().unwrap()];
        args.extend(extra);
        self.rf(&args)
    }

    /// A PUT of the page body, answered with the page at `version`.
    fn expect_update(&mut self, version: u64, body: Vec<Matcher>) -> mockito::Mock {
        let response = page_json(&self.server, None, version);
        let mut all = vec![Matcher::PartialJson(json!({ "id": ID, "status": "current", "version": { "number": version } }))];
        all.extend(body);
        self.server
            .mock("PUT", format!("/wiki/api/v2/pages/{ID}").as_str())
            .match_body(Matcher::AllOf(all))
            .with_body(response)
            .create()
    }
}

impl Drop for Site {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn err(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

/// The ADF a PUT sent, as a string with escaped JSON inside (`body.value`).
fn sent_body_contains(text: &str) -> Matcher {
    let quoted = serde_json::to_string(text).unwrap();
    Matcher::Regex(regex_escape(&quoted[1..quoted.len() - 1]))
}

fn regex_escape(s: &str) -> String {
    s.chars().map(|c| if "\\.+*?()|[]{}^$".contains(c) { format!("\\{c}") } else { c.to_string() }).collect()
}

#[test]
fn uploads_edits_images_and_labels_then_writes_back_the_version() {
    let property = json!({
        "id": "77", "key": "rfluence", "version": { "number": 4 },
        "value": { "managed": true, "version": 2, "path": "old.md", "config_labels": ["x"] },
    });
    let mut site = Site::new("edit", None, 2, Some(property));
    site.fetch();
    let md = site.read();
    let edited = md.replace("labels: []", "labels: [New Label]") + "\nAn added paragraph.\n\n![](page.assets/new.png)\n";
    site.write(&edited);
    std::fs::write(site.path("page.assets/new.png"), b"\x89PNG new").unwrap();

    let attach = site
        .server
        .mock("POST", format!("/wiki/rest/api/content/{ID}/child/attachment").as_str())
        .match_header("x-atlassian-token", "no-check")
        .match_header("content-type", Matcher::Regex("^multipart/form-data; boundary=".into()))
        .match_body(Matcher::AllOf(vec![
            Matcher::Regex(r#"name="file"; filename="new.png""#.into()),
            Matcher::Regex("Content-Type: image/png".into()),
            Matcher::Regex(r#"name="minorEdit"\r\n\r\ntrue"#.into()),
        ]))
        .with_body(json!({ "results": [{ "id": "att9", "title": "new.png", "extensions": { "fileId": "new-file-id", "fileSize": 8 } }] }).to_string())
        .create();
    let update = site.expect_update(3, vec![
        Matcher::PartialJson(json!({ "title": "rfluence image API test" })),
        sent_body_contains("An added paragraph."),
        sent_body_contains(r#""id":"new-file-id""#),
        // The existing images keep their attachments' fileIds.
        sent_body_contains(r#""id":"b9e35bcf-7773-4ae5-a0af-0bc4d58060fe""#),
    ]);
    let label = site
        .server
        .mock("POST", format!("/wiki/rest/api/content/{ID}/label").as_str())
        .match_body(Matcher::Json(json!([{ "prefix": "global", "name": "new-label" }])))
        .with_body(json!({ "results": [] }).to_string())
        .create();
    let property_update = site
        .server
        .mock("PUT", format!("/wiki/api/v2/pages/{ID}/properties/77").as_str())
        .match_body(Matcher::Json(json!({
            "key": "rfluence",
            "value": { "managed": true, "version": 3, "path": "page.md", "config_labels": ["x"] },
            "version": { "number": 5 },
        })))
        .with_body("{}")
        .create();

    let o = site.upload(&[]);
    assert!(o.status.success(), "{}", err(&o));
    attach.assert();
    update.assert();
    label.assert();
    property_update.assert();
    assert!(out(&o).contains("(version 2 -> 3)"), "{}", out(&o));
    assert!(out(&o).contains("images uploaded: new.png"), "{}", out(&o));
    assert!(out(&o).contains("labels added: new-label"), "{}", out(&o));
    // Only the rfluence: block changed: the new version and the label as Confluence has it.
    let expected = edited.replace("  version: 2\n", "  version: 3\n").replace("labels: [New Label]", "labels: [new-label]");
    similar_asserts::assert_eq!(site.read(), expected);
}

#[test]
fn an_unchanged_file_sends_nothing() {
    let mut site = Site::new("unchanged", None, 2, None);
    site.fetch();
    let update = site.server.mock("PUT", Matcher::Any).expect(0).create();
    let post = site.server.mock("POST", Matcher::Any).expect(0).create();
    let before = site.read();
    let o = site.upload(&[]);
    assert!(o.status.success(), "{}", err(&o));
    assert!(out(&o).contains("is up to date (version 2)"), "{}", out(&o));
    update.assert();
    post.assert();
    assert_eq!(site.read(), before);
}

#[test]
fn dry_run_shows_the_plan_and_changes_nothing() {
    let mut site = Site::new("dry-run", None, 2, None);
    site.fetch();
    let edited = site.read().replace("# rfluence image API test", "# Renamed") + "\n![](page.assets/new.png)\n";
    site.write(&edited);
    std::fs::write(site.path("page.assets/new.png"), b"new").unwrap();
    // A changed image of the same size as its attachment is compared by content.
    let green = site.path("page.assets/green.png");
    let mut bytes = std::fs::read(&green).unwrap();
    *bytes.last_mut().unwrap() ^= 1;
    std::fs::write(&green, bytes).unwrap();
    let put = site.server.mock("PUT", Matcher::Any).expect(0).create();
    let post = site.server.mock("POST", Matcher::Any).expect(0).create();

    let o = site.upload(&["--dry-run"]);
    assert!(o.status.success(), "{}", err(&o));
    let text = out(&o);
    assert!(text.starts_with(&format!("Dry run: would update page {ID} \"rfluence image API test\" (version 2 -> 3)")), "{text}");
    assert!(text.contains("title: \"rfluence image API test\" -> \"Renamed\""), "{text}");
    assert!(text.contains("images to upload: new.png"), "{text}");
    assert!(text.contains("images to update: green.png"), "{text}");
    put.assert();
    post.assert();
    assert_eq!(site.read(), edited);
}

#[test]
fn refuses_to_overwrite_changes_made_in_confluence() {
    let mut site = Site::new("conflict", None, 2, None);
    site.fetch();
    site.write(&(site.read() + "\nLocal edit.\n"));
    // Someone edits the page in Confluence: now version 5.
    let newer = page_json(&site.server, None, 5);
    site.server.mock("GET", format!("/wiki/api/v2/pages/{ID}").as_str()).match_query(Matcher::Any).with_body(newer).create();
    let put = site.server.mock("PUT", Matcher::Any).expect(0).create();
    let o = site.upload(&[]);
    assert_eq!(o.status.code(), Some(6), "{}", err(&o));
    assert!(err(&o).contains("changed in Confluence since this file was fetched (version 2 -> 5)"), "{}", err(&o));
    put.assert();

    // --force overwrites them.
    let update = site.expect_update(6, vec![sent_body_contains("Local edit.")]);
    let o = site.upload(&["--force"]);
    assert!(o.status.success(), "{}", err(&o));
    update.assert();
    assert!(site.read().contains("  version: 6\n"));
}

#[test]
fn keeps_inline_comments_on_their_text() {
    let annotation = json!({ "type": "annotation", "attrs": { "annotationType": "inlineComment", "id": "c-1" } });
    let adf = json!({ "type": "doc", "version": 1, "content": [
        { "type": "paragraph", "content": [
            { "type": "text", "text": "Keep " },
            { "type": "text", "text": "this text", "marks": [annotation] },
            { "type": "text", "text": " and " },
            { "type": "text", "text": "drop this", "marks": [{ "type": "annotation", "attrs": { "annotationType": "inlineComment", "id": "c-2" } }] },
        ]},
    ]});
    let mut site = Site::new("comments", Some(adf), 2, None);
    site.fetch();
    let md = site.read();
    assert!(md.contains("Keep this text and drop this\n"), "{md}");
    site.write(&md.replace("Keep this text and drop this", "First, keep this text and that"));
    let update = site.expect_update(3, vec![sent_body_contains(
        r#"{"type":"text","marks":[{"type":"annotation","attrs":{"annotationType":"inlineComment","id":"c-1"}}],"text":"this text"}"#,
    )]);
    let o = site.upload(&[]);
    assert!(o.status.success(), "{}", err(&o));
    update.assert();
    assert!(out(&o).contains(r#"1 inline comment was detached (text changed or no longer unique): "drop this""#), "{}", out(&o));
}

#[test]
fn stops_before_sending_anything() {
    let site = Site::new("stops", None, 2, None);
    site.fetch();
    let md = site.read();
    let cases = [
        // `rfluence check` errors.
        (md.clone() + "\nA footnote[^1].\n\n[^1]: Note.\n", 1, "2 errors (content Confluence can't store), nothing uploaded"),
        (md.clone() + "\n[setup](./setup.md)\n", 2, "links to files without a Confluence page: line"),
        (md.replace("  labels: []", "  labels: [\"a.b\"]"), 2, "contains '.'"),
        (md.replace("  version: 2", "  verison: 2"), 2, "unknown keys under `rfluence:` in the frontmatter: verison"),
        (md.replace("  id: \"131074\"\n", ""), 2, "has no page ID"),
        (md.replace("](page.assets/green.png)", "](page.assets/missing.png)"), 2, "image files not found"),
    ];
    for (file, code, message) in cases {
        site.write(&file);
        let o = site.upload(&[]);
        assert_eq!(o.status.code(), Some(code), "{message}: {}", err(&o));
        assert!(err(&o).contains(message), "{message}: {}", err(&o));
    }
}
