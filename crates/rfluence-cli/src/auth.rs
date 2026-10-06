//! `rfluence auth`: log in to Confluence, like `gh auth`. One account: an email and its API
//! token, which works on every site the account is on. Which site commands use by
//! default is a setting: `rfluence config set default-site`. See design.md, "Auth".

use std::io::{BufRead, IsTerminal, Read, Write};
use std::process::ExitCode;

use rfluence_client::Client;
use rfluence_client::auth::{self, Credentials, Source};

use crate::{EXIT_AUTH, EXIT_USAGE, fail};

/// How many times `auth login` asks for a token that isn't accepted.
const TOKEN_TRIES: u32 = 5;

/// Log in: the email (suggesting the saved one, or git's) and the API token. The token is
/// checked on the default site, if one is set.
pub fn login(email: Option<String>, with_token: bool) -> ExitCode {
    let mut site = match auth::default_site() {
        Ok(site) => site,
        Err(e) => return fail(&e),
    };
    // The token is checked on the default site: without one, ask for it (if someone can answer).
    if site.is_none() && interactive() && !with_token {
        eprintln!("No default site is set yet. rfluence needs one to check your token.");
        let Some(answer) = prompt(
            "Confluence site (e.g. example, or example.atlassian.net)",
            None,
        ) else {
            return ExitCode::from(EXIT_USAGE);
        };
        match crate::settings::store_default_site(&answer) {
            Ok(base_url) => {
                eprintln!(
                    "Default site: {} (change it with `rfluence config set default-site`).",
                    auth::host(&base_url)
                );
                site = Some(base_url);
            }
            Err(e) => return fail(&e),
        }
    }
    let saved = auth::account().ok().flatten().map(|a| a.email);
    let suggested = saved.or_else(git_email);
    let Some(email) = email.or_else(|| prompt("Atlassian account email", suggested.as_deref()))
    else {
        return ExitCode::from(EXIT_USAGE);
    };
    if !with_token {
        eprintln!(
            "Create an API token at https://id.atlassian.com/manage-profile/security/api-tokens"
        );
    }
    let (token, user) = match &site {
        Some(base_url) => match ask_until_accepted(&email, base_url, with_token) {
            Ok((token, user)) => (token, Some(user)),
            Err(code) => return code,
        },
        None => match read_token(with_token) {
            Some(token) => (token, None),
            None => return ExitCode::from(EXIT_USAGE),
        },
    };
    let saved = match auth::save_account(&email, &token) {
        Ok(source) => source,
        Err(e) => return fail(&e),
    };
    let saved = match saved {
        Source::Keyring => "Token saved in the system keyring.".to_string(),
        _ => format!(
            "No system keyring: token saved in {}.",
            auth::token_file()
                .map(|p| p.display().to_string())
                .unwrap_or_default()
        ),
    };
    match (user, site) {
        (Some(user), Some(site)) => eprintln!(
            "Logged in as {user} ({email}) on {}. {saved}",
            auth::host(&site)
        ),
        _ => eprintln!(
            "Logged in as {email}. {saved} Set your Confluence site with `rfluence config set default-site <site>` (e.g. `example` for example.atlassian.net); the token is checked there."
        ),
    }
    ExitCode::SUCCESS
}

/// Ask for the token until Confluence accepts it on `base_url` (up to [`TOKEN_TRIES`] times;
/// once with --with-token, which reads it from standard input). Returns it and the user's
/// name.
fn ask_until_accepted(
    email: &str,
    base_url: &str,
    with_token: bool,
) -> Result<(String, String), ExitCode> {
    let host = auth::host(base_url);
    let mut tries = 0;
    loop {
        tries += 1;
        let token = read_token(with_token).ok_or(ExitCode::from(EXIT_USAGE))?;
        let creds = Credentials {
            base_url: base_url.to_string(),
            email: email.to_string(),
            token,
        };
        match Client::new(&creds).current_user() {
            Ok(user) => return Ok((creds.token, user)),
            // A mistyped token: ask again (not with --with-token: no one to ask).
            Err(rfluence_client::Error::Auth(_) | rfluence_client::Error::Forbidden(_))
                if !with_token && tries < TOKEN_TRIES =>
            {
                eprintln!(
                    "That token isn't accepted for {email} on {host}. Try again ({tries} of {TOKEN_TRIES} tries used)."
                );
            }
            Err(rfluence_client::Error::Auth(_) | rfluence_client::Error::Forbidden(_))
                if !with_token =>
            {
                eprintln!("rfluence: failed to authenticate after {TOKEN_TRIES} tries, exiting");
                return Err(ExitCode::from(EXIT_AUTH));
            }
            Err(e) => {
                eprintln!("Not logged in: the credentials didn't work.");
                return Err(fail(&e));
            }
        }
    }
}

/// The token: from standard input with --with-token, else asked for (without echo).
fn read_token(with_token: bool) -> Option<String> {
    if with_token {
        let mut t = String::new();
        if std::io::stdin().read_to_string(&mut t).is_err() || t.trim().is_empty() {
            eprintln!(
                "rfluence: --with-token reads the token from standard input, but there was none"
            );
            return None;
        }
        return Some(t.trim().to_string());
    }
    if !interactive() {
        eprintln!(
            "rfluence: the API token is required: pass --with-token and give it on standard input"
        );
        return None;
    }
    let read = if prompts_from_stdin() {
        read_line().ok_or_else(|| std::io::Error::other("no input"))
    } else {
        rpassword::prompt_password("API token: ")
    };
    match read {
        Ok(t) if !t.trim().is_empty() => Some(t.trim().to_string()),
        Ok(_) => {
            eprintln!("rfluence: no token entered");
            None
        }
        Err(e) => {
            eprintln!(
                "rfluence: can't read the token ({e}); use --with-token to read it from standard input"
            );
            None
        }
    }
}

/// Forget the account: its email and token.
pub fn logout() -> ExitCode {
    match auth::logout() {
        Ok(Some(email)) => {
            eprintln!("Logged out: removed the token for {email}.");
            ExitCode::SUCCESS
        }
        Ok(None) => fail(&rfluence_client::Error::NotConfigured),
        Err(e) => fail(&e),
    }
}

/// The account, where its token is, and whether Confluence accepts it on the default site (or
/// `--site`); also the `CONFLUENCE_*` variables' account, if set.
pub fn status(site: Option<String>) -> ExitCode {
    let wanted = match site.map(|s| auth::site_url(&s)).transpose() {
        Ok(w) => w,
        Err(e) => return fail(&e),
    };
    let (env, account, site) = match (
        auth::from_env(|k| std::env::var(k).ok()),
        auth::account(),
        auth::default_site(),
    ) {
        (Ok(env), Ok(account), Ok(site)) => (env, account, site),
        (Err(e), ..) | (_, Err(e), _) | (.., Err(e)) => return fail(&e),
    };
    if env.is_none() && account.is_none() {
        return fail(&rfluence_client::Error::NotConfigured);
    }
    let mut ok = true;
    let mut check = |label: String, creds: Option<Credentials>| match creds {
        None => {
            ok = false;
            println!("  ✗ {label}: no token saved (run `rfluence auth login`)");
        }
        Some(creds) => match Client::new(&creds).current_user() {
            Ok(user) => println!("  ✓ {label}: logged in as {user}"),
            Err(e) => {
                ok = false;
                println!("  ✗ {label}: {e}");
            }
        },
    };
    if let Some(creds) = env.filter(|c| wanted.as_ref().is_none_or(|w| auth::host(w) == c.host())) {
        println!("{} (token: {})", creds.email, Source::Env);
        check(creds.host(), Some(creds));
    }
    if let Some(account) = account {
        let source = account
            .source
            .map_or_else(|| "no token found".to_string(), |s| format!("token: {s}"));
        println!("{} ({source})", account.email);
        let target = match (&wanted, &site) {
            (Some(w), _) => Some((auth::host(w), w.clone())),
            (None, Some(o)) => Some((format!("{} (default site)", auth::host(o)), o.clone())),
            (None, None) => None,
        };
        match target {
            Some((label, base_url)) => {
                let token = auth::saved_token().map(|t| t.0);
                check(
                    label,
                    token.map(|token| Credentials {
                        base_url,
                        email: account.email.clone(),
                        token,
                    }),
                );
            }
            None => println!(
                "  no default site: set one with `rfluence config set default-site <site>`"
            ),
        }
    }
    if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(EXIT_AUTH)
    }
}

/// Print the token rfluence would use for a site.
pub fn token(site: Option<String>) -> ExitCode {
    match auth::resolve(site.as_deref()) {
        Ok((creds, _)) => {
            println!("{}", creds.token);
            ExitCode::SUCCESS
        }
        Err(e) => fail(&e),
    }
}

/// Answers come from the terminal; tests set `RFLUENCE_PROMPTS_FROM_STDIN=1` to give them on
/// standard input (the token too).
fn prompts_from_stdin() -> bool {
    std::env::var_os("RFLUENCE_PROMPTS_FROM_STDIN").is_some()
}

fn interactive() -> bool {
    std::io::stdin().is_terminal() || prompts_from_stdin()
}

fn read_line() -> Option<String> {
    let mut line = String::new();
    let read = std::io::stdin().lock().read_line(&mut line).ok()?;
    (read > 0).then(|| line.trim().to_string())
}

/// Ask a question; the default (shown in parentheses) is taken on Enter.
fn prompt(question: &str, default: Option<&str>) -> Option<String> {
    if !interactive() {
        eprintln!("rfluence: {question} is required (pass it as an option)");
        return None;
    }
    match default {
        Some(d) => eprint!("{question} ({d}): "),
        None => eprint!("{question}: "),
    }
    let _ = std::io::stderr().flush();
    let answer = read_line()?;
    let value = if answer.is_empty() {
        default.unwrap_or("")
    } else {
        &answer
    };
    if value.is_empty() {
        eprintln!("rfluence: {question} is required");
        return None;
    }
    Some(value.to_string())
}

/// `git config --global user.email`, if git is there and it's set.
fn git_email() -> Option<String> {
    let out = std::process::Command::new("git")
        .args(["config", "--global", "--get", "user.email"])
        .output()
        .ok()?;
    let email = String::from_utf8(out.stdout).ok()?.trim().to_string();
    (out.status.success() && !email.is_empty()).then_some(email)
}
