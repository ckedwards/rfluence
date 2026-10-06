//! Confluence-only settings stored in markdown: `key=value` pairs in a fence info string
//! (```` ```shell width=4000 ````) or an `rf:` HTML comment (`<!-- rf: layout=center -->`).
//! See design.md, "Confluence-only settings in markdown".

/// Ordered `key=value` settings. A key without `=` is a flag (`header-column`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings(Vec<(String, Option<String>)>);

impl Settings {
    pub fn new() -> Self {
        Settings::default()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn set(&mut self, key: &str, value: impl ToString) {
        self.0.push((key.to_string(), Some(value.to_string())));
    }

    pub fn flag(&mut self, key: &str) {
        self.0.push((key.to_string(), None));
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.0.iter().find(|(k, _)| k == key).and_then(|(_, v)| v.as_deref())
    }

    pub fn has(&self, key: &str) -> bool {
        self.0.iter().any(|(k, _)| k == key)
    }

    pub fn get_f64(&self, key: &str) -> Option<f64> {
        self.get(key).and_then(|v| v.parse().ok())
    }

    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.0.iter().map(|(k, _)| k.as_str())
    }

    /// Parse `key=value key2="quoted value" flag`.
    pub fn parse(s: &str) -> Settings {
        let mut out = Settings::new();
        let mut chars = s.chars().peekable();
        loop {
            while chars.peek().is_some_and(|c| c.is_whitespace()) {
                chars.next();
            }
            if chars.peek().is_none() {
                break;
            }
            let mut key = String::new();
            while let Some(&c) = chars.peek() {
                if c.is_whitespace() || c == '=' {
                    break;
                }
                key.push(c);
                chars.next();
            }
            if chars.peek() != Some(&'=') {
                out.0.push((key, None));
                continue;
            }
            chars.next();
            let mut value = String::new();
            if chars.peek() == Some(&'"') {
                chars.next();
                while let Some(c) = chars.next() {
                    match c {
                        '"' => break,
                        '\\' => value.extend(chars.next()),
                        c => value.push(c),
                    }
                }
            } else {
                while let Some(&c) = chars.peek() {
                    if c.is_whitespace() {
                        break;
                    }
                    value.push(c);
                    chars.next();
                }
            }
            out.0.push((key, Some(value)));
        }
        out
    }

    /// Parse an `rf:` HTML comment; `None` if `html` isn't one.
    pub fn parse_comment(html: &str) -> Option<Settings> {
        let inner = html.trim().strip_prefix("<!--")?.strip_suffix("-->")?.trim();
        Some(Settings::parse(inner.strip_prefix("rf:")?))
    }

    /// Render as an `rf:` HTML comment.
    pub fn to_comment(&self) -> String {
        format!("<!-- rf: {self} -->")
    }
}

/// Canonical key order, so settings written in any order normalize to fetch's order.
const KEY_ORDER: &[&str] = &[
    "card", "columns", "layout", "breakout", "width", "width-type", "colwidths", "numbered", "no-header-row",
    "header-column", "align", "indent", "border", "border-color", "caption", "theme", "mermaidVersion",
    "useMaxWidth", "column", "end-columns", "tabs", "tab", "title", "end-tabs", "synced-block", "id", "page", "read-only",
    "unavailable", "end-synced-block",
];

fn key_rank(key: &str) -> usize {
    KEY_ORDER.iter().position(|k| *k == key).unwrap_or(KEY_ORDER.len())
}

impl std::fmt::Display for Settings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut entries: Vec<_> = self.0.iter().collect();
        entries.sort_by_key(|(k, _)| key_rank(k));
        for (i, (key, value)) in entries.into_iter().enumerate() {
            if i > 0 {
                f.write_str(" ")?;
            }
            f.write_str(key)?;
            let Some(value) = value else { continue };
            let plain = !value.is_empty() && !value.chars().any(|c| c.is_whitespace() || matches!(c, '"' | '\\' | '>'));
            if plain {
                write!(f, "={value}")?;
            } else {
                // Escape `"` and `\`, and break up `--` so the value can't end the HTML comment.
                let escaped = value.replace('\\', "\\\\").replace('"', "\\\"").replace("--", "-\\-");
                write!(f, "=\"{escaped}\"")?;
            }
        }
        Ok(())
    }
}

/// Format a number without a trailing `.0` (`760.0` -> `760`).
pub fn fmt_num(n: f64) -> String {
    if n.fract() == 0.0 { format!("{}", n as i64) } else { format!("{n}") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_formats() {
        let s = Settings::parse(r#"layout=center width=1070 caption="A -- \"quoted\" caption" header-column"#);
        assert_eq!(s.get("layout"), Some("center"));
        assert_eq!(s.get_f64("width"), Some(1070.0));
        assert_eq!(s.get("caption"), Some(r#"A -- "quoted" caption"#));
        assert!(s.has("header-column"));
        assert_eq!(Settings::parse(&s.to_string()).to_string(), s.to_string());
        assert!(!s.to_comment()[4..s.to_comment().len() - 3].contains("--"));
    }

    #[test]
    fn writes_keys_in_canonical_order() {
        assert_eq!(Settings::parse("width=1070 layout=center").to_string(), "layout=center width=1070");
        assert_eq!(Settings::parse("numbered colwidths=1,2 zzz=1").to_string(), "colwidths=1,2 numbered zzz=1");
    }

    #[test]
    fn parses_comments() {
        let s = Settings::parse_comment("<!-- rf: card=embed width=100 -->").unwrap();
        assert_eq!(s.get("card"), Some("embed"));
        assert!(Settings::parse_comment("<!-- end list -->").is_none());
    }
}
