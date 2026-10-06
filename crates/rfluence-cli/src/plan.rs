//! The page tree an `upload --config` makes: each entry's files as pages and folders under
//! its ancestor, in order. Built from the config and the files alone (no network). See
//! design.md, "Page hierarchy" and "Child page order".

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use rfluence_convert::{frontmatter, labels, upload_title};

use crate::config::{self, Config, Matched};

/// Where an entry's pages go.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ancestor {
    Id(String),
    Title(String),
}

/// An entry's place in Confluence and its tree.
#[derive(Debug, Clone, PartialEq)]
pub struct EntryPlan {
    /// Index into the config's entries.
    pub entry: usize,
    pub space_key: String,
    pub ancestor: Ancestor,
    /// Normalized; added to every page in the entry.
    pub labels: Vec<String>,
    /// The directory that maps to the ancestor, relative to the project root.
    pub root: String,
    /// The ancestor's children, in order.
    pub nodes: Vec<Node>,
}

/// A page (from a file) or a folder (from a directory without an index page).
#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    pub kind: Kind,
    /// The page title (`None` for an existing page whose file sets none: it keeps its title),
    /// or the folder title.
    pub title: Option<String>,
    pub children: Vec<Node>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    Page {
        /// Relative to the project root.
        file: String,
        /// From the file's frontmatter: `None` for a page to create.
        id: Option<String>,
    },
    Folder {
        /// Relative to the project root.
        dir: String,
    },
}

impl Node {
    /// Visit this node and its descendants, parents before children.
    pub fn walk<'a>(&'a self, f: &mut impl FnMut(&'a Node)) {
        f(self);
        for c in &self.children {
            c.walk(f);
        }
    }
}

/// What a file says about its page.
#[derive(Debug, Clone, Default)]
struct FileInfo {
    id: Option<String>,
    title: Option<String>,
    weight: Option<i64>,
}

fn read_file(root: &Path, file: &str) -> Result<FileInfo, String> {
    let md = crate::text::read(root.join(file)).map_err(|e| format!("{file}: {e}"))?;
    let doc = frontmatter::split(&md);
    let fields = doc
        .yaml
        .map(frontmatter::rfluence_fields)
        .unwrap_or_default();
    let (title, _) = upload_title(doc.body, fields.title.as_deref());
    if title.is_none() && fields.id.is_none() {
        return Err(format!(
            "{file}: has no title for its new page: start it with `# Title`, or set `title` under `rfluence:`"
        ));
    }
    Ok(FileInfo {
        id: fields.id,
        title,
        weight: fields.weight,
    })
}

/// Is this file a directory's page?
fn is_index(file_name: &str) -> bool {
    matches!(
        file_name.to_ascii_lowercase().as_str(),
        "index.md" | "readme.md"
    )
}

/// Build every entry's tree. `ancestor_titles` gives each entry's ancestor title (for
/// `{parent}` in folder titles of top-level directories), by entry index.
pub fn build(
    config: &Config,
    matched: &[Matched],
    ancestor_titles: &[String],
) -> Result<Vec<EntryPlan>, String> {
    let mut errors = Vec::new();
    let mut plans = Vec::new();
    for m in matched {
        let entry = &config.entries[m.entry];
        let mut infos = BTreeMap::new();
        for f in &m.files {
            match read_file(&config.root, f) {
                Ok(info) => {
                    infos.insert(f.clone(), info);
                }
                Err(e) => errors.push(e),
            }
        }
        let builder = Builder {
            files: &m.files,
            infos: &infos,
            folder_title: entry.folder_title.as_deref(),
            root: &m.root,
        };
        let ancestor_title = ancestor_titles.get(m.entry).cloned().unwrap_or_default();
        let nodes = match builder.index_of(&m.root) {
            Ok(Some(index)) => vec![builder.dir_node(&m.root, &ancestor_title, Some(index))],
            Ok(None) => builder.children(&m.root, &ancestor_title),
            Err(e) => {
                errors.push(e);
                Vec::new()
            }
        };
        builder.check_indexes(&mut errors);
        let ancestor = match (&entry.ancestor_id, &entry.ancestor) {
            (Some(id), _) => Ancestor::Id(id.clone()),
            (None, Some(title)) => Ancestor::Title(title.clone()),
            (None, None) => unreachable!("checked when the config was loaded"),
        };
        let labels = entry
            .labels
            .iter()
            .filter_map(|l| labels::normalize_label(l).ok())
            .collect();
        plans.push(EntryPlan {
            entry: m.entry,
            space_key: entry.space_key.clone(),
            ancestor,
            labels,
            root: m.root.clone(),
            nodes,
        });
    }
    errors.extend(collisions(&plans));
    if errors.is_empty() {
        Ok(plans)
    } else {
        Err(errors.join("\n"))
    }
}

struct Builder<'a> {
    files: &'a [String],
    infos: &'a BTreeMap<String, FileInfo>,
    folder_title: Option<&'a str>,
    root: &'a str,
}

impl Builder<'_> {
    /// The entry's files directly in `dir` (relative to the project root).
    fn files_in(&self, dir: &str) -> Vec<&String> {
        self.files.iter().filter(|f| parent_dir(f) == dir).collect()
    }

    /// The subdirectories of `dir` that hold any of the entry's files.
    fn subdirs(&self, dir: &str) -> BTreeSet<String> {
        let prefix = if dir.is_empty() {
            String::new()
        } else {
            format!("{dir}/")
        };
        self.files
            .iter()
            .filter_map(|f| f.strip_prefix(&prefix))
            .filter_map(|rest| rest.split_once('/').map(|(d, _)| format!("{prefix}{d}")))
            .collect()
    }

    /// The directory's page (`index.md` or `README.md`), if it has one.
    fn index_of(&self, dir: &str) -> Result<Option<&String>, String> {
        let indexes: Vec<&String> = self
            .files_in(dir)
            .into_iter()
            .filter(|f| is_index(file_name(f)))
            .collect();
        match indexes.as_slice() {
            [] => Ok(None),
            [one] => Ok(Some(one)),
            many => Err(format!(
                "{}: has more than one page for the directory ({}); keep one",
                if dir.is_empty() { "." } else { dir },
                many.iter()
                    .map(|f| f.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
        }
    }

    /// Report directories with more than one index file (below the root, which `build`
    /// checks).
    fn check_indexes(&self, errors: &mut Vec<String>) {
        let mut dirs: BTreeSet<String> = BTreeSet::new();
        for f in self.files {
            let mut d = parent_dir(f).to_string();
            while d.len() > self.root.len() {
                dirs.insert(d.clone());
                d = parent_dir(&d).to_string();
            }
        }
        for d in dirs {
            if let Err(e) = self.index_of(&d) {
                errors.push(e);
            }
        }
    }

    fn page(&self, file: &str, children: Vec<Node>) -> Node {
        let info = &self.infos.get(file).cloned().unwrap_or_default();
        Node {
            kind: Kind::Page {
                file: file.to_string(),
                id: info.id.clone(),
            },
            title: info.title.clone(),
            children,
        }
    }

    /// A directory as a node: its index page with the rest below, or a folder.
    fn dir_node(&self, dir: &str, parent_title: &str, index: Option<&String>) -> Node {
        match index {
            Some(index) => {
                let title = self
                    .infos
                    .get(index)
                    .and_then(|i| i.title.clone())
                    .unwrap_or_else(|| stem(index).to_string());
                let mut node = self.page(index, Vec::new());
                node.children = self.children(dir, &title);
                node
            }
            None => {
                let path = dir
                    .strip_prefix(self.root)
                    .unwrap_or(dir)
                    .trim_start_matches('/');
                let title =
                    config::folder_title(self.folder_title, file_name(dir), parent_title, path);
                let children = self.children(dir, &title);
                Node {
                    kind: Kind::Folder {
                        dir: dir.to_string(),
                    },
                    title: Some(title),
                    children,
                }
            }
        }
    }

    /// The pages and directories in `dir` (except its index page), in order.
    fn children(&self, dir: &str, title: &str) -> Vec<Node> {
        let mut nodes: Vec<(SortKey, Node)> = Vec::new();
        for f in self
            .files_in(dir)
            .into_iter()
            .filter(|f| !is_index(file_name(f)))
        {
            let weight = self.infos.get(f).and_then(|i| i.weight);
            nodes.push((
                SortKey {
                    weight,
                    name: file_name(f).to_string(),
                },
                self.page(f, Vec::new()),
            ));
        }
        for d in self.subdirs(dir) {
            let index = self.index_of(&d).ok().flatten();
            let weight = index.and_then(|i| self.infos.get(i)).and_then(|i| i.weight);
            nodes.push((
                SortKey {
                    weight,
                    name: file_name(&d).to_string(),
                },
                self.dir_node(&d, title, index),
            ));
        }
        nodes.sort_by(|a, b| a.0.cmp(&b.0));
        nodes.into_iter().map(|(_, n)| n).collect()
    }
}

/// Sibling order: weighted first (lower first), then by name, naturally and case-insensitively.
#[derive(Debug, PartialEq, Eq)]
struct SortKey {
    weight: Option<i64>,
    name: String,
}

impl Ord for SortKey {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self.weight, other.weight) {
            (Some(a), Some(b)) if a != b => a.cmp(&b),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            _ => natural_cmp(&self.name, &other.name),
        }
    }
}

impl PartialOrd for SortKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Compare names with digit runs as numbers (`2-setup` before `10-faq`), ignoring case.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    fn chunks(s: &str) -> Vec<(bool, String)> {
        let mut out: Vec<(bool, String)> = Vec::new();
        for c in s.chars().flat_map(char::to_lowercase) {
            let digit = c.is_ascii_digit();
            match out.last_mut() {
                Some((d, run)) if *d == digit => run.push(c),
                _ => out.push((digit, c.to_string())),
            }
        }
        out
    }
    let (ca, cb) = (chunks(a), chunks(b));
    for ((da, ra), (db, rb)) in ca.iter().zip(&cb) {
        let ord = if *da && *db {
            let (ta, tb) = (ra.trim_start_matches('0'), rb.trim_start_matches('0'));
            ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb))
        } else {
            ra.cmp(rb)
        };
        if ord != Ordering::Equal {
            return ord;
        }
    }
    ca.len().cmp(&cb.len()).then_with(|| a.cmp(b))
}

/// Titles that would clash: page titles and folder titles are each unique in a space.
fn collisions(plans: &[EntryPlan]) -> Vec<String> {
    let mut pages: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();
    let mut folders: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();
    for plan in plans {
        for node in &plan.nodes {
            node.walk(&mut |n| {
                let Some(title) = &n.title else { return };
                let key = (plan.space_key.clone(), title.clone());
                match &n.kind {
                    Kind::Page { file, .. } => pages.entry(key).or_default().push(file.clone()),
                    Kind::Folder { dir } => folders.entry(key).or_default().push(format!("{dir}/")),
                }
            });
        }
    }
    let mut errors = Vec::new();
    for ((space, title), files) in pages.iter().filter(|(_, f)| f.len() > 1) {
        errors.push(format!(
            "pages in space {space} would share the title {title:?}: {}; set a different `title` under `rfluence:` in all but one",
            files.join(", ")
        ));
    }
    for ((space, title), dirs) in folders.iter().filter(|(_, d)| d.len() > 1) {
        errors.push(format!(
            "folders in space {space} would share the title {title:?}: {}; set the entry's `folder_title` (e.g. \"{{parent}} / {{dir}}\"), or add an index.md to the directories",
            dirs.join(", ")
        ));
    }
    errors
}

fn parent_dir(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(d, _)| d)
}

fn file_name(path: &str) -> &str {
    path.rsplit_once('/').map_or(path, |(_, f)| f)
}

fn stem(path: &str) -> &str {
    let name = file_name(path);
    name.rsplit_once('.').map_or(name, |(s, _)| s)
}

/// The tree as text, for tests: one line per node, indented.
#[cfg(test)]
pub fn render(nodes: &[Node], depth: usize, out: &mut String) {
    for n in nodes {
        let pad = "  ".repeat(depth);
        let title = n.title.as_deref().unwrap_or("(its current title)");
        match &n.kind {
            Kind::Page { file, id: Some(id) } => {
                out.push_str(&format!("{pad}{title}  page {id}, {file}\n"))
            }
            Kind::Page { file, id: None } => {
                out.push_str(&format!("{pad}{title}  new page, {file}\n"))
            }
            Kind::Folder { dir } => out.push_str(&format!("{pad}{title}  folder, {dir}/\n")),
        }
        render(&n.children, depth + 1, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{FILE_NAME, match_files};

    struct Project(std::path::PathBuf);

    impl Project {
        fn new(name: &str, files: &[(&str, &str)], config: &str) -> Project {
            let dir =
                std::env::temp_dir().join(format!("rfluence-plan-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            for (f, content) in files {
                let path = dir.join(f);
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(path, content).unwrap();
            }
            std::fs::write(dir.join(FILE_NAME), config).unwrap();
            Project(dir)
        }

        fn plan(&self) -> Result<Vec<EntryPlan>, String> {
            let config = Config::load(&self.0.join(FILE_NAME))?;
            let matched = match_files(&config)?;
            build(
                &config,
                &matched,
                &["Engineering Docs".to_string(), "Other".to_string()],
            )
        }

        fn tree(&self) -> String {
            let mut out = String::new();
            for p in self.plan().unwrap() {
                render(&p.nodes, 0, &mut out);
            }
            out
        }
    }

    impl Drop for Project {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    const EXAMPLE: &str = "- globs:\n    - how-to/github/**/*.md\n  exclude:\n    - how-to/github/drafts/**\n  space_key: ENG\n  ancestor_id: \"123000\"\n  labels: [GitHub, ai-generated]\n";

    #[test]
    fn builds_the_design_example() {
        let p = Project::new(
            "example",
            &[
                ("how-to/github/README.md", "# GitHub How-tos\n"),
                ("how-to/github/01-setup.md", "# Setting up GitHub\n"),
                (
                    "how-to/github/02-workflow.md",
                    "---\nrfluence:\n  id: \"77\"\n---\n\n# Our GitHub workflow\n",
                ),
                (
                    "how-to/github/actions/runners.md",
                    "# Self-hosted runners\n",
                ),
                ("how-to/github/actions/secrets.md", "# Managing secrets\n"),
                ("how-to/github/drafts/wip.md", "# WIP\n"),
            ],
            EXAMPLE,
        );
        assert_eq!(
            p.tree(),
            "GitHub How-tos  new page, how-to/github/README.md
  Setting up GitHub  new page, how-to/github/01-setup.md
  Our GitHub workflow  page 77, how-to/github/02-workflow.md
  actions  folder, how-to/github/actions/
    Self-hosted runners  new page, how-to/github/actions/runners.md
    Managing secrets  new page, how-to/github/actions/secrets.md
"
        );
        let plan = &p.plan().unwrap()[0];
        assert_eq!(
            (plan.ancestor.clone(), plan.labels.clone()),
            (
                Ancestor::Id("123000".into()),
                vec!["github".to_string(), "ai-generated".to_string()]
            )
        );
    }

    #[test]
    fn without_a_root_index_files_go_directly_under_the_ancestor() {
        let p = Project::new(
            "no-root-index",
            &[
                ("docs/10-faq.md", "# FAQ\n"),
                ("docs/2-setup.md", "# Setup\n"),
                (
                    "docs/Zeta.md",
                    "---\nrfluence:\n  weight: -1\n---\n\n# Zeta first\n",
                ),
                (
                    "docs/api/index.md",
                    "---\nrfluence:\n  weight: 5\n---\n\n# API\n",
                ),
                ("docs/api/v2/auth.md", "# Auth v2\n"),
            ],
            "- globs: ['docs/**/*.md']\n  space_key: ENG\n  ancestor: Engineering Docs\n  folder_title: '{parent} / {dir}'\n",
        );
        // Weighted first (-1, then 5), then by name with numbers in order.
        assert_eq!(
            p.tree(),
            "Zeta first  new page, docs/Zeta.md
API  new page, docs/api/index.md
  API / v2  folder, docs/api/v2/
    Auth v2  new page, docs/api/v2/auth.md
Setup  new page, docs/2-setup.md
FAQ  new page, docs/10-faq.md
"
        );
    }

    #[test]
    fn folder_titles_use_the_template() {
        let p = Project::new(
            "folder-titles",
            &[("guides/a/x.md", "# X\n"), ("guides/a/b/y.md", "# Y\n")],
            "- globs: ['guides/**/*.md']\n  space_key: ENG\n  ancestor: Engineering Docs\n  folder_title: 'Guides {path} (under {parent})'\n",
        );
        assert_eq!(
            p.tree(),
            "Guides a (under Engineering Docs)  folder, guides/a/
  Guides a/b (under Guides a (under Engineering Docs))  folder, guides/a/b/
    Y  new page, guides/a/b/y.md
  X  new page, guides/a/x.md
"
        );
    }

    #[test]
    fn reports_tree_mistakes() {
        let p = Project::new(
            "mistakes",
            &[
                ("a/one.md", "# Same\n"),
                ("a/two.md", "# Same\n"),
                ("a/x/api/p.md", "# P\n"),
                ("a/y/api/q.md", "# Q\n"),
                ("a/z/index.md", "# Z\n"),
                ("a/z/README.md", "# Z readme\n"),
                ("a/untitled.md", "No title here.\n"),
                ("b/one.md", "# Same\n"),
            ],
            "- globs: ['a/**/*.md']\n  space_key: ENG\n  ancestor: A\n- globs: ['b/*.md']\n  space_key: OTHER\n  ancestor: B\n",
        );
        let err = p.plan().unwrap_err();
        for expected in [
            "pages in space ENG would share the title \"Same\": a/one.md, a/two.md",
            "folders in space ENG would share the title \"api\": a/x/api/, a/y/api/",
            "a/z: has more than one page for the directory (a/z/README.md, a/z/index.md)",
            "a/untitled.md: has no title for its new page",
        ] {
            assert!(err.contains(expected), "{expected}: {err}");
        }
        // Different spaces don't clash.
        assert!(!err.contains("b/one.md"), "{err}");
    }

    #[test]
    fn sorts_naturally() {
        let mut names = vec!["10-faq", "2-setup", "1-intro", "B", "a", "img10", "img9"];
        names.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(
            names,
            ["1-intro", "2-setup", "10-faq", "a", "B", "img9", "img10"]
        );
    }
}
