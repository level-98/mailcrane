//! "Load test mail" for debug builds (#289). Compiled only with the
//! `fixtures` feature, which only debug Android builds turn on (through
//! `mailcrane-ffi`'s `debug-fixtures`); release builds don't contain it.
//!
//! Imports the synthetic messages from `tests/fixtures` into the Inbox of a
//! local-only account, `fixtures@example.org` (files named `*.sent.eml` go
//! in its Sent folder, for conversations over two folders, #573). That
//! account never syncs and never connects anywhere (the store marks it
//! local-only and the connection code refuses it). Messages go through the
//! same path as synced mail:
//! parsed, stored with their snippet and attachment list, indexed for
//! search, threaded, and sanitised when opened.
//!
//! The fixtures are data, never instructions: some of them are hostile on
//! purpose (XSS attempts, header injection), which is why they're useful.

use std::fmt::Write as _;

use mailcrane_store::{
    AccountId, Flags, FolderId, FolderRole, NewAccount, NewFolder, NewMessageHeader, Security,
    Store, StoreError,
};

use crate::sync::{json_string, json_strings, save_body, SyncError};

/// The address of the fixtures account.
pub const FIXTURES_EMAIL: &str = "fixtures@example.org";
/// The display name of the fixtures account.
pub const FIXTURES_NAME: &str = "Fixtures";

/// Dates are spread evenly over about four months before now.
const SPREAD_SECONDS: i64 = 120 * 24 * 60 * 60;
/// Files larger than this are skipped (the fixtures are a few KB each).
const MAX_FILE_BYTES: usize = 5 * 1024 * 1024;
/// At most this many files are imported.
const MAX_FILES: usize = 2_000;

/// One fixture file: its path under `tests/fixtures` and its bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixtureFile {
    /// Path under `tests/fixtures`, with `/` separators, such as
    /// `html/link-mismatch.html`.
    pub path: String,
    /// The file's bytes.
    pub bytes: Vec<u8>,
}

/// What an import did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportReport {
    /// The fixtures account.
    pub account_id: AccountId,
    /// Its Inbox.
    pub inbox: FolderId,
    /// Its Sent folder (#573).
    pub sent: FolderId,
    /// Messages now in the Inbox and Sent.
    pub imported: usize,
    /// Files left out: not `.eml` or `.html`, too large, or over the limit.
    pub skipped: usize,
}

/// Why an import failed.
#[derive(Debug, thiserror::Error)]
pub enum FixtureError {
    /// The local database failed.
    #[error("storage error")]
    Storage(#[from] StoreError),
    /// Storing a message failed.
    #[error("could not store a message")]
    Store(#[from] SyncError),
}

/// Replaces the fixtures account's Inbox and Sent with `files` (`.eml` as
/// they are, `.html` wrapped in a simple message; `*.sent.eml` in Sent),
/// creating the local-only account and its folders the first time. Running
/// it again replaces the messages rather than adding to them. `now` is the
/// current time in Unix seconds.
///
/// Each message is stored with the store's usual calls, which commit one by
/// one, so a failure part-way can leave some messages in place; running it
/// again replaces them. Only the fixtures account is ever touched.
pub fn import_fixtures(
    store: &Store,
    files: &[FixtureFile],
    now: i64,
) -> Result<ImportReport, FixtureError> {
    let mut wanted: Vec<&FixtureFile> = files
        .iter()
        .filter(|f| kind_of(&f.path).is_some() && f.bytes.len() <= MAX_FILE_BYTES)
        .collect();
    wanted.sort_by(|a, b| a.path.cmp(&b.path));
    wanted.truncate(MAX_FILES);
    let skipped = files.len().saturating_sub(wanted.len());

    let account_id = match store.find_local_account(FIXTURES_EMAIL)? {
        Some(id) => id,
        None => store.insert_local_account(&fixtures_account())?,
    };
    let inbox = store.upsert_folder(&inbox_folder(account_id, 0))?;
    let sent = store.upsert_folder(&sent_folder(account_id, 0))?;
    store.forget_folder(inbox)?;
    store.forget_folder(sent)?;

    let count = i64::try_from(wanted.len()).unwrap_or(i64::MAX).max(1);
    let mut imported = 0usize;
    // The last UID used in the Inbox and in Sent.
    let (mut inbox_uid, mut sent_uid) = (0u32, 0u32);
    for (index, file) in wanted.iter().enumerate() {
        let n = i64::try_from(index).unwrap_or(i64::MAX);
        let date = now.saturating_sub(SPREAD_SECONDS.saturating_mul(n) / count);
        let (folder, last) = if is_sent(&file.path) {
            (sent, &mut sent_uid)
        } else {
            (inbox, &mut inbox_uid)
        };
        *last = last.saturating_add(1);
        let raw = match kind_of(&file.path) {
            Some(Kind::Html) => wrap_html(&file.path, &file.bytes, date),
            _ => file.bytes.clone(),
        };
        let header = header_for(folder, *last, &file.path, &raw, date);
        let id = store.upsert_message_header(&header)?;
        save_body(store, id, None, &raw)?;
        imported = imported.saturating_add(1);
    }
    // Marks both folders as synced up to their last message, as a real sync
    // would.
    store.upsert_folder(&inbox_folder(
        account_id,
        i64::from(inbox_uid).saturating_add(1),
    ))?;
    store.upsert_folder(&sent_folder(
        account_id,
        i64::from(sent_uid).saturating_add(1),
    ))?;
    crate::threading::rebuild(store, account_id)?;

    Ok(ImportReport {
        account_id,
        inbox,
        sent,
        imported,
        skipped,
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Eml,
    Html,
}

fn kind_of(path: &str) -> Option<Kind> {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".eml") {
        Some(Kind::Eml)
    } else if lower.ends_with(".html") {
        Some(Kind::Html)
    } else {
        None
    }
}

/// Whether a fixture goes in Sent: its file name ends in `.sent.eml`
/// (tests/fixtures/conversations/README.md, #573).
fn is_sent(path: &str) -> bool {
    path.to_ascii_lowercase().ends_with(".sent.eml")
}

fn fixtures_account() -> NewAccount {
    // No servers: the account is local-only, and nothing connects for it.
    NewAccount {
        display_name: FIXTURES_NAME.into(),
        email: FIXTURES_EMAIL.into(),
        imap_host: String::new(),
        imap_port: 0,
        imap_security: Security::Tls,
        smtp_host: String::new(),
        smtp_port: 0,
        smtp_security: Security::Tls,
        username: String::new(),
    }
}

/// The Inbox; `uidnext` 0 leaves it unsynced (before the import).
fn inbox_folder(account_id: AccountId, uidnext: i64) -> NewFolder {
    folder(account_id, "INBOX", "Inbox", FolderRole::Inbox, uidnext)
}

/// The Sent folder (#573); `uidnext` 0 leaves it unsynced.
fn sent_folder(account_id: AccountId, uidnext: i64) -> NewFolder {
    folder(account_id, "Sent", "Sent", FolderRole::Sent, uidnext)
}

fn folder(
    account_id: AccountId,
    remote_name: &str,
    display_name: &str,
    role: FolderRole,
    uidnext: i64,
) -> NewFolder {
    let synced = uidnext > 0;
    NewFolder {
        account_id,
        remote_name: remote_name.into(),
        display_name: display_name.into(),
        delimiter: Some("/".into()),
        role: Some(role),
        uidvalidity: synced.then_some(1),
        uidnext: synced.then_some(uidnext),
        highestmodseq: None,
        is_subscribed: true,
        selectable: true,
        virtual_role: None,
    }
}

/// The header row for a fixture, from the parsed message where it parses.
/// Dates come from `date` so the list spreads out; the subject falls back to
/// the file's path, so every message can be found.
fn header_for(folder: FolderId, uid: u32, path: &str, raw: &[u8], date: i64) -> NewMessageHeader {
    let parsed = mailcrane_mime::parse_message(raw).ok();
    let clean = |s: &str| -> String { s.chars().filter(|c| !c.is_control()).collect() };
    let subject = parsed
        .as_ref()
        .and_then(|p| p.subject.as_deref())
        .map(clean)
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| clean(path));
    let from = parsed.as_ref().and_then(|p| p.from.first());
    let addresses = |pick: fn(&mailcrane_mime::ParsedMessage) -> &Vec<mailcrane_mime::Address>| {
        parsed
            .as_ref()
            .map_or_else(|| "[]".to_owned(), |p| json_addresses(pick(p)))
    };
    NewMessageHeader {
        folder_id: folder,
        uid,
        message_id_header: parsed
            .as_ref()
            .and_then(|p| p.message_id.clone())
            .filter(|m| !m.is_empty()),
        in_reply_to: parsed.as_ref().and_then(|p| p.in_reply_to.first().cloned()),
        references_json: json_strings(
            parsed
                .as_ref()
                .map(|p| p.references.as_slice())
                .unwrap_or_default(),
        ),
        thread_id: None,
        subject,
        from_name: from
            .and_then(|a| a.name.as_deref())
            .map(clean)
            .filter(|n| !n.trim().is_empty()),
        from_addr: from.map(|a| clean(&a.email)),
        to_json: addresses(|p| &p.to),
        cc_json: addresses(|p| &p.cc),
        date,
        internal_date: date,
        size: i64::try_from(raw.len()).unwrap_or(i64::MAX),
        flags: Flags::default(),
        keywords_json: "[]".into(),
        // Both are set from the body by `save_body`.
        has_attachments: false,
        snippet: String::new(),
    }
}

/// `[{"name":..,"email":..}, ...]`, the shape sync stores.
fn json_addresses(list: &[mailcrane_mime::Address]) -> String {
    let mut out = String::from("[");
    for (i, a) in list.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str("{\"name\":");
        match a.name.as_deref().filter(|n| !n.trim().is_empty()) {
            Some(name) => json_string(name, &mut out),
            None => out.push_str("null"),
        }
        out.push_str(",\"email\":");
        json_string(&a.email, &mut out);
        out.push('}');
    }
    out.push(']');
    out
}

/// A simple HTML message around `html`, from and to the fixtures account,
/// with the file's path as its subject. The body is base64, so any bytes
/// and any line length survive.
fn wrap_html(path: &str, html: &[u8], date: i64) -> Vec<u8> {
    // The path is ours (asset names), but keep the header clean anyway.
    let subject: String = path.chars().filter(|c| !c.is_control()).collect();
    let id: String = path
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let mut out = String::new();
    let _ = write!(
        out,
        "From: \"{FIXTURES_NAME}\" <{FIXTURES_EMAIL}>\r\n\
         To: {FIXTURES_EMAIL}\r\n\
         Subject: {subject}\r\n\
         Date: {date}\r\n\
         Message-ID: <fixture-{id}@example.org>\r\n\
         MIME-Version: 1.0\r\n\
         Content-Type: text/html; charset=utf-8\r\n\
         Content-Transfer-Encoding: base64\r\n\
         \r\n",
        date = mailcrane_mime::format_date(date),
    );
    base64_lines(html, &mut out);
    out.into_bytes()
}

/// Base64 (RFC 2045 §6.8) in lines of 76 characters, each ending in CRLF.
fn base64_lines(bytes: &[u8], out: &mut String) {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let sextet = |v: u32| -> char {
        let i = usize::try_from(v & 0x3F).unwrap_or(0);
        ALPHABET.get(i).map_or('A', |&b| char::from(b))
    };
    // 57 input bytes make one 76-character line.
    for line in bytes.chunks(57) {
        for chunk in line.chunks(3) {
            let b0 = chunk.first().copied().map_or(0, u32::from);
            let b1 = chunk.get(1).copied().map_or(0, u32::from);
            let b2 = chunk.get(2).copied().map_or(0, u32::from);
            let n = (b0 << 16) | (b1 << 8) | b2;
            out.push(sextet(n >> 18));
            out.push(sextet(n >> 12));
            out.push(if chunk.len() > 1 { sextet(n >> 6) } else { '=' });
            out.push(if chunk.len() > 2 { sextet(n) } else { '=' });
        }
        out.push_str("\r\n");
    }
}

#[cfg(test)]
mod tests;
