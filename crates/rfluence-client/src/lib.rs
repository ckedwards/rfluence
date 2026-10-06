//! Confluence API client and credentials for rfluence. See design.md, "rfluence (the command)".

mod api;
pub mod auth;
mod error;

pub use api::{
    Attachment, Child, Client, Content, InstalledMacro, Kind, Page, PageRef, Property,
    PropertyVersion, SearchResult, SearchResults, Space, Updated, clean_excerpt, file_names,
    page_ref_site, parse_page_ref, search_cql,
};
pub use error::{Error, Result};
