//! Credentials: one account (an email and its API token, which works on every site the
//! account is on), saved by `rfluence auth login`, or given by the `CONFLUENCE_*` environment
//! variables; and the default site (Confluence site), set by `rfluence config set
//! default-site`. See design.md, "Auth".

use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

const ENV_BASE_URL: &str = "CONFLUENCE_BASE_URL";
const ENV_EMAIL: &str = "CONFLUENCE_EMAIL";
const ENV_TOKEN: &str = "CONFLUENCE_API_KEY";
/// The site to use when a command doesn't say (`--site`, or a page URL's host).
const ENV_SITE: &str = "RFLUENCE_SITE";
/// Use this directory for the config and token files instead of `~/.config/rfluence`.
const ENV_CONFIG_DIR: &str = "RFLUENCE_CONFIG_DIR";
/// Don't use the system keyring (tokens go in files). For CI, sandboxes and tests.
const ENV_NO_KEYRING: &str = "RFLUENCE_NO_KEYRING";

/// Keyring access goes over D-Bus on Linux, which can hang (SSH, CI, sandboxes) or wait for
/// an unlock prompt; give up after this and fall back to the token file.
const KEYRING_TIMEOUT: Duration = Duration::from_secs(2);
const KEYRING_SERVICE: &str = "rfluence";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Credentials {
    /// The site, e.g. `https://example.atlassian.net` (no `/wiki`, no trailing slash).
    pub base_url: String,
    pub email: String,
    pub token: String,
}

impl Credentials {
    pub fn host(&self) -> String {
        host(&self.base_url)
    }
}

/// Where a token came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Env,
    Keyring,
    TokenFile,
}

impl std::fmt::Display for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Source::Env => "CONFLUENCE_* environment variables",
            Source::Keyring => "system keyring",
            Source::TokenFile => "token file",
        })
    }
}

/// The saved account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    pub email: String,
    /// Where its token is; `None` if it has gone missing.
    pub source: Option<Source>,
}

/// `https://x.atlassian.net/wiki/` -> `https://x.atlassian.net`.
pub fn normalize_base_url(url: &str) -> String {
    let url = url.trim().trim_end_matches('/');
    url.strip_suffix("/wiki").unwrap_or(url).to_string()
}

/// A site as typed: `example` (an Atlassian Cloud site name), `example.atlassian.net`, or a
/// URL. Returns its base URL.
pub fn site_url(site: &str) -> Result<String> {
    let s = site.trim();
    if s.is_empty() || s.contains(char::is_whitespace) {
        return Err(Error::Invalid(format!("not a Confluence site: {site:?}")));
    }
    Ok(if s.contains("://") {
        normalize_base_url(s)
    } else if !s.contains(['.', ':']) {
        format!("https://{s}.atlassian.net")
    } else {
        normalize_base_url(&format!("https://{s}"))
    })
}

/// The host of a base URL (`https://example.atlassian.net` -> `example.atlassian.net`); sites
/// are identified by it.
pub fn host(base_url: &str) -> String {
    let rest = base_url.split_once("://").map_or(base_url, |(_, r)| r);
    rest.split('/').next().unwrap_or(rest).to_ascii_lowercase()
}

/// Credentials from environment variables: all three, or none (`Ok(None)`). Only some is an
/// error, rather than silently mixing them with stored credentials.
pub fn from_env(var: impl Fn(&str) -> Option<String>) -> Result<Option<Credentials>> {
    let get = |k: &str| var(k).filter(|v| !v.trim().is_empty());
    let (base, email, token) = (get(ENV_BASE_URL), get(ENV_EMAIL), get(ENV_TOKEN));
    match (base, email, token) {
        (Some(base), Some(email), Some(token)) => {
            Ok(Some(Credentials { base_url: normalize_base_url(&base), email, token }))
        }
        (None, None, None) => Ok(None),
        (base, email, token) => {
            let missing = [(ENV_BASE_URL, base.is_none()), (ENV_EMAIL, email.is_none()), (ENV_TOKEN, token.is_none())]
                .into_iter()
                .filter_map(|(k, m)| m.then_some(k))
                .collect();
            Err(Error::PartialEnv(missing))
        }
    }
}

/// The credentials for a command. The site is `site` (a page URL's host, or `--site`), else
/// `RFLUENCE_SITE`, else the `CONFLUENCE_*` variables' site, else the default site. The
/// `CONFLUENCE_*` variables are used for their own site; any other site uses the saved
/// account, whose token works on every site the account is on (added or not).
pub fn resolve(site: Option<&str>) -> Result<(Credentials, Source)> {
    let env = from_env(|k| std::env::var(k).ok())?;
    let requested = match site.map(str::to_string).or_else(|| std::env::var(ENV_SITE).ok().filter(|s| !s.trim().is_empty())) {
        Some(s) => Some(site_url(&s)?),
        None => None,
    };
    if let Some(creds) = env
        && requested.as_ref().is_none_or(|u| host(u) == creds.host()) {
        return Ok((creds, Source::Env));
    }
    let config = read_config()?;
    let Some(email) = config.email.clone() else { return Err(Error::NotConfigured) };
    let base_url = match requested.or(default_site()?) {
        Some(u) => u,
        None => return Err(Error::NoSite),
    };
    match read_token(&email) {
        Some((token, source)) => Ok((Credentials { base_url, email, token }, source)),
        None => Err(Error::NotConfigured),
    }
}

/// The saved account, if any.
pub fn account() -> Result<Option<Account>> {
    let config = read_config()?;
    Ok(config.email.map(|email| Account { source: read_token(&email).map(|t| t.1), email }))
}

/// The saved account's token, and where it is stored.
pub fn saved_token() -> Option<(String, Source)> {
    read_config().ok()?.email.and_then(|e| read_token(&e))
}

/// Save the account: the email in the config file, the token in the system keyring, or in a
/// token file readable only by the user when there is no keyring. Replaces any saved account.
pub fn save_account(email: &str, token: &str) -> Result<Source> {
    let mut config = read_config()?;
    if let Some(old) = config.email.as_deref().filter(|old| *old != email) {
        forget_token(old);
    }
    config.email = Some(email.to_string());
    write_config(&config)?;
    if keyring_set(email, token) {
        // A token file from an earlier fallback would be stale now.
        let _ = std::fs::remove_file(token_path()?);
        return Ok(Source::Keyring);
    }
    write_private(&token_path()?, token)?;
    Ok(Source::TokenFile)
}

/// Forget the account: its token and email. Returns the email, if there was one.
pub fn logout() -> Result<Option<String>> {
    let config = read_config()?;
    if let Some(email) = &config.email {
        forget_token(email);
    }
    write_config(&Config::default())?;
    Ok(config.email)
}

fn forget_token(email: &str) {
    keyring_delete(email);
    if let Ok(path) = token_path() {
        let _ = std::fs::remove_file(path);
    }
}

/// The default site: the Confluence site commands use when none is named (base URL).
pub fn default_site() -> Result<Option<String>> {
    Ok(read_settings()?.default_site)
}

/// Set the default site (a site name, host or URL; stored as its base URL), or clear it.
pub fn set_default_site(site: Option<&str>) -> Result<Option<String>> {
    let mut settings = read_settings()?;
    settings.default_site = site.map(site_url).transpose()?;
    write_json("config.json", &settings)?;
    Ok(settings.default_site)
}

/// Where the token is saved when there is no keyring (for messages).
pub fn token_file() -> Result<PathBuf> {
    token_path()
}

/// `auth.json`: the account's email (its token is in the keyring or the token file).
#[derive(Debug, Default, Serialize, Deserialize)]
struct Config {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    email: Option<String>,
}

/// `config.json`: settings (`rfluence config`).
#[derive(Debug, Default, Serialize, Deserialize)]
struct Settings {
    #[serde(default, rename = "default-site", skip_serializing_if = "Option::is_none")]
    default_site: Option<String>,
}

fn config_dir() -> Result<PathBuf> {
    if let Some(dir) = std::env::var_os(ENV_CONFIG_DIR).filter(|d| !d.is_empty()) {
        return Ok(PathBuf::from(dir));
    }
    dirs::config_dir()
        .map(|d| d.join("rfluence"))
        .ok_or_else(|| Error::Io("can't find the user config directory".into()))
}

/// The token file (when there is no keyring).
fn token_path() -> Result<PathBuf> {
    Ok(config_dir()?.join("token"))
}

fn read_config() -> Result<Config> {
    read_json("auth.json")
}

fn write_config(config: &Config) -> Result<()> {
    write_json("auth.json", config)
}

fn read_settings() -> Result<Settings> {
    read_json("config.json")
}

/// A JSON file in the config directory (the default value if there's none).
fn read_json<T: serde::de::DeserializeOwned + Default>(name: &str) -> Result<T> {
    let path = config_dir()?.join(name);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(T::default()),
        Err(e) => return Err(Error::Io(format!("{}: {e}", path.display()))),
    };
    serde_json::from_str(&text).map_err(|e| Error::Io(format!("{}: {e}", path.display())))
}

fn write_json<T: Serialize>(name: &str, value: &T) -> Result<()> {
    let dir = config_dir()?;
    std::fs::create_dir_all(&dir).map_err(|e| Error::Io(format!("{}: {e}", dir.display())))?;
    let path = dir.join(name);
    std::fs::write(&path, serde_json::to_string_pretty(value).expect("settings serialize") + "\n")
        .map_err(|e| Error::Io(format!("{}: {e}", path.display())))
}

/// The account's token: from the keyring, else the token file.
fn read_token(email: &str) -> Option<(String, Source)> {
    if let Some(token) = keyring_get(email) {
        return Some((token, Source::Keyring));
    }
    let token = std::fs::read_to_string(token_path().ok()?).ok()?;
    let token = token.trim();
    (!token.is_empty()).then(|| (token.to_string(), Source::TokenFile))
}

fn write_private(path: &PathBuf, contents: &str) -> Result<()> {
    let io = |e: std::io::Error| Error::Io(format!("{}: {e}", path.display()));
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(io)?;
    }
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(path).map_err(io)?;
        f.write_all(contents.as_bytes()).map_err(io)
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, contents).map_err(io)
    }
}

/// Run a keyring operation on a thread, giving up after [`KEYRING_TIMEOUT`].
fn with_keyring<T: Send + 'static>(f: impl FnOnce() -> Option<T> + Send + 'static) -> Option<T> {
    if std::env::var_os(ENV_NO_KEYRING).is_some_and(|v| !v.is_empty()) {
        return None;
    }
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        if init_store() {
            let _ = tx.send(f());
        }
    });
    rx.recv_timeout(KEYRING_TIMEOUT).ok().flatten()
}

/// Keyring entries: service `rfluence`, user the account's email.
fn keyring_get(email: &str) -> Option<String> {
    let user = email.to_string();
    with_keyring(move || keyring_core::Entry::new(KEYRING_SERVICE, &user).ok()?.get_password().ok())
}

fn keyring_set(email: &str, token: &str) -> bool {
    let (user, token) = (email.to_string(), token.to_string());
    with_keyring(move || keyring_core::Entry::new(KEYRING_SERVICE, &user).ok()?.set_password(&token).ok()).is_some()
}

fn keyring_delete(email: &str) {
    let user = email.to_string();
    with_keyring(move || keyring_core::Entry::new(KEYRING_SERVICE, &user).ok()?.delete_credential().ok());
}

/// Register the platform's credential store with keyring-core, once.
fn init_store() -> bool {
    static INIT: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *INIT.get_or_init(|| {
        #[cfg(target_os = "linux")]
        let store = zbus_secret_service_keyring_store::Store::new().map(|s| s as std::sync::Arc<keyring_core::CredentialStore>);
        #[cfg(target_os = "macos")]
        let store = apple_native_keyring_store::keychain::Store::new().map(|s| s as std::sync::Arc<keyring_core::CredentialStore>);
        #[cfg(windows)]
        let store = windows_native_keyring_store::Store::new().map(|s| s as std::sync::Arc<keyring_core::CredentialStore>);
        #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
        let store: keyring_core::Result<std::sync::Arc<keyring_core::CredentialStore>> =
            Err(keyring_core::Error::NoDefaultStore);
        match store {
            Ok(store) => {
                keyring_core::set_default_store(store);
                true
            }
            Err(_) => false,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env<'a>(vars: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |k| vars.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string())
    }

    #[test]
    fn env_credentials_are_all_or_nothing() {
        let all = [(ENV_BASE_URL, "https://x.atlassian.net/wiki/"), (ENV_EMAIL, "a@b.c"), (ENV_TOKEN, "t")];
        let creds = from_env(env(&all)).unwrap().unwrap();
        assert_eq!(creds.base_url, "https://x.atlassian.net");
        assert!(from_env(env(&[])).unwrap().is_none());
        match from_env(env(&[(ENV_EMAIL, "a@b.c")])) {
            Err(Error::PartialEnv(missing)) => assert_eq!(missing, [ENV_BASE_URL, ENV_TOKEN]),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn parses_sites() {
        for s in ["tech-accounts11", "tech-accounts11.atlassian.net", "https://tech-accounts11.atlassian.net/wiki/"] {
            assert_eq!(site_url(s).unwrap(), "https://tech-accounts11.atlassian.net", "{s}");
        }
        assert_eq!(site_url("http://127.0.0.1:8080").unwrap(), "http://127.0.0.1:8080");
        assert_eq!(host("https://Tech-Accounts11.atlassian.net"), "tech-accounts11.atlassian.net");
        assert_eq!(host("http://127.0.0.1:8080"), "127.0.0.1:8080");
        assert!(site_url("").is_err() && site_url("a b").is_err());
    }
}
