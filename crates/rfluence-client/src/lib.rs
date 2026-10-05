//! Confluence API client and credentials for rfluence. See design.md, "rfluence (the command)".

mod api;
pub mod auth;
mod error;

pub use api::{Attachment, Client, Page, PageRef, file_names, page_ref_site, parse_page_ref};
pub use error::{Error, Result};
