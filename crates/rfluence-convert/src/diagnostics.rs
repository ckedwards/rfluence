//! What `rfluence check` and upload report about markdown Confluence can't store exactly.

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// Uploaded as the closest Confluence equivalent; normalize makes the same change, so
    /// round trips still hold.
    Warning,
    /// Can't be represented in Confluence. Upload refuses unless forced.
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Diagnostic {
    pub line: usize,
    pub severity: Severity,
    pub message: String,
}

impl Diagnostic {
    pub fn warning(line: usize, message: impl Into<String>) -> Self {
        Diagnostic { line, severity: Severity::Warning, message: message.into() }
    }

    pub fn error(line: usize, message: impl Into<String>) -> Self {
        Diagnostic { line, severity: Severity::Error, message: message.into() }
    }
}

impl std::fmt::Display for Severity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Severity::Warning => "warning",
            Severity::Error => "error",
        })
    }
}

impl std::fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "line {}: {}: {}", self.line, self.severity, self.message)
    }
}
