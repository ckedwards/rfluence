//! `rfluence upload --config`: upload a project's files as a page tree. See design.md, "Upload
//! config", "Page hierarchy" and "Child page order".

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use rfluence_client::{Client, Kind, Space, auth};
use rfluence_convert::local_images;

use crate::config::{self, Config};
use crate::plan::{self, Ancestor, EntryPlan, Node};
use crate::upload::{self, InTree, Stop};
use crate::{EXIT_CONFLICT, EXIT_ERRORS_FOUND, EXIT_USAGE, fail};

pub struct Options {
    pub config: PathBuf,
    pub dry_run: bool,
    pub force: bool,
    pub move_pages: bool,
    pub prune: bool,
    pub prune_labels: bool,
    pub warnings_are_errors: bool,
    pub site: Option<String>,
}

/// Why an upload stopped: a message (with its exit code), or a file's upload stopping.
enum Failure {
    Message(String, u8),
    File(PathBuf, Stop),
}

impl From<rfluence_client::Error> for Failure {
    fn from(e: rfluence_client::Error) -> Self {
        Failure::File(PathBuf::new(), Stop::Client(e))
    }
}

fn usage(message: String) -> Failure {
    Failure::Message(message, EXIT_USAGE)
}

pub fn run(opts: &Options) -> ExitCode {
    let mut counts = Counts::default();
    let code = match upload_tree(opts, &mut counts) {
        Ok(()) => return ExitCode::SUCCESS,
        Err(Failure::Message(m, code)) => {
            for line in m.lines() {
                eprintln!("rfluence: {line}");
            }
            ExitCode::from(code)
        }
        Err(Failure::File(path, Stop::Client(e))) if path.as_os_str().is_empty() => fail(&e),
        Err(Failure::File(path, stop)) => upload::exit(&file_options(opts, &path, None), stop),
    };
    // Stopped part-way: say what was done. New pages' IDs are in their files already, so
    // running again continues where this stopped, without duplicates.
    if !opts.dry_run && counts.did_anything() {
        eprintln!(
            "rfluence: stopped part-way; done before the error: {} Run the same command again to continue (every new page's ID is in its file).",
            counts.summary(false)
        );
    }
    code
}

/// The single-file upload options for one of the config's files.
fn file_options(opts: &Options, path: &Path, tree: Option<InTree>) -> upload::Options {
    upload::Options {
        path: path.to_path_buf(),
        dry_run: opts.dry_run,
        force: opts.force,
        site: opts.site.clone(),
        space: None,
        parent: None,
        json: false,
        move_pages: opts.move_pages,
        prune_labels: opts.prune_labels,
        warnings_are_errors: opts.warnings_are_errors,
        tree,
    }
}

/// Where a node is (or will be) in Confluence.
#[derive(Debug, Clone, PartialEq)]
enum Place {
    /// It exists: its ID.
    Existing(String),
    /// A file without an ID whose title is taken by this page; `--force` uploads to it.
    Adopted(String),
    /// It will be created (with `--dry-run`, it isn't).
    New,
    /// Created by this upload.
    Created(String),
}

impl Place {
    fn id(&self) -> Option<&str> {
        match self {
            Place::Existing(id) | Place::Adopted(id) | Place::Created(id) => Some(id),
            Place::New => None,
        }
    }
}

/// An entry's resolved ancestor.
#[derive(Debug, Clone)]
enum Resolved {
    Content { id: String, kind: Kind, title: String },
    /// A page of this upload (by file), created in the first pass if it's new.
    InUpload { file: String, title: String },
}

fn upload_tree(opts: &Options, counts: &mut Counts) -> Result<(), Failure> {
    let config = Config::load(&opts.config).map_err(usage)?;
    let matched = config::match_files(&config).map_err(usage)?;
    let (creds, _) = auth::resolve(opts.site.as_deref())?;
    let client = Client::new(&creds);

    // Spaces, and ancestors given by ID (their titles are needed for `{parent}`).
    let mut spaces: HashMap<String, Space> = HashMap::new();
    for e in &config.entries {
        if !spaces.contains_key(&e.space_key) {
            spaces.insert(e.space_key.clone(), client.space(&e.space_key)?);
        }
    }
    let mut resolved: Vec<Option<Resolved>> = vec![None; config.entries.len()];
    let mut ancestor_titles = vec![String::new(); config.entries.len()];
    for (i, e) in config.entries.iter().enumerate() {
        match (&e.ancestor_id, &e.ancestor) {
            (Some(id), _) => {
                let c = client.content(id)?;
                if c.space_id.as_deref() != Some(spaces[&e.space_key].id.as_str()) {
                    return Err(usage(format!("entry {}: ancestor {id} ({:?}) isn't in space {}", i + 1, c.title, e.space_key)));
                }
                ancestor_titles[i] = c.title.clone();
                resolved[i] = Some(Resolved::Content { id: c.id, kind: c.kind, title: c.title });
            }
            (None, Some(title)) => ancestor_titles[i] = title.clone(),
            (None, None) => unreachable!("checked when the config was loaded"),
        }
    }
    let plans = plan::build(&config, &matched, &ancestor_titles).map_err(usage)?;

    // Everything that can fail without Confluence, for every file, before anything is sent.
    let mut pending: HashSet<PathBuf> = HashSet::new();
    for_each_page(&plans, &mut |_, node| {
        if let plan::Kind::Page { file, id: None } = &node.kind
            && let Ok(c) = config.root.join(file).canonicalize() {
            pending.insert(c);
        }
    });
    let mut problems = Vec::new();
    let mut check_failed = false;
    for_each_page(&plans, &mut |plan, node| {
        let plan::Kind::Page { file, id } = &node.kind else { return };
        let path = config.path_of(file);
        // Forced, so that a file's other problems are found too; its check errors are
        // counted here.
        let o = upload::Options { force: true, ..file_options(opts, &path, None) };
        let md = match std::fs::read_to_string(&path) {
            Ok(md) => md,
            Err(e) => return problems.push(format!("{file}: {e}")),
        };
        let local = match upload::check_file(&o, &md) {
            Ok(local) => local,
            Err(Stop::CheckErrors) => unreachable!("forced"),
            Err(Stop::Usage(m) | Stop::Conflict(m)) => return problems.push(format!("{file}: {m}")),
            Err(Stop::Client(e)) => return problems.push(format!("{file}: {e}")),
        };
        if rfluence_convert::check(&md).iter().any(|d| d.severity == rfluence_convert::Severity::Error || opts.warnings_are_errors) {
            check_failed = true;
        }
        if let Err(Stop::Usage(m)) = upload::link_targets(local.dir, &local.body, &creds.base_url, Some(&plan.space_key), Some(&pending)) {
            problems.push(format!("{file}: {m}"));
        }
        if id.is_none() {
            for (line, src) in local_images(&local.body) {
                let decoded = percent_encoding::percent_decode_str(&src).decode_utf8_lossy().into_owned();
                if !local.dir.join(&decoded).is_file() {
                    problems.push(format!("{file}:{line}: image file not found: {src}"));
                }
            }
        }
    });
    // Only content problems (as `rfluence check` reports them): exit 1, like check and
    // single-file upload; anything else is a usage problem (2).
    let code = if problems.is_empty() { EXIT_ERRORS_FOUND } else { EXIT_USAGE };
    if check_failed && !opts.force {
        let what = if opts.warnings_are_errors { "content Confluence can't store exactly (errors and, with --warnings-are-errors, warnings)" } else { "content Confluence can't store" };
        problems.push(format!("files have {what} (listed above); fix them, or use --force to upload the approximations"));
    }
    if !problems.is_empty() {
        return Err(Failure::Message(format!("nothing uploaded:\n{}", problems.join("\n")), code));
    }

    // Ancestors given by title: a page or folder in Confluence, or a page of this upload.
    for (i, plan) in plans.iter().enumerate() {
        let Ancestor::Title(title) = &plan.ancestor else { continue };
        let page = match client.page_id_by_title(&plan.space_key, title) {
            Ok(id) => Some(id),
            Err(rfluence_client::Error::NotFound(_)) => None,
            Err(e) => return Err(e.into()),
        };
        let folders = client.folders_titled(&plan.space_key, title)?;
        resolved[i] = Some(match (page, folders.as_slice()) {
            (Some(p), [f, ..]) => {
                return Err(usage(format!(
                    "entry {}: space {} has both a page ({p}) and a folder ({f}) titled {title:?}; set `ancestor_id` to the one you mean",
                    plan.entry + 1,
                    plan.space_key
                )));
            }
            (Some(id), []) => Resolved::Content { id, kind: Kind::Page, title: title.clone() },
            (None, [id, ..]) => Resolved::Content { id: id.clone(), kind: Kind::Folder, title: title.clone() },
            (None, []) => match find_page_titled(&plans, &plan.space_key, title) {
                Some(file) => Resolved::InUpload { file, title: title.clone() },
                None => {
                    return Err(usage(format!(
                        "entry {}: ancestor {title:?} isn't a page or folder in space {}, or a page of this upload",
                        plan.entry + 1,
                        plan.space_key
                    )));
                }
            },
        });
    }

    // Files without an ID whose title is already taken: never create a duplicate.
    let mut places: HashMap<String, Place> = HashMap::new();
    let mut taken = Vec::new();
    for_each_page(&plans, &mut |plan, node| {
        if let plan::Kind::Page { file, id } = &node.kind {
            let place = match (id, &node.title) {
                (Some(id), _) => Place::Existing(id.clone()),
                (None, Some(title)) => match client.page_id_by_title(&plan.space_key, title) {
                    Ok(existing) => {
                        taken.push(format!("{file}: space {} already has a page titled {title:?} (page {existing})", plan.space_key));
                        Place::Adopted(existing)
                    }
                    Err(_) => Place::New,
                },
                (None, None) => Place::New,
            };
            places.insert(file.clone(), place);
        }
    });
    if !taken.is_empty() && !opts.force {
        return Err(Failure::Message(
            format!(
                "nothing uploaded: these files have no page ID, but their titles are taken (if a file is that page, add its `id` under `rfluence:`; or use --force to upload to those pages):\n{}",
                taken.join("\n")
            ),
            EXIT_CONFLICT,
        ));
    }

    // Pass 1, top-down: folders found or created, new pages created empty, so every file has
    // a page ID before the bodies (which link to each other) are uploaded.
    let mut parents: HashMap<String, (Option<String>, String)> = HashMap::new();
    let mut done = vec![false; plans.len()];
    loop {
        let mut progress = false;
        for (i, plan) in plans.iter().enumerate() {
            if done[i] {
                continue;
            }
            let (parent, parent_kind, parent_title) = match resolved[i].as_ref().expect("resolved above") {
                Resolved::Content { id, kind, title } => (Some(id.clone()), *kind, title.clone()),
                Resolved::InUpload { file, title } => match places.get(file) {
                    Some(place @ (Place::Existing(_) | Place::Adopted(_) | Place::Created(_))) => {
                        (place.id().map(str::to_string), Kind::Page, title.clone())
                    }
                    // A new page: under it everything is new (or, with --dry-run, would be).
                    Some(Place::New) if opts.dry_run => (None, Kind::Page, title.clone()),
                    _ => continue,
                },
            };
            let space = &spaces[&plan.space_key];
            let mut pass = Pass { opts, config: &config, client: &client, space, plan, places: &mut places, parents: &mut parents, counts: &mut *counts };
            pass.nodes(&plan.nodes, parent.as_deref(), parent_kind, &parent_title)?;
            done[i] = true;
            progress = true;
        }
        if done.iter().all(|d| *d) {
            break;
        }
        if !progress {
            return Err(usage("entries' ancestors depend on each other in a cycle; set `ancestor_id`".into()));
        }
    }

    // Show the plan: each entry's ancestor and tree.
    for (i, plan) in plans.iter().enumerate() {
        let ancestor = match resolved[i].as_ref().expect("resolved above") {
            Resolved::Content { id, kind, title } => format!("{title:?} ({} {id})", if *kind == Kind::Folder { "folder" } else { "page" }),
            Resolved::InUpload { file, title } => format!("{title:?} (from {file})"),
        };
        println!("{} {}: space {}, under {ancestor}", if opts.dry_run { "Plan for entry" } else { "Entry" }, plan.entry + 1, plan.space_key);
        print_tree(&plan.nodes, 1, &places, opts.dry_run);
    }

    // Pass 2: every file's body, labels and property, as single-file upload does.
    println!("{}", if opts.dry_run { "Pages:" } else { "Uploading:" });
    let mut failure = None;
    for_each_page(&plans, &mut |plan, node| {
        if failure.is_some() {
            return;
        }
        let plan::Kind::Page { file, .. } = &node.kind else { return };
        let path = config.path_of(file);
        let place = places[file].clone();
        if opts.dry_run && place == Place::New {
            println!("  {file}  would be created");
            counts.created += 1;
            return;
        }
        let (parent, parent_title) = parents[file].clone();
        let created = matches!(place, Place::Created(_));
        let tree = InTree { parent: parent.unwrap_or_default(), parent_title, labels: plan.labels.clone(), created };
        let o = file_options(opts, &path, Some(tree));
        let result = (|| {
            let md = std::fs::read_to_string(&path).map_err(|e| Stop::Usage(e.to_string()))?;
            let local = upload::check_file(&o, &md)?;
            let links = upload::link_targets(local.dir, &local.body, &creds.base_url, Some(&plan.space_key), opts.dry_run.then_some(&pending))?;
            upload::update(&o, &local, &client, place.id().expect("placed in pass 1"), links)
        })();
        match result {
            Ok(outcome) => {
                match place {
                    // Counted when it was created, in pass 1.
                    Place::Created(_) => {}
                    _ if outcome.changed => counts.updated += 1,
                    _ => counts.unchanged += 1,
                }
                if outcome.misplaced {
                    counts.misplaced += 1;
                }
                if outcome.moved {
                    counts.moved += 1;
                }
            }
            Err(stop) => {
                // A page created moments ago that Confluence can't find: its problem.
                let stop = match &place {
                    Place::Created(id) => upload::vanished(stop, id),
                    _ => stop,
                };
                failure = Some(Failure::File(path, stop));
            }
        }
    });
    if let Some(f) = failure {
        return Err(f);
    }

    // Pass 3: siblings in the config's order (moving pages doesn't create versions).
    for (i, plan) in plans.iter().enumerate() {
        let (parent, kind, title) = match resolved[i].as_ref().expect("resolved above") {
            Resolved::Content { id, kind, title } => (Some(id.clone()), *kind, title.clone()),
            Resolved::InUpload { file, title } => (places[file].id().map(str::to_string), Kind::Page, title.clone()),
        };
        let mut ordering = Ordering { opts, client: &client, places: &places, counts: &mut *counts };
        ordering.level(parent.as_deref(), kind, &title, &plan.nodes)?;
    }

    // Pass 4: pages and folders rfluence created whose files are gone (design.md, "Renames
    // and deletions"). Found under every entry's ancestor, which takes a request per page, so
    // only with --prune or --dry-run; trashed only with --prune.
    let known: HashSet<String> = places.values().filter_map(|p| p.id().map(str::to_string)).collect();
    let mut roots: Vec<(String, Kind)> = Vec::new();
    for r in resolved.iter().flatten() {
        let root = match r {
            Resolved::Content { id, kind, .. } => Some((id.clone(), *kind)),
            Resolved::InUpload { file, .. } => places[file].id().map(|id| (id.to_string(), Kind::Page)),
        };
        if let Some(root) = root.filter(|r| !roots.contains(r)) {
            roots.push(root);
        }
    }
    if opts.prune || opts.dry_run {
        prune(opts, &client, &roots, &known, counts)?;
    }
    println!("{}", counts.summary(opts.dry_run));
    Ok(())
}

/// The file of the page titled `title` in the upload set for `space`.
fn find_page_titled(plans: &[EntryPlan], space: &str, title: &str) -> Option<String> {
    let mut found = None;
    for plan in plans.iter().filter(|p| p.space_key == space) {
        for n in &plan.nodes {
            n.walk(&mut |n| {
                if let (plan::Kind::Page { file, .. }, Some(t)) = (&n.kind, &n.title)
                    && t == title && found.is_none() {
                    found = Some(file.clone());
                }
            });
        }
    }
    found
}

/// Visit every node of every entry, parents before children.
fn for_each_page<'a>(plans: &'a [EntryPlan], f: &mut impl FnMut(&'a EntryPlan, &'a Node)) {
    for plan in plans {
        for n in &plan.nodes {
            n.walk(&mut |n| f(plan, n));
        }
    }
}

/// The first pass over one entry's tree.
struct Pass<'a> {
    opts: &'a Options,
    config: &'a Config,
    client: &'a Client,
    space: &'a Space,
    plan: &'a EntryPlan,
    places: &'a mut HashMap<String, Place>,
    /// Each file's parent in the tree: its ID (none if it's new, with --dry-run) and title.
    parents: &'a mut HashMap<String, (Option<String>, String)>,
    counts: &'a mut Counts,
}

impl Pass<'_> {
    /// Place `nodes` under a parent (`None`: it doesn't exist yet, with --dry-run).
    fn nodes(&mut self, nodes: &[Node], parent: Option<&str>, parent_kind: Kind, parent_title: &str) -> Result<(), Failure> {
        // The parent's children, to find existing folders by title.
        let children = match parent {
            Some(id) if nodes.iter().any(|n| matches!(n.kind, plan::Kind::Folder { .. })) => self.client.children(parent_kind, id)?,
            _ => Vec::new(),
        };
        for node in nodes {
            let title = node.title.clone().unwrap_or_default();
            match &node.kind {
                plan::Kind::Folder { dir } => {
                    let existing = children.iter().find(|c| c.kind == "folder" && c.title == title).map(|c| c.id.clone());
                    let place = match existing {
                        Some(id) => Place::Existing(id),
                        None => {
                            // Folder titles are unique in a space.
                            if let Some(other) = self.client.folders_titled(&self.plan.space_key, &title)?.first() {
                                return Err(usage(format!(
                                    "{dir}/: space {} already has a folder titled {title:?} (folder {other}) somewhere else; set the entry's `folder_title`, or add an index.md to the directory",
                                    self.plan.space_key
                                )));
                            }
                            match (parent, self.opts.dry_run) {
                                (Some(p), false) => {
                                    let id = self.client.create_folder(&self.space.id, p, &title)?;
                                    let value = serde_json::json!({ "managed": true, "path": dir, "config_labels": [] });
                                    self.client.set_property_of(Kind::Folder, &id, upload::PROPERTY, value, None)?;
                                    self.counts.folders_created += 1;
                                    Place::Created(id)
                                }
                                _ => {
                                    self.counts.folders_created += 1;
                                    Place::New
                                }
                            }
                        }
                    };
                    let id = place.id().map(str::to_string);
                    self.places.insert(format!("{dir}/"), place);
                    self.nodes(&node.children, id.as_deref(), Kind::Folder, &title)?;
                }
                plan::Kind::Page { file, .. } => {
                    self.parents.insert(file.clone(), (parent.map(str::to_string), parent_title.to_string()));
                    if self.places[file] == Place::New
                        && let (Some(p), false) = (parent, self.opts.dry_run) {
                        let path = self.config.path_of(file);
                        let id = upload::create_empty(self.client, &path, self.space, p, &title, &self.plan.labels)
                            .map_err(|stop| Failure::File(path.clone(), stop))?;
                        self.places.insert(file.clone(), Place::Created(id));
                        self.counts.created += 1;
                    }
                    let id = self.places[file].id().map(str::to_string);
                    let title = node.title.clone().unwrap_or_else(|| file.clone());
                    self.nodes(&node.children, id.as_deref(), Kind::Page, &title)?;
                }
            }
        }
        Ok(())
    }
}

/// A page or folder under an entry's ancestor, for finding orphans.
struct Found {
    id: String,
    kind: Kind,
    title: String,
    /// Has the `rfluence` property: rfluence created it.
    managed: bool,
    /// The version rfluence last uploaded (pages), from the property.
    uploaded_version: Option<u64>,
    /// The `path` recorded in the property.
    path: Option<String>,
    children: Vec<usize>,
}

/// Find orphans under the ancestors: pages and folders rfluence created that aren't in the
/// tree any more. Report them, and with --prune trash those that can go: not edited in
/// Confluence since rfluence's last upload (unless --force), and with nothing under them that
/// stays (trashing moves children up a level).
fn prune(opts: &Options, client: &Client, roots: &[(String, Kind)], known: &HashSet<String>, counts: &mut Counts) -> Result<(), Failure> {
    // Everything under the ancestors, with the `rfluence` property of each.
    let mut found: Vec<Found> = Vec::new();
    let mut top: Vec<usize> = Vec::new();
    let mut queue: Vec<(Option<usize>, String, Kind)> = roots.iter().map(|(id, kind)| (None, id.clone(), *kind)).collect();
    let mut seen: HashSet<String> = roots.iter().map(|(id, _)| id.clone()).collect();
    while let Some((parent, id, kind)) = queue.pop() {
        for child in client.children(kind, &id)? {
            if !seen.insert(child.id.clone()) {
                continue;
            }
            let kind = if child.kind == "folder" { Kind::Folder } else { Kind::Page };
            let i = found.len();
            found.push(Found { id: child.id.clone(), kind, title: child.title, managed: false, uploaded_version: None, path: None, children: vec![] });
            match parent {
                Some(p) => found[p].children.push(i),
                None => top.push(i),
            }
            queue.push((Some(i), child.id, kind));
        }
    }
    const PARALLEL: usize = 6;
    for chunk in (0..found.len()).collect::<Vec<_>>().chunks(PARALLEL) {
        let properties: Vec<_> = std::thread::scope(|s| {
            let handles: Vec<_> = chunk.iter().map(|&i| {
                let (kind, id) = (found[i].kind, found[i].id.clone());
                s.spawn(move || client.property_of(kind, &id, upload::PROPERTY))
            }).collect();
            handles.into_iter().map(|h| h.join().expect("property thread doesn't panic")).collect()
        });
        for (&i, property) in chunk.iter().zip(properties) {
            if let Some(p) = property? {
                found[i].managed = p.value.get("managed").and_then(|m| m.as_bool()).unwrap_or(false);
                found[i].uploaded_version = p.value.get("version").and_then(|v| v.as_u64());
                found[i].path = p.value.get("path").and_then(|v| v.as_str()).map(str::to_string);
            }
        }
    }

    // Decide bottom-up: a node can go if it's an orphan, unchanged since rfluence's upload,
    // and everything under it goes too.
    let mut goes = vec![false; found.len()];
    let mut reasons: Vec<(usize, String)> = Vec::new();
    fn decide(i: usize, found: &[Found], known: &HashSet<String>, client: &Client, force: bool, goes: &mut Vec<bool>, reasons: &mut Vec<(usize, String)>) -> Result<bool, Failure> {
        let mut all_children_go = true;
        for &c in &found[i].children {
            all_children_go &= decide(c, found, known, client, force, goes, reasons)?;
        }
        let f = &found[i];
        if !f.managed || known.contains(&f.id) {
            return Ok(false);
        }
        if !all_children_go {
            let staying: Vec<String> = f.children.iter().filter(|&&c| !goes[c]).map(|&c| format!("{:?}", found[c].title)).collect();
            reasons.push((i, format!("has pages or folders that stay under it ({})", staying.join(", "))));
            return Ok(false);
        }
        if f.kind == Kind::Page && !force {
            let current = client.content(&f.id)?.version;
            if current != f.uploaded_version {
                reasons.push((
                    i,
                    format!(
                        "was edited in Confluence since rfluence uploaded it (version {} -> {}); --force trashes it anyway",
                        f.uploaded_version.map_or("?".into(), |v| v.to_string()),
                        current.map_or("?".into(), |v| v.to_string())
                    ),
                ));
                return Ok(false);
            }
        }
        goes[i] = true;
        Ok(true)
    }
    for &i in &top {
        decide(i, &found, known, client, opts.force, &mut goes, &mut reasons)?;
    }

    let orphans: Vec<usize> = (0..found.len()).filter(|&i| found[i].managed && !known.contains(&found[i].id)).collect();
    if orphans.is_empty() {
        return Ok(());
    }
    println!("Orphans (rfluence created them; their files or directories are gone):");
    // Children before parents, so folders are empty when they go.
    let mut order: Vec<usize> = Vec::new();
    fn post_order(i: usize, found: &[Found], out: &mut Vec<usize>) {
        for &c in &found[i].children {
            post_order(c, found, out);
        }
        out.push(i);
    }
    for &i in &top {
        post_order(i, &found, &mut order);
    }
    for i in order.into_iter().filter(|i| orphans.contains(i)) {
        let f = &found[i];
        let what = format!(
            "{} {} {:?}{}",
            if f.kind == Kind::Folder { "folder" } else { "page" },
            f.id,
            f.title,
            f.path.as_deref().map(|p| format!(" (was {p})")).unwrap_or_default()
        );
        if let Some((_, reason)) = reasons.iter().find(|(r, _)| *r == i) {
            println!("  kept {what}: {reason}");
            counts.orphans_kept += 1;
        } else if !opts.prune {
            println!("  {what}");
            counts.orphans += 1;
        } else {
            if !opts.dry_run {
                client.trash(f.kind, &f.id)?;
            }
            println!("  {} {what}", if opts.dry_run { "would trash" } else { "trashed" });
            counts.pruned += 1;
        }
    }
    Ok(())
}

/// The third pass: each parent's children in the config's order.
struct Ordering<'a> {
    opts: &'a Options,
    client: &'a Client,
    places: &'a HashMap<String, Place>,
    counts: &'a mut Counts,
}

impl Ordering<'_> {
    fn level(&mut self, parent: Option<&str>, kind: Kind, parent_title: &str, nodes: &[Node]) -> Result<(), Failure> {
        let ids: Vec<Option<String>> = nodes.iter().map(|n| self.places.get(&key(n)).and_then(|p| p.id().map(str::to_string))).collect();
        if let Some(parent) = parent.filter(|_| nodes.len() > 1) {
            let desired: Vec<String> = ids.iter().flatten().cloned().collect();
            let current: Vec<String> = self.client.children(kind, parent)?.into_iter().map(|c| c.id).collect();
            let new: HashSet<String> =
                nodes.iter().filter_map(|n| match self.places.get(&key(n)) { Some(Place::Created(id)) => Some(id.clone()), _ => None }).collect();
            let (moves, unmoved) = crate::order::moves(&desired, &current, &new, self.opts.move_pages);
            let title_of = |id: &str| {
                let i = ids.iter().position(|x| x.as_deref() == Some(id)).expect("a node of this level");
                nodes[i].title.clone().unwrap_or_else(|| id.to_string())
            };
            for m in &moves {
                if !self.opts.dry_run {
                    self.client.move_next_to(&m.id, m.after, &m.target)?;
                }
                if !new.contains(&m.id) {
                    let verb = if self.opts.dry_run { "would move" } else { "moved" };
                    println!("  {verb} {:?} {} {:?} (under {parent_title:?})", title_of(&m.id), if m.after { "after" } else { "before" }, title_of(&m.target));
                    self.counts.reordered += 1;
                }
            }
            if !unmoved.is_empty() {
                let titles: Vec<String> = unmoved.iter().map(|id| format!("{:?}", title_of(id))).collect();
                eprintln!(
                    "rfluence: warning: under {parent_title:?}, {} out of the config's order; --move reorders",
                    if titles.len() == 1 { format!("{} is", titles[0]) } else { format!("{} are", titles.join(", ")) }
                );
                self.counts.out_of_order += unmoved.len();
            }
        }
        for (node, id) in nodes.iter().zip(&ids) {
            if !node.children.is_empty() {
                let kind = if matches!(node.kind, plan::Kind::Folder { .. }) { Kind::Folder } else { Kind::Page };
                let title = node.title.clone().unwrap_or_default();
                self.level(id.as_deref(), kind, &title, &node.children)?;
            }
        }
        Ok(())
    }
}

/// A node's key in the places map: its file, or its directory with a trailing `/`.
fn key(node: &Node) -> String {
    match &node.kind {
        plan::Kind::Page { file, .. } => file.clone(),
        plan::Kind::Folder { dir } => format!("{dir}/"),
    }
}

fn print_tree(nodes: &[Node], depth: usize, places: &HashMap<String, Place>, dry_run: bool) {
    for n in nodes {
        let pad = "  ".repeat(depth);
        let title = n.title.as_deref().unwrap_or("(its current title)");
        let (key, what) = match &n.kind {
            plan::Kind::Page { file, .. } => (file.clone(), file.clone()),
            plan::Kind::Folder { dir } => (format!("{dir}/"), format!("{dir}/")),
        };
        let kind = if matches!(n.kind, plan::Kind::Folder { .. }) { "folder" } else { "page" };
        let status = match places.get(&key) {
            Some(Place::Existing(id)) => format!("{kind} {id}"),
            Some(Place::Adopted(id)) => format!("{kind} {id}, found by title (--force)"),
            Some(Place::Created(id)) => format!("new {kind} {id}"),
            Some(Place::New) | None => format!("new {kind}{}", if dry_run { "" } else { " (not created)" }),
        };
        println!("{pad}{title}  [{status}]  {what}");
        print_tree(&n.children, depth + 1, places, dry_run);
    }
}

#[derive(Default)]
pub struct Counts {
    created: usize,
    updated: usize,
    unchanged: usize,
    folders_created: usize,
    misplaced: usize,
    moved: usize,
    reordered: usize,
    out_of_order: usize,
    /// Orphans found without --prune.
    orphans: usize,
    /// Orphans --prune keeps (edited, or with things under them that stay).
    orphans_kept: usize,
    pruned: usize,
}

impl Counts {
    /// Did this run change anything in Confluence?
    fn did_anything(&self) -> bool {
        self.created + self.updated + self.folders_created + self.moved + self.reordered + self.pruned > 0
    }

    fn summary(&self, dry_run: bool) -> String {
        let (create, update) = if dry_run { ("to create", "to update") } else { ("created", "updated") };
        let mut parts = vec![
            format!("{} page{} {create}", self.created, plural(self.created)),
            format!("{} {update}", self.updated),
            format!("{} up to date", self.unchanged),
        ];
        if self.folders_created > 0 {
            parts.push(format!("{} folder{} {create}", self.folders_created, plural(self.folders_created)));
        }
        let verb = if dry_run { "to move" } else { "moved" };
        if self.moved > 0 {
            parts.push(format!("{} {verb} to another parent", self.moved));
        }
        if self.reordered > 0 {
            parts.push(format!("{} {verb} into order", self.reordered));
        }
        if self.pruned > 0 {
            parts.push(format!("{} {}", self.pruned, if dry_run { "to trash" } else { "trashed" }));
        }
        let mut out = format!("{}{}.", if dry_run { "Dry run: " } else { "" }, parts.join(", "));
        if self.misplaced > 0 {
            out.push_str(&format!(
                " {} page{} not where the config puts {} (left in place; see the warnings; --move moves them).",
                self.misplaced,
                if self.misplaced == 1 { " is" } else { "s are" },
                if self.misplaced == 1 { "it" } else { "them" }
            ));
        }
        if self.out_of_order > 0 {
            out.push_str(&format!(" {} out of the config's order (--move reorders).", self.out_of_order));
        }
        if self.orphans > 0 {
            out.push_str(&format!(" {} orphan{} (--prune trashes them).", self.orphans, plural(self.orphans)));
        }
        if self.orphans_kept > 0 {
            out.push_str(&format!(" {} orphan{} kept (see above).", self.orphans_kept, plural(self.orphans_kept)));
        }
        out
    }
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}
