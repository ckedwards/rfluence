//! Pure markdown <-> Confluence ADF conversion. No network or filesystem access.
//! See design.md for the rules this implements.

pub mod adf;
pub mod anchors;
pub mod annotations;
mod approx;
mod diagnostics;
pub mod emoji;
pub mod frontmatter;
mod html_table;
mod inline;
pub mod labels;
pub mod language;
pub mod links;
mod markdown;
pub mod mermaid;
mod normalize;
mod page;
pub mod select;
pub mod settings;
pub mod synced;
mod to_adf;
mod to_md;

pub use diagnostics::{Diagnostic, Severity};
pub use mermaid::{MermaidApp, MermaidKind, is_merfluence};
pub use normalize::normalize;
pub use page::{
    PageMeta, frontmatter, page_markdown, simplified_frontmatter, starts_with_h1, upload_title,
};
pub use to_adf::{
    Error, PageRef, Upload, UploadContext, check, local_images, local_links, local_synced_copies,
    markdown_to_adf,
};
pub use to_md::{FetchContext, LinkTarget, adf_to_markdown};
