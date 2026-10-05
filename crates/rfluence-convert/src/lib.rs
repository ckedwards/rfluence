//! Pure markdown <-> Confluence ADF conversion. No network or filesystem access.
//! See design.md for the rules this implements.

pub mod adf;
mod approx;
pub mod annotations;
pub mod anchors;
mod diagnostics;
pub mod frontmatter;
mod html_table;
pub mod emoji;
mod inline;
pub mod labels;
pub mod language;
pub mod links;
mod markdown;
mod normalize;
mod page;
pub mod select;
pub mod settings;
mod to_adf;
mod to_md;

pub use diagnostics::{Diagnostic, Severity};
pub use normalize::normalize;
pub use page::{PageMeta, frontmatter, page_markdown, simplified_frontmatter, starts_with_h1, upload_title};
pub use to_adf::{Error, MermaidApp, PageRef, Upload, UploadContext, check, local_images, local_links, markdown_to_adf};
pub use to_md::{FetchContext, LinkTarget, adf_to_markdown, is_merfluence};
