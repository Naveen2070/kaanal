//! `.gitignore` handling for `kaanal init` (and `kaanal demo --eject`): add `.kaanal/` once,
//! never rewriting a byte that is already there.

use std::io::Write;
use std::path::Path;

use crate::STATE_DIR;

/// **Private.** A `.gitignore` line that already ignores the state directory.
fn ignores_state_dir(line: &str) -> bool {
    matches!(
        line.trim(),
        ".kaanal" | ".kaanal/" | "/.kaanal" | "/.kaanal/" | ".kaanal/*" | ".kaanal/**"
    )
}

/// **Private.** Adds `.kaanal/` to `.gitignore` (created when missing): the file is read only to
/// find the line, then appended to, so existing bytes are never rewritten, and in its own line
/// ending. `Ok(false)` when it was already listed; `Err` carries the reason it was left alone (a
/// file that cannot be read as UTF-8 is never touched).
pub fn ignore_state_dir(path: &Path) -> Result<bool, String> {
    let old = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(e) => return Err(e.to_string()),
    };
    let text = std::str::from_utf8(&old).map_err(|_| "it is not valid UTF-8".to_string())?;
    if text
        .strip_prefix('\u{feff}')
        .unwrap_or(text)
        .lines()
        .any(ignores_state_dir)
    {
        return Ok(false);
    }
    let nl = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let lead = if text.is_empty() || text.ends_with('\n') {
        ""
    } else {
        nl
    };
    std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(path)
        .and_then(|mut f| write!(f, "{lead}{STATE_DIR}/{nl}"))
        .map_err(|e| e.to_string())?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn scratch(name: &str) -> PathBuf {
        let d =
            std::env::temp_dir().join(format!("kaanal-init-unit-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn gitignore_is_appended_to_in_its_own_line_ending() {
        let dir = scratch("ignore");
        let f = dir.join(".gitignore");
        assert_eq!(ignore_state_dir(&f), Ok(true));
        assert_eq!(std::fs::read(&f).unwrap(), b".kaanal/\n");
        std::fs::write(&f, "a\r\nb").unwrap();
        assert_eq!(ignore_state_dir(&f), Ok(true));
        assert_eq!(std::fs::read(&f).unwrap(), b"a\r\nb\r\n.kaanal/\r\n");
        assert_eq!(ignore_state_dir(&f), Ok(false));
        std::fs::write(&f, "a").unwrap();
        assert_eq!(ignore_state_dir(&f), Ok(true));
        assert_eq!(std::fs::read(&f).unwrap(), b"a\n.kaanal/\n");
    }

    #[test]
    fn existing_ignore_lines_add_nothing() {
        let dir = scratch("variants");
        let f = dir.join(".gitignore");
        for line in [
            ".kaanal",
            ".kaanal/",
            "/.kaanal",
            "/.kaanal/",
            ".kaanal/*",
            ".kaanal/**",
            "  .kaanal/  ",
        ] {
            let text = format!("# c\n{line}\nx\n");
            std::fs::write(&f, &text).unwrap();
            assert_eq!(ignore_state_dir(&f), Ok(false), "{line}");
            assert_eq!(std::fs::read_to_string(&f).unwrap(), text);
        }
    }

    #[test]
    fn a_bom_on_the_first_gitignore_line_does_not_hide_the_entry() {
        let dir = scratch("bom");
        let f = dir.join(".gitignore");
        std::fs::write(&f, "\u{feff}.kaanal/\nx\n").unwrap();
        assert_eq!(ignore_state_dir(&f), Ok(false));
    }

    #[test]
    fn an_unreadable_gitignore_is_left_alone() {
        let dir = scratch("unreadable");
        let f = dir.join(".gitignore");
        let utf16: Vec<u8> = [0xff, 0xfe, b'a', 0, b'\n', 0].to_vec();
        std::fs::write(&f, &utf16).unwrap();
        assert!(ignore_state_dir(&f).is_err());
        assert_eq!(std::fs::read(&f).unwrap(), utf16);
        let d = dir.join("sub");
        std::fs::create_dir(&d).unwrap();
        assert!(ignore_state_dir(&d).is_err());
    }
}
