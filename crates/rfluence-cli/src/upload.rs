//! `rfluence upload <file>`: send a markdown file back to its page. See design.md, "Commands".

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use rfluence_client::{Attachment, Client, Page, Property, auth};
use rfluence_convert::adf::Node;
use rfluence_convert::annotations::{self, Reanchored};
use rfluence_convert::{
    FetchContext, LinkTarget, PageMeta, PageRef, Severity, UploadContext, adf_to_markdown, frontmatter, labels, local_images, local_links,
    markdown_to_adf, select, starts_with_h1, upload_title,
};
use serde::Serialize;

use crate::{EXIT_CONFLICT, EXIT_ERRORS_FOUND, EXIT_USAGE, fail, fetch, project};

pub struct Options {
    pub path: PathBuf,
    pub dry_run: bool,
    pub force: bool,
    pub site: Option<String>,
    pub json: bool,
}

/// The content property on pages rfluence uploads (design.md, "Renames and deletions").
const PROPERTY: &str = "rfluence";

/// Why an upload stopped.
enum Stop {
    Client(rfluence_client::Error),
    Usage(String),
    /// It would overwrite changes made in Confluence.
    Conflict(String),
    /// `rfluence check` errors (already printed).
    CheckErrors,
}

impl From<rfluence_client::Error> for Stop {
    fn from(e: rfluence_client::Error) -> Self {
        Stop::Client(e)
    }
}

pub fn run(opts: &Options) -> ExitCode {
    match upload(opts) {
        Ok(()) => ExitCode::SUCCESS,
        Err(Stop::Client(e)) => fail(&e),
        Err(Stop::Usage(m)) => {
            eprintln!("rfluence: {}: {m}", opts.path.display());
            ExitCode::from(EXIT_USAGE)
        }
        Err(Stop::Conflict(m)) => {
            eprintln!("rfluence: {}: {m}", opts.path.display());
            ExitCode::from(EXIT_CONFLICT)
        }
        Err(Stop::CheckErrors) => ExitCode::from(EXIT_ERRORS_FOUND),
    }
}

/// What happens to a local image's attachment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    /// No attachment with its name yet.
    Upload,
    /// The attachment's content differs: upload a new version.
    NewVersion,
    /// The attachment has the same content, or the local file is missing.
    Reuse,
}

struct Image {
    /// As written in the markdown.
    src: String,
    /// The attachment name: the file name.
    name: String,
    data: Option<Vec<u8>>,
    action: Action,
    /// The existing attachment with this name.
    attachment: Option<Attachment>,
    /// The `fileId` for the body: the attachment's, or the new upload's.
    file_id: Option<String>,
}

#[derive(Serialize)]
struct Json<'a> {
    path: String,
    id: &'a str,
    title: &'a str,
    url: &'a str,
    dry_run: bool,
    /// The body or title changed, so there is (or would be) a new version.
    changed: bool,
    version: u64,
    previous_version: u64,
    images_uploaded: Vec<&'a str>,
    images_updated: Vec<&'a str>,
    labels_added: &'a [String],
    comments_kept: usize,
    comments_detached: &'a [String],
}

fn upload(opts: &Options) -> Result<(), Stop> {
    let path = &opts.path;
    let md = std::fs::read_to_string(path).map_err(|e| Stop::Usage(e.to_string()))?;
    let doc = frontmatter::split(&md);
    let yaml = doc.yaml.unwrap_or_default();
    let unknown = frontmatter::unknown_keys(yaml);
    if !unknown.is_empty() {
        return Err(Stop::Usage(format!("unknown keys under `rfluence:` in the frontmatter: {}", unknown.join(", "))));
    }
    let fields = frontmatter::rfluence_fields(yaml);
    if fields.simplified {
        return Err(Stop::Usage("was fetched with --simplified, which can't be uploaded; fetch it without --simplified".into()));
    }
    if fields.partial {
        return Err(Stop::Usage("holds part of a page (--section / --max-chars), which can't be uploaded; fetch the whole page".into()));
    }
    let Some(id) = fields.id.clone() else {
        return Err(Stop::Usage("has no page ID (`rfluence.id`); creating pages isn't supported yet".into()));
    };
    let mut wanted_labels: Vec<String> = Vec::new();
    for label in &fields.labels {
        let l = labels::normalize_label(label).map_err(Stop::Usage)?;
        if !wanted_labels.contains(&l) {
            wanted_labels.push(l);
        }
    }

    // The same checks as `rfluence check`, before anything is sent.
    let diags = rfluence_convert::check(&md);
    for d in &diags {
        eprintln!("{}:{}: {}: {}", path.display(), d.line, d.severity, d.message);
    }
    let errors = diags.iter().filter(|d| d.severity == Severity::Error).count();
    if errors > 0 && !opts.force {
        eprintln!(
            "rfluence: {}: {errors} error{} (content Confluence can't store), nothing uploaded; fix them, or use --force to upload the approximations",
            path.display(),
            plural(errors)
        );
        return Err(Stop::CheckErrors);
    }

    let (title, body) = upload_title(doc.body, fields.title.as_deref());
    let dir = path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let pages = link_targets(dir, body, fields.url.as_deref())?;

    // The page's site: its URL's, else --site, else the default.
    let site = fields.url.as_deref().and_then(rfluence_client::page_ref_site).or_else(|| opts.site.clone());
    let (creds, _) = auth::resolve(site.as_deref())?;
    let client = Client::new(&creds);
    let (remote, attachments, property) = std::thread::scope(|s| {
        let property = s.spawn(|| client.property(&id, PROPERTY));
        let both = client.page_with_attachments(&id);
        let property = property.join().expect("property thread doesn't panic");
        both.and_then(|(page, attachments)| Ok((page, attachments, property?)))
    })?;

    if remote.meta.version > fields.version.unwrap_or(0) && !opts.force {
        return Err(Stop::Conflict(match fields.version {
            Some(v) => format!(
                "the page has changed in Confluence since this file was fetched (version {v} -> {}); fetch it again and redo your changes, or use --force to overwrite Confluence's changes",
                remote.meta.version
            ),
            None => "has no version in its frontmatter, so changes made in Confluence can't be ruled out; use --force to upload anyway".into(),
        }));
    }
    if let (Some(local), Some(current)) = (fields.parent.as_deref(), remote.meta.parent.as_deref()) {
        if local != current {
            eprintln!(
                "rfluence: {}: warning: `parent` is {local}, but the page is under {current}; single-file upload doesn't move pages",
                path.display()
            );
        }
    }

    let mut images = plan_images(&client, dir, body, &attachments)?;
    let title = title.unwrap_or_else(|| remote.meta.title.clone());
    let mut ctx = UploadContext { page_id: Some(id.clone()), pages, ..Default::default() };
    ctx.learn_from(&remote.adf);
    if ctx.mermaid.is_none() && body.contains("```mermaid") {
        eprintln!(
            "rfluence: {}: warning: Mermaid diagrams are uploaded as code blocks; the merfluence app's IDs are read from a diagram already on the page",
            path.display()
        );
    }

    // Convert before sending anything, so unresolved links fail the upload early. Images
    // still to be uploaded get stand-in IDs until they are.
    ctx.media = media(&images);
    let mut new_doc = markdown_to_adf(body, &ctx).map_err(|e| Stop::Usage(e.to_string()))?.doc;
    if !opts.dry_run && images.iter().any(|i| i.action != Action::Reuse) {
        for image in images.iter_mut().filter(|i| i.action != Action::Reuse) {
            let data = image.data.as_deref().unwrap_or_default();
            let uploaded = match &image.attachment {
                Some(existing) => client.update_attachment(&id, existing, data)?,
                None => client.upload_attachment(&id, &image.name, data)?,
            };
            image.file_id = Some(uploaded.file_id);
        }
        ctx.media = media(&images);
        new_doc = markdown_to_adf(body, &ctx).map_err(|e| Stop::Usage(e.to_string()))?.doc;
    }
    let comments = annotations::reanchor(&remote.adf, &mut new_doc);

    let mut changed = title != remote.meta.title || images.iter().any(|i| i.action != Action::Reuse) || {
        // Compared as fetch would show them: ignores what Confluence adds on save.
        let ctx = compare_ctx(&remote, &new_doc, &attachments);
        adf_to_markdown(&remote.adf, &ctx) != adf_to_markdown(&new_doc, &ctx)
    };
    let labels_added: Vec<String> = wanted_labels.iter().filter(|l| !remote.meta.labels.contains(l)).cloned().collect();
    let previous = remote.meta.version;
    let mut meta = remote.meta.clone();
    if changed {
        meta.version = previous + 1;
        meta.title = title.clone();
    }
    meta.labels.extend(labels_added.iter().cloned());

    if !opts.dry_run {
        if changed {
            let updated = client.update_page(&id, &title, &new_doc, previous + 1)?;
            // Confluence makes no new version if the body is the same after its rewrites.
            changed = updated.version != previous;
            meta.version = updated.version;
            meta.title = updated.title;
            meta.parent = updated.parent.or(meta.parent);
        }
        client.add_labels(&id, &labels_added)?;
        // Only pages rfluence created have the property (they're the ones `--prune` may
        // trash); keep its version current.
        if let Some(existing) = property.as_ref().filter(|_| changed) {
            let value = property_value(&project_path(dir, path), meta.version, existing);
            client.set_property(&id, PROPERTY, value, Some(existing))?;
        }
        write_back(path, &md, doc.yaml, doc.body, &new_doc, &meta, fields.title.is_some())?;
    }

    let summary = Summary { meta: &meta, previous: &remote.meta, changed, images: &images, labels_added: &labels_added, comments: &comments };
    if opts.json {
        print_json(opts, &summary);
    } else {
        print_text(opts, &summary);
    }
    Ok(())
}

/// The pages relative links point at, from the linked files' frontmatter: their URLs on the
/// site of `page_url`, and their headings for anchors. Links to files without a page ID fail
/// the upload (design.md, "Links" > "Upload").
fn link_targets(dir: &Path, body: &str, page_url: Option<&str>) -> Result<HashMap<String, PageRef>, Stop> {
    let mut pages = HashMap::new();
    let mut unresolved = Vec::new();
    for (line, link) in local_links(body) {
        if pages.contains_key(&link) || unresolved.iter().any(|(_, l, _)| *l == link) {
            continue;
        }
        let file = dir.join(&link);
        let Ok(text) = std::fs::read_to_string(&file) else {
            unresolved.push((line, link, "no such file"));
            continue;
        };
        let target = frontmatter::split(&text);
        let fields = target.yaml.map(frontmatter::rfluence_fields).unwrap_or_default();
        let site = fields.url.as_deref().or(page_url).and_then(rfluence_client::page_ref_site);
        let space = fields.space_key.clone().or_else(|| space_key(fields.url.as_deref()?)).or_else(|| space_key(page_url?));
        let (Some(id), Some(site), Some(space)) = (fields.id, site, space) else {
            unresolved.push((line, link, "no page ID yet; upload it first"));
            continue;
        };
        let (_, target_body) = upload_title(target.body, fields.title.as_deref());
        let headings = select::heading_titles(target_body);
        pages.insert(link, PageRef { url: format!("{site}/wiki/spaces/{space}/pages/{id}"), headings });
    }
    if unresolved.is_empty() {
        return Ok(pages);
    }
    let list: Vec<String> = unresolved.iter().map(|(line, link, why)| format!("line {line}: {link} ({why})")).collect();
    Err(Stop::Usage(format!("links to files without a Confluence page: {}", list.join(", "))))
}

/// The space key in a page URL (`.../wiki/spaces/<KEY>/pages/...`).
fn space_key(url: &str) -> Option<String> {
    let rest = url.split("/spaces/").nth(1)?;
    Some(rest.split('/').next()?.to_string()).filter(|k| !k.is_empty())
}

/// What to do with each local image: compared with the page's attachment of the same name
/// (by size, then content). See design.md, "Images and attachments" > "Upload".
fn plan_images(client: &Client, dir: &Path, body: &str, attachments: &[Attachment]) -> Result<Vec<Image>, Stop> {
    let mut images: Vec<Image> = Vec::new();
    let mut missing = Vec::new();
    for (line, src) in local_images(body) {
        if images.iter().any(|i| i.src == src) {
            continue;
        }
        let decoded = percent_encoding::percent_decode_str(&src).decode_utf8_lossy().into_owned();
        let file = dir.join(&decoded);
        let name = Path::new(&decoded).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if let Some(other) = images.iter().find(|i| i.name == name) {
            return Err(Stop::Usage(format!(
                "two different images are named {name} ({} and {src}); attachment names are unique per page, so rename one",
                other.src
            )));
        }
        let attachment = attachments.iter().find(|a| a.title == name).cloned();
        let data = std::fs::read(&file).ok();
        let action = match (&data, &attachment) {
            (None, None) => {
                missing.push(format!("line {line}: {src}"));
                continue;
            }
            (None, Some(_)) => Action::Reuse,
            (Some(_), None) => Action::Upload,
            (Some(d), Some(a)) if a.file_size != Some(d.len() as u64) => Action::NewVersion,
            // Same size: compared by content below.
            (Some(_), Some(_)) => Action::Reuse,
        };
        let file_id = attachment.as_ref().filter(|_| action == Action::Reuse).map(|a| a.file_id.clone());
        images.push(Image { src, name, data, action, attachment, file_id });
    }
    if !missing.is_empty() {
        return Err(Stop::Usage(format!("image files not found, and the page has no attachment with their names: {}", missing.join(", "))));
    }

    // Same size as the attachment: download it to compare, in parallel.
    const PARALLEL: usize = 6;
    let to_compare: Vec<usize> = (0..images.len()).filter(|&i| images[i].data.is_some() && images[i].action == Action::Reuse).collect();
    for chunk in to_compare.chunks(PARALLEL) {
        let results: Vec<_> = std::thread::scope(|s| {
            let handles: Vec<_> = chunk
                .iter()
                .map(|&i| {
                    let attachment = images[i].attachment.as_ref().expect("compared images have an attachment");
                    s.spawn(move || (i, client.download(attachment)))
                })
                .collect();
            handles.into_iter().map(|h| h.join().expect("download thread doesn't panic")).collect()
        });
        for (i, remote) in results {
            if images[i].data.as_deref() != Some(remote?.as_slice()) {
                images[i].action = Action::NewVersion;
                images[i].file_id = None;
            }
        }
    }
    Ok(images)
}

/// `fileId`s by image path as written, with stand-ins for images not uploaded yet.
fn media(images: &[Image]) -> HashMap<String, String> {
    images
        .iter()
        .map(|i| (i.src.clone(), i.file_id.clone().unwrap_or_else(|| format!("pending-upload:{}", i.name))))
        .collect()
}

/// How the page and the upload are compared to decide whether there is anything to send.
/// Links to pages are compared by page ID and anchor: Confluence adds or removes the title in
/// stored page URLs (design.md, "Links").
fn compare_ctx(remote: &Page, new_doc: &Node, attachments: &[Attachment]) -> FetchContext {
    let host = auth::host(&remote.meta.url);
    let mut links = HashMap::new();
    for doc in [&remote.adf, new_doc] {
        for id in rfluence_convert::links::linked_page_ids(doc, &host) {
            links.insert(id.clone(), LinkTarget { path: format!("page-{id}"), headings: Vec::new() });
        }
    }
    FetchContext {
        page_id: Some(remote.meta.id.clone()),
        assets_dir: "assets".into(),
        attachments: rfluence_client::file_names(attachments),
        site_host: Some(host),
        links,
        ..Default::default()
    }
}

/// The file's path from the project root, for the content property.
fn project_path(dir: &Path, path: &Path) -> String {
    let root = project::root(dir);
    let rel = project::relative(&root, path);
    rel.strip_prefix("./").unwrap_or(&rel).to_string()
}

/// The property after an upload: the new version and path, the rest kept.
fn property_value(path: &str, version: u64, existing: &Property) -> serde_json::Value {
    let mut value = existing.value.clone();
    if !value.is_object() {
        value = serde_json::json!({ "managed": true, "config_labels": [] });
    }
    value["version"] = version.into();
    value["path"] = path.into();
    value
}

/// Rewrite the file's `rfluence:` block to what `rfluence fetch` would write for the page
/// now (design.md, "Frontmatter" > "Reading and writing"). The body is left as it is.
fn write_back(path: &Path, md: &str, yaml: Option<&str>, body: &str, doc: &Node, meta: &PageMeta, title_set: bool) -> Result<(), Stop> {
    let fetched = rfluence_convert::frontmatter(meta, title_set || starts_with_h1(doc));
    let output = format!("{}\n{body}", frontmatter::merge(yaml, &fetched));
    if output != md {
        fetch::write_atomically(path, &output).map_err(|e| Stop::Usage(format!("writing the new version back: {e}")))?;
    }
    Ok(())
}

struct Summary<'a> {
    meta: &'a PageMeta,
    previous: &'a PageMeta,
    changed: bool,
    images: &'a [Image],
    labels_added: &'a [String],
    comments: &'a Reanchored,
}

impl Summary<'_> {
    fn images(&self, action: Action) -> Vec<&str> {
        self.images.iter().filter(|i| i.action == action).map(|i| i.name.as_str()).collect()
    }
}

fn print_json(opts: &Options, s: &Summary) {
    let json = Json {
        path: opts.path.display().to_string(),
        id: &s.meta.id,
        title: &s.meta.title,
        url: &s.meta.url,
        dry_run: opts.dry_run,
        changed: s.changed,
        version: s.meta.version,
        previous_version: s.previous.version,
        images_uploaded: s.images(Action::Upload),
        images_updated: s.images(Action::NewVersion),
        labels_added: s.labels_added,
        comments_kept: s.comments.kept,
        comments_detached: &s.comments.lost,
    };
    println!("{}", serde_json::to_string_pretty(&json).expect("JSON serializes"));
}

fn print_text(opts: &Options, s: &Summary) {
    let (m, p) = (s.meta, s.previous);
    let head = match (opts.dry_run, s.changed) {
        (true, true) => format!("Dry run: would update page {} {:?} (version {} -> {})", m.id, p.title, p.version, m.version),
        (false, true) => format!("Uploaded {} to page {} {:?} (version {} -> {})", opts.path.display(), m.id, m.title, p.version, m.version),
        (_, false) => format!("{}: page {} {:?} is up to date (version {})", opts.path.display(), m.id, m.title, m.version),
    };
    println!("{head}\n  {}", m.url);
    if m.title != p.title {
        println!("  title: {:?} -> {:?}", p.title, m.title);
    }
    let verb = |done: &'static str, todo: &'static str| if opts.dry_run { todo } else { done };
    let uploads = s.images(Action::Upload);
    let updates = s.images(Action::NewVersion);
    if !uploads.is_empty() {
        println!("  images {}: {}", verb("uploaded", "to upload"), uploads.join(", "));
    }
    if !updates.is_empty() {
        println!("  images {}: {}", verb("updated", "to update"), updates.join(", "));
    }
    if !s.labels_added.is_empty() {
        println!("  labels {}: {}", verb("added", "to add"), s.labels_added.join(", "));
    }
    let lost = &s.comments.lost;
    if !lost.is_empty() {
        let quoted: Vec<String> = lost.iter().map(|t| format!("{t:?}")).collect();
        println!(
            "  {} inline comment{} {} detached (text changed or no longer unique): {}",
            lost.len(),
            plural(lost.len()),
            verb("was", "will be"),
            quoted.join(", ")
        );
    }
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}
