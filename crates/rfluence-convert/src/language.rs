//! Code block languages. See design.md, "Code block languages and widths".

/// The language Confluence stores for a fence language: lowercased, with Confluence's
/// renames applied (verified on test page 98404). `None` for an empty language, which
/// Confluence removes.
pub fn canonical(language: &str) -> Option<String> {
    let lower = language.to_lowercase();
    let renamed = match lower.as_str() {
        "" => return None,
        "bash" => "shell",
        "js" => "javascript",
        "py" => "python",
        "cpp" => "c++",
        "c#" => "csharp",
        "vb" => "visualbasic",
        "text" => "plaintext",
        other => other,
    };
    Some(renamed.to_string())
}

#[cfg(test)]
mod tests {
    use super::canonical;

    #[test]
    fn applies_confluence_renames() {
        assert_eq!(canonical("Bash").as_deref(), Some("shell"));
        assert_eq!(canonical("JS").as_deref(), Some("javascript"));
        assert_eq!(canonical("golang").as_deref(), Some("golang"));
        assert_eq!(canonical(""), None);
    }
}
