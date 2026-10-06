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
fn page_json(
    server: &mockito::ServerGuard,
    adf: Option<&serde_json::Value>,
    version: u64,
) -> String {
    let mut page: serde_json::Value = serde_json::from_str(&fixture("page.json")).unwrap();
    let adf = adf
        .cloned()
        .unwrap_or_else(|| serde_json::from_str(&fixture("adf.json")).unwrap());
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
    fn new(
        name: &str,
        adf: Option<serde_json::Value>,
        version: u64,
        property: Option<serde_json::Value>,
    ) -> Site {
        let mut server = mockito::Server::new();
        let body = page_json(&server, adf.as_ref(), version);
        server
            .mock("GET", format!("/wiki/api/v2/pages/{ID}").as_str())
            .match_query(Matcher::Any)
            .with_body(body)
            .create();
        let attachments = fixture("attachments.json");
        for a in serde_json::from_str::<serde_json::Value>(&attachments).unwrap()["results"]
            .as_array()
            .unwrap()
        {
            let bytes = std::fs::read(
                fixtures()
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
                format!("/wiki/api/v2/pages/{ID}/attachments").as_str(),
            )
            .match_query(Matcher::Any)
            .with_body(attachments)
            .create();
        server
            .mock(
                "GET",
                format!("/wiki/api/v2/pages/{ID}/properties").as_str(),
            )
            .match_query(Matcher::UrlEncoded("key".into(), "rfluence".into()))
            .with_body(
                json!({ "results": property.into_iter().collect::<Vec<_>>(), "_links": {} })
                    .to_string(),
            )
            .create();
        Site::bare(name, server)
    }

    /// A mock site serving nothing yet, and an empty project directory.
    fn bare(name: &str, server: mockito::ServerGuard) -> Site {
        let dir =
            std::env::temp_dir().join(format!("rfluence-upload-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Site { server, dir }
    }

    /// A site with space ENG (homepage 100), where `existing` is the page titled "New page".
    fn with_space(name: &str, existing: Option<&str>) -> Site {
        let mut server = mockito::Server::new();
        server
            .mock("GET", "/wiki/api/v2/spaces")
            .match_query(Matcher::UrlEncoded("keys".into(), "ENG".into()))
            .with_body(json!({ "results": [{ "id": "9", "key": "ENG", "homepageId": "100" }], "_links": {} }).to_string())
            .create();
        let found: Vec<_> = existing.iter().map(|id| json!({ "id": id })).collect();
        server
            .mock("GET", "/wiki/rest/api/content")
            .match_query(Matcher::AllOf(vec![
                Matcher::UrlEncoded("spaceKey".into(), "ENG".into()),
                Matcher::UrlEncoded("title".into(), "New page".into()),
            ]))
            .with_body(json!({ "results": found }).to_string())
            .create();
        Site::bare(name, server)
    }

    /// The v2 response for page 500 in ENG.
    fn created(&self, version: u64) -> String {
        json!({
            "id": "500", "title": "New page", "parentId": "100", "spaceId": "9",
            "version": { "number": version, "createdAt": "2026-10-05T10:00:00.000Z" },
            "_links": { "webui": "/spaces/ENG/pages/500/New+page", "base": format!("{}/wiki", self.server.url()) },
        })
        .to_string()
    }

    fn expect_labels_and_property(
        &mut self,
        labels: serde_json::Value,
        version: u64,
        path: &str,
    ) -> (mockito::Mock, mockito::Mock) {
        let label = self
            .server
            .mock("POST", "/wiki/rest/api/content/500/label")
            .match_body(Matcher::Json(labels))
            .with_body(json!({ "results": [] }).to_string())
            .create();
        let property = self
            .server
            .mock("POST", "/wiki/api/v2/pages/500/properties")
            .match_body(Matcher::Json(json!({
                "key": "rfluence",
                "value": { "managed": true, "version": version, "path": path, "config_labels": [] },
            })))
            .with_body("{}")
            .create();
        (label, property)
    }

    fn write_file(&self, file: &str, md: &str) {
        std::fs::write(self.path(file), md).unwrap();
    }

    fn upload_file(&self, file: &str, extra: &[&str]) -> Output {
        let path = self.path(file);
        let mut args = vec!["upload", path.to_str().unwrap()];
        args.extend(extra);
        self.rf(&args)
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
        let mut all = vec![Matcher::PartialJson(
            json!({ "id": ID, "status": "current", "version": { "number": version } }),
        )];
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
    s.chars()
        .map(|c| {
            if "\\.+*?()|[]{}^$".contains(c) {
                format!("\\{c}")
            } else {
                c.to_string()
            }
        })
        .collect()
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
    let edited = md.replace("labels: []", "labels: [New Label]")
        + "\nAn added paragraph.\n\n![](page.assets/new.png)\n";
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
    let update = site.expect_update(
        3,
        vec![
            Matcher::PartialJson(json!({ "title": "rfluence image API test" })),
            sent_body_contains("An added paragraph."),
            sent_body_contains(r#""id":"new-file-id""#),
            // The existing images keep their attachments' fileIds.
            sent_body_contains(r#""id":"b9e35bcf-7773-4ae5-a0af-0bc4d58060fe""#),
        ],
    );
    let label = site
        .server
        .mock(
            "POST",
            format!("/wiki/rest/api/content/{ID}/label").as_str(),
        )
        .match_body(Matcher::Json(
            json!([{ "prefix": "global", "name": "new-label" }]),
        ))
        .with_body(json!({ "results": [] }).to_string())
        .create();
    let property_update = site
        .server
        .mock(
            "PUT",
            format!("/wiki/api/v2/pages/{ID}/properties/77").as_str(),
        )
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
    let expected = edited
        .replace("  version: 2\n", "  version: 3\n")
        .replace("labels: [New Label]", "labels: [new-label]");
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
    let edited = site
        .read()
        .replace("# rfluence image API test", "# Renamed")
        + "\n![](page.assets/new.png)\n";
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
    assert!(
        text.starts_with(&format!(
            "Dry run: would update page {ID} \"rfluence image API test\" (version 2 -> 3)"
        )),
        "{text}"
    );
    assert!(
        text.contains("title: \"rfluence image API test\" -> \"Renamed\""),
        "{text}"
    );
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
    site.server
        .mock("GET", format!("/wiki/api/v2/pages/{ID}").as_str())
        .match_query(Matcher::Any)
        .with_body(newer)
        .create();
    let put = site.server.mock("PUT", Matcher::Any).expect(0).create();
    let o = site.upload(&[]);
    assert_eq!(o.status.code(), Some(6), "{}", err(&o));
    assert!(
        err(&o).contains("changed in Confluence since this file was fetched (version 2 -> 5)"),
        "{}",
        err(&o)
    );
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
    site.write(&md.replace(
        "Keep this text and drop this",
        "First, keep this text and that",
    ));
    let update = site.expect_update(3, vec![sent_body_contains(
        r#"{"type":"text","marks":[{"type":"annotation","attrs":{"annotationType":"inlineComment","id":"c-1"}}],"text":"this text"}"#,
    )]);
    let o = site.upload(&[]);
    assert!(o.status.success(), "{}", err(&o));
    update.assert();
    assert!(
        out(&o).contains(
            r#"1 inline comment was detached (text changed or no longer unique): "drop this""#
        ),
        "{}",
        out(&o)
    );
}

#[test]
fn stops_before_sending_anything() {
    let site = Site::new("stops", None, 2, None);
    site.fetch();
    let md = site.read();
    // Line numbers are the file's (frontmatter and title included).
    let appended = md.lines().count() + 2;
    let green = md
        .lines()
        .position(|l| l.contains("](page.assets/green.png)"))
        .unwrap()
        + 1;
    let cases = [
        // `rfluence check` errors.
        (
            md.clone() + "\nA footnote[^1].\n\n[^1]: Note.\n",
            1,
            "2 errors (content Confluence can't store), nothing uploaded".to_string(),
        ),
        (
            md.clone() + "\n[setup](./setup.md)\n",
            2,
            format!(
                "links to files without a Confluence page: line {appended}: ./setup.md (no such file)"
            ),
        ),
        (
            md.replace("  labels: []", "  labels: [\"a.b\"]"),
            2,
            "contains '.'".to_string(),
        ),
        (
            md.replace("  version: 2", "  verison: 2"),
            2,
            "unknown keys under `rfluence:` in the frontmatter: verison".to_string(),
        ),
        (
            md.replacen("](page.assets/green.png)", "](page.assets/missing.png)", 1),
            2,
            format!(
                "image files not found, and the page has no attachment with their names: line {green}: page.assets/missing.png"
            ),
        ),
    ];
    for (file, code, message) in cases {
        let message = message.as_str();
        site.write(&file);
        let o = site.upload(&[]);
        assert_eq!(o.status.code(), Some(code), "{message}: {}", err(&o));
        assert!(err(&o).contains(message), "{message}: {}", err(&o));
    }
}

#[test]
fn creates_a_page_and_records_it_in_the_file() {
    let mut site = Site::with_space("create", None);
    let md = "---\ntags: [kept]\nrfluence:\n  space_key: ENG\n  labels: [Docs]\n---\n\n# New page\n\nHello.\n";
    site.write_file("new.md", md);
    let create = site
        .server
        .mock("POST", "/wiki/api/v2/pages")
        .match_body(Matcher::AllOf(vec![
            Matcher::PartialJson(json!({ "spaceId": "9", "parentId": "100", "title": "New page", "status": "current" })),
            sent_body_contains(r#""text":"Hello.""#),
        ]))
        .with_body(site.created(1))
        .create();
    let (label, property) = site.expect_labels_and_property(
        json!([{ "prefix": "global", "name": "docs" }]),
        1,
        "new.md",
    );

    let o = site.upload_file("new.md", &[]);
    assert!(o.status.success(), "{}", err(&o));
    create.assert();
    label.assert();
    property.assert();
    let url = format!("{}/wiki/spaces/ENG/pages/500", site.server.url());
    assert_eq!(
        out(&o),
        format!(
            "Created page 500 \"New page\" in space ENG from {} (version 1)\n  {url}\n  labels added: docs\n",
            site.path("new.md").display()
        )
    );
    let expected = format!(
        "---\ntags: [kept]\nrfluence:\n  id: \"500\"\n  space_key: ENG\n  parent: \"100\"\n  version: 1\n  url: {url}\n  labels: [docs]\n---\n\n# New page\n\nHello.\n"
    );
    similar_asserts::assert_eq!(
        std::fs::read_to_string(site.path("new.md")).unwrap(),
        expected
    );
}

#[test]
fn creates_a_page_with_images_then_attaches_them() {
    let mut site = Site::with_space("create-images", None);
    // No frontmatter at all: --space and --parent say where.
    site.write_file("new.md", "# New page\n\n![box](new.assets/box.png)\n");
    std::fs::create_dir_all(site.path("new.assets")).unwrap();
    std::fs::write(site.path("new.assets/box.png"), b"\x89PNG box").unwrap();
    let create = site
        .server
        .mock("POST", "/wiki/api/v2/pages")
        .match_body(Matcher::PartialJson(
            json!({ "parentId": "42", "title": "New page" }),
        ))
        .with_body(site.created(1))
        .create();
    let attach = site
        .server
        .mock("POST", "/wiki/rest/api/content/500/child/attachment")
        .match_body(Matcher::Regex(r#"filename="box.png""#.into()))
        .with_body(json!({ "results": [{ "id": "att1", "title": "box.png", "extensions": { "fileId": "box-file" } }] }).to_string())
        .create();
    let body = site
        .server
        .mock("PUT", "/wiki/api/v2/pages/500")
        .match_body(Matcher::AllOf(vec![
            Matcher::PartialJson(json!({ "version": { "number": 2 } })),
            sent_body_contains(r#""collection":"contentId-500""#),
            sent_body_contains(r#""id":"box-file""#),
        ]))
        .with_body(site.created(2))
        .create();
    let (label, property) = site.expect_labels_and_property(json!([]), 2, "new.md");
    let label = label.expect(0);

    let o = site.upload_file("new.md", &["--space", "ENG", "--parent", "42"]);
    assert!(o.status.success(), "{}", err(&o));
    create.assert();
    attach.assert();
    body.assert();
    label.assert();
    property.assert();
    assert!(
        out(&o).contains("(version 2)") && out(&o).contains("images uploaded: box.png"),
        "{}",
        out(&o)
    );
    let written = std::fs::read_to_string(site.path("new.md")).unwrap();
    assert!(
        written.starts_with(
            "---\nrfluence:\n  id: \"500\"\n  space_key: ENG\n  parent: \"100\"\n  version: 2\n"
        ),
        "{written}"
    );
    assert!(
        written.ends_with("---\n\n# New page\n\n![box](new.assets/box.png)\n"),
        "{written}"
    );
}

#[test]
fn never_creates_a_duplicate_title() {
    let mut site = Site::with_space("duplicate", Some("321"));
    site.write_file("new.md", "---\nrfluence:\n  space_key: ENG\n---\n\n# New page\n\nRewritten by an LLM that dropped the ID.\n");
    let post = site.server.mock("POST", Matcher::Any).expect(0).create();
    let o = site.upload_file("new.md", &[]);
    assert_eq!(o.status.code(), Some(6), "{}", err(&o));
    assert!(
        err(&o).contains("space ENG already has a page titled \"New page\" (page 321)"),
        "{}",
        err(&o)
    );
    post.assert();

    // --force overwrites that page, and the file gets its ID.
    let page = json!({
        "id": "321", "title": "New page", "parentId": "100",
        "version": { "number": 4 }, "labels": { "results": [] },
        "body": { "atlas_doc_format": { "value": r#"{"type":"doc","version":1,"content":[]}"# } },
        "_links": { "webui": "/spaces/ENG/pages/321", "base": format!("{}/wiki", site.server.url()) },
    });
    site.server
        .mock("GET", "/wiki/api/v2/pages/321")
        .match_query(Matcher::Any)
        .with_body(page.to_string())
        .create();
    site.server
        .mock("GET", "/wiki/api/v2/pages/321/attachments")
        .match_query(Matcher::Any)
        .with_body(r#"{"results":[]}"#)
        .create();
    site.server
        .mock("GET", "/wiki/api/v2/pages/321/properties")
        .match_query(Matcher::Any)
        .with_body(r#"{"results":[]}"#)
        .create();
    let mut updated = page.clone();
    updated["version"]["number"] = 5.into();
    let put = site
        .server
        .mock("PUT", "/wiki/api/v2/pages/321")
        .match_body(Matcher::AllOf(vec![
            Matcher::PartialJson(json!({ "version": { "number": 5 } })),
            sent_body_contains("dropped the ID"),
        ]))
        .with_body(updated.to_string())
        .create();
    let o = site.upload_file("new.md", &["--force"]);
    assert!(o.status.success(), "{}", err(&o));
    put.assert();
    assert!(
        std::fs::read_to_string(site.path("new.md"))
            .unwrap()
            .contains("  id: \"321\"\n  space_key: ENG\n  parent: \"100\"\n  version: 5\n")
    );
}

#[test]
fn dry_run_create_and_what_a_new_page_needs() {
    let mut site = Site::with_space("create-checks", None);
    let post = site.server.mock("POST", Matcher::Any).expect(0).create();
    site.write_file("new.md", "# New page\n\nText.\n");
    let o = site.upload_file("new.md", &["--space", "ENG", "--dry-run"]);
    assert!(o.status.success(), "{}", err(&o));
    assert_eq!(
        out(&o),
        "Dry run: would create page \"New page\" in space ENG under 100\n"
    );
    assert_eq!(
        std::fs::read_to_string(site.path("new.md")).unwrap(),
        "# New page\n\nText.\n"
    );

    let o = site.upload_file("new.md", &[]);
    assert_eq!(o.status.code(), Some(2));
    assert!(
        err(&o).contains("no space to create the page in"),
        "{}",
        err(&o)
    );
    site.write_file("untitled.md", "Text without a title.\n");
    let o = site.upload_file("untitled.md", &["--space", "ENG"]);
    assert_eq!(o.status.code(), Some(2));
    assert!(
        err(&o).contains("has no title for the new page"),
        "{}",
        err(&o)
    );
    site.write_file("new.md", "# New page\n\n![](missing.png)\n");
    let o = site.upload_file("new.md", &["--space", "ENG"]);
    assert_eq!(o.status.code(), Some(2));
    assert!(err(&o).contains("image files not found"), "{}", err(&o));
    post.assert();
}

/// A Mermaid fence becomes a merfluence diagram if the site has merfluence, else a Mermaid
/// Diagrams Viewer diagram (source expand + macro), else a code block.
#[test]
fn uploads_mermaid_for_the_apps_the_site_has() {
    const MERFLUENCE: (&str, &str) = (
        "5321c3d1-955d-42ac-9f09-d4d6f0802224",
        "04b85365-6260-47d9-9f03-8f42e258aab7",
    );
    const VIEWER: (&str, &str) = (
        "23392b90-4271-4239-98ca-a3e96c663cbb",
        "63d4d207-ac2f-4273-865c-0240d37f044a",
    );
    /// (case, the site's Mermaid apps (none: the lookup fails), what the new page's body has)
    type Case<'a> = (&'a str, Option<Vec<(&'a str, &'a str)>>, Vec<&'a str>);
    let cases: [Case; 4] = [
        (
            "both",
            Some(vec![VIEWER, MERFLUENCE]),
            vec![
                r#""extensionKey":"5321c3d1-955d-42ac-9f09-d4d6f0802224/04b85365"#,
                r#""source":"graph LR\n  a --> b""#,
            ],
        ),
        (
            "viewer",
            Some(vec![VIEWER]),
            vec![
                r#""title":"Mermaid source""#,
                r#""extensionKey":"23392b90-4271-4239-98ca-a3e96c663cbb/63d4d207"#,
            ],
        ),
        (
            "neither",
            Some(vec![]),
            vec![r#"{"type":"codeBlock","attrs":{"language":"mermaid"}"#],
        ),
        (
            "lookup fails",
            None,
            vec![r#"{"type":"codeBlock","attrs":{"language":"mermaid"}"#],
        ),
    ];
    for (name, installed, sent) in cases {
        let mut site = Site::with_space(&format!("mermaid-{}", name.replace(' ', "-")), None);
        site.write_file(
            "new.md",
            "# New page\n\n```mermaid\ngraph LR\n  a --> b\n```\n",
        );
        site.server
            .mock("GET", "/_edge/tenant_info")
            .with_body(r#"{"cloudId":"cloud-1"}"#)
            .create();
        let graphql = site.server.mock("POST", "/gateway/api/graphql").match_body(Matcher::PartialJson(json!({
            "operationName": "rfluence_macros",
            "variables": { "contextIds": ["ari:cloud:confluence::site/cloud-1"], "type": "xen:macro" },
        })));
        let graphql = match &installed {
            Some(apps) => {
                let macros: Vec<_> = apps.iter().map(|(app, env)| json!({ "appId": app, "environmentId": env, "key": "mermaid-diagram" })).collect();
                graphql.with_body(json!({ "data": { "extensionContexts": [{ "extensionsByType": macros }] } }).to_string())
            }
            None => graphql.with_body(json!({ "errors": [{ "message": "Not allowed" }] }).to_string()),
        }
        .create();
        let create = site
            .server
            .mock("POST", "/wiki/api/v2/pages")
            .match_body(Matcher::AllOf(
                sent.iter().map(|s| sent_body_contains(s)).collect(),
            ))
            .with_body(site.created(1))
            .create();
        site.server
            .mock("POST", "/wiki/api/v2/pages/500/properties")
            .with_body("{}")
            .create();

        let o = site.upload_file("new.md", &["--space", "ENG"]);
        assert!(o.status.success(), "{name}: {}", err(&o));
        graphql.assert();
        create.assert();
        let warned = err(&o).contains("couldn't find out which Mermaid app the site has (Confluence returned HTTP 200: Not allowed)");
        assert_eq!(warned, installed.is_none(), "{name}: {}", err(&o));
    }
}

/// Single file: a `parent` that differs from the page's is a warning; `--move` moves the page
/// (a new version without a body: Confluence keeps it) and writes the new version back.
#[test]
fn moves_a_page_to_its_parent() {
    let mut site = Site::new("move", None, 2, None);
    site.fetch();
    let md = site
        .read()
        .replace("  parent: \"753877\"", "  parent: \"999\"");
    site.write(&md);
    let put = site.server.mock("PUT", Matcher::Any).expect(0).create();
    let o = site.upload(&[]);
    assert!(o.status.success(), "{}", err(&o));
    assert!(
        err(&o).contains(
            "`parent` is 999, but the page is under 753877; it's left where it is; --move moves it"
        ),
        "{}",
        err(&o)
    );
    assert!(out(&o).contains("is up to date"), "{}", out(&o));
    put.assert();
    put.remove();
    assert!(
        site.read().contains("  parent: \"999\"\n"),
        "the requested parent is kept: {}",
        site.read()
    );

    let mut moved: serde_json::Value =
        serde_json::from_str(&page_json(&site.server, None, 3)).unwrap();
    moved["parentId"] = "999".into();
    let put = site
        .server
        .mock("PUT", format!("/wiki/api/v2/pages/{ID}").as_str())
        .match_body(Matcher::PartialJson(json!({ "parentId": "999", "version": { "number": 3 }, "title": "rfluence image API test" })))
        .with_body(moved.to_string())
        .create();
    let o = site.upload(&["--move"]);
    assert!(o.status.success(), "{}", err(&o));
    put.assert();
    assert!(
        out(&o).contains("(version 2 -> 3)") && out(&o).contains("  parent: 753877 -> 999"),
        "{}",
        out(&o)
    );
    assert!(
        site.read().contains("  parent: \"999\"\n  version: 3\n"),
        "{}",
        site.read()
    );
}

/// `--warnings-are-errors`: content uploaded as a close equivalent (here `<kbd>`, as inline
/// code) stops the upload too, unless `--force`.
#[test]
fn warnings_can_stop_an_upload() {
    let site = Site::new("warnings", None, 2, None);
    site.fetch();
    site.write(&(site.read() + "\nPress <kbd>Ctrl</kbd>.\n"));
    let o = site.upload(&["--dry-run"]);
    assert!(o.status.success(), "{}", err(&o));
    assert!(
        err(&o).contains("warning: `<kbd>` written as inline code"),
        "{}",
        err(&o)
    );

    let o = site.upload(&["--dry-run", "--warnings-are-errors"]);
    assert_eq!(o.status.code(), Some(1), "{}", err(&o));
    assert!(err(&o).contains("1 warning (--warnings-are-errors), nothing uploaded; fix them, or use --force to upload the approximations"), "{}", err(&o));

    let o = site.upload(&["--dry-run", "--warnings-are-errors", "--force"]);
    assert!(o.status.success(), "{}", err(&o));
}

/// Files with Windows line endings (`\r\n`) work like any other, and keep their line endings
/// when the upload writes the new version back.
#[test]
fn handles_windows_line_endings() {
    let mut site = Site::new("crlf", None, 2, None);
    site.fetch();
    let crlf = |s: &str| s.replace('\n', "\r\n");
    site.write(&crlf(&site.read()));
    let unchanged = site.upload(&[]);
    assert!(unchanged.status.success(), "{}", err(&unchanged));
    assert!(
        out(&unchanged).contains("is up to date (version 2)"),
        "the frontmatter was read: {}",
        out(&unchanged)
    );

    site.write(&crlf(
        &(site.read().replace("\r\n", "\n") + "\nLocal edit.\n"),
    ));
    let update = site.expect_update(3, vec![sent_body_contains("Local edit.")]);
    let o = site.upload(&[]);
    assert!(o.status.success(), "{}", err(&o));
    update.assert();
    let md = site.read();
    assert!(
        md.contains("  version: 3\r\n") && !md.replace("\r\n", "").contains('\n'),
        "still \\r\\n everywhere: {md:?}"
    );
}
