//! Reading and writing the user's text files. rfluence works with `\n` line endings; files with
//! Windows line endings (`\r\n`) are read as `\n`, and keep `\r\n` when rfluence rewrites them.

use std::path::Path;

/// A text file's contents, with `\r\n` as `\n`.
pub fn read(path: impl AsRef<Path>) -> std::io::Result<String> {
    let text = std::fs::read_to_string(path)?;
    Ok(if text.contains("\r\n") { text.replace("\r\n", "\n") } else { text })
}

/// Write a file via a temporary file and a rename, so an interrupted write never leaves half
/// a file. If the file being replaced has `\r\n` line endings, the new contents get them too.
pub fn write_atomically(path: &Path, contents: &str) -> std::io::Result<()> {
    let crlf = std::fs::read(path).is_ok_and(|old| old.windows(2).any(|w| w == b"\r\n"));
    let contents = if crlf { contents.replace("\r\n", "\n").replace('\n', "\r\n") } else { contents.to_string() };
    let tmp = path.with_extension("md.rfluence-tmp");
    std::fs::write(&tmp, contents)?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_windows_line_endings() {
        let dir = std::env::temp_dir().join(format!("rfluence-text-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (crlf, lf) = (dir.join("crlf.md"), dir.join("lf.md"));
        std::fs::write(&crlf, "---\r\nrfluence:\r\n  id: \"1\"\r\n---\r\n\r\n# Title\r\n").unwrap();
        std::fs::write(&lf, "# Title\n").unwrap();
        assert_eq!(read(&crlf).unwrap(), "---\nrfluence:\n  id: \"1\"\n---\n\n# Title\n");
        write_atomically(&crlf, "# New\n\nBody\n").unwrap();
        assert_eq!(std::fs::read_to_string(&crlf).unwrap(), "# New\r\n\r\nBody\r\n");
        write_atomically(&lf, "# New\n").unwrap();
        assert_eq!(std::fs::read_to_string(&lf).unwrap(), "# New\n");
        let new = dir.join("new.md");
        write_atomically(&new, "# New\n").unwrap();
        assert_eq!(std::fs::read_to_string(&new).unwrap(), "# New\n");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
