//! Unicode bidirectional formatting characters in text from a sender.
//!
//! An override such as U+202E (RLO) makes everything after it display right
//! to left, so `invoice\u{202E}fdp.exe` shows as `invoiceexe.pdf`. Text from a
//! message that the app shows on its own line (a subject, an attachment name,
//! a sender's name) has no use for these characters, so they are removed
//! before display (#315, #314). Text inside a message body is left alone:
//! right-to-left languages rely on them there, and the body renders in a
//! sandboxed view.

use std::borrow::Cow;

/// Whether `c` is a bidi embedding, override, isolate or mark: U+202A–U+202E
/// (LRE, RLE, PDF, LRO, RLO), U+2066–U+2069 (LRI, RLI, FSI, PDI), U+200E and
/// U+200F (LRM, RLM) and U+061C (ALM).
pub fn is_bidi_control(c: char) -> bool {
    matches!(
        c,
        '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{200e}' | '\u{200f}' | '\u{061c}'
    )
}

/// `text` without bidi controls ([`is_bidi_control`]), so it displays in the
/// order it is stored. Borrows when there is nothing to remove.
pub fn strip_bidi(text: &str) -> Cow<'_, str> {
    if text.chars().any(is_bidi_control) {
        Cow::Owned(text.chars().filter(|&c| !is_bidi_control(c)).collect())
    } else {
        Cow::Borrowed(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removes_overrides_isolates_and_marks() {
        let cases = [
            ("RLO", "invoice\u{202e}fdp.exe", "invoicefdp.exe"),
            ("LRO", "\u{202d}abc\u{202c}", "abc"),
            ("RLI", "pay \u{2067}cba\u{2069} now", "pay cba now"),
            ("FSI", "\u{2068}x\u{2069}", "x"),
            ("PDI alone", "a\u{2069}b", "ab"),
            ("LRE/RLE/PDF", "\u{202a}a\u{202b}b\u{202c}", "ab"),
            ("LRI", "\u{2066}a", "a"),
            ("marks", "a\u{200e}b\u{200f}c\u{061c}d", "abcd"),
        ];
        for (what, input, want) in cases {
            assert_eq!(strip_bidi(input), want, "{what}");
        }
    }

    #[test]
    fn keeps_right_to_left_letters_and_borrows_clean_text() {
        let hebrew = "שלום עולם";
        assert!(matches!(strip_bidi(hebrew), Cow::Borrowed(s) if s == hebrew));
        let arabic = "مرحبا.pdf";
        assert_eq!(strip_bidi(arabic), arabic);
        // Zero-width joiners matter for emoji and some scripts; not bidi.
        assert_eq!(strip_bidi("👩\u{200d}💻"), "👩\u{200d}💻");
    }

    #[test]
    fn detects_each_control() {
        for c in [
            '\u{202a}', '\u{202e}', '\u{2066}', '\u{2069}', '\u{200e}', '\u{200f}', '\u{061c}',
        ] {
            assert!(is_bidi_control(c), "{:04X}", u32::from(c));
        }
        for c in ['a', '\u{2029}', '\u{2065}', '\u{200d}', '\u{202f}'] {
            assert!(!is_bidi_control(c), "{:04X}", u32::from(c));
        }
    }
}
