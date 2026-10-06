//! The `rfluence` command. See design.md, "Commands".

mod auth;
mod check;
mod config;
mod fetch;
mod order;
mod plan;
mod project;
mod search;
mod upload;
mod upload_tree;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "rfluence", version, about = "Fetch, search and upload Confluence pages as markdown")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Fetch a page as markdown.
    ///
    /// Prints the round-trip form (frontmatter, title, body), which `rfluence upload` can send back.
    /// Use --simplified for a shorter form to read (it can't be uploaded).
    Fetch {
        /// Page ID, page URL (including tiny links), or SPACE:Title.
        page: String,
        /// Shorter markdown for reading: drops settings, macros' markup, and attachments'
        /// paths. Can't be uploaded.
        #[arg(long)]
        simplified: bool,
        /// Only the section under this heading (its text, or its #anchor).
        #[arg(long, value_name = "HEADING")]
        section: Option<String>,
        /// Cut the body to about this many characters, listing the sections to read the rest.
        #[arg(long, value_name = "N")]
        max_chars: Option<usize>,
        /// Print the page's metadata and markdown as JSON.
        #[arg(long)]
        json: bool,
        /// The site to use (default: the page URL's site, or the default site).
        #[arg(long, value_name = "SITE")]
        site: Option<String>,
        /// Write the page to this file, with its images in `<name>.assets/` next to it, and
        /// links to other pages in the project as relative paths.
        #[arg(short, long, value_name = "FILE")]
        output: Option<PathBuf>,
        /// With -o: overwrite the file even if it has local changes or holds another page.
        #[arg(long)]
        force: bool,
    },
    /// Upload a markdown file to its page, or create a page for it.
    ///
    /// The page ID is in the file's frontmatter; a file without one creates a page (titled by
    /// its leading `# Title`), unless the space already has a page with that title. Refuses if the
    /// page changed in Confluence since the file was fetched, or if the file has content
    /// Confluence can't store (see `rfluence check`); --force overrides these. Local images
    /// are attached, links to other markdown files become page links, and inline comments stay
    /// on their text. Afterwards the file's frontmatter has the page ID and new version.
    Upload {
        /// The markdown file.
        #[arg(required_unless_present = "config", conflicts_with = "config")]
        path: Option<PathBuf>,
        /// Upload the files listed in a config file (default: .rfluence.yaml in this
        /// directory or the nearest one above it), as a page tree.
        #[arg(long, value_name = "FILE", num_args = 0..=1)]
        #[allow(clippy::option_option)]
        config: Option<Option<PathBuf>>,
        /// Show what would change, without changing anything.
        #[arg(long)]
        dry_run: bool,
        /// Upload even if the page changed in Confluence (overwriting those changes), or the
        /// file has content Confluence can't store (uploading the approximations).
        #[arg(long)]
        force: bool,
        /// The site to use (default: the site in the file's `url`, or the default site).
        #[arg(long, value_name = "SITE")]
        site: Option<String>,
        /// Move pages that aren't where they belong: under the config tree's parent (with
        /// --config) or the file's `parent`, and (with --config) in the config's order.
        #[arg(long = "move")]
        move_pages: bool,
        /// With --config: trash pages and folders rfluence created whose files or directories
        /// are gone (skipping pages edited in Confluence since, unless --force).
        #[arg(long, requires = "config")]
        prune: bool,
        /// Remove labels the file doesn't list (and, with --config, labels taken out of the
        /// config). Without it, upload only adds labels.
        #[arg(long)]
        prune_labels: bool,
        /// Refuse files with `rfluence check` warnings too (content uploaded as a close
        /// equivalent), not only errors. --force uploads anyway.
        #[arg(long)]
        warnings_are_errors: bool,
        /// For a new page: the space to create it in, if the file has no `space_key`.
        #[arg(long, value_name = "KEY")]
        space: Option<String>,
        /// For a new page: the parent page or folder ID, if the file has no `parent`
        /// (default: the space's homepage).
        #[arg(long, value_name = "ID")]
        parent: Option<String>,
        /// Print the result as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Search for pages: free text, optionally in spaces and with labels, or a raw CQL query.
    ///
    /// Lists each page's ID, title, space, last update, labels, URL and an excerpt.
    Search {
        /// Text to search for (CQL `text ~ "<query>"`, pages only).
        #[arg(required_unless_present = "cql", conflicts_with = "cql")]
        query: Option<String>,
        /// Only in this space (repeat for several).
        #[arg(long, value_name = "KEY")]
        space: Vec<String>,
        /// Only pages with this label (repeat to require several).
        #[arg(long)]
        label: Vec<String>,
        /// A raw CQL query, instead of a query and --space / --label.
        #[arg(long, conflicts_with_all = ["space", "label"])]
        cql: Option<String>,
        /// The most results to show.
        #[arg(long, default_value_t = 10)]
        limit: usize,
        /// The site to search (default: the default site).
        #[arg(long, value_name = "SITE")]
        site: Option<String>,
        /// Print the results as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Check markdown files for content Confluence can't store exactly.
    ///
    /// Warnings are uploaded as the closest Confluence equivalent (e.g. `<kbd>` as inline
    /// code). Errors can't be represented in Confluence, and upload refuses them.
    /// Exits 1 if there are errors (or, with --warnings-are-errors, warnings).
    Check {
        /// Markdown files to check.
        #[arg(required = true)]
        paths: Vec<PathBuf>,
        /// Print diagnostics as JSON.
        #[arg(long)]
        json: bool,
        /// Exit 1 if there are warnings too, not only errors (e.g. in CI, to keep files
        /// exactly representable in Confluence).
        #[arg(long)]
        warnings_are_errors: bool,
    },
    /// Log in to Confluence sites: one account per site.
    ///
    /// Tokens are kept in the system keyring, or a file readable only by you if there is none.
    /// CONFLUENCE_BASE_URL, CONFLUENCE_EMAIL and CONFLUENCE_API_KEY, if all set, are used for
    /// their own site.
    Auth {
        #[command(subcommand)]
        action: AuthAction,
    },
}

#[derive(Subcommand)]
enum AuthAction {
    /// Log in to a site (and make it the default). Prompts for anything not given.
    Login {
        /// The site: a name (`example`), a host, or a URL.
        #[arg(long, value_name = "SITE")]
        site: Option<String>,
        /// The Atlassian account email.
        #[arg(long)]
        email: Option<String>,
        /// Read the API token from standard input.
        #[arg(long)]
        with_token: bool,
    },
    /// Log out of a site (default: the default site).
    Logout {
        #[arg(long, value_name = "SITE")]
        site: Option<String>,
    },
    /// Show the saved accounts and check them against Confluence.
    Status {
        /// Only this site.
        #[arg(long, value_name = "SITE")]
        site: Option<String>,
    },
    /// Print the API token for a site (default: the site rfluence would use).
    Token {
        #[arg(long, value_name = "SITE")]
        site: Option<String>,
    },
    /// Change the default site. Without --site, switches between two saved sites.
    Switch {
        #[arg(long, value_name = "SITE")]
        site: Option<String>,
    },
}

/// Exit codes (design.md, "Output and errors").
pub const EXIT_ERRORS_FOUND: u8 = 1;
pub const EXIT_USAGE: u8 = 2;
pub const EXIT_NOT_FOUND: u8 = 3;
pub const EXIT_AUTH: u8 = 4;
pub const EXIT_API: u8 = 5;
/// Writing would lose changes (local edits, or a file holding another page).
pub const EXIT_CONFLICT: u8 = 6;

/// Print a client error and return its exit code.
pub fn fail(e: &rfluence_client::Error) -> ExitCode {
    use rfluence_client::Error;
    eprintln!("rfluence: {e}");
    ExitCode::from(match e {
        Error::NotConfigured | Error::NotLoggedIn(_) | Error::PartialEnv(_) | Error::Invalid(_) | Error::Io(_) => EXIT_USAGE,
        Error::NotFound(_) => EXIT_NOT_FOUND,
        Error::Auth(_) | Error::Forbidden(_) => EXIT_AUTH,
        Error::Conflict(_) => EXIT_CONFLICT,
        Error::Api { .. } | Error::Network(_) => EXIT_API,
    })
}

fn main() -> ExitCode {
    match Cli::parse().command {
        Command::Fetch { page, simplified, section, max_chars, json, site, output, force } => {
            fetch::run(&fetch::Options { page, simplified, section, max_chars, json, site, output, force })
        }
        Command::Upload { path, config, dry_run, force, move_pages, prune, prune_labels, warnings_are_errors, site, space, parent, json } => match (path, config) {
            (_, Some(config)) => {
                if space.is_some() || parent.is_some() || json {
                    eprintln!("rfluence: --space, --parent and --json can't be used with --config");
                    return ExitCode::from(EXIT_USAGE);
                }
                let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
                let config = config.unwrap_or_else(|| {
                    // Relative to here when it's here or below, so messages show short paths.
                    let found = project::root(&cwd).join(config::FILE_NAME);
                    found.strip_prefix(&cwd).map(PathBuf::from).unwrap_or(found)
                });
                upload_tree::run(&upload_tree::Options { config, dry_run, force, move_pages, prune, prune_labels, warnings_are_errors, site })
            }
            (Some(path), None) => {
                upload::run(&upload::Options {
                    path,
                    dry_run,
                    force,
                    site,
                    space,
                    parent,
                    json,
                    move_pages,
                    prune_labels,
                    warnings_are_errors,
                    tree: None,
                })
            }
            (None, None) => unreachable!("clap requires one"),
        },
        Command::Search { query, space, label, cql, limit, site, json } => {
            search::run(&search::Options { query, space, label, cql, limit, site, json })
        }
        Command::Check { paths, json, warnings_are_errors } => check::run(&paths, json, warnings_are_errors),
        Command::Auth { action } => match action {
            AuthAction::Login { site, email, with_token } => auth::login(site, email, with_token),
            AuthAction::Logout { site } => auth::logout(site),
            AuthAction::Status { site } => auth::status(site),
            AuthAction::Token { site } => auth::token(site),
            AuthAction::Switch { site } => auth::switch(site),
        },
    }
}
