//! Reads the Rust core out of a built APK, to check which core it packages.
//!
//! An APK is a zip file. AGP stores native libraries uncompressed (so Android
//! can map them in place), which lets this read them without a zip library:
//! find the central directory, find the entry, slice out its bytes.

/// The UniFFI symbol that only a core built with `debug-fixtures` exports
/// ("Load test mail", #289).
pub const DEBUG_ONLY_SYMBOL: &[u8] = b"uniffi_mailcrane_ffi_fn_func_load_test_mail";

/// The core as packaged for one ABI.
pub const CORE_ENTRY: &str = "lib/x86_64/libmailcrane_ffi.so";

const EOCD_SIG: u32 = 0x0605_4b50;
const CENTRAL_SIG: u32 = 0x0201_4b50;
const LOCAL_SIG: u32 = 0x0403_4b50;
const EOCD_LEN: usize = 22;
const STORED: u16 = 0;

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    let s: [u8; 2] = b.get(at..at.checked_add(2)?)?.try_into().ok()?;
    Some(u16::from_le_bytes(s))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    let s: [u8; 4] = b.get(at..at.checked_add(4)?)?.try_into().ok()?;
    Some(u32::from_le_bytes(s))
}

/// The bytes of the stored (uncompressed) entry `name` in the zip `zip`.
pub fn stored_entry<'a>(zip: &'a [u8], name: &str) -> Result<&'a [u8], String> {
    let bad = || "not a zip file (or a zip64 one)".to_string();
    // The end-of-central-directory record is last, before a comment of at
    // most 64 KiB.
    let last = zip.len().checked_sub(EOCD_LEN).ok_or_else(bad)?;
    let first = last.saturating_sub(usize::from(u16::MAX));
    let eocd = (first..=last)
        .rev()
        .find(|&at| u32_at(zip, at) == Some(EOCD_SIG))
        .ok_or_else(bad)?;
    let entries = u16_at(zip, eocd.checked_add(10).ok_or_else(bad)?).ok_or_else(bad)?;
    let cd = u32_at(zip, eocd.checked_add(16).ok_or_else(bad)?).ok_or_else(bad)?;
    let mut at = usize::try_from(cd).map_err(|_| bad())?;
    for _ in 0..entries {
        if u32_at(zip, at) != Some(CENTRAL_SIG) {
            return Err(bad());
        }
        let field = |off: usize| u16_at(zip, at.checked_add(off)?).map(usize::from);
        let method = u16_at(zip, at.checked_add(10).ok_or_else(bad)?).ok_or_else(bad)?;
        let size = u32_at(zip, at.checked_add(20).ok_or_else(bad)?).ok_or_else(bad)?;
        let (name_len, extra_len, comment_len) = (
            field(28).ok_or_else(bad)?,
            field(30).ok_or_else(bad)?,
            field(32).ok_or_else(bad)?,
        );
        let local = u32_at(zip, at.checked_add(42).ok_or_else(bad)?).ok_or_else(bad)?;
        let name_at = at.checked_add(46).ok_or_else(bad)?;
        let entry_name = zip
            .get(name_at..name_at.checked_add(name_len).ok_or_else(bad)?)
            .ok_or_else(bad)?;
        if entry_name == name.as_bytes() {
            if method != STORED {
                return Err(format!(
                    "{name} is compressed in the APK; expected it stored"
                ));
            }
            return local_data(zip, local, size).ok_or_else(bad);
        }
        at = name_at
            .checked_add(name_len)
            .and_then(|n| n.checked_add(extra_len))
            .and_then(|n| n.checked_add(comment_len))
            .ok_or_else(bad)?;
    }
    Err(format!("{name} is not in the APK"))
}

/// The data of the entry whose local header starts at `local`.
fn local_data(zip: &[u8], local: u32, size: u32) -> Option<&[u8]> {
    let at = usize::try_from(local).ok()?;
    if u32_at(zip, at)? != LOCAL_SIG {
        return None;
    }
    let name_len = usize::from(u16_at(zip, at.checked_add(26)?)?);
    let extra_len = usize::from(u16_at(zip, at.checked_add(28)?)?);
    let start = at
        .checked_add(30)?
        .checked_add(name_len)?
        .checked_add(extra_len)?;
    zip.get(start..start.checked_add(usize::try_from(size).ok()?)?)
}

/// Whether `haystack` contains `needle`.
pub fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty() && haystack.windows(needle.len()).any(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A zip of stored entries, laid out the way zip writers do it.
    fn zip(entries: &[(&str, &[u8], u16)]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut central = Vec::new();
        for (name, data, method) in entries {
            let offset = out.len() as u32;
            out.extend_from_slice(&LOCAL_SIG.to_le_bytes());
            out.extend_from_slice(&[20, 0, 0, 0]);
            out.extend_from_slice(&method.to_le_bytes());
            out.extend_from_slice(&[0; 8]); // time, date, crc
            out.extend_from_slice(&(data.len() as u32).to_le_bytes());
            out.extend_from_slice(&(data.len() as u32).to_le_bytes());
            out.extend_from_slice(&(name.len() as u16).to_le_bytes());
            out.extend_from_slice(&3u16.to_le_bytes()); // extra, as AGP's alignment
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(&[0; 3]);
            out.extend_from_slice(data);

            central.extend_from_slice(&CENTRAL_SIG.to_le_bytes());
            central.extend_from_slice(&[20, 0, 20, 0, 0, 0]);
            central.extend_from_slice(&method.to_le_bytes());
            central.extend_from_slice(&[0; 8]);
            central.extend_from_slice(&(data.len() as u32).to_le_bytes());
            central.extend_from_slice(&(data.len() as u32).to_le_bytes());
            central.extend_from_slice(&(name.len() as u16).to_le_bytes());
            central.extend_from_slice(&[0; 12]); // extra, comment, disk, attrs
            central.extend_from_slice(&offset.to_le_bytes());
            central.extend_from_slice(name.as_bytes());
        }
        let cd_offset = out.len() as u32;
        out.extend_from_slice(&central);
        out.extend_from_slice(&EOCD_SIG.to_le_bytes());
        out.extend_from_slice(&[0; 4]);
        out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        out.extend_from_slice(&(central.len() as u32).to_le_bytes());
        out.extend_from_slice(&cd_offset.to_le_bytes());
        out.extend_from_slice(&[0, 0]);
        out
    }

    #[test]
    fn finds_a_stored_entry_after_others() {
        let z = zip(&[
            ("classes.dex", b"load_test_mail in the bindings", STORED),
            (CORE_ENTRY, b"\x7fELF core", STORED),
        ]);
        assert_eq!(stored_entry(&z, CORE_ENTRY), Ok(&b"\x7fELF core"[..]));
    }

    #[test]
    fn refuses_compressed_missing_and_broken() {
        let z = zip(&[(CORE_ENTRY, b"x", 8)]);
        assert!(stored_entry(&z, CORE_ENTRY).is_err_and(|e| e.contains("compressed")));
        let z = zip(&[("other", b"x", STORED)]);
        assert!(stored_entry(&z, CORE_ENTRY).is_err_and(|e| e.contains("not in the APK")));
        assert!(stored_entry(b"PK", CORE_ENTRY).is_err());
        let mut z = zip(&[(CORE_ENTRY, b"x", STORED)]);
        let len = z.len();
        z[len - 6] = 0xff; // central directory offset past the end
        assert!(stored_entry(&z, CORE_ENTRY).is_err());
    }

    #[test]
    fn contains_finds_the_symbol() {
        let core = [b"\0\0".as_slice(), DEBUG_ONLY_SYMBOL, b"\0"].concat();
        assert!(contains(&core, DEBUG_ONLY_SYMBOL));
        assert!(!contains(
            b"uniffi_mailcrane_ffi_fn_func_load_test",
            DEBUG_ONLY_SYMBOL
        ));
        assert!(!contains(b"abc", b""));
    }
}
