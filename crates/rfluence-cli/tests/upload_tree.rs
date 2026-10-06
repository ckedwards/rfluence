//! `rfluence upload --config` against an in-memory Confluence.

mod fake;

use std::path::PathBuf;
use std::process::{Command, Output};

use fake::{Fake, HOMEPAGE};

const ANCESTOR: &str = "123000";

struct Project {
    dir: PathBuf,
    fake: Fake,
}

impl Project {
    /// The design.md example: a README, two pages and a directory without an index.
    fn new(name: &str, config: &str) -> Project {
        let dir = std::env::temp_dir().join(format!("rfluence-upload-tree-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let p = Project { dir, fake: Fake::start() };
        p.fake.state().add_with_id(ANCESTOR, "page", "Engineering Docs", Some(HOMEPAGE));
        p.write(".rfluence.yaml", config);
        p.write("how-to/github/README.md", "# GitHub How-tos\n\nStart with [setup](./01-setup.md).\n");
        p.write("how-to/github/01-setup.md", "# Setting up GitHub\n\nSee the [workflow](./02-workflow.md#steps).\n");
        p.write("how-to/github/02-workflow.md", "# Our GitHub workflow\n\n## Steps\n\nBranch, commit, merge.\n");
        p.write("how-to/github/actions/runners.md", "# Self-hosted runners\n\nRunners.\n");
        p.write("how-to/github/actions/secrets.md", "# Managing secrets\n\nSecrets.\n");
        p.write("how-to/github/drafts/wip.md", "# WIP\n");
        p
    }

    fn write(&self, file: &str, text: &str) {
        let path = self.dir.join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn read(&self, file: &str) -> String {
        std::fs::read_to_string(self.dir.join(file)).unwrap()
    }

    fn upload(&self, extra: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_rfluence"))
            .args(["upload", "--config"])
            .args(extra)
            .current_dir(&self.dir)
            .env("CONFLUENCE_BASE_URL", &self.fake.url)
            .env("CONFLUENCE_EMAIL", "me@example.com")
            .env("CONFLUENCE_API_KEY", "secret")
            .env("RFLUENCE_CONFIG_DIR", self.dir.join(".config"))
            .env("RFLUENCE_NO_KEYRING", "1")
            .env_remove("RFLUENCE_SITE")
            .output()
            .unwrap()
    }

    /// The page ID recorded in a file.
    fn id(&self, file: &str) -> String {
        let md = self.read(file);
        let fields = rfluence_convert::frontmatter::rfluence_fields(rfluence_convert::frontmatter::split(&md).yaml.unwrap_or_default());
        fields.id.unwrap_or_else(|| panic!("{file} has no ID:\n{md}"))
    }
}

impl Drop for Project {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn err(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

const EXAMPLE: &str = "- globs:\n    - how-to/github/**/*.md\n  exclude:\n    - how-to/github/drafts/**\n  space_key: ENG\n  ancestor_id: \"123000\"\n  labels: [github, AI Generated]\n";

#[test]
fn uploads_the_design_example() {
    let p = Project::new("example", EXAMPLE);

    // A dry run shows the tree and changes nothing.
    let o = p.upload(&["--dry-run"]);
    assert!(o.status.success(), "{}", err(&o));
    let text = out(&o);
    for expected in [
        "Plan for entry 1: space ENG, under \"Engineering Docs\" (page 123000)",
        "  GitHub How-tos  [new page]  how-to/github/README.md",
        "    Setting up GitHub  [new page]  how-to/github/01-setup.md",
        "    actions  [new folder]  how-to/github/actions/",
        "      Self-hosted runners  [new page]  how-to/github/actions/runners.md",
        "Dry run: 5 pages to create, 0 to update, 0 up to date, 1 folder to create.",
    ] {
        assert!(text.contains(expected), "{expected}\n{text}");
    }
    assert!(p.fake.state().writes().is_empty(), "{:?}", p.fake.state().writes());
    assert!(!p.read("how-to/github/README.md").contains("rfluence:"));

    // The upload.
    let o = p.upload(&[]);
    assert!(o.status.success(), "{}\n{}", out(&o), err(&o));
    assert!(out(&o).contains("5 pages created, 0 updated, 0 up to date, 1 folder created."), "{}", out(&o));
    {
        let s = p.fake.state();
        let page = |title: &str| s.by_title("page", title).unwrap_or_else(|| panic!("no page {title}"));
        let readme = page("GitHub How-tos");
        let folder = s.by_title("folder", "actions").expect("folder");
        assert_eq!(readme.parent.as_deref(), Some(ANCESTOR));
        assert_eq!(page("Setting up GitHub").parent.as_deref(), Some(readme.id.as_str()));
        assert_eq!(page("Our GitHub workflow").parent.as_deref(), Some(readme.id.as_str()));
        assert_eq!(folder.parent.as_deref(), Some(readme.id.as_str()));
        assert_eq!(page("Managing secrets").parent.as_deref(), Some(folder.id.as_str()));
        assert!(s.by_title("page", "WIP").is_none());
        // Bodies, with links between the new pages resolved.
        let setup = page("Setting up GitHub");
        let workflow = page("Our GitHub workflow");
        assert!(setup.body.contains(&format!("/pages/{}#Steps", workflow.id)), "{}", setup.body);
        assert_eq!(setup.version, 2, "created empty, then the body");
        // Config labels, and the property recording them.
        assert_eq!(setup.labels, ["github", "ai-generated"]);
        let (_, _, value) = &setup.properties["rfluence"];
        assert_eq!(value["config_labels"], serde_json::json!(["github", "ai-generated"]));
        assert_eq!(value["version"], 2);
        assert_eq!(value["path"], "how-to/github/01-setup.md");
        assert_eq!(folder.properties["rfluence"].2["managed"], true);
    }
    // Each file records its page; the config labels are written back.
    let readme = p.read("how-to/github/README.md");
    assert!(readme.contains(&format!("  parent: \"{ANCESTOR}\"\n  version: 2\n")), "{readme}");
    assert!(readme.contains("  labels: [github, ai-generated]\n"), "{readme}");
    assert!(readme.ends_with("# GitHub How-tos\n\nStart with [setup](./01-setup.md).\n"), "{readme}");

    // Again: nothing to do.
    let writes = p.fake.state().writes().len();
    let o = p.upload(&[]);
    assert!(o.status.success(), "{}", err(&o));
    assert!(out(&o).contains("0 pages created, 0 updated, 5 up to date."), "{}", out(&o));
    assert!(out(&o).contains(&format!("  GitHub How-tos  [page {}]", p.id("how-to/github/README.md"))), "{}", out(&o));
    assert_eq!(p.fake.state().writes().len(), writes, "{:?}", &p.fake.state().writes()[writes..]);

    // An edit: only that page changes.
    p.write("how-to/github/actions/runners.md", &p.read("how-to/github/actions/runners.md").replace("Runners.", "Runners, edited."));
    let o = p.upload(&[]);
    assert!(out(&o).contains("0 pages created, 1 updated, 4 up to date."), "{}", out(&o));
}

#[test]
fn an_ancestor_can_be_a_page_of_the_same_upload() {
    // Entry 1 is under a page entry 2 creates; entries can come in any order.
    let config = "- globs: ['notes/*.md']\n  space_key: ENG\n  ancestor: GitHub How-tos\n- globs: ['how-to/github/*.md']\n  space_key: ENG\n  ancestor: Engineering Docs\n";
    let p = Project::new("ancestor-in-upload", config);
    p.write("notes/one.md", "# Note one\n");
    let o = p.upload(&["--dry-run"]);
    assert!(o.status.success(), "{}", err(&o));
    assert!(out(&o).contains("Plan for entry 1: space ENG, under \"GitHub How-tos\" (from how-to/github/README.md)"), "{}", out(&o));
    let o = p.upload(&[]);
    assert!(o.status.success(), "{}\n{}", out(&o), err(&o));
    let s = p.fake.state();
    let readme = s.by_title("page", "GitHub How-tos").unwrap();
    assert_eq!(s.by_title("page", "Note one").unwrap().parent.as_deref(), Some(readme.id.as_str()));
    assert_eq!(readme.parent.as_deref(), Some(ANCESTOR));
}

#[test]
fn never_creates_a_duplicate_title() {
    let p = Project::new("duplicate", EXAMPLE);
    let existing = p.fake.state().add("page", "Managing secrets", Some(HOMEPAGE));
    let o = p.upload(&[]);
    assert_eq!(o.status.code(), Some(6), "{}", err(&o));
    assert!(
        err(&o).contains(&format!("how-to/github/actions/secrets.md: space ENG already has a page titled \"Managing secrets\" (page {existing})")),
        "{}",
        err(&o)
    );
    assert!(p.fake.state().writes().is_empty(), "nothing is created before the check");
}

#[test]
fn warns_about_pages_moved_in_confluence() {
    let p = Project::new("moved", EXAMPLE);
    assert!(p.upload(&[]).status.success());
    let runners = p.id("how-to/github/actions/runners.md");
    p.fake.state().content.get_mut(&runners).unwrap().parent = Some(HOMEPAGE.into());
    let o = p.upload(&[]);
    assert!(o.status.success(), "{}", err(&o));
    assert!(err(&o).contains("runners.md: warning: the config puts this page under \"actions\""), "{}", err(&o));
    assert!(out(&o).contains("1 page is not where the config puts it (left in place; see the warnings; --move moves them)."), "{}", out(&o));
    assert!(err(&o).contains("it's left where it is; --move moves it"), "{}", err(&o));
    assert_eq!(p.fake.state().content[&runners].parent.as_deref(), Some(HOMEPAGE));

    // --move puts it back: a new version (no body change), recorded in the file.
    let version = p.fake.state().content[&runners].version;
    let o = p.upload(&["--move"]);
    assert!(o.status.success(), "{}", err(&o));
    assert!(out(&o).contains("how-to/github/actions/runners.md  ") && out(&o).contains("moved from under 100"), "{}", out(&o));
    assert!(out(&o).contains("1 moved to another parent"), "{}", out(&o));
    let s = p.fake.state();
    let folder = s.by_title("folder", "actions").unwrap().id.clone();
    assert_eq!(s.content[&runners].parent.as_deref(), Some(folder.as_str()));
    assert_eq!(s.content[&runners].version, version + 1);
    // Moved pages go last under their new parent; the order pass puts it back first.
    assert_eq!(s.children(&folder), ["Self-hosted runners", "Managing secrets"]);
    drop(s);
    let md = p.read("how-to/github/actions/runners.md");
    assert!(md.contains(&format!("  parent: \"{folder}\"\n  version: {}\n", version + 1)), "{md}");
}

#[test]
fn keeps_siblings_in_the_config_order() {
    let p = Project::new("order", EXAMPLE);
    assert!(p.upload(&[]).status.success());
    let readme = p.id("how-to/github/README.md");
    let order = |p: &Project| p.fake.state().children(&readme);
    assert_eq!(order(&p), ["Setting up GitHub", "Our GitHub workflow", "actions"]);

    // Rearranged in Confluence: a warning, and nothing moves without --move.
    let (setup, folder) = (p.id("how-to/github/01-setup.md"), p.fake.state().by_title("folder", "actions").unwrap().id.clone());
    p.fake.state().move_next_to(&setup, true, &folder);
    let o = p.upload(&[]);
    assert!(o.status.success(), "{}", err(&o));
    assert!(err(&o).contains("under \"GitHub How-tos\", \"Setting up GitHub\" is out of the config's order; --move reorders"), "{}", err(&o));
    assert_eq!(order(&p), ["Our GitHub workflow", "actions", "Setting up GitHub"]);
    let o = p.upload(&["--move", "--dry-run"]);
    assert!(out(&o).contains("would move \"Setting up GitHub\" before \"Our GitHub workflow\""), "{}", out(&o));
    assert_eq!(order(&p), ["Our GitHub workflow", "actions", "Setting up GitHub"]);
    let o = p.upload(&["--move"]);
    assert!(out(&o).contains("1 moved into order"), "{}", out(&o));
    assert_eq!(order(&p), ["Setting up GitHub", "Our GitHub workflow", "actions"]);

    // A new file in the middle is put in place, without --move (natural order: 015 is 15,
    // so after 02).
    p.write("how-to/github/015-branches.md", "# Branching\n\nBranches.\n");
    let o = p.upload(&[]);
    assert!(o.status.success(), "{}", err(&o));
    assert_eq!(order(&p), ["Setting up GitHub", "Our GitHub workflow", "Branching", "actions"]);
    assert!(!out(&o).contains("moved into order"), "placing new pages isn't reordering: {}", out(&o));
}

#[test]
fn reports_folder_titles_taken_elsewhere() {
    let p = Project::new("folder-taken", EXAMPLE);
    let other = p.fake.state().add("folder", "actions", Some(HOMEPAGE));
    let o = p.upload(&["--dry-run"]);
    assert_eq!(o.status.code(), Some(2), "{}", err(&o));
    assert!(err(&o).contains(&format!("space ENG already has a folder titled \"actions\" (folder {other}) somewhere else")), "{}", err(&o));
}

#[test]
fn checks_every_file_before_sending_anything() {
    let p = Project::new("preflight", EXAMPLE);
    p.write("how-to/github/02-workflow.md", "# Our GitHub workflow\n\nA footnote[^1].\n\n[^1]: Note.\n");
    p.write("how-to/github/actions/secrets.md", "# Managing secrets\n\n[gone](./missing.md)\n\n![](secrets.png)\n");
    let o = p.upload(&[]);
    assert_eq!(o.status.code(), Some(2), "{}", err(&o));
    for expected in [
        "how-to/github/02-workflow.md:3: error: footnotes can't be represented",
        "files have content Confluence can't store",
        "secrets.md: links to files without a Confluence page: line 3: ./missing.md (no such file)",
        "how-to/github/actions/secrets.md:5: image file not found: secrets.png",
    ] {
        assert!(err(&o).contains(expected), "{expected}\n{}", err(&o));
    }
    // Paths in messages are relative to the project.
    assert!(err(&o).starts_with("how-to/github/02-workflow.md:3: error"), "{}", err(&o));
    assert!(p.fake.state().writes().is_empty());
}

/// Pages and folders rfluence created whose files are gone are orphans: listed, and trashed
/// with --prune.
#[test]
fn prunes_orphans() {
    let p = Project::new("prune", EXAMPLE);
    assert!(p.upload(&[]).status.success());
    let (secrets, runners) = (p.id("how-to/github/actions/secrets.md"), p.id("how-to/github/actions/runners.md"));
    let folder = p.fake.state().by_title("folder", "actions").unwrap().id.clone();
    // A page someone added in Confluence is never an orphan.
    let theirs = p.fake.state().add("page", "Added by a person", Some(ANCESTOR));
    std::fs::remove_dir_all(p.dir.join("how-to/github/actions")).unwrap();

    // Only looked for with --prune or --dry-run (it takes a request per page).
    let o = p.upload(&[]);
    assert!(o.status.success(), "{}", err(&o));
    assert!(!out(&o).contains("Orphans"), "{}", out(&o));
    assert!(!p.fake.state().log.iter().any(|l| l.contains("/properties?key=") && l.contains(&theirs)), "no orphan search");

    let o = p.upload(&["--dry-run"]);
    assert!(o.status.success(), "{}", err(&o));
    let text = out(&o);
    for expected in [
        "Orphans (rfluence created them; their files or directories are gone):",
        &format!("  page {runners} \"Self-hosted runners\" (was how-to/github/actions/runners.md)"),
        &format!("  page {secrets} \"Managing secrets\" (was how-to/github/actions/secrets.md)"),
        &format!("  folder {folder} \"actions\" (was how-to/github/actions)"),
        "3 orphans (--prune trashes them).",
    ] {
        assert!(text.contains(expected), "{expected}\n{text}");
    }
    assert!(!text.contains("Added by a person"), "{text}");

    let o = p.upload(&["--prune", "--dry-run"]);
    assert!(out(&o).contains(&format!("  would trash folder {folder} \"actions\"")) && out(&o).contains("3 to trash"), "{}", out(&o));
    assert!(p.fake.state().content.contains_key(&folder));

    let o = p.upload(&["--prune"]);
    assert!(o.status.success(), "{}", err(&o));
    assert!(out(&o).contains("3 trashed"), "{}", out(&o));
    let s = p.fake.state();
    for id in [&runners, &secrets, &folder] {
        assert!(s.trashed.contains_key(id), "{id} trashed");
    }
    assert!(s.content.contains_key(&theirs));
}

#[test]
fn keeps_orphans_that_were_edited_or_hold_other_pages() {
    let p = Project::new("prune-keep", EXAMPLE);
    assert!(p.upload(&[]).status.success());
    let secrets = p.id("how-to/github/actions/secrets.md");
    let folder = p.fake.state().by_title("folder", "actions").unwrap().id.clone();
    std::fs::remove_dir_all(p.dir.join("how-to/github/actions")).unwrap();
    // Edited in Confluence since rfluence uploaded it.
    p.fake.state().content.get_mut(&secrets).unwrap().version += 1;
    // And a page someone put in the folder.
    p.fake.state().add("page", "Someone's page", Some(&folder));

    let o = p.upload(&["--prune"]);
    assert!(o.status.success(), "{}", err(&o));
    let text = out(&o);
    assert!(text.contains(&format!("  kept page {secrets} \"Managing secrets\" (was how-to/github/actions/secrets.md): was edited in Confluence since rfluence uploaded it (version 2 -> 3)")), "{text}");
    assert!(text.contains(&format!("  kept folder {folder} \"actions\" (was how-to/github/actions): has pages or folders that stay under it (\"Managing secrets\", \"Someone's page\")")), "{text}");
    assert!(text.contains("1 trashed") && text.contains("2 orphans kept"), "{text}");

    // --force trashes the edited page; the folder still holds someone's page.
    let o = p.upload(&["--prune", "--force"]);
    assert!(p.fake.state().trashed.contains_key(&secrets), "{}", out(&o));
    assert!(p.fake.state().content.contains_key(&folder));
}

/// A renamed directory: its new folder is created, --move moves the pages into it, and
/// --prune trashes the old (now empty) folder.
#[test]
fn follows_a_renamed_directory() {
    let p = Project::new("rename-dir", EXAMPLE);
    assert!(p.upload(&[]).status.success());
    let old = p.fake.state().by_title("folder", "actions").unwrap().id.clone();
    std::fs::rename(p.dir.join("how-to/github/actions"), p.dir.join("how-to/github/ci")).unwrap();
    let o = p.upload(&["--move", "--prune"]);
    assert!(o.status.success(), "{}\n{}", out(&o), err(&o));
    assert!(out(&o).contains("2 moved to another parent") && out(&o).contains("1 trashed"), "{}", out(&o));
    let s = p.fake.state();
    let new = s.by_title("folder", "ci").expect("the new folder").id.clone();
    assert_eq!(s.children(&new), ["Self-hosted runners", "Managing secrets"]);
    assert!(s.trashed.contains_key(&old));
}

/// Labels are only added, unless --prune-labels: then labels the file doesn't list go, and
/// so do labels taken out of the config (though the file lists them from the last upload).
#[test]
fn prunes_labels_only_when_asked() {
    let p = Project::new("prune-labels", EXAMPLE);
    assert!(p.upload(&[]).status.success());
    let setup = p.id("how-to/github/01-setup.md");
    p.write(".rfluence.yaml", &EXAMPLE.replace("[github, AI Generated]", "[github]"));
    p.fake.state().content.get_mut(&setup).unwrap().labels.push("extra".into());

    let o = p.upload(&[]);
    assert!(o.status.success(), "{}", err(&o));
    assert_eq!(p.fake.state().content[&setup].labels, ["github", "ai-generated", "extra"]);
    // The write-back lists every label the page has.
    assert!(p.read("how-to/github/01-setup.md").contains("  labels: [github, ai-generated, extra]\n"));

    // The file drops "extra"; the config dropped "ai-generated".
    p.write("how-to/github/01-setup.md", &p.read("how-to/github/01-setup.md").replace("[github, ai-generated, extra]", "[github, ai-generated]"));
    let o = p.upload(&["--prune-labels"]);
    assert!(o.status.success(), "{}", err(&o));
    assert!(out(&o).contains("how-to/github/01-setup.md") && out(&o).contains("labels -ai-generated -extra"), "{}", out(&o));
    assert_eq!(p.fake.state().content[&setup].labels, ["github"]);
    assert!(p.read("how-to/github/01-setup.md").contains("  labels: [github]\n"));
    let readme = p.id("how-to/github/README.md");
    assert_eq!(p.fake.state().content[&readme].labels, ["github"]);
    assert_eq!(p.fake.state().content[&readme].properties["rfluence"].2["config_labels"], serde_json::json!(["github"]));
}
