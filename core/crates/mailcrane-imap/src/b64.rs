//! Base64 (RFC 4648 §4), plus the modified alphabet of RFC 3501 §5.1.3,
//! which uses `,` in place of `/` and no padding.

const STANDARD: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
const MODIFIED: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+,";

fn encode_with(input: &[u8], alphabet: &[u8; 64], pad: bool) -> String {
    let mut out = String::with_capacity(input.len().div_ceil(3).saturating_mul(4));
    let sextet = |n: u32| char::from(alphabet.get((n & 63) as usize).copied().unwrap_or(b'A'));
    for chunk in input.chunks(3) {
        let b0 = u32::from(chunk.first().copied().unwrap_or(0));
        let b1 = u32::from(chunk.get(1).copied().unwrap_or(0));
        let b2 = u32::from(chunk.get(2).copied().unwrap_or(0));
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(sextet(n >> 18));
        out.push(sextet(n >> 12));
        if chunk.len() > 1 {
            out.push(sextet(n >> 6));
        } else if pad {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(sextet(n));
        } else if pad {
            out.push('=');
        }
    }
    out
}

/// Standard base64 with padding, for `AUTHENTICATE` (RFC 9051 §6.2.2).
pub(crate) fn encode(input: &[u8]) -> String {
    encode_with(input, STANDARD, true)
}

/// Modified base64 without padding (RFC 3501 §5.1.3).
pub(crate) fn encode_modified(input: &[u8]) -> String {
    encode_with(input, MODIFIED, false)
}

/// Decodes modified base64 without padding. `None` if any character is
/// outside the alphabet or the leftover bits are not zero.
pub(crate) fn decode_modified(input: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(input.len().saturating_mul(3) / 4);
    let mut acc: u32 = 0;
    let mut bits: u32 = 0;
    for &c in input {
        let value = MODIFIED.iter().position(|&a| a == c)?;
        acc = ((acc << 6) | value as u32) & 0xFFFF;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((acc >> bits) & 0xFF) as u8);
        }
    }
    // Leftover bits must be padding zeros, and fewer than 6 of them.
    if bits >= 6 || acc & ((1 << bits) - 1) != 0 {
        return None;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_vectors() {
        // RFC 4648 §10.
        for (input, expected) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(encode(input.as_bytes()), expected);
        }
    }

    #[test]
    fn modified_round_trip() {
        let data: Vec<u8> = (0..=255).collect();
        for len in 0..data.len() {
            let slice = &data[..len];
            let encoded = encode_modified(slice);
            assert!(!encoded.contains('/') && !encoded.contains('='));
            assert_eq!(decode_modified(encoded.as_bytes()).as_deref(), Some(slice));
        }
    }

    #[test]
    fn modified_rejects_bad_input() {
        assert_eq!(decode_modified(b"A/"), None);
        assert_eq!(decode_modified(b"AB=="), None);
        // "AB" leaves 4 non-zero bits.
        assert_eq!(decode_modified(b"AB"), None);
        // A single character cannot make a byte.
        assert_eq!(decode_modified(b"A"), None);
    }
}
