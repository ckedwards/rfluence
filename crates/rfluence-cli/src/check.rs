//! `rfluence check`: what upload would approximate or can't represent, without network access.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use rfluence_convert::{Diagnostic, Severity};
use serde::Serialize;

use crate::{EXIT_ERRORS_FOUND, EXIT_USAGE};

#[derive(Serialize)]
struct FileDiagnostic {
    path: String,
    #[serde(flatten)]
    diagnostic: Diagnostic,
}

/// Check the files; exit 1 if there are errors (or any diagnostics, with
/// `warnings_are_errors`).
pub fn run(paths: &[PathBuf], json: bool, warnings_are_errors: bool) -> ExitCode {
    let mut all = Vec::new();
    for path in paths {
        let md = match crate::text::read(path) {
            Ok(md) => md,
            Err(e) => {
                eprintln!("rfluence: {}: {e}", path.display());
                return ExitCode::from(EXIT_USAGE);
            }
        };
        let mut diags = rfluence_convert::check(&md);
        diags.extend(missing_images(path, &md));
        diags.sort_by_key(|d| d.line);
        all.extend(diags.into_iter().map(|diagnostic| FileDiagnostic {
            path: path.display().to_string(),
            diagnostic,
        }));
    }

    let errors = all
        .iter()
        .filter(|d| d.diagnostic.severity == Severity::Error)
        .count();
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&all).expect("diagnostics serialize")
        );
    } else {
        for d in &all {
            println!(
                "{}:{}: {}: {}",
                d.path, d.diagnostic.line, d.diagnostic.severity, d.diagnostic.message
            );
        }
        let warnings = all.len() - errors;
        let files = paths.len();
        match (errors, warnings) {
            (0, 0) => println!("ok: {files} file{} checked", plural(files)),
            _ => println!(
                "{errors} error{}, {warnings} warning{}",
                plural(errors),
                plural(warnings)
            ),
        }
    }
    let failed = errors > 0 || (warnings_are_errors && !all.is_empty());
    if failed {
        ExitCode::from(EXIT_ERRORS_FOUND)
    } else {
        ExitCode::SUCCESS
    }
}

/// Local images whose file doesn't exist next to the markdown file.
fn missing_images(path: &Path, md: &str) -> Vec<Diagnostic> {
    let dir = path.parent().unwrap_or(Path::new("."));
    rfluence_convert::local_images(md)
        .into_iter()
        .filter(|(_, src)| {
            let decoded = percent_encoding::percent_decode_str(src).decode_utf8_lossy();
            !dir.join(decoded.as_ref()).exists()
        })
        .map(|(line, src)| {
            Diagnostic::warning(
                line,
                format!("image file not found: {src} (upload reuses an attachment with that name if the page has one)"),
            )
        })
        .collect()
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}
