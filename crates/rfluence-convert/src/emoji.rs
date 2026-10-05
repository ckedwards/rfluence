//! Emoji matching. See design.md, "Emoji".
//!
//! Markdown holds emoji as Unicode characters (`🎉`). Shortcodes (`:tada:`, `:+1:`) are
//! accepted as input and normalized to characters. Emoji data (names, shortcodes, skin tones)
//! comes from the `emojis` crate: Confluence renders an emoji node from its `id` and `text`,
//! so Atlassian's own names aren't needed (verified on test page 720904).

use std::ops::Range;

/// An emoji, in its fully-qualified form (`❤️`, not `❤`).
pub type Emoji = &'static emojis::Emoji;

const VS16: char = '\u{FE0F}';

/// Look up a shortcode (GitHub's names, e.g. `tada`, `+1`), with or without the colons.
pub fn lookup(name: &str) -> Option<Emoji> {
    let name = name.strip_prefix(':').and_then(|n| n.strip_suffix(':')).unwrap_or(name);
    emojis::get_by_shortcode(name)
}

/// Look up an emoji by its characters (with or without variation selectors).
pub fn get(text: &str) -> Option<Emoji> {
    emojis::get(text)
}

/// The node `id` Confluence uses: the codepoints in hex, joined by `-` (`1f44d-1f3fd`).
pub fn id(e: Emoji) -> String {
    e.as_str().chars().map(|c| format!("{:x}", c as u32)).collect::<Vec<_>>().join("-")
}

/// The characters for a node `id`, if they are an emoji.
pub fn from_id(id: &str) -> Option<Emoji> {
    let text: Option<String> = id.split('-').map(|cp| u32::from_str_radix(cp, 16).ok().and_then(char::from_u32)).collect();
    get(&text?)
}

/// The node `shortName`: the emoji's GitHub shortcode (the base emoji's, for a skin-tone
/// variant), or the characters when it has none. Confluence doesn't depend on it.
pub fn short_name(e: Emoji) -> String {
    let base = e.skin_tone().and_then(|_| e.with_skin_tone(emojis::SkinTone::Default)).unwrap_or(e);
    match base.shortcode() {
        Some(code) => format!(":{code}:"),
        None => e.as_str().to_string(),
    }
}

/// Characters that are emoji by default (Unicode `Emoji_Presentation`) below U+1F000.
/// Everything else below U+1F000 (`©`, `✔`, `⛹`) is plain text unless followed by the emoji
/// variation selector.
fn default_emoji(c: char) -> bool {
    matches!(c as u32,
        0x231A..=0x231B | 0x23E9..=0x23EC | 0x23F0 | 0x23F3 | 0x25FD..=0x25FE | 0x2614..=0x2615
        | 0x2648..=0x2653 | 0x267F | 0x2693 | 0x26A1 | 0x26AA..=0x26AB | 0x26BD..=0x26BE
        | 0x26C4..=0x26C5 | 0x26CE | 0x26D4 | 0x26EA | 0x26F2..=0x26F3 | 0x26F5 | 0x26FA | 0x26FD
        | 0x2705 | 0x270A..=0x270B | 0x2728 | 0x274C | 0x274E | 0x2753..=0x2755 | 0x2757
        | 0x2795..=0x2797 | 0x27B0 | 0x27BF | 0x2B1B..=0x2B1C | 0x2B50 | 0x2B55)
}

/// Would these characters be shown as an emoji (not as text)?
fn is_emoji_presentation(s: &str) -> bool {
    s.chars().any(|c| matches!(c, VS16 | '\u{20E3}' | '\u{200D}') || c as u32 >= 0x1F000)
        || s.chars().next().is_some_and(default_emoji)
}

/// The longest emoji sequence, in chars (family and subdivision-flag sequences).
const MAX_CHARS: usize = 12;

/// Emoji in text, longest match first (so skin tones, flags and joined sequences match
/// whole), skipping characters written as text (a plain `©`).
pub fn find_emoji(text: &str) -> Vec<(Range<usize>, Emoji)> {
    let mut found = Vec::new();
    let mut i = 0;
    while i < text.len() {
        let c = text[i..].chars().next().expect("i is a char boundary");
        // Emoji start with a symbol or, for keycaps, `#`, `*` or a digit.
        if (c as u32) < 0x2000 && !matches!(c, '#' | '*' | '0'..='9' | '©' | '®') {
            i += c.len_utf8();
            continue;
        }
        let ends: Vec<usize> = text[i..].char_indices().map(|(j, ch)| i + j + ch.len_utf8()).take(MAX_CHARS).collect();
        let hit = ends.iter().rev().find_map(|&end| {
            let e = get(&text[i..end])?;
            // A trailing variation selector belongs to the emoji.
            let end = if text[end..].starts_with(VS16) { end + VS16.len_utf8() } else { end };
            is_emoji_presentation(&text[i..end]).then_some((i..end, e))
        });
        match hit {
            Some((range, e)) => {
                i = range.end;
                found.push((range, e));
            }
            None => i += c.len_utf8(),
        }
    }
    found
}

/// Is `text` exactly one emoji?
pub fn is_emoji(text: &str) -> bool {
    matches!(find_emoji(text).as_slice(), [(r, _)] if *r == (0..text.len()))
}

/// A syntactic shortcode in text: `:name:`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shortcode<'a> {
    /// Byte range of the whole shortcode, colons included.
    pub range: Range<usize>,
    /// The name without the colons, e.g. `tada`.
    pub name: &'a str,
}

fn is_name_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '+' | '-')
}

/// Find shortcode-shaped text: `:name:` where `name` is letters, digits, `_`, `+` or `-`, not
/// directly preceded or followed by a letter or digit (so `10:30:45` and `1:100:3` don't match).
/// Whether the name is a known emoji is up to the caller.
pub fn find_shortcodes(text: &str) -> Vec<Shortcode<'_>> {
    let mut found = Vec::new();
    let mut i = 0;
    while let Some(off) = text[i..].find(':') {
        let start = i + off;
        if text[..start].chars().next_back().is_some_and(char::is_alphanumeric) {
            i = start + 1;
            continue;
        }
        let rest = &text[start + 1..];
        let name_len: usize = rest.chars().take_while(|&c| is_name_char(c)).map(char::len_utf8).sum();
        let end = start + 1 + name_len + 1;
        if name_len == 0
            || !rest[name_len..].starts_with(':')
            || text[end..].chars().next().is_some_and(char::is_alphanumeric)
        {
            i = start + 1;
            continue;
        }
        found.push(Shortcode { range: start..end, name: &text[start + 1..end - 1] });
        i = end;
    }
    found
}

/// Rewrite emoji in text to the form fetch writes: known shortcodes and emoji characters as
/// fully-qualified characters. Custom emoji shortcodes are left alone.
pub fn canonical_text(text: &str) -> String {
    let mut hits: Vec<(Range<usize>, Emoji)> = find_shortcodes(text)
        .into_iter()
        .filter_map(|sc| Some((sc.range, lookup(sc.name)?)))
        .chain(find_emoji(text))
        .collect();
    hits.sort_by_key(|(r, _)| r.start);
    let mut out = String::with_capacity(text.len());
    let mut pos = 0;
    for (range, e) in hits {
        if range.start < pos {
            continue;
        }
        out.push_str(&text[pos..range.start]);
        out.push_str(e.as_str());
        pos = range.end;
    }
    out.push_str(&text[pos..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(text: &str) -> Vec<&str> {
        find_shortcodes(text).into_iter().map(|s| s.name).collect()
    }

    #[test]
    fn finds_shortcodes_with_boundaries() {
        assert_eq!(names(":tada: and :+1:, (:rocket:)"), ["tada", "+1", "rocket"]);
        assert_eq!(names("10:30:45 a:b:c 1:100:3 https://x.y"), Vec::<&str>::new());
        assert_eq!(names(":not_an_emoji: :piñata:"), ["not_an_emoji", "piñata"]);
        assert_eq!(names("a :b c: d"), Vec::<&str>::new());
    }

    #[test]
    fn maps_between_characters_ids_and_names() {
        let thumbs = get("👍🏽").unwrap();
        assert_eq!(id(thumbs), "1f44d-1f3fd");
        assert_eq!(short_name(thumbs), ":+1:");
        assert_eq!(from_id("1f44d-1f3fd").unwrap().as_str(), "👍🏽");
        assert_eq!(from_id("2764").unwrap().as_str(), "❤\u{fe0f}");
        assert!(from_id("8c4f3c94-1ade-4ce8-8b3a-0fdc390d2f04").is_none());
        assert_eq!(lookup(":memo:").unwrap().as_str(), "📝");
        assert!(lookup("not_an_emoji").is_none());
    }

    #[test]
    fn finds_emoji_characters() {
        fn found(t: &str) -> Vec<(&str, &str)> {
            find_emoji(t).into_iter().map(|(r, e)| (&t[r], e.as_str())).collect()
        }
        assert_eq!(found("ok 🎉 and 👍🏽!"), [("🎉", "🎉"), ("👍🏽", "👍🏽")]);
        assert_eq!(found("😮\u{200d}💨 ✅ 🇳🇿 🫨"), [("😮\u{200d}💨", "😮\u{200d}💨"), ("✅", "✅"), ("🇳🇿", "🇳🇿"), ("🫨", "🫨")]);
        // Text-style characters are emoji only with the variation selector.
        assert!(found("© 2026, ✔ done, 1 # *").is_empty());
        assert_eq!(found("✔\u{fe0f} ❤\u{fe0f}"), [("✔\u{fe0f}", "✔\u{fe0f}"), ("❤\u{fe0f}", "❤\u{fe0f}")]);
        assert!(is_emoji("🎉") && !is_emoji(":rfluence:") && !is_emoji("🎉 x"));
    }

    #[test]
    fn canonicalizes_text() {
        assert_eq!(canonical_text("yes :+1: :tada: :rfluence: ⛹ 10:30"), "yes 👍 🎉 :rfluence: ⛹ 10:30");
        assert_eq!(canonical_text("❤ ⛹\u{fe0f} 1\u{20e3}"), "❤ ⛹\u{fe0f} 1\u{fe0f}\u{20e3}");
    }
}
