use std::fmt;

/// Client errors. Each kind maps to an exit code in the CLI (design.md, "Output and errors").
#[derive(Debug)]
pub enum Error {
    /// No credentials: no `CONFLUENCE_*` variables and no saved account.
    NotConfigured,
    /// No saved account for this site (host).
    NotLoggedIn(String),
    /// Some `CONFLUENCE_*` variables are set but not all; the missing ones.
    PartialEnv(Vec<&'static str>),
    /// Something the user passed that isn't valid (a page reference, ...).
    Invalid(String),
    /// Confluence rejected the credentials (HTTP 401 / 403).
    Auth(String),
    /// The page (or space, attachment, ...) doesn't exist, or isn't visible.
    NotFound(String),
    /// Any other HTTP error from Confluence.
    Api { status: u16, message: String },
    /// The request didn't get a response.
    Network(String),
    /// Reading or writing local files (stored credentials).
    Io(String),
}

pub type Result<T> = std::result::Result<T, Error>;

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::NotConfigured => f.write_str(
                "not logged in: run `rfluence auth login`, or set CONFLUENCE_BASE_URL, CONFLUENCE_EMAIL and CONFLUENCE_API_KEY",
            ),
            Error::NotLoggedIn(host) => write!(f, "not logged in to {host}: run `rfluence auth login --site {host}`"),
            Error::PartialEnv(missing) => write!(
                f,
                "set all of CONFLUENCE_BASE_URL, CONFLUENCE_EMAIL and CONFLUENCE_API_KEY, or none (to use saved accounts); missing: {}",
                missing.join(", ")
            ),
            Error::Invalid(m) => f.write_str(m),
            Error::Auth(m) => write!(f, "authentication failed: {m}"),
            Error::NotFound(m) => f.write_str(m),
            Error::Api { status, message } => write!(f, "Confluence returned HTTP {status}: {message}"),
            Error::Network(m) => write!(f, "network error: {m}"),
            Error::Io(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for Error {}
