//! `rfluence auth` with two sites, each a mock server. Every test uses its own config
//! directory and no keyring, so the developer's real accounts are never touched.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

use mockito::Matcher;

struct Env {
    dir: PathBuf,
}

impl Env {
    fn new(name: &str) -> Env {
        let dir = std::env::temp_dir().join(format!("rfluence-auth-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Env { dir }
    }

    /// Run `rfluence` with only this test's config (no CONFLUENCE_* variables, no keyring).
    fn rf(&self, args: &[&str], stdin: Option<&str>) -> Output {
        let mut child = Command::new(env!("CARGO_BIN_EXE_rfluence"))
            .args(args)
            .env("RFLUENCE_CONFIG_DIR", &self.dir)
            .env("RFLUENCE_NO_KEYRING", "1")
            .env_remove("RFLUENCE_SITE")
            .env_remove("CONFLUENCE_BASE_URL")
            .env_remove("CONFLUENCE_EMAIL")
            .env_remove("CONFLUENCE_API_KEY")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(stdin.unwrap_or("").as_bytes()).unwrap();
        child.wait_with_output().unwrap()
    }

    fn login(&self, site: &mockito::ServerGuard, email: &str, token: &str) -> Output {
        self.rf(&["auth", "login", "--site", &site.url(), "--email", email, "--with-token"], Some(token))
    }
}

impl Drop for Env {
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

/// A mock site that accepts one email/token pair and serves one page.
fn site(user: &str, email: &str, token: &str) -> mockito::ServerGuard {
    use base64::Engine;
    let mut server = mockito::Server::new();
    let auth = format!("Basic {}", base64::engine::general_purpose::STANDARD.encode(format!("{email}:{token}")));
    server
        .mock("GET", "/wiki/rest/api/user/current")
        .match_header("authorization", auth.as_str())
        .with_body(format!(r#"{{"displayName":"{user}"}}"#))
        .create();
    // Any other credentials are rejected.
    let good = auth.clone();
    server
        .mock("GET", "/wiki/rest/api/user/current")
        .match_request(move |req| req.header("authorization").iter().all(|v| v.to_str().ok() != Some(good.as_str())))
        .with_status(401)
        .with_body("")
        .create();
    let page = format!(
        r#"{{"id":"1","title":"{user}'s page","parentId":null,"version":{{"number":1}},"labels":{{"results":[]}},"body":{{"atlas_doc_format":{{"value":"{{\"type\":\"doc\",\"content\":[]}}"}}}},"_links":{{"webui":"/spaces/S/pages/1","base":"{}/wiki"}}}}"#,
        server.url()
    );
    server.mock("GET", "/wiki/api/v2/pages/1").match_query(Matcher::Any).match_header("authorization", auth.as_str()).with_body(page).create();
    server
        .mock("GET", "/wiki/api/v2/pages/1/attachments")
        .match_query(Matcher::Any)
        .match_header("authorization", auth.as_str())
        .with_body(r#"{"results":[]}"#)
        .create();
    server
}

fn host(server: &mockito::ServerGuard) -> String {
    server.url().trim_start_matches("http://").to_string()
}

#[test]
fn login_status_token_switch_logout_with_two_sites() {
    let env = Env::new("two-sites");
    let a = site("Alice", "alice@example.com", "token-a");
    let b = site("Bob", "bob@example.com", "token-b");

    let login = env.login(&a, "alice@example.com", "token-a\n");
    assert!(login.status.success(), "{}", err(&login));
    assert!(err(&login).contains("Logged in to") && err(&login).contains("as Alice"), "{}", err(&login));
    assert!(env.login(&b, "bob@example.com", "token-b").status.success());

    // The last login is the default; both accounts work.
    let status = env.rf(&["auth", "status"], None);
    assert!(status.status.success(), "{}", out(&status));
    let s = out(&status);
    assert!(s.contains(&format!("{} (default)\n  ✓ Logged in as Bob", host(&b))), "{s}");
    assert!(s.contains(&format!("{}\n  ✓ Logged in as Alice", host(&a))), "{s}");
    assert!(s.contains("Token: token file"), "{s}");

    assert_eq!(out(&env.rf(&["auth", "token"], None)), "token-b\n");
    assert_eq!(out(&env.rf(&["auth", "token", "--site", &a.url()], None)), "token-a\n");

    // Token files are readable only by the user.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let file = env.dir.join("tokens").join(host(&a).replace(':', "_"));
        assert_eq!(std::fs::metadata(file).unwrap().permissions().mode() & 0o777, 0o600);
    }

    // Two sites: switch toggles the default.
    assert!(env.rf(&["auth", "switch"], None).status.success());
    assert_eq!(out(&env.rf(&["auth", "token"], None)), "token-a\n");

    // A page URL uses its own site's account, whatever the default.
    let page_b = env.rf(&["fetch", &format!("{}/wiki/spaces/S/pages/1", b.url())], None);
    assert!(page_b.status.success(), "{}", err(&page_b));
    assert!(out(&page_b).contains("# Bob's page"));
    let page_a = env.rf(&["fetch", "1"], None);
    assert!(out(&page_a).contains("# Alice's page"), "{}", err(&page_a));
    assert!(out(&env.rf(&["fetch", "1", "--site", &b.url()], None)).contains("# Bob's page"));

    // Logging out of the default site makes the other one the default.
    let logout = env.rf(&["auth", "logout"], None);
    assert!(logout.status.success());
    assert!(err(&logout).contains(&format!("{} is now the default site", host(&b))), "{}", err(&logout));
    let after = env.rf(&["fetch", &format!("{}/wiki/spaces/S/pages/1", a.url())], None);
    assert_eq!(after.status.code(), Some(2));
    assert!(err(&after).contains(&format!("not logged in to {}", host(&a))), "{}", err(&after));
}

#[test]
fn rejected_credentials_are_not_saved() {
    let env = Env::new("rejected");
    let a = site("Alice", "alice@example.com", "token-a");
    let login = env.login(&a, "alice@example.com", "wrong");
    assert_eq!(login.status.code(), Some(4));
    assert!(err(&login).contains("Not logged in"));
    assert_eq!(env.rf(&["auth", "status"], None).status.code(), Some(2));
}

#[test]
fn login_needs_options_without_a_terminal() {
    let env = Env::new("no-tty");
    let o = env.rf(&["auth", "login"], None);
    assert_eq!(o.status.code(), Some(2));
    assert!(err(&o).contains("is required (pass it as an option)"), "{}", err(&o));
}

#[test]
fn reads_the_single_account_format() {
    let env = Env::new("legacy");
    let a = site("Alice", "alice@example.com", "token-a");
    std::fs::write(env.dir.join("auth.json"), format!(r#"{{"base_url":"{}","email":"alice@example.com"}}"#, a.url())).unwrap();
    std::fs::write(env.dir.join("token"), "token-a\n").unwrap();
    let status = env.rf(&["auth", "status"], None);
    assert!(out(&status).contains(&format!("{} (default)\n  ✓ Logged in as Alice", host(&a))), "{}", out(&status));
}

#[test]
fn environment_credentials_are_for_their_own_site() {
    let env = Env::new("env-site");
    let a = site("Alice", "alice@example.com", "token-a");
    let b = site("Bob", "bob@example.com", "token-b");
    assert!(env.login(&a, "alice@example.com", "token-a").status.success());
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_rfluence"))
            .args(args)
            .env("RFLUENCE_CONFIG_DIR", &env.dir)
            .env("RFLUENCE_NO_KEYRING", "1")
            .env_remove("RFLUENCE_SITE")
            .env("CONFLUENCE_BASE_URL", b.url())
            .env("CONFLUENCE_EMAIL", "bob@example.com")
            .env("CONFLUENCE_API_KEY", "token-b")
            .output()
            .unwrap()
    };
    // No site named: the environment's site.
    assert_eq!(out(&run(&["auth", "token"])), "token-b\n");
    // Another site: its saved account.
    assert_eq!(out(&run(&["auth", "token", "--site", &a.url()])), "token-a\n");
    let s = out(&run(&["auth", "status"]));
    assert!(s.contains("Token: CONFLUENCE_* environment variables") && s.contains("Token: token file"), "{s}");
}
