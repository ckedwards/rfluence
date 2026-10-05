//! Pure markdown <-> Confluence ADF conversion. No network or filesystem access.
//! See design.md for the rules this implements.

pub mod adf;
mod approx;
pub mod anchors;
mod diagnostics;
mod html_table;
pub mod emoji;
mod inline;
pub mod language;
mod markdown;
mod normalize;
pub mod settings;
mod to_adf;
mod to_md;

pub use diagnostics::{Diagnostic, Severity};
pub use normalize::normalize;
pub use to_adf::{Error, MermaidApp, Upload, UploadContext, check, local_images, markdown_to_adf};
pub use to_md::{FetchContext, adf_to_markdown, is_merfluence};
