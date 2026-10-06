//! `rfluence config`: settings. See design.md, "Auth".

use std::process::ExitCode;

use rfluence_client::Client;
use rfluence_client::auth::{self, Credentials};

use crate::{EXIT_NOT_FOUND, EXIT_USAGE, fail};

/// The settings `rfluence config` knows.
const KEYS: [&str; 1] = ["default-site"];

fn check_key(key: &str) -> Result<(), ExitCode> {
    if KEYS.contains(&key) {
        return Ok(());
    }
    eprintln!(
        "rfluence: unknown setting {key:?} (settings: {})",
        KEYS.join(", ")
    );
    Err(ExitCode::from(EXIT_USAGE))
}

/// `config set default-site <site>`: the Confluence site commands use when none is named. The
/// saved token is checked there.
pub fn set(key: &str, value: &str) -> ExitCode {
    if let Err(code) = check_key(key) {
        return code;
    }
    let base_url = match store_default_site(value) {
        Ok(u) => u,
        Err(e) => return fail(&e),
    };
    let host = auth::host(&base_url);
    let account = auth::account().ok().flatten();
    match (account, auth::saved_token()) {
        (Some(account), Some((token, _))) => {
            let creds = Credentials {
                base_url,
                email: account.email.clone(),
                token,
            };
            match Client::new(&creds).current_user() {
                Ok(user) => eprintln!(
                    "Default site: {host}. Logged in there as {user} ({}).",
                    account.email
                ),
                Err(e) => eprintln!("Default site: {host}. Warning: {e}"),
            }
        }
        _ => eprintln!("Default site: {host}. Log in with `rfluence auth login`."),
    }
    ExitCode::SUCCESS
}

/// Save the default site, and say so if `.atlassian.net` was added to a bare name. Returns
/// its base URL.
pub fn store_default_site(value: &str) -> rfluence_client::Result<String> {
    let base_url = auth::set_default_site(Some(value))?.expect("just set");
    let name = value.trim();
    if !name.contains("://") && !name.contains(['.', ':']) {
        eprintln!(
            "Added .atlassian.net: {name:?} is {}.",
            auth::host(&base_url)
        );
    }
    Ok(base_url)
}

/// `config get default-site`: print it (its host).
pub fn get(key: &str) -> ExitCode {
    if let Err(code) = check_key(key) {
        return code;
    }
    match auth::default_site() {
        Ok(Some(base_url)) => {
            println!("{}", auth::host(&base_url));
            ExitCode::SUCCESS
        }
        Ok(None) => {
            eprintln!("rfluence: {key} isn't set (set it with `rfluence config set {key} <site>`)");
            ExitCode::from(EXIT_NOT_FOUND)
        }
        Err(e) => fail(&e),
    }
}

/// `config unset default-site`.
pub fn unset(key: &str) -> ExitCode {
    if let Err(code) = check_key(key) {
        return code;
    }
    match auth::set_default_site(None) {
        Ok(_) => {
            eprintln!("{key} unset.");
            ExitCode::SUCCESS
        }
        Err(e) => fail(&e),
    }
}
