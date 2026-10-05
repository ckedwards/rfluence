//! Round trips: `normalize(md) == fetch(upload(md))` offline, and fetch -> upload -> fetch
//! on real pages. See design.md, "What 'same content' means".

mod common;

use std::collections::HashMap;

use common::*;
use rfluence_convert::{
    Diagnostic, FetchContext, MermaidApp, Severity, UploadContext, adf_to_markdown, check, markdown_to_adf, normalize,
};

/// Upload and fetch contexts for a corpus file: its images get made-up attachment IDs.
fn corpus_ctx(name: &str, md: &str) -> (UploadContext, FetchContext) {
    let assets = format!("{name}.assets");
    let mut media = HashMap::new();
    let mut attachments = HashMap::new();
    for (i, path) in md.match_indices(&format!("{assets}/")).map(|(i, _)| i).enumerate() {
        let end = md[path..].find([')', ' ', '"']).unwrap() + path;
        let file = md[path + assets.len() + 1..end].to_string();
        let id = format!("file-{i}");
        media.insert(format!("{assets}/{file}"), id.clone());
        attachments.insert(id, file);
    }
    let upload = UploadContext {
        page_id: Some("1".into()),
        media,
        custom_emoji: HashMap::new(),
        mermaid: MermaidApp::from_extension_key(MERMAID),
    };
    let fetch = FetchContext { page_id: Some("1".into()), assets_dir: assets, attachments };
    (upload, fetch)
}

fn strip_frontmatter(md: &str) -> String {
    match md.strip_prefix("---\n").and_then(|rest| rest.split_once("\n---\n")) {
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
            failures.push(format!("{name}:\n{}", similar_asserts::SimpleDiff::from_str(&expected, &back, "normalize(md)", "fetch(upload(md))")));
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
            diags.iter().any(|d| d.severity == severity && d.message.contains(e)),
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
        &["footnotes", "raw HTML block", "inline images", "nested quote", "alert in a quote"],
    );
    assert_eq!(check(&md), upload.diagnostics);
}

/// Markdown with a close equivalent is reported as warnings.
#[test]
fn approximated_markdown_is_a_warning() {
    let diags = check(&corpus_file("approximated"));
    assert!(diags.iter().all(|d| d.severity == Severity::Warning), "{diags:#?}");
    assert_reported(
        &diags,
        Severity::Warning,
        &["`<kbd>` written as inline code", "`<b>` written as markdown", "`<ins>` written as `<u>`", "`<abbr", "`<br>`", "image title", "alert title", "`<details open>`"],
    );
}

#[test]
fn check_reports_broken_structure() {
    let cases = [
        ("<details><summary>x</summary>\n\nbody\n", "without a matching `</details>`"),
        ("text\n\n</details>\n", "without a matching `<details>`"),
        ("- item\n\n  <details><summary>x</summary>\n\n  body\n\n  </details>\n", "can't be an expand here"),
        ("<!-- rf: columns=50,50 -->\n\na\n", "without `<!-- rf: end-columns -->`"),
        ("<!-- rf: columns=50,50 -->\n\na\n\n<!-- rf: end-columns -->\n", "1 columns but `columns=` lists 2"),
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
    assert_eq!((outer.kind.as_str(), outer.attr_str("title")), ("expand", Some("Outer")));
    assert_eq!(outer.content[0].kind, "nestedExpand");
}

/// fetch -> upload -> fetch gives the same markdown for every captured page.
#[test]
fn fixtures_round_trip() {
    let mut failures = Vec::new();
    for name in pages() {
        let doc = page_adf(&name, "adf.json");
        let md = adf_to_markdown(&doc, &fetch_ctx(&name));
        let upload = markdown_to_adf(&md, &upload_ctx(&name, &doc)).unwrap_or_else(|e| panic!("{name}: {e}"));
        let back = adf_to_markdown(&upload.doc, &fetch_ctx(&name));
        if back != md {
            failures.push(format!("{name}:\n{}", similar_asserts::SimpleDiff::from_str(&md, &back, "fetch(page)", "fetch(upload(fetch(page)))")));
        }
        if !upload.diagnostics.is_empty() {
            failures.push(format!("{name} diagnostics: {:#?}", upload.diagnostics));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}
