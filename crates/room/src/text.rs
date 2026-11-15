//! Everything a person types passes through here before it is stored or sent on.

/// Trims, drops control characters, caps runs of blank lines, and cuts to `max` characters.
/// Returns `None` when nothing is left.
pub fn clean_text(raw: &str, max: usize) -> Option<String> {
    let mut out = String::with_capacity(raw.len().min(max * 4));
    let mut newlines = 0u8;
    let mut count = 0usize;
    for c in raw.trim().chars() {
        if count >= max {
            break;
        }
        let c = match c {
            '\r' => continue,
            '\n' => {
                newlines += 1;
                if newlines > 2 {
                    continue;
                }
                '\n'
            }
            '\t' => ' ',
            // Control and invisible direction marks: they can hide or reorder what is shown.
            c if c.is_control()
                || matches!(c, '\u{200B}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{FEFF}') =>
            {
                continue;
            }
            c => c,
        };
        if c != '\n' {
            newlines = 0;
        }
        out.push(c);
        count += 1;
    }
    let trimmed = out.trim_end();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

/// A name: one line, at most 24 characters.
pub fn clean_name(raw: &str) -> Option<String> {
    let one_line: String = raw
        .chars()
        .map(|c| if c == '\n' { ' ' } else { c })
        .collect();
    let cleaned = clean_text(&one_line, 24)?;
    let squeezed = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    (!squeezed.is_empty()).then_some(squeezed)
}

/// An emoji or short mark used as a reaction: no spaces, nothing a keyboard sentence would be.
pub fn clean_mark(raw: &str) -> Option<&str> {
    let ok = !raw.is_empty()
        && raw.len() <= 32
        && raw.chars().count() <= 8
        && !raw
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || c.is_ascii_alphanumeric());
    ok.then_some(raw)
}

/// Compares two secrets without stopping at the first difference.
pub fn same_secret(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() || a.is_empty() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// A random URL-safe token of `bytes` bytes of entropy.
pub fn token(bytes: usize) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut raw = vec![0u8; bytes];
    getrandom::fill(&mut raw).expect("the system has no randomness");
    // 6 bits per character keeps every character uniform.
    let mut out = String::with_capacity(bytes * 4 / 3 + 1);
    let (mut acc, mut bits) = (0u32, 0u32);
    for b in raw {
        acc = (acc << 8) | u32::from(b);
        bits += 8;
        while bits >= 6 {
            bits -= 6;
            out.push(ALPHABET[((acc >> bits) & 63) as usize] as char);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_is_trimmed_and_capped() {
        assert_eq!(clean_text("  hi  ", 10).as_deref(), Some("hi"));
        assert_eq!(clean_text("abcdef", 3).as_deref(), Some("abc"));
        assert_eq!(clean_text("   \n\t ", 10), None);
        assert_eq!(clean_text("a\n\n\n\n\nb", 20).as_deref(), Some("a\n\nb"));
    }

    #[test]
    fn hidden_characters_are_removed() {
        assert_eq!(
            clean_text("a\u{202E}b\u{0007}c\u{200B}", 10).as_deref(),
            Some("abc")
        );
    }

    #[test]
    fn long_emoji_text_is_cut_by_characters_not_bytes() {
        let s = "😀".repeat(10);
        assert_eq!(clean_text(&s, 4).unwrap().chars().count(), 4);
    }

    #[test]
    fn names_are_one_line() {
        assert_eq!(
            clean_name("  Ana \n Marija  ").as_deref(),
            Some("Ana Marija")
        );
        assert_eq!(clean_name("\n"), None);
        assert_eq!(clean_name(&"x".repeat(80)).unwrap().len(), 24);
    }

    #[test]
    fn marks_are_emoji_like() {
        assert!(clean_mark("❤️").is_some());
        assert!(clean_mark("👍🏽").is_some());
        assert!(clean_mark("hello").is_none());
        assert!(clean_mark("").is_none());
        assert!(clean_mark("a b").is_none());
    }

    #[test]
    fn secrets_compare_fully() {
        assert!(same_secret("abc", "abc"));
        assert!(!same_secret("abc", "abd"));
        assert!(!same_secret("abc", "ab"));
        assert!(!same_secret("", ""));
    }

    #[test]
    fn tokens_are_long_and_differ() {
        let (a, b) = (token(18), token(18));
        assert_eq!(a.len(), 24);
        assert_ne!(a, b);
        assert!(
            a.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        );
    }
}
