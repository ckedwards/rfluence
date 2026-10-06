//! `.rfluence.yaml`: which markdown files go where in Confluence. See design.md, "Upload config".

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use globset::{Glob, GlobBuilder, GlobSet, GlobSetBuilder};
use serde::Deserialize;

/// The config file's name, in the project root.
pub const FILE_NAME: &str = ".rfluence.yaml";

/// A loaded config: its entries, and the directory paths are relative to.
#[derive(Debug, Clone)]
pub struct Config {
    pub file: PathBuf,
    /// The project root: the directory containing the config file.
    pub root: PathBuf,
    pub entries: Vec<Entry>,
}

/// One entry: a set of files and the place in Confluence they go.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    #[serde(default)]
    pub paths: Vec<String>,
    #[serde(default)]
    pub globs: Vec<String>,
    #[serde(default)]
    pub exclude: Vec<String>,
    pub space_key: String,
    #[serde(default)]
    pub ancestor: Option<String>,
    #[serde(default, deserialize_with = "string_or_number")]
    pub ancestor_id: Option<String>,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default)]
    pub root: Option<String>,
    #[serde(default)]
    pub folder_title: Option<String>,
}

/// IDs are strings, but an unquoted ID reads as a number in YAML.
fn string_or_number<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    match serde_norway::Value::deserialize(d)? {
        serde_norway::Value::Null => Ok(None),
        serde_norway::Value::String(s) => Ok(Some(s)),
        serde_norway::Value::Number(n) => Ok(Some(n.to_string())),
        other => Err(serde::de::Error::custom(format!("expected a page or folder ID, got {other:?}"))),
    }
}

/// The variables `folder_title` can use.
const FOLDER_TITLE_VARIABLES: [&str; 3] = ["dir", "parent", "path"];

impl Config {
    /// A file's path from the current directory (files in the config are relative to the
    /// project root).
    pub fn path_of(&self, file: &str) -> PathBuf {
        if self.root == Path::new(".") { PathBuf::from(file) } else { self.root.join(file) }
    }

    /// Read and check a config file.
    pub fn load(file: &Path) -> Result<Config, String> {
        let text = crate::text::read(file).map_err(|e| format!("{}: {e}", file.display()))?;
        let entries: Vec<Entry> =
            serde_norway::from_str(&text).map_err(|e| format!("{}: {e} (the file is a list of entries; see design.md, \"Upload config\")", file.display()))?;
        let root = file.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new(".")).to_path_buf();
        let config = Config { file: file.to_path_buf(), root, entries };
        let errors = config.check();
        if errors.is_empty() { Ok(config) } else { Err(errors.join("\n")) }
    }

    /// What's wrong with the entries, one message each.
    fn check(&self) -> Vec<String> {
        let mut errors = Vec::new();
        if self.entries.is_empty() {
            errors.push(format!("{}: no entries", self.file.display()));
        }
        for (i, e) in self.entries.iter().enumerate() {
            let mut err = |m: String| errors.push(format!("{}: entry {}: {m}", self.file.display(), i + 1));
            if e.paths.is_empty() && e.globs.is_empty() {
                err("needs `paths` or `globs`".into());
            }
            match (&e.ancestor, &e.ancestor_id) {
                (None, None) => err("needs `ancestor` (a page or folder title) or `ancestor_id`".into()),
                (Some(_), Some(_)) => err("has both `ancestor` and `ancestor_id`; use one".into()),
                _ => {}
            }
            if e.space_key.trim().is_empty() {
                err("`space_key` is empty".into());
            }
            for label in &e.labels {
                if let Err(m) = rfluence_convert::labels::normalize_label(label) {
                    err(m);
                }
            }
            if let Some(template) = &e.folder_title {
                for var in template_variables(template) {
                    if !FOLDER_TITLE_VARIABLES.contains(&var.as_str()) {
                        err(format!("`folder_title` has an unknown variable {{{var}}} (it can use {{dir}}, {{parent}} and {{path}})"));
                    }
                }
            }
            for pattern in e.globs.iter().chain(&e.exclude) {
                if let Err(m) = glob(pattern) {
                    err(m);
                }
            }
        }
        errors
    }
}

/// The `{name}`s in a template.
fn template_variables(template: &str) -> Vec<String> {
    template.split('{').skip(1).filter_map(|rest| rest.split_once('}').map(|(v, _)| v.to_string())).collect()
}

/// A folder title from an entry's `folder_title` template (default `{dir}`).
pub fn folder_title(template: Option<&str>, dir: &str, parent: &str, path: &str) -> String {
    template.unwrap_or("{dir}").replace("{dir}", dir).replace("{parent}", parent).replace("{path}", path)
}

fn glob(pattern: &str) -> Result<Glob, String> {
    // `*` doesn't cross directories; `**` does.
    GlobBuilder::new(pattern).literal_separator(true).build().map_err(|e| format!("invalid glob {pattern:?}: {e}"))
}

fn glob_set(patterns: &[String]) -> GlobSet {
    let mut set = GlobSetBuilder::new();
    for p in patterns {
        if let Ok(g) = glob(p) {
            set.add(g);
        }
    }
    set.build().unwrap_or_else(|_| GlobSet::empty())
}

/// An entry's files: paths relative to the project root, with `/` separators, sorted.
#[derive(Debug, Clone, PartialEq)]
pub struct Matched {
    /// Index into [`Config::entries`].
    pub entry: usize,
    pub files: Vec<String>,
    /// The directory that maps to the entry's ancestor, relative to the project root (`""`
    /// for the root itself). See design.md, "Upload config" > "Entry root".
    pub root: String,
}

/// Match every entry's files, and check that no file is in two entries.
pub fn match_files(config: &Config) -> Result<Vec<Matched>, String> {
    let all = project_markdown(&config.root);
    let mut errors = Vec::new();
    let mut matched = Vec::new();
    for (i, entry) in config.entries.iter().enumerate() {
        let mut err = |m: String| errors.push(format!("{}: entry {}: {m}", config.file.display(), i + 1));
        let globs = glob_set(&entry.globs);
        let exclude = glob_set(&entry.exclude);
        let mut files: Vec<String> = all.iter().filter(|f| globs.is_match(f.as_str()) && !exclude.is_match(f.as_str())).cloned().collect();
        for listed in &entry.paths {
            let path = clean(listed);
            let on_disk = config.root.join(&path);
            if !on_disk.is_file() {
                err(format!("listed file {listed} doesn't exist"));
            } else if !path.to_ascii_lowercase().ends_with(".md") {
                err(format!("listed file {listed} isn't a markdown (.md) file"));
            } else if exclude.is_match(&path) {
                err(format!("listed file {listed} is also excluded by `exclude`"));
            } else {
                files.push(path);
            }
        }
        files.sort();
        files.dedup();
        let root = match &entry.root {
            Some(r) => {
                let r = clean(r).trim_end_matches('/').to_string();
                for f in files.iter().filter(|f| !inside(f, &r)) {
                    err(format!("{f} isn't inside `root` ({r})"));
                }
                r
            }
            None => default_root(entry),
        };
        matched.push(Matched { entry: i, files, root });
    }
    let mut owners: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for m in &matched {
        for f in &m.files {
            owners.entry(f).or_default().push(m.entry + 1);
        }
    }
    for (file, entries) in owners.iter().filter(|(_, e)| e.len() > 1) {
        let list: Vec<String> = entries.iter().map(|e| e.to_string()).collect();
        errors.push(format!(
            "{}: {file} is matched by entries {}; a file can only go to one place, so leave it out of all but one (with `exclude`)",
            config.file.display(),
            list.join(" and ")
        ));
    }
    if errors.is_empty() { Ok(matched) } else { Err(errors.join("\n")) }
}

/// Every markdown file in the project, skipping files ignored by `.gitignore`.
fn project_markdown(root: &Path) -> Vec<String> {
    let mut files = Vec::new();
    for entry in ignore::WalkBuilder::new(root).build().flatten() {
        let path = entry.path();
        if entry.file_type().is_some_and(|t| t.is_file()) && path.extension().is_some_and(|e| e.eq_ignore_ascii_case("md"))
            && let Ok(rel) = path.strip_prefix(root) {
            files.push(rel.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/"));
        }
    }
    files
}

/// A config path as written (`./docs/x.md`, `docs\x.md`) in canonical form (`docs/x.md`).
fn clean(path: &str) -> String {
    let path = path.replace('\\', "/");
    path.split('/').filter(|c| !c.is_empty() && *c != ".").collect::<Vec<_>>().join("/")
}

fn inside(file: &str, dir: &str) -> bool {
    dir.is_empty() || file.strip_prefix(dir).is_some_and(|rest| rest.starts_with('/'))
}

/// The entry root from the config text: the deepest directory shared by every glob's static
/// prefix and every listed file's directory.
fn default_root(entry: &Entry) -> String {
    let mut dirs: Vec<Vec<String>> = Vec::new();
    for g in &entry.globs {
        let parts: Vec<String> = clean(g).split('/').map(str::to_string).collect();
        let wild = parts.iter().position(|p| p.contains(['*', '?', '[', '{']));
        let static_dirs = match wild {
            Some(w) => parts[..w].to_vec(),
            // No wildcard: a single file; its directory.
            None => parts[..parts.len().saturating_sub(1)].to_vec(),
        };
        dirs.push(static_dirs);
    }
    for p in &entry.paths {
        let parts: Vec<String> = clean(p).split('/').map(str::to_string).collect();
        dirs.push(parts[..parts.len().saturating_sub(1)].to_vec());
    }
    let Some(first) = dirs.first() else { return String::new() };
    let common = (0..first.len()).take_while(|&i| dirs.iter().all(|d| d.get(i) == first.get(i))).count();
    first[..common].join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Project(PathBuf);

    impl Project {
        fn new(name: &str, files: &[&str], config: &str) -> Project {
            let dir = std::env::temp_dir().join(format!("rfluence-config-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            for f in files {
                let path = dir.join(f);
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(path, "# x\n").unwrap();
            }
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join(FILE_NAME), config).unwrap();
            Project(dir)
        }

        fn load(&self) -> Result<Config, String> {
            Config::load(&self.0.join(FILE_NAME))
        }
    }

    impl Drop for Project {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    const FILES: &[&str] = &[
        "how-to/github/README.md",
        "how-to/github/01-setup.md",
        "how-to/github/02-workflow.md",
        "how-to/github/actions/runners.md",
        "how-to/github/actions/secrets.md",
        "how-to/github/actions/notes.txt",
        "how-to/github/drafts/wip.md",
        "other/a.md",
    ];

    #[test]
    fn matches_the_design_example() {
        let p = Project::new(
            "example",
            FILES,
            "- globs:\n    - how-to/github/**/*.md\n  exclude:\n    - how-to/github/drafts/**\n  space_key: ENG\n  ancestor_id: 123000\n  labels: [github, ai-generated]\n",
        );
        let config = p.load().unwrap();
        assert_eq!(config.entries[0].ancestor_id.as_deref(), Some("123000"));
        let matched = match_files(&config).unwrap();
        assert_eq!(
            matched[0].files,
            [
                "how-to/github/01-setup.md",
                "how-to/github/02-workflow.md",
                "how-to/github/README.md",
                "how-to/github/actions/runners.md",
                "how-to/github/actions/secrets.md",
            ]
        );
        assert_eq!(matched[0].root, "how-to/github");
    }

    #[test]
    fn computes_entry_roots_from_the_config_text() {
        let root = |globs: &[&str], paths: &[&str]| {
            default_root(&Entry {
                globs: globs.iter().map(|s| s.to_string()).collect(),
                paths: paths.iter().map(|s| s.to_string()).collect(),
                ..Default::default()
            })
        };
        assert_eq!(root(&["docs/**/*.md"], &[]), "docs");
        assert_eq!(root(&["docs/api/*.md", "docs/cli/**"], &[]), "docs");
        assert_eq!(root(&["./docs/a/b.md"], &[]), "docs/a");
        assert_eq!(root(&[], &["docs/x.md", "docs/sub/y.md"]), "docs");
        assert_eq!(root(&["*.md"], &[]), "");
        assert_eq!(root(&["docs/v[12]/*.md"], &[]), "docs");
    }

    #[test]
    fn listed_paths_and_roots() {
        let p = Project::new(
            "paths",
            FILES,
            "- paths: [how-to/github/README.md, ./other/a.md]\n  space_key: ENG\n  ancestor: Docs\n  root: how-to\n",
        );
        let errors = match_files(&p.load().unwrap()).unwrap_err();
        assert!(errors.contains("other/a.md isn't inside `root` (how-to)"), "{errors}");

        let p = Project::new(
            "paths-ok",
            FILES,
            "- paths: [how-to/github/README.md, other/a.md]\n  space_key: ENG\n  ancestor: Docs\n",
        );
        let m = match_files(&p.load().unwrap()).unwrap();
        assert_eq!((m[0].files.len(), m[0].root.as_str()), (2, ""));
    }

    #[test]
    fn reports_config_mistakes() {
        let cases = [
            ("- space_key: ENG\n  ancestor: Docs\n", "needs `paths` or `globs`"),
            ("- globs: ['*.md']\n  space_key: ENG\n", "needs `ancestor`"),
            ("- globs: ['*.md']\n  space_key: ENG\n  ancestor: A\n  ancestor_id: '1'\n", "has both `ancestor` and `ancestor_id`"),
            ("- globs: ['*.md']\n  space_key: ENG\n  ancestor: A\n  lables: [x]\n", "unknown field `lables`"),
            ("- globs: ['*.md']\n  space_key: ENG\n  ancestor: A\n  labels: ['a.b']\n", "contains '.'"),
            ("- globs: ['*.md']\n  space_key: ENG\n  ancestor: A\n  folder_title: '{name}'\n", "unknown variable {name}"),
            ("- globs: ['docs/[*.md']\n  space_key: ENG\n  ancestor: A\n", "invalid glob"),
            ("[]\n", "no entries"),
            ("space_key: ENG\n", "the file is a list of entries"),
        ];
        for (i, (config, expected)) in cases.into_iter().enumerate() {
            let p = Project::new(&format!("mistake-{i}"), &[], config);
            let err = p.load().unwrap_err();
            assert!(err.contains(expected), "{expected}: {err}");
        }
    }

    #[test]
    fn reports_file_mistakes() {
        let p = Project::new(
            "file-mistakes",
            FILES,
            "- paths: [missing.md, how-to/github/actions/notes.txt, how-to/github/drafts/wip.md]\n  exclude: ['**/drafts/**']\n  space_key: ENG\n  ancestor: A\n- globs: ['other/*.md']\n  space_key: ENG\n  ancestor: B\n- globs: ['other/**']\n  space_key: ENG\n  ancestor: C\n",
        );
        let errors = match_files(&p.load().unwrap()).unwrap_err();
        for expected in [
            "entry 1: listed file missing.md doesn't exist",
            "entry 1: listed file how-to/github/actions/notes.txt isn't a markdown (.md) file",
            "entry 1: listed file how-to/github/drafts/wip.md is also excluded by `exclude`",
            "other/a.md is matched by entries 2 and 3",
        ] {
            assert!(errors.contains(expected), "{expected}: {errors}");
        }
    }

    #[test]
    fn fills_in_folder_titles() {
        assert_eq!(folder_title(None, "actions", "GitHub How-tos", "actions"), "actions");
        assert_eq!(folder_title(Some("{parent} / {dir}"), "actions", "GitHub How-tos", "actions"), "GitHub How-tos / actions");
        assert_eq!(folder_title(Some("Docs: {path}"), "v2", "API", "api/v2"), "Docs: api/v2");
    }
}
