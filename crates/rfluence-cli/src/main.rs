//! The `rf` command. See design.md, "Commands".

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use rfluence_convert::{Diagnostic, Severity};
use serde::Serialize;

#[derive(Parser)]
#[command(name = "rf", version, about = "Fetch, search and upload Confluence pages as markdown")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Check markdown files for content Confluence can't store exactly.
    ///
    /// Warnings are uploaded as the closest Confluence equivalent (e.g. `<kbd>` as inline
    /// code). Errors can't be represented in Confluence, and upload refuses them.
    /// Exits 1 if there are errors.
    Check {
        /// Markdown files to check.
        #[arg(required = true)]
        paths: Vec<PathBuf>,
        /// Print diagnostics as JSON.
        #[arg(long)]
        json: bool,
    },
}

/// Exit codes (design.md, "Output and errors").
const EXIT_ERRORS_FOUND: u8 = 1;
const EXIT_USAGE: u8 = 2;

fn main() -> ExitCode {
    match Cli::parse().command {
        Command::Check { paths, json } => check(&paths, json),
    }
}

#[derive(Serialize)]
struct FileDiagnostic {
    path: String,
    #[serde(flatten)]
    diagnostic: Diagnostic,
}

fn check(paths: &[PathBuf], json: bool) -> ExitCode {
    let mut all = Vec::new();
    for path in paths {
        let md = match std::fs::read_to_string(path) {
            Ok(md) => md,
            Err(e) => {
                eprintln!("rf: {}: {e}", path.display());
                return ExitCode::from(EXIT_USAGE);
            }
        };
        let mut diags = rfluence_convert::check(&md);
        diags.extend(missing_images(path, &md));
        diags.sort_by_key(|d| d.line);
        all.extend(diags.into_iter().map(|diagnostic| FileDiagnostic { path: path.display().to_string(), diagnostic }));
    }

    let errors = all.iter().filter(|d| d.diagnostic.severity == Severity::Error).count();
    if json {
        println!("{}", serde_json::to_string_pretty(&all).expect("diagnostics serialize"));
    } else {
        for d in &all {
            println!("{}:{}: {}: {}", d.path, d.diagnostic.line, d.diagnostic.severity, d.diagnostic.message);
        }
        let warnings = all.len() - errors;
        let files = paths.len();
        match (errors, warnings) {
            (0, 0) => println!("ok: {files} file{} checked", plural(files)),
            _ => println!("{errors} error{}, {warnings} warning{}", plural(errors), plural(warnings)),
        }
    }
    if errors > 0 { ExitCode::from(EXIT_ERRORS_FOUND) } else { ExitCode::SUCCESS }
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
