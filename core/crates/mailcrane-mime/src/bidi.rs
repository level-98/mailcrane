//! Unicode bidirectional controls (UAX #9). In a name or address they can
//! make text display in a different order than it is stored, so
//! `"\u{202E}moc.elpmaxe@oec"` shows as `ceo@example.com`. They have no
//! place in a From name, so they are removed before display and before any
//! check.

/// Whether `c` is a bidi embedding, override, isolate or mark: U+202A–U+202E
/// (LRE, RLE, PDF, LRO, RLO), U+2066–U+2069 (LRI, RLI, FSI, PDI), U+200E and
/// U+200F (LRM, RLM) and U+061C (ALM).
pub fn is_bidi_control(c: char) -> bool {
    matches!(
        c,
        '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{200E}' | '\u{200F}' | '\u{061C}'
    )
}

/// `text` without bidi controls.
pub fn strip_bidi_controls(text: &str) -> String {
    text.chars().filter(|c| !is_bidi_control(*c)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_bidi_control_is_removed() {
        for (name, c) in [
            ("LRE", '\u{202A}'),
            ("RLE", '\u{202B}'),
            ("PDF", '\u{202C}'),
            ("LRO", '\u{202D}'),
            ("RLO", '\u{202E}'),
            ("LRI", '\u{2066}'),
            ("RLI", '\u{2067}'),
            ("FSI", '\u{2068}'),
            ("PDI", '\u{2069}'),
            ("LRM", '\u{200E}'),
            ("RLM", '\u{200F}'),
            ("ALM", '\u{061C}'),
        ] {
            assert!(is_bidi_control(c), "{name}");
            assert_eq!(strip_bidi_controls(&format!("a{c}b{c}")), "ab", "{name}");
        }
    }

    #[test]
    fn ordinary_text_is_kept() {
        for text in [
            "Ann Example",
            "שלום עולם",
            "مرحبا",
            "Zoë 例え",
            "a\u{200B}b",
        ] {
            assert_eq!(strip_bidi_controls(text), text);
        }
        assert!(!is_bidi_control('\u{2029}'));
        assert!(!is_bidi_control('\u{2065}'));
        assert!(!is_bidi_control('\u{202F}'));
    }
}
