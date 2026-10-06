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
        .map(|(k, v)| (k.trim().to_string(), v.trim().trim_matches('"').trim_matches('\'').to_string()))
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
        assert!(out.status.success(), "{name}: {}", String::from_utf8_lossy(&out.stderr));
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
            times[0], times[times.len() / 2], times[times.len() - 1]
        );
    }
}

#[test]
fn searches_the_test_space() {
    if !live() {
        return;
    }
    let (out, took) = rf(&["search", "emoji", "--space", "rfluencete"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("426008  rfluence emoji API test\n"), "{text}");
    assert!(text.contains("720904  rfluence emoji names API test\n"), "{text}");
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
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
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
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    eprintln!("rfluence upload (create, with an image and a label): {took:?}\n{stdout}");
    let md = std::fs::read_to_string(&page).unwrap();
    let fields = rfluence_convert::frontmatter::rfluence_fields(rfluence_convert::frontmatter::split(&md).yaml.unwrap());
    let id = fields.id.clone().expect("the file has the new page's ID");
    let result = std::panic::catch_unwind(|| {
        assert_eq!((fields.version, fields.labels.as_slice()), (Some(2), ["live-test".to_string()].as_slice()), "{md}");

        // Fetching the page gives the file back.
        let (out, _) = rf(&["fetch", &id, "-o", file]);
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        assert!(String::from_utf8_lossy(&out.stderr).contains("is up to date"), "{}", String::from_utf8_lossy(&out.stderr));

        // An edit, uploaded.
        std::fs::write(&page, md.replace("See [code", "Edited. See [code")).unwrap();
        let (out, took) = rf(&["upload", file]);
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        assert!(String::from_utf8_lossy(&out.stdout).contains("(version 2 -> 3)"), "{}", String::from_utf8_lossy(&out.stdout));
        eprintln!("rfluence upload (an edit): {took:?}");
        let (out, took) = rf(&["upload", file]);
        assert!(String::from_utf8_lossy(&out.stdout).contains("is up to date (version 3)"), "{}", String::from_utf8_lossy(&out.stdout));
        eprintln!("rfluence upload (unchanged, with an image): {took:?}");

        // A copy without the ID would be a duplicate.
        let copy = dir.join("copy.md");
        std::fs::write(&copy, format!("---\nrfluence:\n  space_key: rfluencete\n---\n\n# {title}\n")).unwrap();
        let (out, _) = rf(&["upload", copy.to_str().unwrap()]);
        assert_eq!(out.status.code(), Some(6), "{}", String::from_utf8_lossy(&out.stderr));
    });

    let env: std::collections::HashMap<String, String> = dotenv().into_iter().collect();
    let creds = rfluence_client::auth::from_env(|k| std::env::var(k).ok().or_else(|| env.get(k).cloned())).unwrap().unwrap();
    rfluence_client::Client::new(&creds).trash_page(&id).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    if let Err(e) = result {
        std::panic::resume_unwind(e);
    }
}
