//! Labels, normalized and checked before upload. See design.md, "Labels".

/// A label as Confluence stores it: lowercase, spaces as `-`. Characters Confluence rejects
/// (with no reason given) or would split the label on (`,`) are reported here instead.
pub fn normalize_label(label: &str) -> Result<String, String> {
    let normalized: String = label
        .trim()
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("-");
    if normalized.is_empty() {
        return Err("empty label".into());
    }
    if let Some(bad) = normalized
        .chars()
        .find(|c| !(c.is_alphanumeric() || matches!(c, '-' | '_' | '/')))
    {
        return Err(format!(
            "label {label:?} contains {bad:?}; labels can only have letters, digits, `-`, `_` and `/`"
        ));
    }
    Ok(normalized)
}

#[cfg(test)]
mod tests {
    use super::normalize_label;

    #[test]
    fn normalizes_like_confluence() {
        assert_eq!(normalize_label("UPPER").unwrap(), "upper");
        assert_eq!(normalize_label("two words").unwrap(), "two-words");
        assert_eq!(normalize_label("ünïcode").unwrap(), "ünïcode");
        assert_eq!(normalize_label("a/b_c-1").unwrap(), "a/b_c-1");
        for bad in [
            "comma,label",
            "dot.ted",
            "colon:x",
            "amp&",
            "hash#",
            "paren(",
            "bang!",
            "",
        ] {
            assert!(normalize_label(bad).is_err(), "{bad}");
        }
    }
}
