//! `rfluence fetch`: a page as markdown. See design.md, "Commands".

use std::process::ExitCode;

use rfluence_client::{Client, auth, parse_page_ref};
use rfluence_convert::{FetchContext, page_markdown, select};
use serde::Serialize;

use crate::{EXIT_NOT_FOUND, fail};

pub struct Options {
    pub page: String,
    pub simplified: bool,
    pub section: Option<String>,
    pub max_chars: Option<usize>,
    pub json: bool,
    pub site: Option<String>,
}

#[derive(Serialize)]
struct Json<'a> {
    id: &'a str,
    title: &'a str,
    space_key: &'a str,
    version: u64,
    url: &'a str,
    labels: &'a [String],
    simplified: bool,
    partial: bool,
    markdown: &'a str,
}

pub fn run(opts: &Options) -> ExitCode {
    let result = (|| {
        let reference = parse_page_ref(&opts.page)?;
        // A page URL names its site; otherwise --site, or the default.
        let site = rfluence_client::page_ref_site(&opts.page).or_else(|| opts.site.clone());
        let (creds, _) = auth::resolve(site.as_deref())?;
        let client = Client::new(&creds);
        let id = client.resolve(&reference)?;
        client.page_with_attachments(&id)
    })();
    let (page, attachments) = match result {
        Ok(r) => r,
        Err(e) => return fail(&e),
    };

    let ctx = FetchContext {
        page_id: Some(page.meta.id.clone()),
        // Images are written relative to the markdown file (design.md, "Images and
        // attachments"); on stdout there is no file name, so name the folder after the page.
        assets_dir: format!("{}.assets", slug(&page.meta.title, &page.meta.id)),
        attachments: rfluence_client::file_names(&attachments),
        simplified: opts.simplified,
    };
    let markdown = page_markdown(&page.adf, &page.meta, &ctx);
    let (frontmatter, body) = select::split_frontmatter(&markdown);

    let mut part = body.to_string();
    if let Some(heading) = &opts.section {
        match select::section(body, heading) {
            Some(s) => part = s,
            None => {
                eprintln!("rfluence: no section {heading:?} on this page; headings: {}", select::heading_titles(body).join("; "));
                return ExitCode::from(EXIT_NOT_FOUND);
            }
        }
    }
    if let Some(max) = opts.max_chars {
        if let Some(cut) = select::truncate(&part, max) {
            part = cut;
        }
    }
    let partial = part != body;
    let frontmatter = if partial { select::mark_partial(frontmatter) } else { frontmatter.to_string() };
    let output = format!("{frontmatter}\n{part}");

    if opts.json {
        let m = &page.meta;
        let json = Json {
            id: &m.id,
            title: &m.title,
            space_key: &m.space_key,
            version: m.version,
            url: &m.url,
            labels: &m.labels,
            simplified: opts.simplified,
            partial,
            markdown: &output,
        };
        println!("{}", serde_json::to_string_pretty(&json).expect("JSON serializes"));
    } else {
        print!("{output}");
    }
    ExitCode::SUCCESS
}

/// A file-name-safe version of the title (`Ingestion: Overview` -> `ingestion-overview`).
fn slug(title: &str, fallback: &str) -> String {
    let mut out = String::new();
    for c in title.chars().flat_map(char::to_lowercase) {
        if c.is_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let out: String = out.trim_end_matches('-').chars().take(60).collect();
    if out.is_empty() { fallback.to_string() } else { out.trim_end_matches('-').to_string() }
}

#[cfg(test)]
mod tests {
    #[test]
    fn slugs_titles() {
        assert_eq!(super::slug("rfluence ADF reference", "1"), "rfluence-adf-reference");
        assert_eq!(super::slug("Ingestion: Overview (v2)", "1"), "ingestion-overview-v2");
        assert_eq!(super::slug("!!!", "123"), "123");
    }
}
