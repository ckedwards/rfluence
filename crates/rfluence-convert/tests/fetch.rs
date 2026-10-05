//! ADF -> markdown on real Confluence responses.

mod common;

use common::*;
use rfluence_convert::adf_to_markdown;

/// Every captured page as markdown matches the fixture's page.md. Run with
/// `RF_UPDATE_FIXTURES=1` to rewrite page.md after an intended change, then review the diff.
#[test]
fn fixtures_as_markdown() {
    let update = std::env::var_os("RF_UPDATE_FIXTURES").is_some();
    let mut changed = Vec::new();
    for name in pages() {
        let md = adf_to_markdown(&page_adf(&name, "adf.json"), &fetch_ctx(&name));
        let path = fixtures().join("confluence").join(&name).join("page.md");
        let expected = std::fs::read_to_string(&path).unwrap_or_default();
        if md != expected {
            if update {
                std::fs::write(&path, &md).unwrap();
            } else {
                changed.push(format!(
                    "{name}:\n{}",
                    similar_asserts::SimpleDiff::from_str(&expected, &md, "page.md", "adf_to_markdown(adf.json)")
                ));
            }
        }
    }
    assert!(changed.is_empty(), "{}\n\nRun with RF_UPDATE_FIXTURES=1 to update page.md.", changed.join("\n\n"));
}

/// An editor save rewrites the whole page (localIds, default widths, mark order, ...).
/// None of that may change the markdown. code-languages-and-widths was re-saved in the editor (version
/// 2 -> 3) with two real changes: " XX" typed into the first paragraph, and the editor
/// filling in `width: 1011` for block E's `breakout` `{mode: "wide"}` (design.md, "Code
/// block languages and widths").
#[test]
fn editor_save_doesnt_change_markdown() {
    let ctx = fetch_ctx("code-languages-and-widths");
    let api = adf_to_markdown(&page_adf("code-languages-and-widths", "adf.v2-api-save.json"), &ctx);
    let editor = adf_to_markdown(&page_adf("code-languages-and-widths", "adf.json"), &ctx);
    let expected = api
        .replacen("width variants.\n", "width variants. XX\n", 1)
        .replacen("```plaintext breakout=wide\n", "```plaintext width=1011\n", 1);
    similar_asserts::assert_eq!(expected, editor);
}

/// The emoji page was re-saved in the editor with one paragraph ("XXX") added.
#[test]
fn editor_save_of_emoji_page_only_adds_the_new_paragraph() {
    let ctx = fetch_ctx("emoji");
    let api = adf_to_markdown(&page_adf("emoji", "adf.v1-api-save.json"), &ctx);
    let editor = adf_to_markdown(&page_adf("emoji", "adf.json"), &ctx);
    similar_asserts::assert_eq!(format!("{api}\nXXX\n"), editor);
}

/// adf-reference was edited in the editor only below its "(editor)" heading, but the save
/// rewrote the API-created part above it too.
#[test]
fn editor_save_of_reference_page_keeps_untouched_part() {
    let ctx = fetch_ctx("adf-reference");
    // The pages moved to another space after the saved version was captured, and a move
    // rewrites the space key in page URLs (design.md, "Links"), so ignore it.
    let cut = |md: String| without_space_keys(&md[..md.find("## (editor)").unwrap()]);
    let api = cut(adf_to_markdown(&page_adf("adf-reference", "adf.v2-api-save.json"), &ctx));
    let editor = cut(adf_to_markdown(&page_adf("adf-reference", "adf.json"), &ctx));
    similar_asserts::assert_eq!(api, editor);
}

/// Fetched markdown is already normalized.
#[test]
fn fetched_markdown_is_normalized() {
    for name in pages() {
        let md = adf_to_markdown(&page_adf(&name, "adf.json"), &fetch_ctx(&name));
        similar_asserts::assert_eq!(rfluence_convert::normalize(&md), md, "{name}");
    }
}

/// Page URLs with the space key replaced by `*`: `/wiki/spaces/KEY/pages/1` -> `/wiki/spaces/*/pages/1`.
fn without_space_keys(md: &str) -> String {
    let mut out = String::new();
    let mut rest = md;
    while let Some(i) = rest.find("/wiki/spaces/") {
        let after = &rest[i + "/wiki/spaces/".len()..];
        match after.find('/') {
            Some(j) if after[j..].starts_with("/pages/") => {
                out.push_str(&rest[..i]);
                out.push_str("/wiki/spaces/*");
                rest = &after[j..];
            }
            _ => {
                out.push_str(&rest[..i + 1]);
                rest = &rest[i + 1..];
            }
        }
    }
    out.push_str(rest);
    out
}
