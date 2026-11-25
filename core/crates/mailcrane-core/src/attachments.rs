//! Saving attachments of received mail (M2-04).
//!
//! File names come from the sender and are hostile until cleaned:
//! [`sanitize_filename`] makes a name that can't leave the target folder,
//! hide its real extension or be a reserved device name, and
//! [`save_attachment`] writes the decoded part only inside `dest_dir`.
//!
//! Deviation from the spec: the part is decoded from the message body that
//! was already downloaded (with `BODY.PEEK[]`, so `\Seen` doesn't change)
//! instead of fetching `BODY.PEEK[<part>]` again. The message view always
//! downloads the body first, so this works offline too.

use mailcrane_store::{MessageId, Store, StoreError};
use std::io::Write;
use std::path::{Path, PathBuf};

/// Why an attachment couldn't be saved.
#[derive(Debug, thiserror::Error)]
pub enum AttachmentError {
    /// The message, its body or the part isn't stored.
    #[error("attachment not found")]
    NotFound,
    /// The body couldn't be read as a message.
    #[error("unreadable message")]
    Unreadable,
    /// Writing the file failed.
    #[error("could not write the file")]
    Io,
    /// The local database failed.
    #[error("store error: {0}")]
    Store(#[from] StoreError),
}

/// Windows device names, with or without an extension (`CON.txt` too).
const RESERVED: &[&str] = &[
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
    "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

/// Most bytes in a file name (common file systems allow 255).
const MAX_NAME_BYTES: usize = 255;

/// A safe file name made from a sender's: no path separators or `..`, no
/// control characters, no bidi controls (U+202A–U+202E, U+2066–U+2069,
/// U+200E/F, U+061C) that could disguise the extension, no leading dots,
/// no trailing spaces or dots, not a Windows reserved name, at most 255
/// bytes with the extension kept; `attachment` if nothing is left.
pub fn sanitize_filename(name: &str) -> String {
    // Only the last path component counts.
    let last = name.rsplit(['/', '\\']).next().unwrap_or_default();
    let mut cleaned: String = last
        .chars()
        .filter(|c| {
            !c.is_control()
                && !matches!(
                    c,
                    '\u{202a}'..='\u{202e}'
                        | '\u{2066}'..='\u{2069}'
                        | '\u{200e}'
                        | '\u{200f}'
                        | '\u{061c}'
                        | '\u{feff}'
                )
                && !matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*')
        })
        .collect();
    cleaned = cleaned
        .trim_start_matches(|c: char| c == '.' || c.is_whitespace())
        .trim_end_matches(|c: char| c == '.' || c.is_whitespace())
        .to_owned();
    if cleaned.is_empty() {
        return "attachment".to_owned();
    }
    let stem = cleaned
        .split('.')
        .next()
        .unwrap_or_default()
        .trim()
        .to_lowercase();
    if RESERVED.contains(&stem.as_str()) {
        cleaned = format!("_{cleaned}");
    }
    truncate_keeping_extension(&cleaned)
}

fn truncate_keeping_extension(name: &str) -> String {
    if name.len() <= MAX_NAME_BYTES {
        return name.to_owned();
    }
    let (stem, ext) = match name.rfind('.') {
        // A sensible extension: short and after some name.
        Some(i) if i > 0 && name.len() - i <= 16 => name.split_at(i),
        _ => (name, ""),
    };
    let room = MAX_NAME_BYTES - ext.len();
    let mut out = String::new();
    for c in stem.chars() {
        if out.len() + c.len_utf8() > room {
            break;
        }
        out.push(c);
    }
    out.push_str(ext);
    out
}

/// A path in `dir` for `name` that doesn't exist yet: `name`, then
/// `name (2).ext`, `name (3).ext` …
fn free_path(dir: &Path, name: &str) -> PathBuf {
    let first = dir.join(name);
    if !first.exists() {
        return first;
    }
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => name.split_at(i),
        _ => (name, ""),
    };
    for n in 2..10_000 {
        let candidate = dir.join(truncate_keeping_extension(&format!("{stem} ({n}){ext}")));
        if !candidate.exists() {
            return candidate;
        }
    }
    dir.join(format!("{stem}-{}{ext}", std::process::id()))
}

/// Decodes attachment `part_id` of `message` from its stored body and
/// writes it into `dest_dir` (which must exist) under a sanitised name,
/// never overwriting a file. Returns the path written.
pub fn save_attachment(
    store: &Store,
    message: MessageId,
    part_id: &str,
    dest_dir: &Path,
) -> Result<PathBuf, AttachmentError> {
    let raw = store.get_body(message)?.ok_or(AttachmentError::NotFound)?;
    let part = mailcrane_mime::attachment_part(&raw, part_id)
        .map_err(|_| AttachmentError::Unreadable)?
        .ok_or(AttachmentError::NotFound)?;
    let name = sanitize_filename(part.filename.as_deref().unwrap_or("attachment"));
    let dir = dest_dir.canonicalize().map_err(|_| AttachmentError::Io)?;
    let path = free_path(&dir, &name);
    // The name has no separators, but check anyway: the file must be a
    // direct child of the folder.
    if path.parent() != Some(dir.as_path()) {
        return Err(AttachmentError::Io);
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|_| AttachmentError::Io)?;
    file.write_all(&part.bytes)
        .map_err(|_| AttachmentError::Io)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hostile_names() {
        let long = format!("{}.pdf", "a".repeat(400));
        let cases: &[(&str, &str)] = &[
            ("report.pdf", "report.pdf"),
            ("../../etc/passwd", "passwd"),
            ("..\\x.txt", "x.txt"),
            ("C:\\Windows\\system.ini", "system.ini"),
            ("CON", "_CON"),
            ("con.txt", "_con.txt"),
            ("LPT9.doc", "_LPT9.doc"),
            ("console.txt", "console.txt"),
            ("a\u{202e}gpj.exe", "agpj.exe"),
            ("invoice\u{2066}fdp.scr", "invoicefdp.scr"),
            ("nul\0byte.txt", "nulbyte.txt"),
            ("line\nbreak\r.txt", "linebreak.txt"),
            ("  .hidden  ", "hidden"),
            ("...", "attachment"),
            ("trailing. . ", "trailing"),
            ("", "attachment"),
            ("/", "attachment"),
            ("what?.txt", "what.txt"),
            ("Été à Paris.jpg", "Été à Paris.jpg"),
        ];
        for (input, want) in cases {
            assert_eq!(sanitize_filename(input), *want, "{input:?}");
        }
        let long_out = sanitize_filename(&long);
        assert!(long_out.len() <= 255);
        assert!(long_out.ends_with(".pdf"));
        let multibyte = format!("{}.txt", "é".repeat(200));
        let out = sanitize_filename(&multibyte);
        assert!(out.len() <= 255 && out.ends_with(".txt"));
    }

    #[test]
    fn free_paths_never_overwrite() {
        let dir = tempfile::tempdir().expect("dir");
        let p1 = free_path(dir.path(), "a.txt");
        std::fs::write(&p1, b"1").expect("write");
        let p2 = free_path(dir.path(), "a.txt");
        assert_eq!(p2.file_name().and_then(|n| n.to_str()), Some("a (2).txt"));
    }
}
