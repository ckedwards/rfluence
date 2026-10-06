//! `rfluence auth` with sites that are mock servers. Every test uses its own config directory
//! and no keyring, so the developer's real login is never touched.

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

    fn command(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_rfluence"));
        cmd.args(args)
            .env("RFLUENCE_CONFIG_DIR", &self.dir)
            .env("RFLUENCE_NO_KEYRING", "1")
            .env_remove("RFLUENCE_SITE")
            .env_remove("CONFLUENCE_BASE_URL")
            .env_remove("CONFLUENCE_EMAIL")
            .env_remove("CONFLUENCE_API_KEY");
        cmd
    }

    fn run(mut cmd: Command, stdin: &str) -> Output {
        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(stdin.as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    }

    /// Run `rfluence` with only this test's config (no CONFLUENCE_* variables, no keyring).
    fn rf(&self, args: &[&str], stdin: Option<&str>) -> Output {
        Env::run(self.command(args), stdin.unwrap_or(""))
    }

    /// Run `rfluence` answering its prompts from `answers` (one per line), with a git config
    /// whose global email is git@example.com.
    fn prompted(&self, args: &[&str], answers: &str) -> Output {
        let home = self.dir.join("home");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(
            home.join(".gitconfig"),
            "[user]\n\temail = git@example.com\n",
        )
        .unwrap();
        let mut cmd = self.command(args);
        cmd.env("RFLUENCE_PROMPTS_FROM_STDIN", "1")
            .env("HOME", &home)
            .env("XDG_CONFIG_HOME", home.join(".config"))
            .env("GIT_CONFIG_NOSYSTEM", "1");
        Env::run(cmd, answers)
    }

    fn login(&self, email: &str, token: &str) -> Output {
        self.rf(
            &["auth", "login", "--email", email, "--with-token"],
            Some(token),
        )
    }

    fn set_org(&self, site: &mockito::ServerGuard) -> Output {
        self.rf(&["config", "set", "default-site", &site.url()], None)
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

fn basic(email: &str, token: &str) -> String {
    use base64::Engine;
    format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(format!("{email}:{token}"))
    )
}

/// A mock site that accepts these (user, email, token)s and serves one page (titled after the
/// site's first user).
fn site_for(users: &[(&str, &str, &str)]) -> mockito::ServerGuard {
    let mut server = mockito::Server::new();
    let mut accepted = Vec::new();
    for (user, email, token) in users {
        let auth = basic(email, token);
        server
            .mock("GET", "/wiki/rest/api/user/current")
            .match_header("authorization", auth.as_str())
            .with_body(format!(r#"{{"displayName":"{user}"}}"#))
            .create();
        accepted.push(auth);
    }
    // Any other credentials are rejected.
    let good = accepted.clone();
    server
        .mock("GET", "/wiki/rest/api/user/current")
        .match_request(move |req| {
            req.header("authorization")
                .iter()
                .all(|v| !good.iter().any(|g| v.to_str().ok() == Some(g.as_str())))
        })
        .with_status(401)
        .with_body("")
        .create();
    let any_user = || Matcher::AnyOf(accepted.iter().map(|a| Matcher::Exact(a.clone())).collect());
    let page = format!(
        r#"{{"id":"1","title":"{}'s page","parentId":null,"version":{{"number":1}},"labels":{{"results":[]}},"body":{{"atlas_doc_format":{{"value":"{{\"type\":\"doc\",\"content\":[]}}"}}}},"_links":{{"webui":"/spaces/S/pages/1","base":"{}/wiki"}}}}"#,
        users[0].0,
        server.url()
    );
    server
        .mock("GET", "/wiki/api/v2/pages/1")
        .match_query(Matcher::Any)
        .match_header("authorization", any_user())
        .with_body(page)
        .create();
    server
        .mock("GET", "/wiki/api/v2/pages/1/attachments")
        .match_query(Matcher::Any)
        .match_header("authorization", any_user())
        .with_body(r#"{"results":[]}"#)
        .create();
    server
}

fn site(user: &str, email: &str, token: &str) -> mockito::ServerGuard {
    site_for(&[(user, email, token)])
}

fn host(server: &mockito::ServerGuard) -> String {
    server.url().trim_start_matches("http://").to_string()
}

/// Login is the email and token; the default site says which site commands use. The token
/// works on any site the account is on.
#[test]
fn login_then_set_the_default_site() {
    let env = Env::new("org");
    let a = site("Alice", "alice@example.com", "token-a");
    let b = site("Alice on B", "alice@example.com", "token-a");

    // No default site yet: the token is saved, and checked once there is one.
    let login = env.login("alice@example.com", "token-a\n");
    assert!(login.status.success(), "{}", err(&login));
    assert!(
        err(&login).contains("Logged in as alice@example.com. No system keyring: token saved in "),
        "{}",
        err(&login)
    );
    assert!(
        err(&login)
            .contains("Set your Confluence site with `rfluence config set default-site <site>`"),
        "{}",
        err(&login)
    );
    let nowhere = env.rf(&["fetch", "1"], None);
    assert_eq!(nowhere.status.code(), Some(2));
    assert!(err(&nowhere).contains("which Confluence site? pass --site, or set a default: `rfluence config set default-site <site>`"), "{}", err(&nowhere));

    let set = env.set_org(&a);
    assert!(set.status.success(), "{}", err(&set));
    assert_eq!(
        err(&set),
        format!(
            "Default site: {}. Logged in there as Alice (alice@example.com).\n",
            host(&a)
        )
    );
    assert_eq!(
        out(&env.rf(&["config", "get", "default-site"], None)),
        format!("{}\n", host(&a))
    );

    // Page IDs go to the default site; a page URL to its own site.
    assert!(out(&env.rf(&["fetch", "1"], None)).contains("# Alice's page"));
    assert!(
        out(&env.rf(
            &["fetch", &format!("{}/wiki/spaces/S/pages/1", b.url())],
            None
        ))
        .contains("# Alice on B's page")
    );
    assert!(
        out(&env.rf(&["fetch", "1", "--site", &b.url()], None)).contains("# Alice on B's page")
    );

    let s = out(&env.rf(&["auth", "status"], None));
    assert_eq!(
        s,
        format!(
            "alice@example.com (token: token file)\n  ✓ {} (default site): logged in as Alice\n",
            host(&a)
        )
    );
    let s = out(&env.rf(&["auth", "status", "--site", &b.url()], None));
    assert!(
        s.contains(&format!("  ✓ {}: logged in as Alice on B\n", host(&b))),
        "{s}"
    );
    assert_eq!(out(&env.rf(&["auth", "token"], None)), "token-a\n");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(env.dir.join("token"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }

    // Logging out forgets the account; the default site stays a setting.
    let logout = env.rf(&["auth", "logout"], None);
    assert_eq!(
        err(&logout),
        "Logged out: removed the token for alice@example.com.\n"
    );
    assert!(!env.dir.join("token").exists());
    let after = env.rf(&["fetch", "1"], None);
    assert_eq!(after.status.code(), Some(2));
    assert!(
        err(&after).contains("not logged in: run `rfluence auth login`"),
        "{}",
        err(&after)
    );
    assert!(
        env.rf(&["config", "unset", "default-site"], None)
            .status
            .success()
    );
    assert_eq!(
        env.rf(&["config", "get", "default-site"], None)
            .status
            .code(),
        Some(3)
    );
}

/// With a default site, login checks the token there; a rejected one isn't saved.
#[test]
fn login_checks_the_token_on_the_default_site() {
    let env = Env::new("checked");
    let a = site("Alice", "alice@example.com", "token-a");
    assert!(env.set_org(&a).status.success());
    let login = env.login("alice@example.com", "wrong");
    assert_eq!(login.status.code(), Some(4));
    assert!(err(&login).contains("Not logged in"));
    assert_eq!(env.rf(&["auth", "status"], None).status.code(), Some(2));
    let login = env.login("alice@example.com", "token-a");
    assert!(
        err(&login).contains(&format!(
            "Logged in as Alice (alice@example.com) on {}.",
            host(&a)
        )),
        "{}",
        err(&login)
    );
}

#[test]
fn config_knows_its_settings() {
    let env = Env::new("config-keys");
    let o = env.rf(&["config", "set", "colour", "blue"], None);
    assert_eq!(o.status.code(), Some(2));
    assert!(
        err(&o).contains("unknown setting \"colour\" (settings: default-site)"),
        "{}",
        err(&o)
    );
    // A name means its Atlassian Cloud site.
    let o = env.rf(&["config", "set", "default-site", "tech-accounts11"], None);
    assert_eq!(
        err(&o),
        "Added .atlassian.net: \"tech-accounts11\" is tech-accounts11.atlassian.net.\nDefault site: tech-accounts11.atlassian.net. Log in with `rfluence auth login`.\n"
    );
    assert_eq!(
        out(&env.rf(&["config", "get", "default-site"], None)),
        "tech-accounts11.atlassian.net\n"
    );
    // The full host is stored as it is, without the notice.
    let o = env.rf(
        &[
            "config",
            "set",
            "default-site",
            "tech-accounts11.atlassian.net",
        ],
        None,
    );
    assert_eq!(
        err(&o),
        "Default site: tech-accounts11.atlassian.net. Log in with `rfluence auth login`.\n"
    );
    let stored: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(env.dir.join("config.json")).unwrap())
            .unwrap();
    assert_eq!(
        stored,
        serde_json::json!({ "default-site": "https://tech-accounts11.atlassian.net" })
    );
}

#[test]
fn login_needs_options_without_a_terminal() {
    let env = Env::new("no-tty");
    let o = env.rf(&["auth", "login"], None);
    assert_eq!(o.status.code(), Some(2));
    assert!(
        err(&o).contains("is required (pass it as an option)"),
        "{}",
        err(&o)
    );
}

#[test]
fn environment_credentials_are_for_their_own_site() {
    let env = Env::new("env-site");
    let a = site("Alice", "alice@example.com", "token-a");
    let b = site("Bob", "bob@example.com", "token-b");
    assert!(env.set_org(&a).status.success());
    assert!(env.login("alice@example.com", "token-a").status.success());
    let run = |args: &[&str]| {
        let mut cmd = env.command(args);
        cmd.env("CONFLUENCE_BASE_URL", b.url())
            .env("CONFLUENCE_EMAIL", "bob@example.com")
            .env("CONFLUENCE_API_KEY", "token-b");
        Env::run(cmd, "")
    };
    // No site named: the environment's site.
    assert_eq!(out(&run(&["auth", "token"])), "token-b\n");
    // Another site: the saved account.
    assert_eq!(
        out(&run(&["auth", "token", "--site", &a.url()])),
        "token-a\n"
    );
    let s = out(&run(&["auth", "status"]));
    assert!(
        s.contains("bob@example.com (token: CONFLUENCE_* environment variables)")
            && s.contains("alice@example.com (token: token file)"),
        "{s}"
    );
}

/// Without options, login asks for the email (suggesting git's, then the saved one) and the
/// token, again when it isn't accepted, up to 5 times.
#[test]
fn login_asks_for_the_email_and_token() {
    let env = Env::new("prompts");
    let a = site("Alice", "git@example.com", "token-a");
    assert!(env.set_org(&a).status.success());
    let o = env.prompted(&["auth", "login"], "\nwrong-1\nwrong-2\ntoken-a\n");
    assert!(o.status.success(), "{}", err(&o));
    let text = err(&o);
    assert!(text.starts_with("Atlassian account email (git@example.com): Create an API token at https://id.atlassian.com/manage-profile/security/api-tokens\n"), "{text}");
    assert!(
        text.contains(&format!(
            "That token isn't accepted for git@example.com on {}. Try again (1 of 5 tries used).",
            host(&a)
        )),
        "{text}"
    );
    assert!(text.contains("Try again (2 of 5 tries used)."), "{text}");
    assert!(
        text.contains(&format!(
            "Logged in as Alice (git@example.com) on {}.",
            host(&a)
        )),
        "{text}"
    );

    // Again: the saved email is suggested.
    let o = env.prompted(&["auth", "login"], "\ntoken-a\n");
    assert!(
        err(&o).starts_with("Atlassian account email (git@example.com): "),
        "{}",
        err(&o)
    );

    // Five wrong tokens: give up, nothing saved.
    let env = Env::new("prompts-fail");
    assert!(env.set_org(&a).status.success());
    let o = env.prompted(&["auth", "login"], "\nw1\nw2\nw3\nw4\nw5\n");
    assert_eq!(o.status.code(), Some(4), "{}", err(&o));
    assert!(
        err(&o).ends_with("rfluence: failed to authenticate after 5 tries, exiting\n"),
        "{}",
        err(&o)
    );
    assert_eq!(
        env.rf(&["auth", "status"], None).status.code(),
        Some(2),
        "nothing saved"
    );
}

/// Logging in without a default site: it's needed to check the token, so login asks for it
/// (and keeps it as the default).
#[test]
fn login_asks_for_the_default_site_when_there_is_none() {
    let env = Env::new("ask-site");
    let a = site("Alice", "git@example.com", "token-a");
    let o = env.prompted(&["auth", "login"], &format!("{}\n\ntoken-a\n", a.url()));
    assert!(o.status.success(), "{}", err(&o));
    let text = err(&o);
    assert!(
        text.starts_with(
            "No default site is set yet. rfluence needs one to check your token.\nConfluence site (e.g. example, or example.atlassian.net): "
        ),
        "{text}"
    );
    assert!(
        text.contains(&format!("Default site: {} (change it with `rfluence config set default-site`).\nAtlassian account email (git@example.com): ", host(&a))),
        "{text}"
    );
    assert!(
        text.contains(&format!(
            "Logged in as Alice (git@example.com) on {}.",
            host(&a)
        )),
        "{text}"
    );
    assert_eq!(
        out(&env.rf(&["config", "get", "default-site"], None)),
        format!("{}\n", host(&a))
    );
}
