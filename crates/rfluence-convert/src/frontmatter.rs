//! Reading frontmatter from markdown files, and replacing only its `rfluence:` block.
//! See design.md, "Frontmatter" > "Reading and writing".

/// The `rfluence:` fields rfluence reads back from a file.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RfluenceFields {
    pub id: Option<String>,
    pub space_key: Option<String>,
    pub parent: Option<String>,
    pub title: Option<String>,
    pub version: Option<u64>,
    pub url: Option<String>,
    pub labels: Vec<String>,
    pub weight: Option<i64>,
    pub simplified: bool,
    pub partial: bool,
}

/// A file split into its frontmatter (the YAML between the `---` lines, if any) and body.
#[derive(Debug, Clone, PartialEq)]
pub struct Document<'a> {
    pub yaml: Option<&'a str>,
    pub body: &'a str,
}

/// Split a markdown file into frontmatter YAML and body (the blank line after the closing
/// `---` belongs to neither).
pub fn split(md: &str) -> Document<'_> {
    if let Some(rest) = md.strip_prefix("---\n") {
        let end = if rest.starts_with("---\n") { Some(0) } else { rest.find("\n---\n").map(|e| e + 1) };
        if let Some(end) = end {
            let yaml = &rest[..end];
            let body = &rest[end + 4..];
            return Document { yaml: Some(yaml), body: body.strip_prefix('\n').unwrap_or(body) };
        }
    }
    Document { yaml: None, body: md }
}

/// The `rfluence:` fields of a frontmatter block. Invalid YAML or a missing block gives
/// the defaults.
pub fn rfluence_fields(yaml: &str) -> RfluenceFields {
    let Ok(value) = serde_norway::from_str::<serde_norway::Value>(yaml) else { return RfluenceFields::default() };
    let Some(rf) = value.get("rfluence") else { return RfluenceFields::default() };
    let string = |k: &str| match rf.get(k) {
        Some(serde_norway::Value::String(s)) => Some(s.clone()),
        Some(serde_norway::Value::Number(n)) => Some(n.to_string()),
        _ => None,
    };
    let labels = match rf.get("labels") {
        Some(serde_norway::Value::Sequence(items)) => items
            .iter()
            .filter_map(|v| match v {
                serde_norway::Value::String(s) => Some(s.clone()),
                serde_norway::Value::Number(n) => Some(n.to_string()),
                serde_norway::Value::Bool(b) => Some(b.to_string()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    };
    RfluenceFields {
        id: string("id"),
        space_key: string("space_key"),
        parent: string("parent"),
        title: string("title"),
        url: string("url"),
        labels,
        version: rf.get("version").and_then(serde_norway::Value::as_u64),
        weight: rf.get("weight").and_then(serde_norway::Value::as_i64),
        simplified: rf.get("simplified").and_then(serde_norway::Value::as_bool).unwrap_or(false),
        partial: rf.get("partial").and_then(serde_norway::Value::as_bool).unwrap_or(false),
    }
}

/// Keys under `rfluence:` that rfluence doesn't know (typos), which upload refuses.
pub fn unknown_keys(yaml: &str) -> Vec<String> {
    const KNOWN: [&str; 11] =
        ["id", "space_key", "parent", "title", "version", "url", "labels", "weight", "updated", "simplified", "partial"];
    let Ok(value) = serde_norway::from_str::<serde_norway::Value>(yaml) else { return Vec::new() };
    let Some(serde_norway::Value::Mapping(rf)) = value.get("rfluence") else { return Vec::new() };
    rf.keys()
        .map(|k| k.as_str().map_or_else(|| format!("{k:?}"), str::to_string))
        .filter(|k| !KNOWN.contains(&k.as_str()))
        .collect()
}

/// Merge a freshly fetched frontmatter (`---\nrfluence:\n...\n---\n`) into an existing file's
/// frontmatter: its `rfluence:` block is replaced (keeping a user-set `weight`), every other
/// line is kept byte for byte. Without existing frontmatter, the fetched one is used as is.
pub fn merge(existing_yaml: Option<&str>, fetched: &str) -> String {
    let Some(existing) = existing_yaml else { return fetched.to_string() };
    let fetched_yaml = split(fetched).yaml.unwrap_or_default();
    let mut block: Vec<&str> = fetched_yaml.lines().collect();
    let weight = rfluence_fields(existing).weight.map(|w| format!("  weight: {w}"));
    if let Some(w) = &weight {
        block.push(w);
    }

    let lines: Vec<&str> = existing.lines().collect();
    let start = lines.iter().position(|l| l.trim_end() == "rfluence:" || l.starts_with("rfluence:"));
    let mut out: Vec<&str> = Vec::new();
    match start {
        Some(s) => {
            // The block is the key line and every following indented, blank or comment line.
            let mut e = s + 1;
            while e < lines.len() && (lines[e].starts_with([' ', '\t']) || lines[e].trim().is_empty()) {
                e += 1;
            }
            // Keep trailing blank lines outside the block.
            while e > s + 1 && lines[e - 1].trim().is_empty() {
                e -= 1;
            }
            out.extend(&lines[..s]);
            out.extend(&block);
            out.extend(&lines[e..]);
        }
        None => {
            out.extend(&lines);
            out.extend(&block);
        }
    }
    format!("---\n{}\n---\n", out.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const FETCHED: &str = "---\nrfluence:\n  id: \"1\"\n  version: 8\n---\n";

    #[test]
    fn splits_files() {
        let d = split("---\ntitle: x\n---\n\n# Body\n");
        assert_eq!((d.yaml, d.body), (Some("title: x\n"), "# Body\n"));
        assert_eq!(split("# No frontmatter\n").yaml, None);
        assert_eq!(split("---\n---\nbody\n").yaml, Some(""));
    }

    #[test]
    fn reads_rfluence_fields() {
        let f = rfluence_fields("title: x\nrfluence:\n  id: \"123\"\n  version: 7\n  weight: -2\n  labels: [a, b-c]\n  title: \"T: x\"\n");
        assert_eq!(
            f,
            RfluenceFields {
                id: Some("123".into()),
                version: Some(7),
                weight: Some(-2),
                labels: vec!["a".into(), "b-c".into()],
                title: Some("T: x".into()),
                ..Default::default()
            }
        );
        assert_eq!(rfluence_fields("rfluence:\n  id: 123\n").id.as_deref(), Some("123"));
        assert!(rfluence_fields("rfluence:\n  simplified: true\n").simplified);
        assert_eq!(rfluence_fields(": not yaml ["), RfluenceFields::default());
        assert_eq!(unknown_keys("rfluence:\n  id: \"1\"\n  lables: [a]\nother: 1\n"), ["lables"]);
    }

    #[test]
    fn replaces_only_the_rfluence_block() {
        let existing = "# my notes\ntitle: Kept\nrfluence:\n  id: \"1\"\n  version: 7\n  weight: 10 # first\ntags: [a, b]\n";
        assert_eq!(
            merge(Some(existing), FETCHED),
            "---\n# my notes\ntitle: Kept\nrfluence:\n  id: \"1\"\n  version: 8\n  weight: 10\ntags: [a, b]\n---\n"
        );
        assert_eq!(merge(Some("title: Kept\n"), FETCHED), "---\ntitle: Kept\nrfluence:\n  id: \"1\"\n  version: 8\n---\n");
        assert_eq!(merge(None, FETCHED), FETCHED);
    }
}
