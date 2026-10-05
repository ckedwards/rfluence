//! `rfluence fetch`: a page as markdown. See design.md, "Commands".

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use rfluence_client::{Attachment, Client, Page, auth, parse_page_ref};
use rfluence_convert::adf::Node;
use rfluence_convert::{FetchContext, frontmatter, page_markdown, select};
use serde::Serialize;

use crate::{EXIT_CONFLICT, EXIT_NOT_FOUND, EXIT_USAGE, fail, project};

pub struct Options {
    pub page: String,
    pub simplified: bool,
    pub section: Option<String>,
    pub max_chars: Option<usize>,
    pub json: bool,
    pub site: Option<String>,
    /// Write to this file (downloading images) instead of stdout.
    pub output: Option<PathBuf>,
    /// Overwrite a file with local changes, or one holding another page.
    pub force: bool,
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
    if opts.output.is_some() && (opts.section.is_some() || opts.max_chars.is_some()) {
        eprintln!("rfluence: --section and --max-chars give part of a page, so they can't be used with -o");
        return ExitCode::from(EXIT_USAGE);
    }
    let result = (|| {
        let reference = parse_page_ref(&opts.page)?;
        // A page URL names its site; otherwise --site, or the default.
        let site = rfluence_client::page_ref_site(&opts.page).or_else(|| opts.site.clone());
        let (creds, _) = auth::resolve(site.as_deref())?;
        let client = Client::new(&creds);
        let id = client.resolve(&reference)?;
        let (page, attachments) = client.page_with_attachments(&id)?;
        Ok((client, page, attachments))
    })();
    let (client, page, attachments) = match result {
        Ok(r) => r,
        Err(e) => return fail(&e),
    };
    let titles = card_titles(&client, &page);
    if let Some(path) = &opts.output {
        return write_file(opts, &client, &page, &attachments, titles, path);
    }

    let ctx = FetchContext {
        page_id: Some(page.meta.id.clone()),
        // Images are written relative to the markdown file (design.md, "Images and
        // attachments"); on stdout there is no file name, so name the folder after the page.
        assets_dir: format!("{}.assets", slug(&page.meta.title, &page.meta.id)),
        attachments: rfluence_client::file_names(&attachments),
        simplified: opts.simplified,
        site_host: Some(auth::host(&page.meta.url)),
        titles,
        ..Default::default()
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

#[derive(Serialize)]
struct FileJson<'a> {
    path: String,
    id: &'a str,
    title: &'a str,
    version: u64,
    url: &'a str,
    images: usize,
    downloaded: usize,
}

/// `rfluence fetch -o <path>`: write the page to a file, links to other project files as
/// relative paths, and its images to `<name>.assets/` next to it.
fn write_file(
    opts: &Options,
    client: &Client,
    page: &Page,
    attachments: &[Attachment],
    titles: HashMap<String, String>,
    path: &Path,
) -> ExitCode {
    let dir = path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
    if let Err(e) = std::fs::create_dir_all(dir) {
        eprintln!("rfluence: {}: {e}", dir.display());
        return ExitCode::from(EXIT_USAGE);
    }
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "page".into());
    let assets = format!("{stem}.assets");
    let root = project::root(dir);
    let ctx = FetchContext {
        page_id: Some(page.meta.id.clone()),
        assets_dir: assets.clone(),
        attachments: rfluence_client::file_names(attachments),
        simplified: opts.simplified,
        site_host: Some(auth::host(&page.meta.url)),
        links: project::pages(&root, dir, path),
        titles,
    };
    let fetched = page_markdown(&page.adf, &page.meta, &ctx);
    let fetched_doc = frontmatter::split(&fetched);

    // An existing file: keep its other frontmatter, and don't lose local changes.
    let existing = std::fs::read_to_string(path).ok();
    let mut output = fetched.clone();
    if let Some(existing) = &existing {
        let doc = frontmatter::split(existing);
        let fields = doc.yaml.map(frontmatter::rfluence_fields).unwrap_or_default();
        if !opts.force {
            if let Some(other) = fields.id.as_deref().filter(|id| *id != page.meta.id) {
                eprintln!("rfluence: {} holds page {other}, not {}; use --force to overwrite it", path.display(), page.meta.id);
                return ExitCode::from(EXIT_CONFLICT);
            }
            if fields.id.is_some() && !fields.simplified && !fields.partial {
                match local_changes(client, page, &ctx, &fields, doc.body, fetched_doc.body) {
                    Ok(None) => {}
                    Ok(Some(message)) => {
                        eprintln!("rfluence: {}: {message}", path.display());
                        return ExitCode::from(EXIT_CONFLICT);
                    }
                    Err(e) => return fail(&e),
                }
            }
        }
        // The fetched frontmatter: everything before the body, without the blank line.
        let fetched_frontmatter = format!("{}\n", fetched[..fetched.len() - fetched_doc.body.len()].trim_end_matches('\n'));
        let merged = frontmatter::merge(doc.yaml, &fetched_frontmatter);
        output = format!("{merged}\n{}", fetched_doc.body);
    }

    let (images, downloaded) = if opts.simplified {
        (0, 0)
    } else {
        match download_images(client, page, attachments, &dir.join(&assets)) {
            Ok(counts) => counts,
            Err(e) => return fail(&e),
        }
    };
    if let Err(e) = write_atomically(path, &output) {
        eprintln!("rfluence: {}: {e}", path.display());
        return ExitCode::from(EXIT_USAGE);
    }

    if opts.json {
        let m = &page.meta;
        let json = FileJson {
            path: path.display().to_string(),
            id: &m.id,
            title: &m.title,
            version: m.version,
            url: &m.url,
            images,
            downloaded,
        };
        println!("{}", serde_json::to_string_pretty(&json).expect("JSON serializes"));
    } else {
        let unchanged = existing.as_deref() == Some(output.as_str());
        let what = if unchanged { "is up to date" } else { "written" };
        let imgs = match (images, downloaded) {
            (0, _) => String::new(),
            (n, 0) => format!(", {n} images up to date"),
            (n, d) => format!(", {d} of {n} images downloaded to {}", dir.join(&assets).display()),
        };
        eprintln!("{} {what} (version {}{imgs})", path.display(), page.meta.version);
    }
    ExitCode::SUCCESS
}

/// The titles of the pages the page's smart links point at, so they can be written as
/// `[Title](url)`. One request, after the page (the links are in its body), and only when it
/// has smart links to pages on its site. Best effort: without titles they stay `<url>`.
fn card_titles(client: &Client, page: &Page) -> HashMap<String, String> {
    let mut ids = rfluence_convert::links::card_page_ids(&page.adf, &auth::host(&page.meta.url));
    let mut titles = HashMap::new();
    if let Some(pos) = ids.iter().position(|id| *id == page.meta.id) {
        ids.remove(pos);
        titles.insert(page.meta.id.clone(), page.meta.title.clone());
    }
    if !ids.is_empty() {
        titles.extend(client.page_titles(&ids).unwrap_or_default());
    }
    titles
}

/// Would writing the fetched page lose changes made to the file? Compares the file's body
/// with what the version it was fetched at converts to (fetching that version if Confluence
/// has a newer one). Returns the reason to refuse, if so.
fn local_changes(
    client: &Client,
    page: &Page,
    ctx: &FetchContext,
    fields: &frontmatter::RfluenceFields,
    local_body: &str,
    fetched_body: &str,
) -> rfluence_client::Result<Option<String>> {
    let Some(version) = fields.version else {
        return Ok(Some("has no version in its frontmatter, so local changes can't be ruled out; use --force to overwrite it".into()));
    };
    let base = if version == page.meta.version {
        fetched_body.to_string()
    } else {
        let old = client.page_version(&page.meta.id, version)?;
        let md = page_markdown(&old.adf, &old.meta, ctx);
        frontmatter::split(&md).body.to_string()
    };
    if select::same_content(local_body, &base) {
        return Ok(None);
    }
    Ok(Some(if version == page.meta.version {
        "has local changes that would be lost; upload them first, or use --force to overwrite the file".into()
    } else {
        format!(
            "has local changes, and the page has changed in Confluence too (version {version} -> {}); use --force to overwrite the file with Confluence's version",
            page.meta.version
        )
    }))
}

/// Download the page's images (attachments shown by `mediaSingle` nodes; other files stay
/// on Confluence) into `dir`, in parallel, skipping files already there with the same size.
/// Returns (images, downloaded).
fn download_images(client: &Client, page: &Page, attachments: &[Attachment], dir: &Path) -> rfluence_client::Result<(usize, usize)> {
    let mut shown = HashSet::new();
    page.adf.walk(&mut |n: &Node| {
        if let Some(media) = n.content.first().filter(|_| n.is("mediaSingle")) {
            if media.attr_str("type") == Some("file") {
                if let Some(id) = media.attr_str("id") {
                    shown.insert(id.to_string());
                }
            }
        }
    });
    let wanted: Vec<&Attachment> = attachments.iter().filter(|a| shown.contains(&a.file_id)).collect();
    let todo: Vec<&Attachment> = wanted
        .iter()
        .copied()
        .filter(|a| {
            let existing = std::fs::metadata(dir.join(&a.title)).map(|m| m.len()).ok();
            existing.is_none() || existing != a.file_size
        })
        .collect();
    if todo.is_empty() {
        return Ok((wanted.len(), 0));
    }
    std::fs::create_dir_all(dir).map_err(|e| rfluence_client::Error::Io(format!("{}: {e}", dir.display())))?;
    const PARALLEL: usize = 6;
    for chunk in todo.chunks(PARALLEL) {
        let results: Vec<_> = std::thread::scope(|s| {
            let handles: Vec<_> = chunk.iter().map(|a| s.spawn(move || (a, client.download(a)))).collect();
            handles.into_iter().map(|h| h.join().expect("download thread doesn't panic")).collect()
        });
        for (a, bytes) in results {
            let file = dir.join(&a.title);
            std::fs::write(&file, bytes?).map_err(|e| rfluence_client::Error::Io(format!("{}: {e}", file.display())))?;
        }
    }
    Ok((wanted.len(), todo.len()))
}

/// Write via a temporary file and rename, so an interrupted write never leaves half a file.
pub(crate) fn write_atomically(path: &Path, contents: &str) -> std::io::Result<()> {
    let tmp = path.with_extension("md.rfluence-tmp");
    std::fs::write(&tmp, contents)?;
    std::fs::rename(&tmp, path)
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
