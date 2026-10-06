//! `rfluence` against the real test site (fixtures/confluence/README.md). Skipped unless
//! `RFLUENCE_LIVE=1`; credentials come from the environment or the repo's .env. The upload
//! test creates a page in the test space and trashes it at the end.
//!
//!     RFLUENCE_LIVE=1 cargo test -p rfluence-cli --test live -- --nocapture

use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

fn live() -> bool {
    std::env::var_os("RFLUENCE_LIVE").is_some()
}

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Credentials from .env, unless they're in the environment.
fn dotenv() -> Vec<(String, String)> {
    if std::env::var_os("CONFLUENCE_API_KEY").is_some() {
        return Vec::new();
    }
    let text = std::fs::read_to_string(root().join(".env")).unwrap_or_default();
    text.lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| {
            (
                k.trim().to_string(),
                v.trim().trim_matches('"').trim_matches('\'').to_string(),
            )
        })
        .collect()
}

/// `rfluence` with credentials from the environment, or from .env.
fn rf(args: &[&str]) -> (std::process::Output, Duration) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rfluence"));
    cmd.args(args).envs(dotenv());
    let start = Instant::now();
    let out = cmd.output().unwrap();
    (out, start.elapsed())
}

fn fixture(name: &str, file: &str) -> String {
    std::fs::read_to_string(root().join("fixtures/confluence").join(name).join(file)).unwrap()
}

#[test]
fn fetches_reference_pages_like_the_fixtures() {
    if !live() {
        return;
    }
    for (name, id, assets) in [
        ("adf-reference", "458790", "rfluence-adf-reference.assets"),
        ("emoji", "426008", "rfluence-emoji-api-test.assets"),
        ("space-homepage", "295257", "software-development.assets"),
    ] {
        let (out, _) = rf(&["fetch", id]);
        assert!(
            out.status.success(),
            "{name}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let expected = fixture(name, "page.md").replace("](attachments/", &format!("]({assets}/"));
        similar_asserts::assert_eq!(String::from_utf8(out.stdout).unwrap(), expected, "{name}");
    }
}

#[test]
fn times_fetch_to_stdout() {
    if !live() {
        return;
    }
    // A page with images (two requests in parallel) and one without.
    for (name, id) in [("adf-reference", "458790"), ("emoji", "426008")] {
        let mut times: Vec<Duration> = (0..5).map(|_| rf(&["fetch", id]).1).collect();
        times.sort();
        eprintln!(
            "rfluence fetch {id} ({name}): min {:?}, median {:?}, max {:?}",
            times[0],
            times[times.len() / 2],
            times[times.len() - 1]
        );
    }
}

#[test]
fn searches_the_test_space() {
    if !live() {
        return;
    }
    let (out, took) = rf(&["search", "emoji", "--space", "rfluencete"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("426008  rfluence emoji API test\n"), "{text}");
    assert!(
        text.contains("720904  rfluence emoji names API test\n"),
        "{text}"
    );
    eprintln!("rfluence search emoji: {took:?}");
}

/// Create a page from a new file, edit and upload it again, and check that fetching it gives
/// the file back. Then trash the page.
#[test]
fn creates_and_updates_a_page() {
    if !live() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("rfluence-live-upload-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("page.assets")).unwrap();
    let image = root().join("fixtures/confluence/images-api/attachments/green.png");
    std::fs::copy(&image, dir.join("page.assets/green.png")).unwrap();
    // The reference page, as a linked project file.
    std::fs::write(dir.join("reference.md"), "---\nrfluence:\n  id: \"458790\"\n  space_key: rfluencete\n---\n\n# rfluence ADF reference\n\n## Code blocks\n").unwrap();
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let title = format!("rfluence live upload test {stamp} (temporary)");
    let page = dir.join("page.md");
    let file = page.to_str().unwrap();
    std::fs::write(
        &page,
        format!("---\nrfluence:\n  space_key: rfluencete\n  labels: [Live Test]\n---\n\n# {title}\n\n## Setup\n\nSee [code blocks](./reference.md#code-blocks) and [setup](#setup).\n\n![a green box](page.assets/green.png)\n\n```mermaid\nflowchart LR\n  A --> B\n```\n"),
    )
    .unwrap();

    let (out, took) = rf(&["upload", file]);
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    eprintln!("rfluence upload (create, with an image and a label): {took:?}\n{stdout}");
    let md = std::fs::read_to_string(&page).unwrap();
    let fields = rfluence_convert::frontmatter::rfluence_fields(
        rfluence_convert::frontmatter::split(&md).yaml.unwrap(),
    );
    let id = fields.id.clone().expect("the file has the new page's ID");
    let result = std::panic::catch_unwind(|| {
        assert_eq!(
            (fields.version, fields.labels.as_slice()),
            (Some(2), ["live-test".to_string()].as_slice()),
            "{md}"
        );

        // Fetching the page gives the file back.
        let (out, _) = rf(&["fetch", &id, "-o", file]);
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            String::from_utf8_lossy(&out.stderr).contains("is up to date"),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );

        // An edit, uploaded.
        std::fs::write(&page, md.replace("See [code", "Edited. See [code")).unwrap();
        let (out, took) = rf(&["upload", file]);
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            String::from_utf8_lossy(&out.stdout).contains("(version 2 -> 3)"),
            "{}",
            String::from_utf8_lossy(&out.stdout)
        );
        eprintln!("rfluence upload (an edit): {took:?}");
        let (out, took) = rf(&["upload", file]);
        assert!(
            String::from_utf8_lossy(&out.stdout).contains("is up to date (version 3)"),
            "{}",
            String::from_utf8_lossy(&out.stdout)
        );
        eprintln!("rfluence upload (unchanged, with an image): {took:?}");

        // A copy without the ID would be a duplicate.
        let copy = dir.join("copy.md");
        std::fs::write(
            &copy,
            format!("---\nrfluence:\n  space_key: rfluencete\n---\n\n# {title}\n"),
        )
        .unwrap();
        let (out, _) = rf(&["upload", copy.to_str().unwrap()]);
        assert_eq!(
            out.status.code(),
            Some(6),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    });

    let env: std::collections::HashMap<String, String> = dotenv().into_iter().collect();
    let creds =
        rfluence_client::auth::from_env(|k| std::env::var(k).ok().or_else(|| env.get(k).cloned()))
            .unwrap()
            .unwrap();
    rfluence_client::Client::new(&creds)
        .trash_page(&id)
        .unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    if let Err(e) = result {
        std::panic::resume_unwind(e);
    }
}

/// `upload --config`: a small tree (an index page, two pages, a folder) under a temporary
/// ancestor, uploaded twice (the second time nothing changes). Then everything is trashed.
#[test]
fn uploads_a_page_tree() {
    if !live() {
        return;
    }
    let env: std::collections::HashMap<String, String> = dotenv().into_iter().collect();
    let creds =
        rfluence_client::auth::from_env(|k| std::env::var(k).ok().or_else(|| env.get(k).cloned()))
            .unwrap()
            .unwrap();
    let client = rfluence_client::Client::new(&creds);
    let space = client.space("rfluencete").unwrap();
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let empty = rfluence_convert::adf::Node::doc(Vec::new());
    let ancestor = client
        .create_page(
            &space.id,
            space.homepage_id.as_deref().unwrap(),
            &format!("rfluence tree test {stamp} (temporary)"),
            &empty,
        )
        .unwrap();

    let dir = std::env::temp_dir().join(format!("rfluence-live-tree-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let write = |file: &str, text: &str| {
        let path = dir.join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    };
    write(
        ".rfluence.yaml",
        &format!(
            "- globs: ['docs/**/*.md']\n  space_key: rfluencete\n  ancestor_id: \"{}\"\n  labels: [live-tree]\n  folder_title: '{{dir}} {stamp} (temporary)'\n",
            ancestor.id
        ),
    );
    write(
        "docs/README.md",
        &format!("# Tree index {stamp} (temporary)\n\nStart with [setup](./setup.md).\n"),
    );
    write(
        "docs/setup.md",
        &format!("# Tree setup {stamp} (temporary)\n\nSee [runners](./actions/runners.md).\n"),
    );
    write(
        "docs/actions/runners.md",
        &format!("# Tree runners {stamp} (temporary)\n\nRunners.\n"),
    );

    let run = || {
        let start = Instant::now();
        let out = Command::new(env!("CARGO_BIN_EXE_rfluence"))
            .args(["upload", "--config"])
            .current_dir(&dir)
            .envs(dotenv())
            .output()
            .unwrap();
        (out, start.elapsed())
    };
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let (out, took) = run();
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        assert!(
            out.status.success(),
            "{stdout}{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            stdout.contains("3 pages created, 0 updated, 0 up to date, 1 folder created."),
            "{stdout}"
        );
        eprintln!("rfluence upload --config (3 new pages, 1 new folder): {took:?}");
        let (out, took) = run();
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        assert!(
            stdout.contains("0 pages created, 0 updated, 3 up to date."),
            "{stdout}"
        );
        eprintln!("rfluence upload --config (3 pages, up to date): {took:?}");

        // Rearranged in Confluence: setup before the folder (the config's order is by name:
        // actions/, then setup.md), runners moved out of the folder.
        let id = |file: &str| {
            let md = std::fs::read_to_string(dir.join(file)).unwrap();
            rfluence_convert::frontmatter::rfluence_fields(
                rfluence_convert::frontmatter::split(&md).yaml.unwrap(),
            )
            .id
            .unwrap()
        };
        let (readme, setup, runners) = (
            id("docs/README.md"),
            id("docs/setup.md"),
            id("docs/actions/runners.md"),
        );
        let titles = |kind, parent: &str| {
            client
                .children(kind, parent)
                .unwrap()
                .into_iter()
                .map(|c| c.title)
                .collect::<Vec<_>>()
        };
        let folder = client
            .children(rfluence_client::Kind::Page, &readme)
            .unwrap()
            .into_iter()
            .find(|c| c.kind == "folder")
            .unwrap()
            .id;
        client.move_next_to(&setup, false, &folder).unwrap();
        let page = client.page(&runners).unwrap();
        client
            .put_page(
                &runners,
                &page.meta.title,
                None,
                page.meta.version + 1,
                Some(&ancestor.id),
            )
            .unwrap();
        assert_eq!(
            titles(rfluence_client::Kind::Page, &readme),
            [
                format!("Tree setup {stamp} (temporary)"),
                format!("actions {stamp} (temporary)")
            ]
        );

        // Fetching runners' new version first, so the version check passes.
        let o = Command::new(env!("CARGO_BIN_EXE_rfluence"))
            .args([
                "fetch",
                &runners,
                "-o",
                "docs/actions/runners.md",
                "--force",
            ])
            .current_dir(&dir)
            .envs(dotenv())
            .output()
            .unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        let start = Instant::now();
        let out = Command::new(env!("CARGO_BIN_EXE_rfluence"))
            .args(["upload", "--config", "--move"])
            .current_dir(&dir)
            .envs(dotenv())
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        assert!(
            out.status.success(),
            "{stdout}{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            stdout.contains("1 moved to another parent, 1 moved into order."),
            "{stdout}"
        );
        eprintln!(
            "rfluence upload --config --move (a page moved back, a page reordered): {:?}",
            start.elapsed()
        );
        assert_eq!(
            titles(rfluence_client::Kind::Page, &readme),
            [
                format!("actions {stamp} (temporary)"),
                format!("Tree setup {stamp} (temporary)")
            ]
        );
        assert_eq!(
            client.page(&runners).unwrap().meta.parent.as_deref(),
            Some(folder.as_str())
        );
        // The body survived the move without a body.
        assert!(
            rfluence_convert::adf_to_markdown(
                &client.page(&runners).unwrap().adf,
                &Default::default()
            )
            .contains("Runners.")
        );

        // The directory deleted locally, and the config's label taken out: --prune trashes the
        // page and its folder, --prune-labels removes the label.
        std::fs::remove_dir_all(dir.join("docs/actions")).unwrap();
        // (Its link has to go too: upload refuses links to files without a page.)
        let setup_md = std::fs::read_to_string(dir.join("docs/setup.md")).unwrap();
        std::fs::write(
            dir.join("docs/setup.md"),
            setup_md.replace("See [runners](./actions/runners.md).", "No runners."),
        )
        .unwrap();
        let config = std::fs::read_to_string(dir.join(".rfluence.yaml")).unwrap();
        std::fs::write(
            dir.join(".rfluence.yaml"),
            config.replace("labels: [live-tree]", "labels: []"),
        )
        .unwrap();
        let start = Instant::now();
        let out = Command::new(env!("CARGO_BIN_EXE_rfluence"))
            .args(["upload", "--config", "--prune", "--prune-labels"])
            .current_dir(&dir)
            .envs(dotenv())
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        assert!(
            out.status.success(),
            "{stdout}{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(stdout.contains("2 trashed"), "{stdout}");
        eprintln!(
            "rfluence upload --config --prune --prune-labels: {:?}",
            start.elapsed()
        );
        assert!(
            matches!(
                client.content(&runners),
                Err(rfluence_client::Error::NotFound(_))
            ),
            "runners trashed"
        );
        assert!(
            client.page(&setup).unwrap().meta.labels.is_empty(),
            "live-tree removed"
        );
    }));

    // Clean up: every page and folder under the ancestor, then the ancestor.
    let mut stack = vec![(rfluence_client::Kind::Page, ancestor.id.clone())];
    let mut all = Vec::new();
    while let Some((kind, id)) = stack.pop() {
        for c in client.children(kind, &id).unwrap_or_default() {
            let k = if c.kind == "folder" {
                rfluence_client::Kind::Folder
            } else {
                rfluence_client::Kind::Page
            };
            stack.push((k, c.id.clone()));
        }
        all.push((kind, id));
    }
    for (kind, id) in all.into_iter().rev() {
        let _ = client.trash(kind, &id);
    }
    let _ = std::fs::remove_dir_all(&dir);
    if let Err(e) = result {
        std::panic::resume_unwind(e);
    }
}
