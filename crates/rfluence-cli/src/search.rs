//! `rfluence search`: find pages. See design.md, "Commands".

use std::process::ExitCode;

use rfluence_client::{Client, auth, search_cql};

use crate::fail;

pub struct Options {
    pub query: Option<String>,
    pub space: Vec<String>,
    pub label: Vec<String>,
    pub cql: Option<String>,
    pub limit: usize,
    pub site: Option<String>,
    pub json: bool,
}

pub fn run(opts: &Options) -> ExitCode {
    let cql = match (&opts.cql, &opts.query) {
        (Some(cql), _) => cql.clone(),
        (None, Some(q)) => search_cql(q, &opts.space, &opts.label),
        (None, None) => unreachable!("clap requires a query or --cql"),
    };
    let found = auth::resolve(opts.site.as_deref()).and_then(|(creds, _)| Client::new(&creds).search(&cql, opts.limit));
    let found = match found {
        Ok(f) => f,
        Err(e) => return fail(&e),
    };
    if opts.json {
        println!("{}", serde_json::to_string_pretty(&found).expect("JSON serializes"));
        return ExitCode::SUCCESS;
    }
    if found.results.is_empty() {
        println!("No results.");
        return ExitCode::SUCCESS;
    }
    for r in &found.results {
        println!("{}  {}", r.id, r.title);
        let mut details = vec![r.space_key.clone()];
        if let Some(updated) = &r.updated {
            details.push(format!("updated {updated}"));
        }
        if !r.labels.is_empty() {
            details.push(format!("labels: {}", r.labels.join(", ")));
        }
        println!("  {}", details.join(" · "));
        println!("  {}", r.url);
        if !r.excerpt.is_empty() {
            println!("  {}", r.excerpt);
        }
        println!();
    }
    let shown = found.results.len() as u64;
    if found.total > shown {
        println!("{shown} of {} results (--limit for more). Read one with `rfluence fetch <id>`.", found.total);
    } else {
        println!("{shown} result{}. Read one with `rfluence fetch <id>`.", if shown == 1 { "" } else { "s" });
    }
    ExitCode::SUCCESS
}
