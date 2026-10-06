//! The local project around a markdown file: its root, and which files hold which pages.
//! See design.md, "Links" > "Fetch".

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

use rfluence_convert::{LinkTarget, frontmatter, select};

/// The project root: the nearest directory (from `dir` up) with a `.rfluence.yaml`, else `dir`.
pub fn root(dir: &Path) -> PathBuf {
    dir.ancestors().find(|d| d.join(".rfluence.yaml").is_file()).unwrap_or(dir).to_path_buf()
}

/// The pages held by markdown files under `root` (skipping files ignored by `.gitignore`),
/// as link targets relative to `from_dir`. `skip` (the file being written) is left out.
pub fn pages(root: &Path, from_dir: &Path, skip: &Path) -> HashMap<String, LinkTarget> {
    let mut pages = HashMap::new();
    for entry in ignore::WalkBuilder::new(root).build().flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "md") || same_file(path, skip) {
            continue;
        }
        let Ok(text) = crate::text::read(path) else { continue };
        let doc = frontmatter::split(&text);
        let Some(id) = doc.yaml.map(frontmatter::rfluence_fields).and_then(|f| f.id) else { continue };
        pages.insert(id, LinkTarget { path: relative(from_dir, path), headings: select::heading_titles(doc.body) });
    }
    pages
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// `to` relative to the directory `from`, with `/` separators: `./setup.md`, `../api/auth.md`.
pub fn relative(from: &Path, to: &Path) -> String {
    let abs = |p: &Path| std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf());
    let (from, to) = (normalize(&abs(from)), normalize(&abs(to)));
    let common = from.iter().zip(to.iter()).take_while(|(a, b)| a == b).count();
    let mut parts: Vec<String> = vec!["..".into(); from.len() - common];
    parts.extend(to[common..].iter().cloned());
    let joined = parts.join("/");
    if joined.starts_with("..") { joined } else { format!("./{joined}") }
}

/// Path components with `.` and `..` resolved.
fn normalize(p: &Path) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for c in p.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::Normal(s) => out.push(s.to_string_lossy().into_owned()),
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_paths() {
        assert_eq!(relative(Path::new("/p/docs"), Path::new("/p/docs/setup.md")), "./setup.md");
        assert_eq!(relative(Path::new("/p/docs/guides"), Path::new("/p/docs/api/auth.md")), "../api/auth.md");
        assert_eq!(relative(Path::new("/p/docs/./x/.."), Path::new("/p/readme.md")), "../readme.md");
    }
}
