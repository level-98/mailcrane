//! Short preview text for message lists.

/// How many characters the snippet holds.
pub const SNIPPET_CHARS: usize = 200;

/// The first [`SNIPPET_CHARS`] characters of `text`, with every run of
/// whitespace (and control characters) collapsed to one space.
pub fn snippet(text: &str) -> String {
    let mut out = String::new();
    let mut count = 0;
    let mut pending_space = false;
    for c in text.chars() {
        if c.is_whitespace() || c.is_control() {
            pending_space = !out.is_empty();
            continue;
        }
        if pending_space {
            out.push(' ');
            count += 1;
            pending_space = false;
        }
        if count >= SNIPPET_CHARS {
            break;
        }
        out.push(c);
        count += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collapses_whitespace() {
        assert_eq!(snippet("  a \r\n\t b  "), "a b");
    }

    #[test]
    fn caps_length() {
        assert_eq!(snippet(&"x".repeat(500)).chars().count(), SNIPPET_CHARS);
        assert!(snippet(&"x ".repeat(500)).chars().count() <= SNIPPET_CHARS);
    }
}
