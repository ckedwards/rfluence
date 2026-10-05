//! `rfluence auth`: log in and out of Confluence sites, one account per site, like `gh auth`.
//! See design.md, "Auth".

use std::io::{BufRead, IsTerminal, Read, Write};
use std::process::ExitCode;

use rfluence_client::Client;
use rfluence_client::auth::{self, Credentials, Source};

use crate::{EXIT_AUTH, EXIT_USAGE, fail};

/// Log in to a site and make it the default. Prompts for what isn't given.
pub fn login(site: Option<String>, email: Option<String>, with_token: bool) -> ExitCode {
    let accounts = auth::accounts().unwrap_or_default();
    let default_site = accounts.iter().find(|a| a.default).map(|a| a.base_url.clone());
    let site = match site.or_else(|| prompt("Confluence site (e.g. example, or https://example.atlassian.net)", default_site.as_deref())) {
        Some(s) => s,
        None => return ExitCode::from(EXIT_USAGE),
    };
    let base_url = match auth::site_url(&site) {
        Ok(u) => u,
        Err(e) => return fail(&e),
    };
    let host = auth::host(&base_url);
    let known_email = accounts.iter().find(|a| a.host == host).map(|a| a.email.clone());
    let Some(email) = email.or_else(|| prompt("Atlassian account email", known_email.as_deref())) else {
        return ExitCode::from(EXIT_USAGE);
    };
    let token = if with_token {
        let mut t = String::new();
        if std::io::stdin().read_to_string(&mut t).is_err() || t.trim().is_empty() {
            eprintln!("rfluence: --with-token reads the token from standard input, but there was none");
            return ExitCode::from(EXIT_USAGE);
        }
        t.trim().to_string()
    } else {
        eprintln!("Create an API token at https://id.atlassian.com/manage-profile/security/api-tokens");
        match rpassword::prompt_password("API token: ") {
            Ok(t) if !t.trim().is_empty() => t.trim().to_string(),
            Ok(_) => {
                eprintln!("rfluence: no token entered");
                return ExitCode::from(EXIT_USAGE);
            }
            Err(e) => {
                eprintln!("rfluence: can't read the token ({e}); use --with-token to read it from standard input");
                return ExitCode::from(EXIT_USAGE);
            }
        }
    };
    let creds = Credentials { base_url, email, token };
    let user = match Client::new(&creds).current_user() {
        Ok(u) => u,
        Err(e) => {
            eprintln!("Not logged in: the credentials didn't work.");
            return fail(&e);
        }
    };
    match auth::login(&creds) {
        Ok(Source::Keyring) => eprintln!("Logged in to {host} as {user} ({}). Token saved in the system keyring.", creds.email),
        Ok(_) => {
            let file = auth::token_file(&host).map(|p| p.display().to_string()).unwrap_or_default();
            eprintln!("Logged in to {host} as {user} ({}). No system keyring: token saved in {file}.", creds.email);
        }
        Err(e) => return fail(&e),
    }
    if accounts.iter().any(|a| a.host != host) {
        eprintln!("{host} is now the default site.");
    }
    ExitCode::SUCCESS
}

/// Remove a site's account (the default site's if none is given).
pub fn logout(site: Option<String>) -> ExitCode {
    let host = match site_host(site) {
        Ok(h) => h,
        Err(code) => return code,
    };
    match auth::logout(&host) {
        Ok(true) => {
            eprintln!("Logged out of {host}.");
            if let Some(next) = auth::accounts().ok().and_then(|a| a.into_iter().find(|a| a.default)) {
                eprintln!("{} is now the default site.", next.host);
            }
            ExitCode::SUCCESS
        }
        Ok(false) => fail(&rfluence_client::Error::NotLoggedIn(host)),
        Err(e) => fail(&e),
    }
}

/// Every account (or one site's), where its token is, and whether Confluence accepts it.
pub fn status(site: Option<String>) -> ExitCode {
    let wanted = match site.map(|s| auth::site_url(&s).map(|u| auth::host(&u))).transpose() {
        Ok(w) => w,
        Err(e) => return fail(&e),
    };
    let mut entries: Vec<(String, Credentials, Option<Source>, bool)> = Vec::new();
    match auth::from_env(|k| std::env::var(k).ok()) {
        Ok(Some(creds)) => entries.push((creds.host(), creds, Some(Source::Env), false)),
        Ok(None) => {}
        Err(e) => return fail(&e),
    }
    let accounts = match auth::accounts() {
        Ok(a) => a,
        Err(e) => return fail(&e),
    };
    for a in accounts {
        let token = auth::token(&a.host).ok().flatten();
        let creds = Credentials { base_url: a.base_url, email: a.email, token: token.as_ref().map(|t| t.0.clone()).unwrap_or_default() };
        entries.push((a.host, creds, token.map(|t| t.1), a.default));
    }
    entries.retain(|(host, ..)| wanted.as_ref().is_none_or(|w| w == host));
    if entries.is_empty() {
        match wanted {
            Some(host) => return fail(&rfluence_client::Error::NotLoggedIn(host)),
            None => return fail(&rfluence_client::Error::NotConfigured),
        }
    }
    let mut ok = true;
    for (host, creds, source, default) in entries {
        println!("{host}{}", if default { " (default)" } else { "" });
        match source {
            None => {
                ok = false;
                println!("  ✗ No token found for {} (run `rfluence auth login --site {host}`)", creds.email);
            }
            Some(source) => {
                match Client::new(&creds).current_user() {
                    Ok(user) => println!("  ✓ Logged in as {user} ({})", creds.email),
                    Err(e) => {
                        ok = false;
                        println!("  ✗ {} ({e})", creds.email);
                    }
                }
                println!("  - Token: {source}");
            }
        }
    }
    if ok { ExitCode::SUCCESS } else { ExitCode::from(EXIT_AUTH) }
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

/// Make a site the default. Without --site, switches between two saved sites.
pub fn switch(site: Option<String>) -> ExitCode {
    let accounts = auth::accounts().unwrap_or_default();
    let host = match site {
        Some(s) => match auth::site_url(&s) {
            Ok(u) => auth::host(&u),
            Err(e) => return fail(&e),
        },
        None => match accounts.as_slice() {
            [a, b] => if a.default { b.host.clone() } else { a.host.clone() },
            [] => return fail(&rfluence_client::Error::NotConfigured),
            _ => {
                let hosts: Vec<_> = accounts.iter().map(|a| a.host.as_str()).collect();
                eprintln!("rfluence: choose a site with --site: {}", hosts.join(", "));
                return ExitCode::from(EXIT_USAGE);
            }
        },
    };
    match auth::switch(&host) {
        Ok(()) => {
            eprintln!("{host} is now the default site.");
            ExitCode::SUCCESS
        }
        Err(e) => fail(&e),
    }
}

/// The host for `--site`, or the default site's.
fn site_host(site: Option<String>) -> Result<String, ExitCode> {
    match site {
        Some(s) => auth::site_url(&s).map(|u| auth::host(&u)).map_err(|e| fail(&e)),
        None => match auth::accounts() {
            Ok(a) => a.into_iter().find(|a| a.default).map(|a| a.host).ok_or_else(|| fail(&rfluence_client::Error::NotConfigured)),
            Err(e) => Err(fail(&e)),
        },
    }
}

/// Read a line from stdin, showing `default` and using it for an empty answer. `None` if
/// stdin isn't a terminal (nothing to prompt) or the answer is empty without a default.
fn prompt(question: &str, default: Option<&str>) -> Option<String> {
    if !std::io::stdin().is_terminal() {
        eprintln!("rfluence: {question} is required (pass it as an option)");
        return None;
    }
    match default {
        Some(d) => eprint!("{question} [{d}]: "),
        None => eprint!("{question}: "),
    }
    let _ = std::io::stderr().flush();
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line).ok()?;
    let answer = line.trim();
    let value = if answer.is_empty() { default.unwrap_or("") } else { answer };
    if value.is_empty() {
        eprintln!("rfluence: {question} is required");
        return None;
    }
    Some(value.to_string())
}
