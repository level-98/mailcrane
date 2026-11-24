//! Modified UTF-7 mailbox names (RFC 3501 §5.1.3).
//!
//! Printable US-ASCII other than `&` stands for itself, `&-` is `&`, and
//! anything else is UTF-16BE in modified base64 between `&` and `-`.

use crate::b64;

/// Why a name is not valid modified UTF-7.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mutf7Error;

/// Decodes a mailbox name from modified UTF-7.
pub fn decode(name: &str) -> Result<String, Mutf7Error> {
    let bytes = name.as_bytes();
    let mut out = String::with_capacity(bytes.len());
    let mut i = 0;
    while let Some(&b) = bytes.get(i) {
        if b == b'&' {
            let rest = bytes.get(i + 1..).ok_or(Mutf7Error)?;
            let end = rest.iter().position(|&c| c == b'-').ok_or(Mutf7Error)?;
            let encoded = rest.get(..end).ok_or(Mutf7Error)?;
            if encoded.is_empty() {
                out.push('&');
            } else {
                let raw = b64::decode_modified(encoded).ok_or(Mutf7Error)?;
                if raw.len() % 2 != 0 {
                    return Err(Mutf7Error);
                }
                let units = raw
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|p| u16::from_be_bytes(*p));
                let mut decoded = String::new();
                for c in char::decode_utf16(units) {
                    let c = c.map_err(|_| Mutf7Error)?;
                    // RFC 3501 §5.1.3: printable ASCII must not be encoded.
                    if (' '..='~').contains(&c) {
                        return Err(Mutf7Error);
                    }
                    decoded.push(c);
                }
                out.push_str(&decoded);
            }
            i = i.checked_add(end + 2).ok_or(Mutf7Error)?;
        } else if (0x20..=0x7e).contains(&b) {
            out.push(char::from(b));
            i += 1;
        } else {
            return Err(Mutf7Error);
        }
    }
    Ok(out)
}

/// Encodes a mailbox name as modified UTF-7.
pub fn encode(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut pending: Vec<u16> = Vec::new();
    let flush = |pending: &mut Vec<u16>, out: &mut String| {
        if !pending.is_empty() {
            let bytes: Vec<u8> = pending.iter().flat_map(|u| u.to_be_bytes()).collect();
            out.push('&');
            out.push_str(&b64::encode_modified(&bytes));
            out.push('-');
            pending.clear();
        }
    };
    for c in name.chars() {
        if (' '..='~').contains(&c) {
            flush(&mut pending, &mut out);
            if c == '&' {
                out.push_str("&-");
            } else {
                out.push(c);
            }
        } else {
            let mut buf = [0u16; 2];
            pending.extend_from_slice(c.encode_utf16(&mut buf));
        }
    }
    flush(&mut pending, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    // The example in RFC 3501 §5.1.3.
    const RFC_ENCODED: &str = "~peter/mail/&U,BTFw-/&ZeVnLIqe-";
    const RFC_DECODED: &str = "~peter/mail/\u{53f0}\u{5317}/\u{65e5}\u{672c}\u{8a9e}";

    #[test]
    fn rfc_3501_example_decodes() {
        assert_eq!(decode(RFC_ENCODED).as_deref(), Ok(RFC_DECODED));
    }

    #[test]
    fn rfc_3501_example_encodes() {
        assert_eq!(encode(RFC_DECODED), RFC_ENCODED);
    }

    #[test]
    fn ampersand_and_ascii() {
        assert_eq!(decode("Tom &- Jerry").as_deref(), Ok("Tom & Jerry"));
        assert_eq!(encode("Tom & Jerry"), "Tom &- Jerry");
        assert_eq!(encode("INBOX"), "INBOX");
        assert_eq!(decode("Re&AOc-us").as_deref(), Ok("Re\u{e7}us"));
        assert_eq!(encode("Re\u{e7}us"), "Re&AOc-us");
    }

    #[test]
    fn astral_characters_use_surrogate_pairs() {
        let name = "Mail \u{1F426}";
        let encoded = encode(name);
        assert_eq!(encoded, "Mail &2D3cJg-");
        assert_eq!(decode(&encoded).as_deref(), Ok(name));
    }

    #[test]
    fn invalid_names_are_errors() {
        for bad in [
            "&",         // unterminated
            "&U,BTFw",   // unterminated
            "&AGE-",     // encodes printable ASCII "a"
            "&2D0-",     // lone surrogate
            "caf\u{e9}", // raw 8-bit
            "tab\there", // control character
            "&A-",       // too short for a byte
            "&AOc",      // missing '-'
        ] {
            assert_eq!(decode(bad), Err(Mutf7Error), "{bad:?}");
        }
    }

    proptest! {
        #[test]
        fn round_trips(name in "\\PC*") {
            let encoded = encode(&name);
            prop_assert!(encoded.bytes().all(|b| (0x20..=0x7e).contains(&b)));
            prop_assert_eq!(decode(&encoded), Ok(name));
        }

        #[test]
        fn decode_never_panics(name in ".*") {
            let _ = decode(&name);
        }
    }
}
