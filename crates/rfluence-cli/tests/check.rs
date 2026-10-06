//! `rfluence check` on the markdown corpus.

use std::path::PathBuf;
use std::process::{Command, Output};

fn corpus(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/markdown")
        .join(name)
}

fn rf(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rfluence"))
        .args(args)
        .output()
        .unwrap()
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

#[test]
fn clean_file_is_ok() {
    let out = rf(&["check", corpus("runbook.md").to_str().unwrap()]);
    assert!(out.status.success());
    assert_eq!(stdout(&out), "ok: 1 file checked\n");
}

#[test]
fn warnings_dont_fail() {
    let out = rf(&["check", corpus("approximated.md").to_str().unwrap()]);
    assert!(out.status.success(), "{}", stdout(&out));
    let text = stdout(&out);
    assert!(
        text.contains("approximated.md:5: warning: `<kbd>` written as inline code"),
        "{text}"
    );
    assert!(text.ends_with("0 errors, 13 warnings\n"), "{text}");
}

#[test]
fn errors_exit_1() {
    let out = rf(&["check", corpus("unsupported.md").to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(1));
    let text = stdout(&out);
    assert!(
        text.contains(": error: footnotes can't be represented"),
        "{text}"
    );
}

#[test]
fn missing_image_file_is_a_warning() {
    let dir = std::env::temp_dir().join(format!("rf-check-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let md = dir.join("page.md");
    std::fs::write(&md, "# Page\n\n![x](page.assets/missing.png)\n").unwrap();
    let out = rf(&["check", md.to_str().unwrap()]);
    std::fs::remove_dir_all(&dir).unwrap();
    assert!(out.status.success());
    assert!(
        stdout(&out).contains(":3: warning: image file not found: page.assets/missing.png"),
        "{}",
        stdout(&out)
    );
}

#[test]
fn json_output() {
    let out = rf(&[
        "check",
        "--json",
        corpus("unsupported.md").to_str().unwrap(),
    ]);
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let first = &v.as_array().unwrap()[0];
    assert!(first["path"].as_str().unwrap().ends_with("unsupported.md"));
    assert_eq!(first["severity"], "error");
    assert!(first["line"].is_u64() && first["message"].is_string());
}

#[test]
fn unreadable_file_exits_2() {
    let out = rf(&["check", "/nonexistent/page.md"]);
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn warnings_can_fail_too() {
    let approximated = corpus("approximated.md");
    let out = rf(&[
        "check",
        "--warnings-are-errors",
        approximated.to_str().unwrap(),
    ]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stdout(&out).ends_with("0 errors, 13 warnings\n"),
        "{}",
        stdout(&out)
    );
    // Clean files still pass, and errors still fail.
    assert!(
        rf(&[
            "check",
            "--warnings-are-errors",
            corpus("runbook.md").to_str().unwrap()
        ])
        .status
        .success()
    );
    assert_eq!(
        rf(&[
            "check",
            "--warnings-are-errors",
            corpus("unsupported.md").to_str().unwrap()
        ])
        .status
        .code(),
        Some(1)
    );
}
