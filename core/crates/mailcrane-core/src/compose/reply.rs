//! Reply, Reply All and Forward (M2-03): a prefilled [`ComposeDraft`] from a
//! stored message, with `In-Reply-To` and `References` set so threads stay
//! intact in every mail client (RFC 5322 §3.6.4).
//!
//! No user-facing text lives here: the apps pass the localised quote header
//! ("On 24 Nov 2026, at 10:42, Ana wrote:", or the forward intro line).

use std::collections::HashSet;

use mailcrane_mime::{format_date, parse_message, Address, ParsedMessage};
use mailcrane_store::{MessageId, Store};

use super::ComposeDraft;

/// What kind of draft to prepare.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplyKind {
    /// To the sender (or `Reply-To`).
    Reply,
    /// To the sender and every other recipient, except the account itself.
    ReplyAll,
    /// To nobody yet, with the original included.
    Forward,
}

/// Why a draft could not be prepared.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ReplyError {
    /// The message, its folder or its account is not in the database.
    #[error("message not found")]
    NotFound,
    /// The message body hasn't been downloaded yet; fetch it first.
    #[error("the message body is not downloaded")]
    BodyNotDownloaded,
    /// The local database failed.
    #[error("storage failure")]
    Storage,
}

/// `References` keeps at most this many ids: the first and the newest 19.
const MAX_REFERENCES: usize = 20;

/// Prepares a reply, reply-all or forward of `message`. `quote_header` is
/// the localised line above the quoted text (for a forward, the line above
/// the forwarded headers). Reads only the database.
pub fn prepare_reply(
    store: &Store,
    message: MessageId,
    kind: ReplyKind,
    quote_header: &str,
) -> Result<ComposeDraft, ReplyError> {
    let header = store
        .get_message_header(message)
        .map_err(|_| ReplyError::Storage)?
        .ok_or(ReplyError::NotFound)?;
    let folder = store
        .get_folder(header.details.folder_id)
        .map_err(|_| ReplyError::Storage)?
        .ok_or(ReplyError::NotFound)?;
    let account = store
        .get_account(folder.details.account_id)
        .map_err(|_| ReplyError::Storage)?
        .ok_or(ReplyError::NotFound)?;
    let raw = store
        .get_body(message)
        .map_err(|_| ReplyError::Storage)?
        .ok_or(ReplyError::BodyNotDownloaded)?;
    // A message too broken to parse still gets a reply, from what the
    // header row knows.
    let parsed = parse_message(&raw).unwrap_or_else(|_| ParsedMessage {
        subject: Some(header.details.subject.clone()),
        from: header
            .details
            .from_addr
            .clone()
            .map(|email| Address {
                name: header.details.from_name.clone(),
                email,
            })
            .into_iter()
            .collect(),
        message_id: header.details.message_id_header.clone(),
        ..ParsedMessage::default()
    });
    let mut draft = draft_for(
        &parsed,
        kind,
        quote_header,
        &account.details.email,
        folder.details.account_id,
        header.details.date,
    );
    // A forward takes the original's attachments along (M2-04); they are
    // read from its stored body when sending.
    let has_attachments = parsed
        .attachments
        .iter()
        .any(|a| a.disposition == mailcrane_mime::Disposition::Attachment);
    if kind == ReplyKind::Forward && has_attachments {
        draft.forward_attachments_of = Some(message);
    }
    Ok(draft)
}

/// The draft for `original`, sent from `own_email`. Pure, for tests.
fn draft_for(
    original: &ParsedMessage,
    kind: ReplyKind,
    quote_header: &str,
    own_email: &str,
    account_id: mailcrane_store::AccountId,
    stored_date: i64,
) -> ComposeDraft {
    let (to, cc) = recipients(original, kind, own_email);
    let subject = original.subject.as_deref().unwrap_or("").trim();
    let text = original.text_plain.as_deref().unwrap_or("");
    let (in_reply_to, references, subject, body_text) = match kind {
        ReplyKind::Reply | ReplyKind::ReplyAll => (
            original.message_id.clone(),
            references(original),
            prefixed(subject, "Re: ", &["re:"]),
            quoted(text, quote_header),
        ),
        // A forward starts a new thread.
        ReplyKind::Forward => (
            None,
            Vec::new(),
            prefixed(subject, "Fwd: ", &["fwd:", "fw:"]),
            forwarded(original, text, quote_header, stored_date),
        ),
    };
    ComposeDraft {
        account_id,
        to,
        cc,
        bcc: Vec::new(),
        subject,
        body_text,
        in_reply_to,
        references,
        attachments: Vec::new(),
        forward_attachments_of: None,
        rich: None,
    }
}

/// To and Cc entries, as the compose screen shows them.
fn recipients(
    original: &ParsedMessage,
    kind: ReplyKind,
    own_email: &str,
) -> (Vec<String>, Vec<String>) {
    let reply_to: &[Address] = if original.reply_to.is_empty() {
        &original.from
    } else {
        &original.reply_to
    };
    match kind {
        ReplyKind::Forward => (Vec::new(), Vec::new()),
        ReplyKind::Reply => {
            let mut seen = HashSet::new();
            (entries(reply_to, &mut seen, None), Vec::new())
        }
        ReplyKind::ReplyAll => {
            let own = own_email.to_ascii_lowercase();
            let mut seen = HashSet::new();
            let to_list: Vec<Address> = reply_to.iter().chain(&original.to).cloned().collect();
            let to = entries(&to_list, &mut seen, Some(&own));
            let cc = entries(&original.cc, &mut seen, Some(&own));
            (to, cc)
        }
    }
}

/// Formats each address once (compared ignoring case), leaving out `own`.
fn entries(list: &[Address], seen: &mut HashSet<String>, own: Option<&str>) -> Vec<String> {
    list.iter()
        .filter(|a| !a.email.is_empty())
        .filter(|a| own != Some(a.email.to_ascii_lowercase().as_str()))
        .filter(|a| seen.insert(a.email.to_ascii_lowercase()))
        .map(format_entry)
        .collect()
}

/// `Name <email>`, quoting the name when it has characters that would be
/// read as address syntax (RFC 5322 §3.2.3), so the entry parses back to
/// the same address.
pub(crate) fn format_entry(a: &Address) -> String {
    let name = a
        .name
        .as_deref()
        .map(|n| {
            n.chars()
                .filter(|c| !c.is_control())
                .collect::<String>()
                .trim()
                .to_owned()
        })
        .filter(|n| !n.is_empty());
    match name {
        None => a.email.clone(),
        Some(n)
            if n.chars()
                .all(|c| c.is_alphanumeric() || " .-'_".contains(c)) =>
        {
            format!("{n} <{}>", a.email)
        }
        Some(n) => {
            let escaped = n.replace('\\', "\\\\").replace('"', "\\\"");
            format!("\"{escaped}\" <{}>", a.email)
        }
    }
}

/// `prefix` + `subject`, unless it already starts with one of `existing`
/// (ignoring case). Prefixes are never stacked or translated.
fn prefixed(subject: &str, prefix: &str, existing: &[&str]) -> String {
    let lower = subject.to_lowercase();
    if existing.iter().any(|p| lower.starts_with(p)) {
        subject.to_owned()
    } else {
        format!("{prefix}{subject}")
    }
}

/// The original's `References` (or its `In-Reply-To` if it has none) plus
/// its `Message-ID`, trimmed to the first and the newest 19 ids.
fn references(original: &ParsedMessage) -> Vec<String> {
    let mut ids: Vec<String> = if original.references.is_empty() {
        original.in_reply_to.clone()
    } else {
        original.references.clone()
    };
    if let Some(id) = &original.message_id {
        ids.push(id.clone());
    }
    let extra = ids.len().saturating_sub(MAX_REFERENCES);
    if extra > 0 {
        // Keep the first id (the thread's root) and drop the oldest after it.
        ids = ids
            .iter()
            .take(1)
            .chain(ids.iter().skip(1 + extra))
            .cloned()
            .collect();
    }
    ids
}

/// An empty line for the cursor, the quote header, then each line of
/// `text` prefixed `> `.
fn quoted(text: &str, quote_header: &str) -> String {
    let mut out = String::from("\n\n");
    out.push_str(quote_header);
    out.push('\n');
    for line in text.trim_end().lines() {
        if line.is_empty() {
            out.push_str(">\n");
        } else {
            out.push_str("> ");
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// An empty line for the cursor, the intro line, the original's headers,
/// then its text.
fn forwarded(original: &ParsedMessage, text: &str, intro: &str, stored_date: i64) -> String {
    let list = |addrs: &[Address]| {
        addrs
            .iter()
            .map(format_entry)
            .collect::<Vec<_>>()
            .join(", ")
    };
    let mut out = String::from("\n\n");
    out.push_str(intro);
    out.push('\n');
    out.push_str(&format!("From: {}\n", list(&original.from)));
    out.push_str(&format!(
        "Date: {}\n",
        format_date(original.date.unwrap_or(stored_date))
    ));
    out.push_str(&format!(
        "Subject: {}\n",
        original.subject.as_deref().unwrap_or("")
    ));
    out.push_str(&format!("To: {}\n", list(&original.to)));
    if !original.cc.is_empty() {
        out.push_str(&format!("Cc: {}\n", list(&original.cc)));
    }
    out.push('\n');
    out.push_str(text.trim_end());
    out.push('\n');
    out
}

#[cfg(test)]
mod tests;
