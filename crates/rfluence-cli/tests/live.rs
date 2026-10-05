//! `rfluence fetch` against the real test site (fixtures/confluence/README.md). Skipped unless
//! `RFLUENCE_LIVE=1`; credentials come from the environment or the repo's .env.
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

/// `rfluence` with credentials from the environment, or from .env.
fn rf(args: &[&str]) -> (std::process::Output, Duration) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rfluence"));
    cmd.args(args);
    if std::env::var_os("CONFLUENCE_API_KEY").is_none() {
        if let Ok(dotenv) = std::fs::read_to_string(root().join(".env")) {
            for line in dotenv.lines().filter(|l| !l.trim_start().starts_with('#')) {
                if let Some((k, v)) = line.split_once('=') {
                    cmd.env(k.trim(), v.trim().trim_matches('"').trim_matches('\''));
                }
            }
        }
    }
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
