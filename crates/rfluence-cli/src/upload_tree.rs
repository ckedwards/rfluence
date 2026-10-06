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
use crate::{EXIT_CONFLICT, EXIT_USAGE, fail};

pub struct Options {
    pub config: PathBuf,
    pub dry_run: bool,
    pub force: bool,
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
    match upload_tree(opts) {
        Ok(()) => ExitCode::SUCCESS,
        Err(Failure::Message(m, code)) => {
            for line in m.lines() {
                eprintln!("rfluence: {line}");
            }
            ExitCode::from(code)
        }
        Err(Failure::File(path, Stop::Client(e))) if path.as_os_str().is_empty() => fail(&e),
        Err(Failure::File(path, stop)) => upload::exit(&file_options(opts, &path, None), stop),
    }
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

fn upload_tree(opts: &Options) -> Result<(), Failure> {
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
        if let plan::Kind::Page { file, id: None } = &node.kind {
            if let Ok(c) = config.root.join(file).canonicalize() {
                pending.insert(c);
            }
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
        if rfluence_convert::check(&md).iter().any(|d| d.severity == rfluence_convert::Severity::Error) {
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
    if check_failed && !opts.force {
        problems.push("files have content Confluence can't store (listed above); fix them, or use --force to upload the approximations".into());
    }
    if !problems.is_empty() {
        return Err(usage(format!("nothing uploaded:\n{}", problems.join("\n"))));
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
    let mut folders_created = 0;
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
            let mut pass = Pass { opts, config: &config, client: &client, space, plan, places: &mut places, parents: &mut parents, folders_created: 0 };
            pass.nodes(&plan.nodes, parent.as_deref(), parent_kind, &parent_title)?;
            folders_created += pass.folders_created;
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
    let mut counts = Counts { folders_created, ..Default::default() };
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
                    Place::Created(_) => counts.created += 1,
                    _ if outcome.changed => counts.updated += 1,
                    _ => counts.unchanged += 1,
                }
                if outcome.misplaced {
                    counts.misplaced += 1;
                }
            }
            Err(stop) => failure = Some(Failure::File(path, stop)),
        }
    });
    if let Some(f) = failure {
        return Err(f);
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
                if let (plan::Kind::Page { file, .. }, Some(t)) = (&n.kind, &n.title) {
                    if t == title && found.is_none() {
                        found = Some(file.clone());
                    }
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
    folders_created: usize,
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
                                    self.folders_created += 1;
                                    Place::Created(id)
                                }
                                _ => {
                                    self.folders_created += 1;
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
                    if self.places[file] == Place::New {
                        if let (Some(p), false) = (parent, self.opts.dry_run) {
                            let path = self.config.path_of(file);
                            let id = upload::create_empty(self.client, &path, self.space, p, &title, &self.plan.labels)
                                .map_err(|stop| Failure::File(path.clone(), stop))?;
                            self.places.insert(file.clone(), Place::Created(id));
                        }
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
struct Counts {
    created: usize,
    updated: usize,
    unchanged: usize,
    folders_created: usize,
    misplaced: usize,
}

impl Counts {
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
        let mut out = format!("{}{}.", if dry_run { "Dry run: " } else { "" }, parts.join(", "));
        if self.misplaced > 0 {
            out.push_str(&format!(
                " {} page{} not where the config puts {} (left in place; see the warnings).",
                self.misplaced,
                if self.misplaced == 1 { " is" } else { "s are" },
                if self.misplaced == 1 { "it" } else { "them" }
            ));
        }
        out
    }
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}
