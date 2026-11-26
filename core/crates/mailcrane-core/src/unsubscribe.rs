//! One-tap unsubscribe (M4-10).
//!
//! [`unsubscribe_options`] reads a downloaded message's `List-Unsubscribe`
//! header. One-click (RFC 8058) is offered only when the sender asks for it
//! with an `https:` URI **and** the `Authentication-Results` header the
//! user's own provider added (the topmost one whose authserv-id belongs to
//! the same organisation as the `by` host of the topmost `Received`
//! header) shows `dkim=pass` for a domain aligned with the From domain; otherwise the app falls back to a prepared
//! email or the sender's web page. Nothing is sent without the user
//! confirming: [`one_click_unsubscribe`] is only called after that, and the
//! `mailto:` and web fallbacks are carried out by the user in the app.

mod http;

pub use mailcrane_mime::unsubscribe::MailtoTarget;

use mailcrane_mime::parse_message;
use mailcrane_mime::unsubscribe::{
    dkim_aligned, dkim_pass_domains, parse_list_unsubscribe, trusted_results_with,
};
use mailcrane_store::{MessageId, Store, StoreError};

/// Why unsubscribing didn't work.
#[derive(Debug, thiserror::Error)]
pub enum UnsubscribeError {
    /// One-click isn't offered for this message (any more).
    #[error("one-click unsubscribe is not offered")]
    NotOffered,
    /// The message or its body isn't stored.
    #[error("unknown message")]
    UnknownMessage,
    /// The sender's server answered with this status (a redirect counts as
    /// failure: redirects aren't followed).
    #[error("the server answered {0}")]
    Refused(u16),
    /// No answer in 15 seconds.
    #[error("timed out")]
    Timeout,
    /// The connection failed.
    #[error("network failure")]
    Network,
    /// The certificate didn't check out.
    #[error("TLS failure")]
    Tls,
    /// The local database failed.
    #[error("store error: {0}")]
    Store(#[from] StoreError),
}

/// What the message offers, best first.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UnsubscribeOptions {
    /// The sender's address, for "Unsubscribe from …?".
    pub sender: String,
    /// The `https:` URI to post to, when one-click is offered and the
    /// message is authenticated.
    pub one_click: Option<String>,
    /// An email to prepare (never sent without the user).
    pub mailto: Option<MailtoTarget>,
    /// A page to open in the browser after the user confirms.
    pub web: Option<String>,
}

impl UnsubscribeOptions {
    /// Whether there is anything to offer.
    pub fn is_empty(&self) -> bool {
        self.one_click.is_none() && self.mailto.is_none() && self.web.is_none()
    }
}

/// The unsubscribe options of a downloaded message, or `None` if it has
/// none (or its body isn't downloaded yet).
pub fn unsubscribe_options(
    store: &Store,
    message: MessageId,
) -> Result<Option<UnsubscribeOptions>, UnsubscribeError> {
    let Some(raw) = store.get_body(message)? else {
        return Ok(None);
    };
    let provider_id = store.provider_authserv_id_for_message(message)?;
    Ok(options_from_raw_with(&raw, provider_id.as_deref()))
}

/// [`unsubscribe_options`] for raw message bytes, for an account with no
/// provider authserv-id set.
pub fn options_from_raw(raw: &[u8]) -> Option<UnsubscribeOptions> {
    options_from_raw_with(raw, None)
}

/// [`options_from_raw`] for an account whose provider authserv-id is
/// `provider_authserv_id` (M5-08, see [`trusted_results_with`]).
pub fn options_from_raw_with(
    raw: &[u8],
    provider_authserv_id: Option<&str>,
) -> Option<UnsubscribeOptions> {
    let parsed = parse_message(raw).ok()?;
    let header = parsed.list_unsubscribe.as_deref()?;
    let list = parse_list_unsubscribe(header, parsed.list_unsubscribe_post.as_deref());
    if list.is_empty() {
        return None;
    }
    let sender = parsed
        .from
        .first()
        .map(|a| a.email.clone())
        .unwrap_or_default();
    let from_domain = sender.rsplit_once('@').map(|(_, d)| d).unwrap_or_default();
    let authenticated = trusted_results_with(&parsed.trace, provider_authserv_id)
        .map(dkim_pass_domains)
        .unwrap_or_default()
        .iter()
        .any(|d| dkim_aligned(d, from_domain));
    let first_https = list.https.first().cloned();
    let options = UnsubscribeOptions {
        sender,
        one_click: if list.one_click && authenticated {
            first_https.clone()
        } else {
            None
        },
        mailto: list.mailto.first().cloned(),
        web: first_https,
    };
    (!options.is_empty()).then_some(options)
}

/// Sends the RFC 8058 one-click request for `message`. Call only after the
/// user confirmed. `extra_roots` is for tests (an extra trusted root
/// certificate); the apps pass none.
pub async fn one_click_unsubscribe(
    store: &Store,
    message: MessageId,
    extra_roots: &[Vec<u8>],
) -> Result<(), UnsubscribeError> {
    let raw = store
        .get_body(message)?
        .ok_or(UnsubscribeError::UnknownMessage)?;
    let provider_id = store.provider_authserv_id_for_message(message)?;
    let target = options_from_raw_with(&raw, provider_id.as_deref())
        .and_then(|o| o.one_click)
        .ok_or(UnsubscribeError::NotOffered)?;
    post_one_click(&target, extra_roots).await
}

/// Posts to `target`: a 2xx is success, anything else (redirects included)
/// is failure.
pub async fn post_one_click(target: &str, extra_roots: &[Vec<u8>]) -> Result<(), UnsubscribeError> {
    let url = url::Url::parse(target).map_err(|_| UnsubscribeError::NotOffered)?;
    let status = http::post(&url, extra_roots).await?;
    tracing::debug!(status, "one-click unsubscribe answered");
    if (200..300).contains(&status) {
        Ok(())
    } else {
        Err(UnsubscribeError::Refused(status))
    }
}

#[cfg(test)]
mod tests;
