//! Credentials: one account per Confluence site, saved by `rfluence auth login`, or the
//! `CONFLUENCE_*` environment variables. See design.md, "Auth".

use std::collections::BTreeMap;
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

/// A saved account: one per site.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    pub host: String,
    pub base_url: String,
    pub email: String,
    pub default: bool,
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
/// `CONFLUENCE_*` variables are used for their own site; other sites use saved accounts.
pub fn resolve(site: Option<&str>) -> Result<(Credentials, Source)> {
    let env = from_env(|k| std::env::var(k).ok())?;
    let requested = match site.map(str::to_string).or_else(|| std::env::var(ENV_SITE).ok().filter(|s| !s.trim().is_empty())) {
        Some(s) => Some(host(&site_url(&s)?)),
        None => None,
    };
    if let Some(creds) = env {
        if requested.as_ref().is_none_or(|h| *h == creds.host()) {
            return Ok((creds, Source::Env));
        }
    }
    let config = read_config()?;
    let host = match requested.or(config.default.clone()) {
        Some(h) => h,
        None => return Err(Error::NotConfigured),
    };
    let Some(site) = config.sites.get(&host) else { return Err(Error::NotLoggedIn(host)) };
    match read_token(&host, site, &config) {
        Some((token, source)) => Ok((Credentials { base_url: site.base_url.clone(), email: site.email.clone(), token }, source)),
        None => Err(Error::NotLoggedIn(host)),
    }
}

/// The saved accounts, by host.
pub fn accounts() -> Result<Vec<Account>> {
    let config = read_config()?;
    Ok(config
        .sites
        .iter()
        .map(|(host, s)| Account {
            host: host.clone(),
            base_url: s.base_url.clone(),
            email: s.email.clone(),
            default: config.default.as_deref() == Some(host.as_str()),
        })
        .collect())
}

/// A saved account's token, and where it is stored.
pub fn token(host: &str) -> Result<Option<(String, Source)>> {
    let config = read_config()?;
    Ok(config.sites.get(host).and_then(|site| read_token(host, site, &config)))
}

/// Save an account and make its site the default: site and email in the config file, the
/// token in the system keyring, or in a token file readable only by the user when there is
/// no keyring.
pub fn login(creds: &Credentials) -> Result<Source> {
    let mut config = read_config()?;
    let host = creds.host();
    let site = SiteConfig { base_url: normalize_base_url(&creds.base_url), email: creds.email.clone() };
    // A different account on the same site replaces the old one.
    if let Some(old) = config.sites.get(&host).filter(|old| old.email != site.email) {
        keyring_delete(old);
    }
    config.sites.insert(host.clone(), site.clone());
    config.default = Some(host.clone());
    config.legacy_token = false;
    write_config(&config)?;
    if keyring_set(&site, &creds.token) {
        // A token file from an earlier fallback would be stale now.
        let _ = std::fs::remove_file(token_path(&host)?);
        return Ok(Source::Keyring);
    }
    write_private(&token_path(&host)?, &creds.token)?;
    Ok(Source::TokenFile)
}

/// Remove a site's account and token. If it was the default, another saved site (if any)
/// becomes the default. Returns false if there was no account for the site.
pub fn logout(host: &str) -> Result<bool> {
    let mut config = read_config()?;
    let Some(site) = config.sites.remove(host) else { return Ok(false) };
    keyring_delete(&site);
    let _ = std::fs::remove_file(token_path(host)?);
    if config.default.as_deref() == Some(host) {
        config.default = config.sites.keys().next().cloned();
    }
    if config.legacy_token {
        let _ = std::fs::remove_file(config_dir()?.join("token"));
        config.legacy_token = false;
    }
    write_config(&config)?;
    Ok(true)
}

/// Make a saved site the default.
pub fn switch(host: &str) -> Result<()> {
    let mut config = read_config()?;
    if !config.sites.contains_key(host) {
        return Err(Error::NotLoggedIn(host.to_string()));
    }
    config.default = Some(host.to_string());
    write_config(&config)
}

/// Where tokens are saved when there is no keyring (for messages).
pub fn token_file(host: &str) -> Result<PathBuf> {
    token_path(host)
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Config {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    default: Option<String>,
    #[serde(default)]
    sites: BTreeMap<String, SiteConfig>,
    /// Migrated from the single-account format, whose token file was `token`.
    #[serde(skip)]
    legacy_token: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SiteConfig {
    base_url: String,
    email: String,
}

fn config_dir() -> Result<PathBuf> {
    if let Some(dir) = std::env::var_os(ENV_CONFIG_DIR).filter(|d| !d.is_empty()) {
        return Ok(PathBuf::from(dir));
    }
    dirs::config_dir()
        .map(|d| d.join("rfluence"))
        .ok_or_else(|| Error::Io("can't find the user config directory".into()))
}

fn token_path(host: &str) -> Result<PathBuf> {
    Ok(config_dir()?.join("tokens").join(host.replace([':', '/', '\\'], "_")))
}

fn read_config() -> Result<Config> {
    let path = config_dir()?.join("auth.json");
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Config::default()),
        Err(e) => return Err(Error::Io(format!("{}: {e}", path.display()))),
    };
    let bad = |e: serde_json::Error| Error::Io(format!("{}: {e}", path.display()));
    let value: serde_json::Value = serde_json::from_str(&text).map_err(bad)?;
    if value.get("sites").is_some() || value.get("base_url").is_none() {
        return serde_json::from_value(value).map_err(bad);
    }
    // The single-account format: {base_url, email}, with the token in `token`.
    let old: SiteConfig = serde_json::from_value(value).map_err(bad)?;
    let host = host(&old.base_url);
    Ok(Config { default: Some(host.clone()), sites: BTreeMap::from([(host, old)]), legacy_token: true })
}

fn write_config(config: &Config) -> Result<()> {
    let dir = config_dir()?;
    std::fs::create_dir_all(&dir).map_err(|e| Error::Io(format!("{}: {e}", dir.display())))?;
    let path = dir.join("auth.json");
    std::fs::write(&path, serde_json::to_string_pretty(config).expect("config serializes") + "\n")
        .map_err(|e| Error::Io(format!("{}: {e}", path.display())))
}

fn read_token(host: &str, site: &SiteConfig, config: &Config) -> Option<(String, Source)> {
    if let Some(token) = keyring_get(site) {
        return Some((token, Source::Keyring));
    }
    let mut files = vec![token_path(host).ok()?];
    if config.legacy_token {
        files.push(config_dir().ok()?.join("token"));
    }
    files.into_iter().find_map(|f| {
        let token = std::fs::read_to_string(f).ok()?;
        let token = token.trim();
        (!token.is_empty()).then(|| (token.to_string(), Source::TokenFile))
    })
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

/// The keyring entry's user: email and site.
fn keyring_user(site: &SiteConfig) -> String {
    format!("{} on {}", site.email, site.base_url)
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

fn keyring_get(site: &SiteConfig) -> Option<String> {
    let user = keyring_user(site);
    with_keyring(move || keyring_core::Entry::new(KEYRING_SERVICE, &user).ok()?.get_password().ok())
}

fn keyring_set(site: &SiteConfig, token: &str) -> bool {
    let (user, token) = (keyring_user(site), token.to_string());
    with_keyring(move || keyring_core::Entry::new(KEYRING_SERVICE, &user).ok()?.set_password(&token).ok()).is_some()
}

fn keyring_delete(site: &SiteConfig) {
    let user = keyring_user(site);
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
