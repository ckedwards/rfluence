//! `rfluence upload <file>`: send a markdown file back to its page. See design.md, "Commands".

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use rfluence_client::{Attachment, Client, InstalledMacro, Page, Property, auth};
use rfluence_convert::adf::Node;
use rfluence_convert::annotations::{self, Reanchored};
use rfluence_convert::mermaid;
use rfluence_convert::{
    FetchContext, LinkTarget, MermaidApp, PageMeta, PageRef, Severity, UploadContext,
    adf_to_markdown, frontmatter, labels, local_images, local_links, local_synced_copies,
    markdown_to_adf, select, starts_with_h1, upload_title,
};
use serde::Serialize;

use crate::{EXIT_CONFLICT, EXIT_ERRORS_FOUND, EXIT_USAGE, fail, project};

pub struct Options {
    pub path: PathBuf,
    pub dry_run: bool,
    pub force: bool,
    pub site: Option<String>,
    /// For a new page: the space, if the file has no `space_key`.
    pub space: Option<String>,
    /// For a new page: the parent page or folder, if the file has no `parent`.
    pub parent: Option<String>,
    pub json: bool,
    /// Move pages that aren't where they belong (design.md, "Page hierarchy").
    pub move_pages: bool,
    /// Remove labels the file (and config entry) don't have (design.md, "Labels").
    pub prune_labels: bool,
    /// Refuse to upload files with `rfluence check` warnings too, not only errors.
    pub warnings_are_errors: bool,
    /// Set when the file is uploaded as part of `upload --config`.
    pub tree: Option<InTree>,
}

/// What `upload --config` says about a file's page (design.md, "Page hierarchy").
#[derive(Debug, Clone, Default)]
pub struct InTree {
    /// The page or folder the config tree puts the page under.
    pub parent: String,
    /// Its title, for messages.
    pub parent_title: String,
    /// Labels from the config entry, added to the page and recorded in its property.
    pub labels: Vec<String>,
    /// The page was created (empty) by this upload's first pass.
    pub created: bool,
}

/// What an upload did (or, with `--dry-run`, would do) to a page.
#[derive(Debug, Clone, Default)]
pub struct Outcome {
    /// A new version was (or would be) made.
    pub changed: bool,
    /// The page isn't where the config tree puts it (and wasn't moved).
    pub misplaced: bool,
    /// The page was (or would be) moved to where it belongs.
    pub moved: bool,
}

/// The content property on pages rfluence uploads (design.md, "Renames and deletions").
pub const PROPERTY: &str = "rfluence";

/// Why an upload stopped.
pub enum Stop {
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
        Ok(_) => ExitCode::SUCCESS,
        Err(stop) => exit(opts, stop),
    }
}

/// Print why an upload stopped, and its exit code.
pub fn exit(opts: &Options, stop: Stop) -> ExitCode {
    match stop {
        Stop::Client(e) => fail(&e),
        Stop::Usage(m) => {
            eprintln!("rfluence: {}: {m}", opts.path.display());
            ExitCode::from(EXIT_USAGE)
        }
        Stop::Conflict(m) => {
            eprintln!("rfluence: {}: {m}", opts.path.display());
            ExitCode::from(EXIT_CONFLICT)
        }
        Stop::CheckErrors => ExitCode::from(EXIT_ERRORS_FOUND),
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
    /// A new page was (or would be) created.
    created: bool,
    /// The body or title changed, so there is (or would be) a new version.
    changed: bool,
    version: u64,
    /// The version before the upload (none for a new page).
    previous_version: Option<u64>,
    images_uploaded: Vec<&'a str>,
    images_updated: Vec<&'a str>,
    labels_added: &'a [String],
    labels_removed: &'a [String],
    comments_kept: usize,
    comments_detached: &'a [String],
}

/// The file, checked: everything that can fail without asking Confluence has.
pub struct Local<'a> {
    md: &'a str,
    yaml: Option<&'a str>,
    /// The body as written (with any title H1).
    file_body: &'a str,
    pub fields: frontmatter::RfluenceFields,
    /// Normalized.
    labels: Vec<String>,
    /// From `title` or the leading H1.
    pub title: Option<String>,
    /// The body to upload (without the title H1), with blank lines in place of the
    /// frontmatter and title, so that line numbers in messages are the file's.
    pub body: String,
    pub dir: &'a Path,
}

fn upload(opts: &Options) -> Result<Outcome, Stop> {
    let md = crate::text::read(&opts.path).map_err(|e| Stop::Usage(e.to_string()))?;
    let local = check_file(opts, &md)?;
    match local.fields.id.clone() {
        Some(id) => {
            // The page's site: its URL's, else --site, else the default.
            let url = local.fields.url.as_deref();
            let site = url
                .and_then(rfluence_client::page_ref_site)
                .or_else(|| opts.site.clone());
            let (creds, _) = auth::resolve(site.as_deref())?;
            let space = local.fields.space_key.clone().or_else(|| space_key(url?));
            let pages = link_targets(
                local.dir,
                &local.body,
                &creds.base_url,
                space.as_deref(),
                None,
            )?;
            update(opts, &local, &Client::new(&creds), &id, pages)
        }
        None => create(opts, &local),
    }
}

pub fn check_file<'a>(opts: &'a Options, md: &'a str) -> Result<Local<'a>, Stop> {
    let path = &opts.path;
    let doc = frontmatter::split(md);
    let yaml = doc.yaml.unwrap_or_default();
    let unknown = frontmatter::unknown_keys(yaml);
    if !unknown.is_empty() {
        return Err(Stop::Usage(format!(
            "unknown keys under `rfluence:` in the frontmatter: {}",
            unknown.join(", ")
        )));
    }
    let fields = frontmatter::rfluence_fields(yaml);
    if fields.simplified {
        return Err(Stop::Usage(
            "was fetched with --simplified, which can't be uploaded; fetch it without --simplified"
                .into(),
        ));
    }
    if fields.partial {
        return Err(Stop::Usage("holds part of a page (--section / --max-chars), which can't be uploaded; fetch the whole page".into()));
    }
    let mut labels: Vec<String> = Vec::new();
    for label in &fields.labels {
        let l = labels::normalize_label(label).map_err(Stop::Usage)?;
        if !labels.contains(&l) {
            labels.push(l);
        }
    }

    // The same checks as `rfluence check`, before anything is sent (`upload --config` has
    // run them for every file already).
    let diags = if opts.tree.is_none() {
        rfluence_convert::check(md)
    } else {
        Vec::new()
    };
    for d in &diags {
        eprintln!(
            "{}:{}: {}: {}",
            path.display(),
            d.line,
            d.severity,
            d.message
        );
    }
    let errors = diags
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .count();
    let warnings = if opts.warnings_are_errors {
        diags.len() - errors
    } else {
        0
    };
    if errors + warnings > 0 && !opts.force {
        let mut what = Vec::new();
        if errors > 0 {
            what.push(format!(
                "{errors} error{} (content Confluence can't store)",
                plural(errors)
            ));
        }
        if warnings > 0 {
            what.push(format!(
                "{warnings} warning{} (--warnings-are-errors)",
                plural(warnings)
            ));
        }
        eprintln!(
            "rfluence: {}: {}, nothing uploaded; fix them, or use --force to upload the approximations",
            path.display(),
            what.join(" and ")
        );
        return Err(Stop::CheckErrors);
    }

    let (title, body) = upload_title(doc.body, fields.title.as_deref());
    let body = "\n".repeat(md[..md.len() - body.len()].matches('\n').count()) + body;
    let dir = path
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    Ok(Local {
        md,
        yaml: doc.yaml,
        file_body: doc.body,
        fields,
        labels,
        title,
        body,
        dir,
    })
}

/// Upload to an existing page.
pub fn update(
    opts: &Options,
    local: &Local,
    client: &Client,
    id: &str,
    pages: HashMap<String, PageRef>,
) -> Result<Outcome, Stop> {
    let path = &opts.path;
    let (remote, attachments, property, detected) = std::thread::scope(|s| {
        let property = s.spawn(|| client.property(id, PROPERTY));
        let detected = has_mermaid(&local.body).then(|| s.spawn(|| client.installed_macros()));
        let both = client.page_with_attachments(id);
        let property = property.join().expect("property thread doesn't panic");
        let detected = detected.map(|d| d.join().expect("macros thread doesn't panic"));
        both.and_then(|(page, attachments)| Ok((page, attachments, property?, detected)))
    })?;

    let fields = &local.fields;
    if remote.meta.version > fields.version.unwrap_or(0) && !opts.force {
        return Err(Stop::Conflict(match fields.version {
            Some(v) => format!(
                "the page has changed in Confluence since this file was fetched (version {v} -> {}); fetch it again and redo your changes, or use --force to overwrite Confluence's changes",
                remote.meta.version
            ),
            None => "has no version in its frontmatter, so changes made in Confluence can't be ruled out; use --force to upload anyway".into(),
        }));
    }
    // Where the page belongs: the config tree's parent, or (single file) `parent`.
    let current_parent = remote.meta.parent.as_deref().unwrap_or("the space");
    let (wanted_parent, described) = match &opts.tree {
        Some(tree) => {
            let target = if tree.parent.is_empty() {
                "to be created".to_string()
            } else {
                tree.parent.clone()
            };
            (
                Some(tree.parent.clone()),
                format!(
                    "the config puts this page under {:?} ({target})",
                    tree.parent_title
                ),
            )
        }
        None => (
            fields.parent.clone(),
            format!(
                "`parent` is {}",
                fields.parent.as_deref().unwrap_or_default()
            ),
        ),
    };
    let misplaced = wanted_parent
        .as_deref()
        .is_some_and(|w| remote.meta.parent.as_deref() != Some(w));
    // A parent that doesn't exist yet (--dry-run) can't be moved to.
    let move_to = wanted_parent.filter(|w| misplaced && opts.move_pages && !w.is_empty());
    if misplaced && move_to.is_none() {
        let hint = if opts.move_pages {
            "it can't be moved there yet"
        } else {
            "it's left where it is; --move moves it"
        };
        eprintln!(
            "rfluence: {}: warning: {described}, but the page is under {current_parent}; {hint}",
            path.display()
        );
    }

    let mut images = plan_images(client, local.dir, &local.body, &attachments)?;
    let title = local
        .title
        .clone()
        .unwrap_or_else(|| remote.meta.title.clone());
    let mut ctx = UploadContext {
        page_id: Some(id.to_string()),
        pages,
        ..Default::default()
    };
    // A diagram already on the page decides the Mermaid app; else the site's apps do.
    ctx.learn_from(&remote.adf);
    // Synced block copies on the page and in the file, to check their content wasn't changed.
    let mut copies = rfluence_convert::synced::copy_ids(&remote.adf);
    copies.extend(
        local_synced_copies(&local.body)
            .into_iter()
            .filter(|r| !copies.contains(r))
            .collect::<Vec<_>>(),
    );
    ctx.synced_copies = client.synced_copies(&copies);
    if ctx.mermaid.is_none() {
        ctx.mermaid = mermaid_app(opts, detected);
    }

    // Convert before sending anything, so unresolved links fail the upload early. Images
    // still to be uploaded get stand-in IDs until they are.
    ctx.media = media(&images);
    let mut new_doc = convert(&local.body, &ctx)?;
    if !opts.dry_run && images.iter().any(|i| i.action != Action::Reuse) {
        upload_images(client, id, &mut images)?;
        ctx.media = media(&images);
        new_doc = convert(&local.body, &ctx)?;
    }
    let comments = annotations::reanchor(&remote.adf, &mut new_doc);
    let kept = rfluence_convert::synced::originals(&new_doc);
    for id in rfluence_convert::synced::originals(&remote.adf)
        .keys()
        .filter(|id| !kept.contains_key(*id))
    {
        eprintln!(
            "rfluence: {}: warning: synced block {id} {} removed from the page; copies of it on other pages may stop showing its content",
            path.display(),
            if opts.dry_run { "would be" } else { "is" }
        );
    }

    let mut ctx_copies = ctx.synced_copies.clone();
    let mut changed =
        title != remote.meta.title || images.iter().any(|i| i.action != Action::Reuse) || {
            // Compared as fetch would show them: ignores what Confluence adds on save.
            let mut ctx = compare_ctx(&remote, &new_doc, &attachments);
            ctx.synced_copies = std::mem::take(&mut ctx_copies);
            adf_to_markdown(&remote.adf, &ctx) != adf_to_markdown(&new_doc, &ctx)
        };
    let config_labels: &[String] = opts.tree.as_ref().map_or(&[], |t| &t.labels);
    let recorded_config_labels: Vec<String> = property
        .as_ref()
        .and_then(|p| p.value.get("config_labels"))
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|l| l.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    // With --prune-labels, labels dropped from the config entry since the last upload go, even
    // though the frontmatter lists them (it was written back with them; design.md, "Labels").
    let dropped: Vec<&String> = if opts.prune_labels && opts.tree.is_some() {
        recorded_config_labels
            .iter()
            .filter(|l| !config_labels.contains(l))
            .collect()
    } else {
        Vec::new()
    };
    let mut wanted: Vec<String> = local
        .labels
        .iter()
        .filter(|l| !dropped.contains(l))
        .cloned()
        .collect();
    wanted.extend(
        config_labels
            .iter()
            .filter(|l| !wanted.contains(l))
            .cloned()
            .collect::<Vec<_>>(),
    );
    let labels_added: Vec<String> = wanted
        .iter()
        .filter(|l| !remote.meta.labels.contains(l))
        .cloned()
        .collect();
    let labels_removed: Vec<String> = if opts.prune_labels {
        remote
            .meta
            .labels
            .iter()
            .filter(|l| !wanted.contains(l))
            .cloned()
            .collect()
    } else {
        Vec::new()
    };
    let previous = remote.meta.version;
    let mut meta = remote.meta.clone();
    if changed || move_to.is_some() {
        meta.version = previous + 1;
        meta.title = title.clone();
        meta.parent = move_to.clone().or(meta.parent);
    }
    meta.labels.retain(|l| !labels_removed.contains(l));
    meta.labels.extend(labels_added.iter().cloned());

    if !opts.dry_run {
        for label in &labels_removed {
            client.remove_label(id, label)?;
        }
        if changed || move_to.is_some() {
            // One new version for both; without a changed body, Confluence keeps the body.
            let body = changed.then_some(&new_doc);
            let updated = client.put_page(id, &title, body, previous + 1, move_to.as_deref())?;
            // Confluence makes no new version if the body is the same after its rewrites.
            changed = updated.version != previous;
            meta.version = updated.version;
            meta.title = updated.title;
            meta.parent = updated.parent.or(meta.parent);
        }
        // Only pages rfluence created have the property (they're the ones `--prune` may
        // trash); keep its version and config labels current.
        // Config labels stay recorded until they're pruned, so that a later --prune-labels
        // still knows a label taken out of the config came from it.
        let mut to_record: Vec<String> = config_labels.to_vec();
        if !opts.prune_labels {
            to_record.extend(
                recorded_config_labels
                    .iter()
                    .filter(|l| !config_labels.contains(l))
                    .cloned(),
            );
        }
        let config_changed = opts.tree.is_some() && recorded_config_labels != to_record;
        let property = property.as_ref().filter(|_| changed || config_changed);
        let value = property.map(|p| {
            let mut v = property_value(&project_path(local.dir, path), meta.version, Some(p));
            if opts.tree.is_some() {
                v["config_labels"] = serde_json::json!(to_record);
            }
            v
        });
        labels_and_property(client, id, &labels_added, value.map(|v| (v, property)))?;
        // A single file's `parent` asks for a move: kept until the page is moved (the
        // warning repeats), rather than overwritten with where the page is now.
        let mut written = meta.clone();
        if misplaced && move_to.is_none() && opts.tree.is_none() {
            written.parent = fields.parent.clone();
        }
        write_back(path, local, &new_doc, &written)?;
    }

    let moved = move_to.is_some();
    report(
        opts,
        &Summary {
            meta: &meta,
            previous: Some(&remote.meta),
            changed: changed || moved,
            images: &images,
            labels_added: &labels_added,
            labels_removed: &labels_removed,
            comments: &comments,
        },
    );
    Ok(Outcome {
        changed,
        misplaced: misplaced && !moved,
        moved,
    })
}

/// Create a page for a file without a page ID (design.md, "Frontmatter" > "New pages").
fn create(opts: &Options, local: &Local) -> Result<Outcome, Stop> {
    let path = &opts.path;
    let Some(title) = local.title.clone() else {
        return Err(Stop::Usage("has no title for the new page: start it with `# Title`, or set `title` under `rfluence:`".into()));
    };
    let Some(space_key) = local
        .fields
        .space_key
        .clone()
        .or_else(|| opts.space.clone())
    else {
        return Err(Stop::Usage("has no page ID, and no space to create the page in: set `space_key` under `rfluence:`, or pass --space".into()));
    };
    let (creds, _) = auth::resolve(opts.site.as_deref())?;
    let pages = link_targets(
        local.dir,
        &local.body,
        &creds.base_url,
        Some(&space_key),
        None,
    )?;
    let client = Client::new(&creds);
    // A new page has no attachments: every image is uploaded, and must exist.
    let mut images = plan_images(&client, local.dir, &local.body, &[])?;
    let mut ctx = UploadContext {
        pages,
        ..Default::default()
    };
    ctx.synced_copies = client.synced_copies(&local_synced_copies(&local.body));
    ctx.media = media(&images);
    // Before asking Confluence anything, so unresolved links fail early.
    convert(&local.body, &ctx)?;

    let (space, existing, detected) = std::thread::scope(|s| {
        let existing = s.spawn(|| client.page_id_by_title(&space_key, &title));
        let detected = has_mermaid(&local.body).then(|| s.spawn(|| client.installed_macros()));
        let space = client.space(&space_key);
        let detected = detected.map(|d| d.join().expect("macros thread doesn't panic"));
        let existing = match existing.join().expect("title lookup thread doesn't panic") {
            Ok(id) => Ok(Some(id)),
            Err(rfluence_client::Error::NotFound(_)) => Ok(None),
            Err(e) => Err(e),
        };
        space.and_then(|space| Ok((space, existing?, detected)))
    })?;
    ctx.mermaid = mermaid_app(opts, detected);
    let mut doc = convert(&local.body, &ctx)?;

    // The file lost its link to the page (or never had one): never create a duplicate.
    if let Some(id) = existing {
        if !opts.force {
            return Err(Stop::Conflict(format!(
                "has no page ID, but space {space_key} already has a page titled {title:?} (page {id}). If this file is that page, add `id: \"{id}\"` under `rfluence:` and fetch it to see its changes, or use --force to overwrite it; otherwise change the title"
            )));
        }
        eprintln!(
            "rfluence: {}: overwriting page {id} {title:?} (--force)",
            path.display()
        );
        let pages = std::mem::take(&mut ctx.pages);
        return update(opts, local, &client, &id, pages);
    }

    let parent = local
        .fields
        .parent
        .clone()
        .or_else(|| opts.parent.clone())
        .or(space.homepage_id.clone());
    let Some(parent) = parent else {
        return Err(Stop::Usage(format!(
            "space {space_key} has no homepage to put the page under: set `parent` under `rfluence:`, or pass --parent"
        )));
    };
    let mut meta = PageMeta {
        title: title.clone(),
        space_key: space.key.clone(),
        parent: Some(parent.clone()),
        version: 1,
        ..Default::default()
    };

    if !opts.dry_run {
        let empty = images.is_empty().then_some(&doc);
        let created = client.create_page(
            &space.id,
            &parent,
            &title,
            empty.unwrap_or(&Node::doc(Vec::new())),
        )?;
        let id = created.id.clone();
        (|| {
            meta = if images.is_empty() {
                created
            } else {
                // Files can only be attached to a page that exists: create it empty, record
                // its ID in the file (so a failure from here on can't lead to a duplicate),
                // attach the images, then write the body.
                write_back(path, local, &Node::doc(Vec::new()), &created)?;
                upload_images(&client, &created.id, &mut images)?;
                ctx.page_id = Some(created.id.clone());
                ctx.media = media(&images);
                doc = convert(&local.body, &ctx)?;
                let updated = client.update_page(&created.id, &title, &doc, created.version + 1)?;
                PageMeta {
                    version: updated.version,
                    title: updated.title,
                    ..created
                }
            };
            let value = property_value(&project_path(local.dir, path), meta.version, None);
            labels_and_property(&client, &meta.id, &local.labels, Some((value, None)))?;
            meta.labels = local.labels.clone();
            write_back(path, local, &doc, &meta)
        })()
        .map_err(|stop| vanished(stop, &id))?;
    }

    let comments = Reanchored::default();
    report(
        opts,
        &Summary {
            meta: &meta,
            previous: None,
            changed: true,
            images: &images,
            labels_added: &local.labels,
            labels_removed: &[],
            comments: &comments,
        },
    );
    Ok(Outcome {
        changed: true,
        misplaced: false,
        moved: false,
    })
}

/// Create an empty page for a file in `upload --config`'s first pass, so that every file has
/// a page ID before any body (with links between them) is uploaded. Records the page in the
/// file and gives it the `rfluence` property. Returns its ID.
pub fn create_empty(
    client: &Client,
    path: &Path,
    space: &rfluence_client::Space,
    parent: &str,
    title: &str,
    config_labels: &[String],
) -> Result<String, Stop> {
    let md = crate::text::read(path).map_err(|e| Stop::Usage(e.to_string()))?;
    let doc = frontmatter::split(&md);
    let meta = client.create_page(&space.id, parent, title, &Node::doc(Vec::new()))?;
    let fields = doc
        .yaml
        .map(frontmatter::rfluence_fields)
        .unwrap_or_default();
    // Like `write_back`, before the body is uploaded: the title key only if the file sets one.
    let fetched = rfluence_convert::frontmatter(&meta, fields.title.is_some());
    let output = format!("{}\n{}", frontmatter::merge(doc.yaml, &fetched), doc.body);
    crate::text::write_atomically(path, &output)
        .map_err(|e| Stop::Usage(format!("writing the new page's ID: {e}")))?;
    let dir = path
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut value = property_value(&project_path(dir, path), meta.version, None);
    value["config_labels"] = serde_json::json!(config_labels);
    client
        .set_property(&meta.id, PROPERTY, value, None)
        .map_err(|e| vanished(e.into(), &meta.id))?;
    Ok(meta.id)
}

/// A page Confluence created moments ago that it now says doesn't exist: a problem on
/// Confluence's side (seen during an incident on the test site), not a missing page. Reported
/// as an API error (exit 5) that says so.
pub fn vanished(stop: Stop, id: &str) -> Stop {
    match stop {
        Stop::Client(rfluence_client::Error::NotFound(m)) => {
            Stop::Client(rfluence_client::Error::Api {
                status: 404,
                message: format!(
                    "Confluence created page {id} moments ago, but now can't find it ({m}); this is likely a problem on Confluence's side: check https://status.atlassian.com and try again later"
                ),
            })
        }
        other => other,
    }
}

/// Add labels and set the `rfluence` property (if given), in parallel.
fn labels_and_property(
    client: &Client,
    id: &str,
    labels: &[String],
    property: Option<(serde_json::Value, Option<&Property>)>,
) -> Result<(), Stop> {
    std::thread::scope(|s| {
        let labels = s.spawn(|| client.add_labels(id, labels));
        if let Some((value, existing)) = property {
            client.set_property(id, PROPERTY, value, existing)?;
        }
        labels.join().expect("labels thread doesn't panic")
    })?;
    Ok(())
}

fn convert(body: &str, ctx: &UploadContext) -> Result<Node, Stop> {
    Ok(markdown_to_adf(body, ctx)
        .map_err(|e| Stop::Usage(e.to_string()))?
        .doc)
}

/// Does the body have a Mermaid fence? (Only then is the site asked for its Mermaid app.)
fn has_mermaid(body: &str) -> bool {
    body.lines().any(|l| {
        let l = l.trim_start();
        (l.starts_with("```") || l.starts_with("~~~"))
            && l.trim_start_matches(['`', '~'])
                .trim_start()
                .starts_with("mermaid")
    })
}

/// The Mermaid app to upload diagrams for, from the site's installed macros: merfluence,
/// else Mermaid Diagrams Viewer, else none (code blocks). See design.md, "Mermaid diagrams on
/// sites without merfluence".
fn mermaid_app(
    opts: &Options,
    detected: Option<rfluence_client::Result<Vec<InstalledMacro>>>,
) -> Option<MermaidApp> {
    let macros = match detected? {
        Ok(m) => m,
        Err(e) => {
            eprintln!(
                "rfluence: {}: warning: couldn't find out which Mermaid app the site has ({e}); Mermaid diagrams are uploaded as code blocks",
                opts.path.display()
            );
            return None;
        }
    };
    let find = |app: &str| {
        macros
            .iter()
            .find(|m| m.app_id == app && m.key == "mermaid-diagram")
    };
    let found = find(mermaid::MERFLUENCE_APP_ID).or_else(|| find(mermaid::VIEWER_APP_ID))?;
    Some(MermaidApp::new(&found.app_id, &found.environment_id))
}

/// Upload the images that are new or changed, keeping their new `fileId`s.
fn upload_images(client: &Client, page_id: &str, images: &mut [Image]) -> Result<(), Stop> {
    for image in images.iter_mut().filter(|i| i.action != Action::Reuse) {
        let data = image.data.as_deref().unwrap_or_default();
        let uploaded = match &image.attachment {
            Some(existing) => client.update_attachment(page_id, existing, data)?,
            None => client.upload_attachment(page_id, &image.name, data)?,
        };
        image.file_id = Some(uploaded.file_id);
    }
    Ok(())
}

/// The pages relative links point at, from the linked files' frontmatter: their URLs (on
/// `site`, in `space` unless the files say otherwise), and their headings for anchors. Links
/// to files without a page ID fail the upload (design.md, "Links" > "Upload"), except files in
/// `pending` (canonical paths): files of the same `upload --config` that get a page first.
pub fn link_targets(
    dir: &Path,
    body: &str,
    site: &str,
    space: Option<&str>,
    pending: Option<&std::collections::HashSet<PathBuf>>,
) -> Result<HashMap<String, PageRef>, Stop> {
    let mut pages = HashMap::new();
    let mut unresolved = Vec::new();
    for (line, link) in local_links(body) {
        if pages.contains_key(&link) || unresolved.iter().any(|(_, l, _)| *l == link) {
            continue;
        }
        let file = dir.join(&link);
        let Ok(text) = crate::text::read(&file) else {
            unresolved.push((line, link, "no such file"));
            continue;
        };
        let target = frontmatter::split(&text);
        let fields = target
            .yaml
            .map(frontmatter::rfluence_fields)
            .unwrap_or_default();
        let site = fields
            .url
            .as_deref()
            .and_then(rfluence_client::page_ref_site)
            .unwrap_or_else(|| site.to_string());
        let space = fields
            .space_key
            .clone()
            .or_else(|| space_key(fields.url.as_deref()?))
            .or_else(|| space.map(str::to_string));
        let (Some(id), Some(space)) = (fields.id, space) else {
            if !pending.is_some_and(|p| file.canonicalize().is_ok_and(|c| p.contains(&c))) {
                unresolved.push((line, link, "no page ID yet; upload it first"));
            }
            continue;
        };
        let (_, target_body) = upload_title(target.body, fields.title.as_deref());
        let headings = select::heading_titles(target_body);
        pages.insert(
            link,
            PageRef {
                url: format!("{site}/wiki/spaces/{space}/pages/{id}"),
                headings,
            },
        );
    }
    if unresolved.is_empty() {
        return Ok(pages);
    }
    let list: Vec<String> = unresolved
        .iter()
        .map(|(line, link, why)| format!("line {line}: {link} ({why})"))
        .collect();
    Err(Stop::Usage(format!(
        "links to files without a Confluence page: {}",
        list.join(", ")
    )))
}

/// The space key in a page URL (`.../wiki/spaces/<KEY>/pages/...`).
fn space_key(url: &str) -> Option<String> {
    let rest = url.split("/spaces/").nth(1)?;
    Some(rest.split('/').next()?.to_string()).filter(|k| !k.is_empty())
}

/// What to do with each local image: compared with the page's attachment of the same name
/// (by size, then content). See design.md, "Images and attachments" > "Upload".
fn plan_images(
    client: &Client,
    dir: &Path,
    body: &str,
    attachments: &[Attachment],
) -> Result<Vec<Image>, Stop> {
    let mut images: Vec<Image> = Vec::new();
    let mut missing = Vec::new();
    for (line, src) in local_images(body) {
        if images.iter().any(|i| i.src == src) {
            continue;
        }
        let decoded = percent_encoding::percent_decode_str(&src)
            .decode_utf8_lossy()
            .into_owned();
        let file = dir.join(&decoded);
        let name = Path::new(&decoded)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
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
        let file_id = attachment
            .as_ref()
            .filter(|_| action == Action::Reuse)
            .map(|a| a.file_id.clone());
        images.push(Image {
            src,
            name,
            data,
            action,
            attachment,
            file_id,
        });
    }
    if !missing.is_empty() {
        return Err(Stop::Usage(format!(
            "image files not found, and the page has no attachment with their names: {}",
            missing.join(", ")
        )));
    }

    // Same size as the attachment: download it to compare, in parallel.
    const PARALLEL: usize = 6;
    let to_compare: Vec<usize> = (0..images.len())
        .filter(|&i| images[i].data.is_some() && images[i].action == Action::Reuse)
        .collect();
    for chunk in to_compare.chunks(PARALLEL) {
        let results: Vec<_> = std::thread::scope(|s| {
            let handles: Vec<_> = chunk
                .iter()
                .map(|&i| {
                    let attachment = images[i]
                        .attachment
                        .as_ref()
                        .expect("compared images have an attachment");
                    s.spawn(move || (i, client.download(attachment)))
                })
                .collect();
            handles
                .into_iter()
                .map(|h| h.join().expect("download thread doesn't panic"))
                .collect()
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
        .map(|i| {
            (
                i.src.clone(),
                i.file_id
                    .clone()
                    .unwrap_or_else(|| format!("pending-upload:{}", i.name)),
            )
        })
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
            links.insert(
                id.clone(),
                LinkTarget {
                    path: format!("page-{id}"),
                    headings: Vec::new(),
                },
            );
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
pub fn project_path(dir: &Path, path: &Path) -> String {
    let dir = std::path::absolute(dir).unwrap_or_else(|_| dir.to_path_buf());
    let root = project::root(&dir);
    let rel = project::relative(&root, path);
    rel.strip_prefix("./").unwrap_or(&rel).to_string()
}

/// The property after an upload: the new version and path, the rest kept.
pub fn property_value(path: &str, version: u64, existing: Option<&Property>) -> serde_json::Value {
    let mut value = existing.map(|p| p.value.clone()).unwrap_or_default();
    if !value.is_object() {
        value = serde_json::json!({ "managed": true, "config_labels": [] });
    }
    value["version"] = version.into();
    value["path"] = path.into();
    value
}

/// Rewrite the file's `rfluence:` block to what `rfluence fetch` would write for the page
/// now (design.md, "Frontmatter" > "Reading and writing"). The body is left as it is.
fn write_back(path: &Path, local: &Local, doc: &Node, meta: &PageMeta) -> Result<(), Stop> {
    let fetched =
        rfluence_convert::frontmatter(meta, local.fields.title.is_some() || starts_with_h1(doc));
    let output = format!(
        "{}\n{}",
        frontmatter::merge(local.yaml, &fetched),
        local.file_body
    );
    if output != local.md {
        crate::text::write_atomically(path, &output)
            .map_err(|e| Stop::Usage(format!("writing the new version back: {e}")))?;
    }
    Ok(())
}

struct Summary<'a> {
    meta: &'a PageMeta,
    /// The page before the upload; none for a new page.
    previous: Option<&'a PageMeta>,
    changed: bool,
    images: &'a [Image],
    labels_added: &'a [String],
    labels_removed: &'a [String],
    comments: &'a Reanchored,
}

impl Summary<'_> {
    fn images(&self, action: Action) -> Vec<&str> {
        self.images
            .iter()
            .filter(|i| i.action == action)
            .map(|i| i.name.as_str())
            .collect()
    }
}

fn report(opts: &Options, s: &Summary) {
    if opts.tree.is_some() && !opts.json {
        // One line per file: `upload --config` prints a summary at the end.
        let m = s.meta;
        let created = opts.tree.as_ref().is_some_and(|t| t.created);
        let what = match (s.previous, s.changed, opts.dry_run) {
            (Some(_), _, false) if created => format!("created (version {})", m.version),
            (Some(p), true, true) => {
                format!("would update (version {} -> {})", p.version, m.version)
            }
            (Some(p), true, false) => format!("updated (version {} -> {})", p.version, m.version),
            (Some(_), false, _) => "up to date".to_string(),
            (None, _, _) => "created".to_string(),
        };
        let mut extra = Vec::new();
        if let Some(p) = s.previous.filter(|p| p.parent != m.parent) {
            let verb = if opts.dry_run { "would move" } else { "moved" };
            extra.push(format!(
                "{verb} from under {}",
                p.parent.as_deref().unwrap_or("the space")
            ));
        }
        if !s.labels_added.is_empty() {
            extra.push(format!("labels +{}", s.labels_added.join(" +")));
        }
        if !s.labels_removed.is_empty() {
            extra.push(format!("labels -{}", s.labels_removed.join(" -")));
        }
        let images = s
            .images
            .iter()
            .filter(|i| i.action != Action::Reuse)
            .count();
        if images > 0 {
            extra.push(format!("{images} image{}", plural(images)));
        }
        if !s.comments.lost.is_empty() {
            extra.push(format!(
                "{} inline comment{} detached",
                s.comments.lost.len(),
                plural(s.comments.lost.len())
            ));
        }
        let extra = if extra.is_empty() {
            String::new()
        } else {
            format!("; {}", extra.join(", "))
        };
        println!("  {}  {} {what}{extra}", opts.path.display(), m.id);
        return;
    }
    if opts.json {
        print_json(opts, s);
    } else {
        print_text(opts, s);
    }
}

fn print_json(opts: &Options, s: &Summary) {
    let json = Json {
        path: opts.path.display().to_string(),
        id: &s.meta.id,
        title: &s.meta.title,
        url: &s.meta.url,
        dry_run: opts.dry_run,
        created: s.previous.is_none(),
        changed: s.changed,
        version: s.meta.version,
        previous_version: s.previous.map(|p| p.version),
        images_uploaded: s.images(Action::Upload),
        images_updated: s.images(Action::NewVersion),
        labels_added: s.labels_added,
        labels_removed: s.labels_removed,
        comments_kept: s.comments.kept,
        comments_detached: &s.comments.lost,
    };
    println!(
        "{}",
        serde_json::to_string_pretty(&json).expect("JSON serializes")
    );
}

fn print_text(opts: &Options, s: &Summary) {
    let m = s.meta;
    let file = opts.path.display();
    match (s.previous, opts.dry_run, s.changed) {
        (None, true, _) => println!(
            "Dry run: would create page {:?} in space {} under {}",
            m.title,
            m.space_key,
            m.parent.as_deref().unwrap_or_default()
        ),
        (None, false, _) => println!(
            "Created page {} {:?} in space {} from {file} (version {})\n  {}",
            m.id, m.title, m.space_key, m.version, m.url
        ),
        (Some(p), true, true) => println!(
            "Dry run: would update page {} {:?} (version {} -> {})\n  {}",
            m.id, p.title, p.version, m.version, m.url
        ),
        (Some(p), false, true) => println!(
            "Uploaded {file} to page {} {:?} (version {} -> {})\n  {}",
            m.id, m.title, p.version, m.version, m.url
        ),
        (Some(_), _, false) => println!(
            "{file}: page {} {:?} is up to date (version {})\n  {}",
            m.id, m.title, m.version, m.url
        ),
    }
    if let Some(p) = s.previous.filter(|p| p.title != m.title) {
        println!("  title: {:?} -> {:?}", p.title, m.title);
    }
    if let Some(p) = s.previous.filter(|p| p.parent != m.parent) {
        println!(
            "  parent: {} -> {}",
            p.parent.as_deref().unwrap_or("the space"),
            m.parent.as_deref().unwrap_or("the space")
        );
    }
    let verb = |done: &'static str, todo: &'static str| if opts.dry_run { todo } else { done };
    let uploads = s.images(Action::Upload);
    let updates = s.images(Action::NewVersion);
    if !uploads.is_empty() {
        println!(
            "  images {}: {}",
            verb("uploaded", "to upload"),
            uploads.join(", ")
        );
    }
    if !updates.is_empty() {
        println!(
            "  images {}: {}",
            verb("updated", "to update"),
            updates.join(", ")
        );
    }
    if !s.labels_added.is_empty() {
        println!(
            "  labels {}: {}",
            verb("added", "to add"),
            s.labels_added.join(", ")
        );
    }
    if !s.labels_removed.is_empty() {
        println!(
            "  labels {}: {}",
            verb("removed", "to remove"),
            s.labels_removed.join(", ")
        );
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
