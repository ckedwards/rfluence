//! Round trips: `normalize(md) == fetch(upload(md))` offline, and fetch -> upload -> fetch
//! on real pages. See design.md, "What 'same content' means".

mod common;

use std::collections::HashMap;

use common::*;
use rfluence_convert::adf::Node;
use rfluence_convert::{
    Diagnostic, FetchContext, LinkTarget, MermaidApp, PageRef, Severity, UploadContext,
    adf_to_markdown, check, local_links, markdown_to_adf, normalize,
};

/// Upload and fetch contexts for a corpus file: its images get made-up attachment IDs.
fn corpus_ctx(name: &str, md: &str) -> (UploadContext, FetchContext) {
    let assets = format!("{name}.assets");
    let mut media = HashMap::new();
    let mut attachments = HashMap::new();
    for (i, path) in md
        .match_indices(&format!("{assets}/"))
        .map(|(i, _)| i)
        .enumerate()
    {
        let end = md[path..].find([')', ' ', '"']).unwrap() + path;
        let file = md[path + assets.len() + 1..end].to_string();
        let id = format!("file-{i}");
        media.insert(format!("{assets}/{file}"), id.clone());
        attachments.insert(id, file);
    }
    // Links to other markdown files get made-up pages.
    let mut pages = HashMap::new();
    let mut links = HashMap::new();
    for (_, path) in local_links(md) {
        let id = (1000 + pages.len()).to_string();
        if !pages.contains_key(&path) {
            pages.insert(
                path.clone(),
                PageRef {
                    url: format!("https://corpus.atlassian.net/wiki/spaces/C/pages/{id}"),
                    headings: vec![],
                },
            );
            links.insert(
                id,
                LinkTarget {
                    path,
                    headings: vec![],
                },
            );
        }
    }
    let upload = UploadContext {
        page_id: Some("1".into()),
        media,
        custom_emoji: HashMap::new(),
        mermaid: MermaidApp::from_extension_key(MERMAID),
        pages,
        ..Default::default()
    };
    let fetch = FetchContext {
        page_id: Some("1".into()),
        assets_dir: assets,
        attachments,
        site_host: Some("corpus.atlassian.net".into()),
        links,
        ..Default::default()
    };
    (upload, fetch)
}

fn strip_frontmatter(md: &str) -> String {
    match md
        .strip_prefix("---\n")
        .and_then(|rest| rest.split_once("\n---\n"))
    {
        Some((_, body)) => body.trim_start_matches('\n').to_string(),
        None => md.to_string(),
    }
}

/// LLM-style markdown survives upload and fetch, in normalized form. `unsupported.md`
/// holds markdown with no Confluence equivalent and is checked separately;
/// `approximated.md` holds markdown that uploads as a close equivalent (warnings only).
#[test]
fn corpus_round_trips() {
    let mut failures = Vec::new();
    for (name, md) in corpus() {
        if name == "unsupported" {
            continue;
        }
        let (up, down) = corpus_ctx(&name, &md);
        let upload = markdown_to_adf(&md, &up).unwrap_or_else(|e| panic!("{name}: {e}"));
        let back = adf_to_markdown(&upload.doc, &down);
        // Upload sends the body only; frontmatter is the CLI's job (design.md, "Frontmatter").
        let expected = strip_frontmatter(&normalize(&md));
        if back != expected {
            failures.push(format!(
                "{name}:\n{}",
                similar_asserts::SimpleDiff::from_str(
                    &expected,
                    &back,
                    "normalize(md)",
                    "fetch(upload(md))"
                )
            ));
        }
        let unexpected: Vec<_> = upload
            .diagnostics
            .iter()
            .filter(|d| name != "approximated" || d.severity == Severity::Error)
            .collect();
        if !unexpected.is_empty() {
            failures.push(format!("{name} diagnostics: {unexpected:#?}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

fn corpus_file(name: &str) -> String {
    corpus().into_iter().find(|(n, _)| n == name).unwrap().1
}

/// Each diagnostic message containing `expected`, with its severity.
fn assert_reported(diags: &[Diagnostic], severity: Severity, expected: &[&str]) {
    for e in expected {
        assert!(
            diags
                .iter()
                .any(|d| d.severity == severity && d.message.contains(e)),
            "no {severity} containing {e:?}: {diags:#?}"
        );
    }
}

/// Markdown with no Confluence equivalent is reported as errors.
#[test]
fn unsupported_markdown_is_an_error() {
    let md = corpus_file("unsupported");
    let (up, _) = corpus_ctx("unsupported", &md);
    let upload = markdown_to_adf(&md, &up).unwrap();
    assert!(upload.has_errors());
    assert_reported(
        &upload.diagnostics,
        Severity::Error,
        &[
            "footnotes",
            "raw HTML block",
            "inline images",
            "nested quote",
            "alert in a quote",
        ],
    );
    assert_eq!(check(&md), upload.diagnostics);
}

/// Markdown with a close equivalent is reported as warnings.
#[test]
fn approximated_markdown_is_a_warning() {
    let diags = check(&corpus_file("approximated"));
    assert!(
        diags.iter().all(|d| d.severity == Severity::Warning),
        "{diags:#?}"
    );
    assert_reported(
        &diags,
        Severity::Warning,
        &[
            "`<kbd>` written as inline code",
            "`<b>` written as markdown",
            "`<ins>` written as `<u>`",
            "`<abbr",
            "`<br>`",
            "image title",
            "alert title",
            "`<details open>`",
        ],
    );
}

#[test]
fn check_reports_broken_structure() {
    let cases = [
        (
            "<details><summary>x</summary>\n\nbody\n",
            "without a matching `</details>`",
        ),
        ("text\n\n</details>\n", "without a matching `<details>`"),
        (
            "- item\n\n  <details><summary>x</summary>\n\n  body\n\n  </details>\n",
            "can't be an expand here",
        ),
        (
            "<!-- rf: columns=50,50 -->\n\na\n",
            "without `<!-- rf: end-columns -->`",
        ),
        (
            "<!-- rf: columns=50,50 -->\n\na\n\n<!-- rf: end-columns -->\n",
            "1 columns but `columns=` lists 2",
        ),
        ("<!-- rf: column -->\n", "layout marker outside a layout"),
        ("```adf\n{not json\n```\n", "invalid ```adf block"),
    ];
    for (md, expected) in cases {
        assert_reported(&check(md), Severity::Error, &[expected]);
    }
    assert!(check("# Clean\n\nNothing to report.\n").is_empty());
}

/// Expands nest one level: `<details>` inside `<details>` is a nested expand.
#[test]
fn details_inside_details_is_a_nested_expand() {
    let md = "<details><summary>Outer</summary>\n\n<details><summary>Inner</summary>\n\ntext\n\n</details>\n\n</details>\n";
    let upload = markdown_to_adf(md, &UploadContext::default()).unwrap();
    assert!(upload.diagnostics.is_empty(), "{:#?}", upload.diagnostics);
    let outer = &upload.doc.content[0];
    assert_eq!(
        (outer.kind.as_str(), outer.attr_str("title")),
        ("expand", Some("Outer"))
    );
    assert_eq!(outer.content[0].kind, "nestedExpand");
}

/// fetch -> upload -> fetch gives the same markdown for every captured page.
#[test]
fn fixtures_round_trip() {
    let mut failures = Vec::new();
    for name in pages() {
        let doc = page_adf(&name, "adf.json");
        let md = adf_to_markdown(&doc, &fetch_ctx(&name));
        let upload = markdown_to_adf(&md, &upload_ctx(&name, &doc))
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        let back = adf_to_markdown(&upload.doc, &fetch_ctx(&name));
        if back != md {
            failures.push(format!(
                "{name}:\n{}",
                similar_asserts::SimpleDiff::from_str(
                    &md,
                    &back,
                    "fetch(page)",
                    "fetch(upload(fetch(page)))"
                )
            ));
        }
        if !upload.diagnostics.is_empty() {
            failures.push(format!("{name} diagnostics: {:#?}", upload.diagnostics));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

/// `[Title](url)<!-- rf: card=inline -->` uploads as a smart link; its text isn't kept.
#[test]
fn marked_links_upload_as_smart_links() {
    let md = "See [Setup guide](https://x.atlassian.net/wiki/spaces/ENG/pages/22)<!-- rf: card=inline --> and [a text link](https://example.com).\n\nAt the end: [Setup guide](https://x.atlassian.net/wiki/spaces/ENG/pages/22)<!-- rf: card=inline -->\n";
    let doc = markdown_to_adf(md, &UploadContext::default()).unwrap().doc;
    let p = &doc.content[0].content;
    assert_eq!(p[1].kind, "inlineCard");
    assert_eq!(
        p[1].attr_str("url"),
        Some("https://x.atlassian.net/wiki/spaces/ENG/pages/22")
    );
    assert!(
        p.iter()
            .any(|n| n.text.as_deref() == Some("a text link") && n.mark("link").is_some())
    );
    assert!(!p.iter().any(|n| n.text.as_deref() == Some("Setup guide")));
    // At the end of a paragraph too (not read as the paragraph's settings).
    assert_eq!(doc.content[1].content.last().unwrap().kind, "inlineCard");
    assert!(doc.content[1].marks.is_empty());
}

/// Links to local markdown files upload as page URLs, with GitHub-style anchors translated
/// to the target page's Confluence anchors, and fetch -o turns them back into the same links.
#[test]
fn relative_links_upload_as_page_urls() {
    let url = "https://x.atlassian.net/wiki/spaces/ENG/pages/22";
    let md = "See [setup](./setup.md), [install](./setup.md#install--setup-v20), [a space](./my%20notes.md) and [Setup](./setup.md)<!-- rf: card=inline -->.\n";
    let headings = vec!["Overview".to_string(), "Install & Setup (v2.0)".to_string()];
    let mut ctx = UploadContext::default();
    ctx.pages.insert(
        "./setup.md".into(),
        PageRef {
            url: url.into(),
            headings: headings.clone(),
        },
    );
    ctx.pages.insert(
        "./my notes.md".into(),
        PageRef {
            url: "https://x.atlassian.net/wiki/spaces/ENG/pages/23".into(),
            headings: vec![],
        },
    );
    assert_eq!(
        local_links(md)
            .iter()
            .map(|(_, p)| p.as_str())
            .collect::<Vec<_>>(),
        ["./setup.md", "./setup.md", "./my notes.md", "./setup.md"]
    );

    let doc = markdown_to_adf(md, &ctx).unwrap().doc;
    let p = &doc.content[0].content;
    let href = |text: &str| {
        p.iter()
            .find(|n| n.text.as_deref() == Some(text))
            .and_then(|n| n.mark("link"))
            .and_then(|m| m.attr_str("href"))
    };
    assert_eq!(href("setup"), Some(url));
    assert_eq!(
        href("install"),
        Some(format!("{url}#Install-&-Setup-(v2.0)").as_str())
    );
    assert_eq!(
        href("a space"),
        Some("https://x.atlassian.net/wiki/spaces/ENG/pages/23")
    );
    assert_eq!(
        p.iter()
            .find(|n| n.is("inlineCard"))
            .and_then(|n| n.attr_str("url")),
        Some(url)
    );

    let fetch = FetchContext {
        site_host: Some("x.atlassian.net".into()),
        links: [
            (
                "22".to_string(),
                LinkTarget {
                    path: "./setup.md".into(),
                    headings,
                },
            ),
            (
                "23".to_string(),
                LinkTarget {
                    path: "./my notes.md".into(),
                    headings: vec![],
                },
            ),
        ]
        .into(),
        titles: [("22".to_string(), "Setup".to_string())].into(),
        ..Default::default()
    };
    assert_eq!(adf_to_markdown(&doc, &fetch), normalize(md));

    // Files without a page, and same-page or external links, are left alone.
    let err = markdown_to_adf(
        "[a](./missing.md) [b](../x/missing.md#top) [c](#here) [d](https://e.com/x.md)\n",
        &UploadContext::default(),
    )
    .unwrap_err();
    assert_eq!(err.links, ["./missing.md", "../x/missing.md"]);
    assert!(
        err.to_string()
            .contains("links to files without a page: ./missing.md, ../x/missing.md"),
        "{err}"
    );
    assert!(check("[a](./missing.md)\n").is_empty());
}

/// On a site with Mermaid Diagrams Viewer (and no merfluence), a Mermaid fence uploads as its
/// source in a collapsed expand plus the viewer's macro, and fetches back as the same fence.
#[test]
fn mermaid_viewer_diagrams_round_trip() {
    let md = "```mermaid\nflowchart TD\n  A --> B\n```\n\n<!-- rf: columns=50,50 -->\n\n```mermaid\nsequenceDiagram\n  A->>B: hi\n```\n\n<!-- rf: column -->\n\ntext\n\n<!-- rf: end-columns -->\n\n- In a list, an expand isn't allowed:\n\n  ```mermaid\n  graph LR\n    x --> y\n  ```\n";
    let ctx = UploadContext {
        mermaid: Some(MermaidApp::new(
            rfluence_convert::mermaid::VIEWER_APP_ID,
            "63d4d207-ac2f-4273-865c-0240d37f044a",
        )),
        ..Default::default()
    };
    let upload = markdown_to_adf(md, &ctx).unwrap();
    assert!(upload.diagnostics.is_empty(), "{:#?}", upload.diagnostics);
    let doc = &upload.doc;
    let kinds: Vec<&str> = doc.content.iter().map(|n| n.kind.as_str()).collect();
    assert_eq!(
        kinds,
        ["expand", "extension", "layoutSection", "bulletList"]
    );
    assert_eq!(doc.content[0].attr_str("title"), Some("Mermaid source"));
    let code = &doc.content[0].content[0];
    assert_eq!(
        (code.attr_str("language"), code.plain_text().as_str()),
        (Some("mermaid"), "flowchart TD\n  A --> B")
    );
    let column: Vec<&str> = doc.content[2].content[0]
        .content
        .iter()
        .map(|n| n.kind.as_str())
        .collect();
    assert_eq!(column, ["expand", "extension"]);
    // Each macro has its own localId, also in its parameters (the viewer needs it).
    let ids: Vec<&str> = [&doc.content[1], &doc.content[2].content[0].content[1]]
        .iter()
        .map(|m| m.attr_str("localId").unwrap())
        .collect();
    assert_ne!(ids[0], ids[1]);
    assert_eq!(doc.content[1].attrs["parameters"]["localId"], ids[0]);
    assert_eq!(
        adf_to_markdown(doc, &FetchContext::default()),
        normalize(md)
    );

    // merfluence-only settings are dropped, with a warning.
    let upload =
        markdown_to_adf("```mermaid theme=dark\ngraph LR\n  a --> b\n```\n", &ctx).unwrap();
    assert_reported(
        &upload.diagnostics,
        Severity::Warning,
        &["Mermaid setting `theme` dropped"],
    );
}

/// Tabs written by hand upload as Confluence's tabs and fetch back the same.
#[test]
fn tabs_round_trip() {
    let md = "<!-- rf: tabs -->\n\n<!-- rf: tab title=\"Install\" -->\n\nRun `make`.\n\n<!-- rf: tab title=\"Use\" -->\n\n- one\n- two\n\n<!-- rf: tab title=\"Empty\" -->\n\n<!-- rf: end-tabs -->\n";
    let upload = markdown_to_adf(md, &UploadContext::default()).unwrap();
    assert!(upload.diagnostics.is_empty(), "{:#?}", upload.diagnostics);
    let tabs = &upload.doc.content[0];
    assert_eq!(
        (tabs.kind.as_str(), tabs.attr_str("extensionKey")),
        ("multiBodiedExtension", Some("native-tabs"))
    );
    let titles: Vec<&str> = tabs.attrs["parameters"]["tabs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["title"].as_str().unwrap())
        .collect();
    assert_eq!(titles, ["Install", "Use", "Empty"]);
    assert!(
        tabs.content
            .iter()
            .all(|f| f.is("extensionFrame") && !f.content.is_empty())
    );
    assert_eq!(
        adf_to_markdown(&upload.doc, &FetchContext::default()),
        normalize(md)
    );

    for (md, expected) in [
        ("<!-- rf: tab title=\"x\" -->\n", "tab marker outside"),
        (
            "<!-- rf: tabs -->\n\nstray\n\n<!-- rf: tab title=\"x\" -->\n\n<!-- rf: end-tabs -->\n",
            "content in tabs before the first",
        ),
        (
            "<!-- rf: tabs -->\n\n<!-- rf: tab title=\"x\" -->\n",
            "tabs without `<!-- rf: end-tabs -->`",
        ),
        (
            "- <!-- rf: tabs -->\n  <!-- rf: tab title=\"x\" -->\n  <!-- rf: end-tabs -->\n",
            "only allowed at the top level",
        ),
    ] {
        assert_reported(&check(md), Severity::Error, &[expected]);
    }
}

/// Synced blocks are read-only: upload sends Confluence's version of the page's own block and
/// a reference for a copy, and refuses content that differs from Confluence's.
#[test]
fn synced_blocks_are_read_only() {
    let original: Node = serde_json::from_value(serde_json::json!({
        "type": "bodiedSyncBlock", "attrs": { "resourceId": "r-1", "localId": "l-1" },
        "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "Shared text" }] }],
    }))
    .unwrap();
    let mut ctx = UploadContext::default();
    ctx.learn_from(&Node::doc(vec![original.clone()]));
    ctx.synced_copies.insert(
        "confluence-page/22/r-2".into(),
        vec![serde_json::from_value(serde_json::json!({ "type": "paragraph", "content": [{ "type": "text", "text": "From page 22" }] })).unwrap()],
    );
    let block = |attrs: &str, content: &str| {
        format!(
            "<!-- rf: synced-block {attrs} read-only -->\n\n{content}<!-- rf: end-synced-block -->\n"
        )
    };

    // Unchanged (or left empty): sent as Confluence has it.
    for content in ["Shared text\n\n", ""] {
        let doc = markdown_to_adf(&block("id=r-1", content), &ctx)
            .unwrap()
            .doc;
        assert_eq!(doc.content, std::slice::from_ref(&original));
    }
    // A copy: a reference to its source, whose content is checked too.
    for content in ["From page 22\n\n", ""] {
        let doc = markdown_to_adf(&block("id=r-2 page=22", content), &ctx)
            .unwrap()
            .doc;
        assert_eq!(doc.content[0].kind, "syncBlock");
        assert_eq!(
            doc.content[0].attr_str("resourceId"),
            Some("confluence-page/22/r-2")
        );
    }

    // Refused: changed content, a block that isn't on the page, a copy whose source couldn't
    // be read.
    for (md, expected) in [
        (
            block("id=r-1", "Edited text\n\n"),
            "the content of synced block r-1 was changed",
        ),
        (
            block("id=r-2 page=22", "Edited\n\n"),
            "the content of synced block r-2 was changed",
        ),
        (
            block("id=new-id", "Text\n\n"),
            "synced block new-id isn't on this page",
        ),
        (
            block("id=r-3 page=33", "Text\n\n"),
            "couldn't be read to check its content",
        ),
    ] {
        let err = markdown_to_adf(&md, &ctx).unwrap_err();
        assert!(err.to_string().contains(expected), "{expected}: {err}");
    }
    assert!(
        markdown_to_adf(&block("id=r-1", "Edited\n\n"), &ctx)
            .unwrap_err()
            .to_string()
            .contains("can only be edited in Confluence's editor")
    );

    // Offline, `check` can only check the markers.
    assert!(check(&block("id=r-1", "Edited\n\n")).is_empty());
    assert_reported(
        &check(&block("", "Text\n\n")),
        Severity::Error,
        &["synced block without `id=`"],
    );
    assert_eq!(
        rfluence_convert::local_synced_copies(
            &(block("id=r-1", "") + "\n" + &block("id=r-2 page=22", ""))
        ),
        ["confluence-page/22/r-2"]
    );
}
